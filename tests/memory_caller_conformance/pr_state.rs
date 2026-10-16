use super::*;

/// store-split M6 (`store-split.allium`: `ReportPrStatus`,
/// `ReportPrUnreadable`). The in-memory caller derives the task's PR state from
/// the host's raw `gh` strings the way the real module does: merged, closed,
/// open with a review decision, a conflict left alone, an unknown string
/// refused in the same words and the unreachable mark — on the same calls,
/// with the same outcome and the same task on both sides.
#[test]
fn memory_caller_derives_pr_state_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-pr-state");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let connector = Arc::new(SpacetimeSdkConnector::new(
            instance.database(),
            rows_real.clone(),
        ));
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone());
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        let identity = accepted.identity.clone();

        let mut woken = rows_real.changed();
        macro_rules! wait_for {
            ($cond:expr) => {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while !$cond {
                        woken.changed().await.expect("subscription must deliver");
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("real store never reached the expected state"))
            };
        }
        let same_task = |id: TaskId| {
            assert_eq!(
                rows_real.task(id).map(|t| task_shape(&t)),
                rows_mem.task(id).map(|t| task_shape(&t)),
                "task {id:?}"
            );
        };

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let create = |sub_status: &str| {
            let row = bindings::Task {
                created_by: identity.clone(),
                owner: identity.clone(),
                status: "review".into(),
                sub_status: sub_status.into(),
                url: "https://github.com/acme/repo/pull/7".into(),
                url_type: "pr".into(),
                tmux_window: "task-1".into(),
                ..blank_task_in_epic(0)
            };
            let real = &real;
            let mem = &mem;
            async move {
                let r = real.create_task(row.clone()).await.unwrap();
                let m = mem.create_task(row).await.unwrap();
                assert_eq!(r, m, "both sides hand out the same task id");
                r
            }
        };
        // Both sides, one report, one outcome.
        let report = |task: TaskId, state: &'static str, decision: &'static str| {
            let real = &real;
            let mem = &mem;
            async move {
                let r = real
                    .report_pr_status(task, "host-a".into(), state.into(), decision.into())
                    .await
                    .unwrap();
                let m = mem
                    .report_pr_status(task, "host-a".into(), state.into(), decision.into())
                    .await
                    .unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(_), ReducerOutcome::Applied(_)) => {}
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words")
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }
        };

        let task = create("awaiting_review").await;
        wait_for!(rows_real.task(task).is_some());
        same_task(task);

        report(task, "OPEN", "APPROVED").await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::Approved));
        same_task(task);

        // An unknown string is refused in the same words and writes nothing.
        report(task, "DRAFT", "").await;
        report(task, "OPEN", "MAYBE").await;
        same_task(task);

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            assert!(caller
                .report_pr_unreadable(task, "host-a".into(), "no access".into())
                .await
                .unwrap()
                .won());
        }
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::PrUnreachable));
        same_task(task);

        report(task, "CLOSED", "").await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::PrClosed));
        same_task(task);

        report(task, "merged", "").await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.status == dispatch_tui::models::TaskStatus::Done));
        same_task(task);

        // A conflict survives an open report; a closed one overrides it.
        let conflicted = create("conflict").await;
        wait_for!(rows_real.task(conflicted).is_some());
        report(conflicted, "OPEN", "APPROVED").await;
        same_task(conflicted);
        assert_eq!(
            rows_real.task(conflicted).unwrap().sub_status,
            dispatch_tui::models::SubStatus::Conflict
        );
    });
}
