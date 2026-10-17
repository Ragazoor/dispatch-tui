use super::*;

/// store-split M7 (`store-split.allium`: `RequestCloseTask`, X2 and X4). The
/// in-memory caller closes a task the way the real module does: the open
/// session request failed "task closed", the Done write (a re-close keeping
/// its completion), `stop_pending` cleared, the epic rolled up, a phoenix task
/// respawned and a `close` row written only where there is host work; a
/// patch moving a task out of running clears `stop_pending` and one moving a
/// phoenix task into Done respawns it; `ensure_managed_epics` builds, keeps
/// and reconciles the managed tree — on the same calls, with the same outcome
/// and the same rows on both sides.
#[test]
fn memory_caller_closes_tasks_and_provisions_managed_epics_like_the_real_module() {
    use dispatch_tui::models::SessionRequestId;

    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-close-task");
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
        let same_epic = |id: EpicId| {
            assert_eq!(
                rows_real
                    .epic(id)
                    .map(|e| (e.status, e.completed_at.is_some())),
                rows_mem
                    .epic(id)
                    .map(|e| (e.status, e.completed_at.is_some())),
                "epic {id:?}"
            );
        };
        // The close rows of `task` on each side, as shapes.
        let close_shapes = |task: TaskId| {
            let pick = |rows: &SharedRows| {
                rows.session_requests()
                    .into_iter()
                    .filter(|r| {
                        r.task_id == task
                            && r.kind == dispatch_tui::models::SessionRequestKind::Close
                    })
                    .map(|r| session_shape(&r))
                    .collect::<Vec<_>>()
            };
            (pick(&rows_real), pick(&rows_mem))
        };
        let both_applied = |r: ReducerOutcome, m: ReducerOutcome| {
            assert!(r.won(), "the real module refused: {r:?}");
            assert!(m.won(), "the in-memory caller refused: {m:?}");
        };

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let create = |row: bindings::Task| {
            let real = &real;
            let mem = &mem;
            async move {
                let r = real.create_task(row.clone()).await.unwrap();
                let m = mem.create_task(row).await.unwrap();
                assert_eq!(r, m, "both sides hand out the same task id");
                r
            }
        };
        let in_epic = |epic_id: i64| bindings::Task {
            created_by: identity.clone(),
            owner: if epic_id == 0 {
                identity.clone()
            } else {
                String::new()
            },
            ..blank_task_in_epic(epic_id)
        };
        let patch = |task: TaskId, patch: bindings::TaskPatch| {
            let real = &real;
            let mem = &mem;
            async move {
                for caller in [real as &dyn ReducerCaller, mem as &dyn ReducerCaller] {
                    caller
                        .patch_task(task, patch.clone())
                        .await
                        .unwrap()
                        .applied()
                        .unwrap();
                }
            }
        };
        let running_on_host_a = |task: TaskId| bindings::TaskPatch {
            status: Some("running".into()),
            sub_status: Some("active".into()),
            worktree: Some(format!("/repo/.worktrees/{}-t", task.0)),
            host: Some("host-a".into()),
            ..blank_task_patch()
        };
        // The window is the current agent's since agents A4, and an agent comes
        // from the backfill (a worktree and none): so a task is patched
        // running, backfilled on both sides, and only then given its window.
        let running_with_window = |task: TaskId| {
            let patch = &patch;
            let instance = &instance;
            let mem = &mem;
            async move {
                patch(task, running_on_host_a(task)).await;
                let backfilled = instance.call("backfill_agents", &[]);
                assert!(backfilled.status.success(), "{}", describe(&backfilled));
                mem.backfill_agents().unwrap().applied().unwrap();
                patch(
                    task,
                    bindings::TaskPatch {
                        tmux_window: Some(format!("task-{}", task.0)),
                        ..blank_task_patch()
                    },
                )
                .await;
            }
        };

        // -- A refusal: no such task --------------------------------------------------
        match (
            real.close_task(TaskId(999), "host-a".into()).await.unwrap(),
            mem.close_task(TaskId(999), "host-a".into()).await.unwrap(),
        ) {
            (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => assert_eq!(r, m),
            other => panic!("both sides must refuse, got {other:?}"),
        }

        // -- The full close: stuck wrap_up, stop_pending, epic rollup, phoenix ---------
        let epic = {
            let row = bindings::Epic {
                created_by: identity.clone(),
                ..blank_epic()
            };
            let r = real.create_epic(row.clone()).await.unwrap();
            let m = mem.create_epic(row).await.unwrap();
            assert_eq!(r, m);
            r
        };
        let task = create(bindings::Task {
            phoenix: true,
            ..in_epic(epic.0)
        })
        .await;
        wait_for!(rows_real.task(task).is_some());
        running_with_window(task).await;
        patch(
            task,
            bindings::TaskPatch {
                stop_pending: Some(true),
                ..blank_task_patch()
            },
        )
        .await;
        let r_stuck = match real
            .request_wrap_up(task, "host-a".into(), "rebase".into())
            .await
            .unwrap()
        {
            ReducerOutcome::Applied(ids) => SessionRequestId(ids[0]),
            other => panic!("{other:?}"),
        };
        let m_stuck = match mem
            .request_wrap_up(task, "host-a".into(), "rebase".into())
            .await
            .unwrap()
        {
            ReducerOutcome::Applied(ids) => SessionRequestId(ids[0]),
            other => panic!("{other:?}"),
        };
        wait_for!(rows_real.session_request(r_stuck).is_some());

        both_applied(
            real.close_task(task, "host-a".into()).await.unwrap(),
            mem.close_task(task, "host-a".into()).await.unwrap(),
        );
        wait_for!(rows_real
            .session_request(r_stuck)
            .is_some_and(|r| r.reason.as_deref() == Some("task closed")));
        wait_for!(!close_shapes(task).0.is_empty());
        assert_eq!(
            rows_real
                .session_request(r_stuck)
                .map(|r| session_shape(&r)),
            rows_mem.session_request(m_stuck).map(|r| session_shape(&r)),
            "the stuck wrap_up is failed the same way"
        );
        let (real_close, mem_close) = close_shapes(task);
        assert_eq!(real_close.len(), 1);
        assert_eq!(real_close, mem_close);
        same_task(task);
        same_epic(epic);
        assert_eq!(
            rows_real.tasks().len(),
            rows_mem.tasks().len(),
            "the phoenix successor landed on both sides"
        );

        // -- A re-close of a done task: no transition ---------------------------------
        both_applied(
            real.close_task(task, "host-a".into()).await.unwrap(),
            mem.close_task(task, "host-a".into()).await.unwrap(),
        );
        same_task(task);
        assert_eq!(
            rows_real.tasks().len(),
            rows_mem.tasks().len(),
            "a done -> done close respawns nothing"
        );

        // -- No host work: no row ---------------------------------------------------------
        let bare = create(in_epic(0)).await;
        wait_for!(rows_real.task(bare).is_some());
        both_applied(
            real.close_task(bare, "host-a".into()).await.unwrap(),
            mem.close_task(bare, "host-a".into()).await.unwrap(),
        );
        wait_for!(rows_real
            .task(bare)
            .is_some_and(|t| t.status == TaskStatus::Done));
        same_task(bare);
        assert_eq!(close_shapes(bare), (vec![], vec![]));

        // -- A pr-typed url alone is host work, on the argument host ----------------------
        let linked = create(in_epic(0)).await;
        wait_for!(rows_real.task(linked).is_some());
        patch(
            linked,
            bindings::TaskPatch {
                url: Some("https://github.com/o/r/pull/9".into()),
                url_type: Some("pr".into()),
                ..blank_task_patch()
            },
        )
        .await;
        both_applied(
            real.close_task(linked, "host-a".into()).await.unwrap(),
            mem.close_task(linked, "host-a".into()).await.unwrap(),
        );
        wait_for!(!close_shapes(linked).0.is_empty());
        let (real_close, mem_close) = close_shapes(linked);
        assert_eq!(real_close, mem_close);
        assert_eq!(real_close[0].tmux_window, None);
        assert_eq!(real_close[0].host, "host-a");

        // -- patch_task: out of running clears stop_pending; into Done respawns ------------
        let leaving = create(in_epic(0)).await;
        wait_for!(rows_real.task(leaving).is_some());
        running_with_window(leaving).await;
        patch(
            leaving,
            bindings::TaskPatch {
                stop_pending: Some(true),
                ..blank_task_patch()
            },
        )
        .await;
        patch(
            leaving,
            bindings::TaskPatch {
                status: Some("review".into()),
                sub_status: Some("awaiting_review".into()),
                ..blank_task_patch()
            },
        )
        .await;
        wait_for!(rows_real
            .task(leaving)
            .is_some_and(|t| t.status == TaskStatus::Review));
        same_task(leaving);
        assert!(!rows_real.task(leaving).unwrap().stop_pending);

        let recurring = create(bindings::Task {
            phoenix: true,
            ..in_epic(0)
        })
        .await;
        wait_for!(rows_real.task(recurring).is_some());
        let before = rows_real.tasks().len();
        patch(
            recurring,
            bindings::TaskPatch {
                status: Some("done".into()),
                sub_status: Some("none".into()),
                ..blank_task_patch()
            },
        )
        .await;
        wait_for!(rows_real.tasks().len() == before + 1);
        same_task(recurring);
        assert_eq!(rows_mem.tasks().len(), before + 1);

        // -- ensure_managed_epics -------------------------------------------------------------
        let managed = |rows: &SharedRows| {
            let mut epics: Vec<_> = rows
                .epics()
                .into_iter()
                .filter(|e| e.feed_role != dispatch_tui::models::FeedRole::None)
                .map(|e| {
                    (
                        e.title,
                        e.feed_role,
                        e.parent_epic_id.is_some(),
                        e.feed_command,
                        e.feed_interval_secs,
                    )
                })
                .collect();
            epics.sort_by_key(|e| e.1.as_str().to_string());
            epics
        };
        for (reviews, reviews_secs, cve, cve_secs) in [
            ("reviews.sh", 600, "", 0),
            ("reviews.sh", 600, "cve.sh", 900),
            ("reviews2.sh", 0, "cve.sh", 1200),
        ] {
            both_applied(
                real.ensure_managed_epics(reviews.into(), reviews_secs, cve.into(), cve_secs)
                    .await
                    .unwrap(),
                mem.ensure_managed_epics(reviews.into(), reviews_secs, cve.into(), cve_secs)
                    .await
                    .unwrap(),
            );
            let expected = rows_mem.epics().len();
            wait_for!(rows_real.epics().len() == expected);
            // A reconcile writes no new row, so wait for the command to land.
            let want = reviews.to_string();
            wait_for!(rows_real.epics().iter().any(|e| {
                e.feed_role == dispatch_tui::models::FeedRole::ReviewsParent
                    && e.feed_command.as_deref() == Some(want.as_str())
            }));
            assert_eq!(managed(&rows_real), managed(&rows_mem));
        }
        assert_eq!(managed(&rows_real).len(), 5);
    });
}
