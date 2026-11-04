use super::*;

/// `tasks.allium: SetBookmark` and `BookmarkEndsWithTheTaskFinishing`. The
/// in-memory caller sets, repeats and clears a bookmark, refuses a Done or
/// missing task in the store's words, and ends the bookmark when a merged PR
/// moves the task to Done (a write path other than `patch_task`) — with the
/// same outcome and the same rows on both sides.
#[test]
fn memory_caller_keeps_bookmarks_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-bookmarks");
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
            .with_sender(accepted.identity.clone())
            .with_workspace_rules();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        join_default_workspace(&real, &mem, &accepted.identity).await;
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

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let row = bindings::Task {
            created_by: identity.clone(),
            owner: identity.clone(),
            status: "review".into(),
            sub_status: "awaiting_review".into(),
            url: "https://github.com/acme/repo/pull/7".into(),
            url_type: "pr".into(),
            ..blank_task_in_epic(0)
        };
        let task = real.create_task(row.clone()).await.unwrap();
        assert_eq!(task, mem.create_task(row).await.unwrap());
        wait_for!(rows_real.task(task).is_some());

        // Both sides, one call, one outcome.
        let set = |id: TaskId, on: bool| {
            let real = &real;
            let mem = &mem;
            let identity = identity.clone();
            async move {
                let r = real.set_bookmark(identity.clone(), id, on).await.unwrap();
                let m = mem.set_bookmark(identity, id, on).await.unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(_), ReducerOutcome::Applied(_)) => None,
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words");
                        Some(r)
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }
        };

        assert_eq!(set(task, true).await, None);
        assert_eq!(set(task, true).await, None, "setting twice is not an error");
        assert_eq!(rows_mem.bookmarked_task_ids(), vec![task]);
        wait_for!(rows_real.bookmarked_task_ids() == vec![task]);

        assert_eq!(set(task, false).await, None);
        assert!(rows_mem.bookmarked_task_ids().is_empty());
        wait_for!(rows_real.bookmarked_task_ids().is_empty());

        assert!(set(TaskId(9_999_999), true).await.is_some(), "missing task");

        // A merged PR moves the task to Done outside patch_task: the bookmark
        // still ends.
        assert_eq!(set(task, true).await, None);
        wait_for!(rows_real.bookmarked_task_ids() == vec![task]);
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .report_pr_status(task, "host-a".into(), "MERGED".into(), String::new())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        assert_eq!(
            rows_mem.task(task).map(|t| t.status.as_str().to_string()),
            Some("done".to_string())
        );
        assert!(rows_mem.bookmarked_task_ids().is_empty());
        wait_for!(rows_real.bookmarked_task_ids().is_empty());

        let refused = set(task, true).await.expect("a done task is refused");
        assert!(refused.contains("done"), "{refused}");
        assert_eq!(set(task, false).await, None, "removing is never refused");
    });
}
