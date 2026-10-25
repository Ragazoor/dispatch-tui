use super::*;

/// The cron every scenario below arms: midnight on New Year's Day, UTC. Its
/// next match is the same instant for both sides' clocks unless the suite
/// runs across a new year, so `next_due` compares exactly.
const YEARLY: &str = "0 0 1 1 *";

/// `schedules.allium` (task #98394). The in-memory caller arms, re-arms and
/// clears a task's schedule the way the real module does — on create, on a
/// patch, and on a phoenix respawn — refuses the same crons and the same
/// non-backlog one-off, and refuses a schedule-origin request for a task that
/// is not due.
///
/// What needs a due task — a schedule-origin request accepted, a dispatched
/// report consuming `next_due`, a failed launch marking the task — cannot be
/// reached here without waiting for a real minute: the store computes
/// `next_due` from its own clock and a client cannot plant one. Those are
/// pinned by `spacetime/module-tests/tests/schedules.rs`, which moves the
/// module's `TestClock`, and on the double by
/// `src/sync/memory_caller/tests/schedules.rs`, with a fixed clock.
#[test]
fn memory_caller_keeps_schedules_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-schedules");
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
        let both_refuse = |r: ReducerOutcome, m: ReducerOutcome| match (r, m) {
            (ReducerOutcome::Refused(_), ReducerOutcome::Refused(_)) => {}
            other => panic!("both sides must refuse, got {other:?}"),
        };

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let mine = |schedule: &str| bindings::Task {
            created_by: identity.clone(),
            owner: identity.clone(),
            schedule: schedule.into(),
            ..blank_task_in_epic(0)
        };
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
        let patch_both = |task: TaskId, patch: bindings::TaskPatch| {
            let real = &real;
            let mem = &mem;
            async move {
                (
                    real.patch_task(task, patch.clone()).await.unwrap(),
                    mem.patch_task(task, patch).await.unwrap(),
                )
            }
        };
        let schedule = |cron: &str| bindings::TaskPatch {
            schedule: Some(cron.into()),
            ..blank_task_patch()
        };

        // -- SetSchedule on create -----------------------------------------------
        let armed = create(mine(YEARLY)).await;
        wait_for!(rows_real.task(armed).is_some());
        same_task(armed);
        assert!(rows_real.task(armed).unwrap().next_due.is_some());

        // A client cannot plant its own next_due or error.
        let planted = create(bindings::Task {
            next_due: "2000-01-01 00:00:00.000".into(),
            schedule_error: "planted".into(),
            ..mine("")
        })
        .await;
        wait_for!(rows_real.task(planted).is_some());
        same_task(planted);
        assert_eq!(rows_real.task(planted).unwrap().next_due, None);

        // An invalid cron creates nothing, on either side.
        let before = rows_real.tasks().len();
        assert!(real.create_task(mine("every monday")).await.is_err());
        assert!(mem.create_task(mine("every monday")).await.is_err());
        assert_eq!(rows_mem.tasks().len(), before);

        // -- SetSchedule / ClearSchedule on a patch --------------------------------
        let (r, m) = patch_both(planted, schedule(YEARLY)).await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(planted)
            .is_some_and(|t| t.schedule.is_some()));
        same_task(planted);

        let (r, m) = patch_both(planted, schedule("")).await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(planted)
            .is_some_and(|t| t.schedule.is_none()));
        same_task(planted);

        let (r, m) = patch_both(planted, schedule("0 0 31 2 *")).await;
        both_refuse(r, m);

        // A one-off outside backlog is refused; a phoenix one is stored unarmed.
        let reviewed = create(mine("")).await;
        wait_for!(rows_real.task(reviewed).is_some());
        let to_review = bindings::TaskPatch {
            status: Some("review".into()),
            sub_status: Some("awaiting_review".into()),
            ..blank_task_patch()
        };
        let (r, m) = patch_both(reviewed, to_review.clone()).await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        let (r, m) = patch_both(reviewed, schedule(YEARLY)).await;
        both_refuse(r, m);
        let (r, m) = patch_both(
            reviewed,
            bindings::TaskPatch {
                phoenix: Some(true),
                schedule: Some(YEARLY.into()),
                ..blank_task_patch()
            },
        )
        .await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real.task(reviewed).is_some_and(|t| t.phoenix));
        same_task(reviewed);
        assert_eq!(rows_real.task(reviewed).unwrap().next_due, None);

        // -- PhoenixRespawn carries the schedule -----------------------------------
        let before = rows_real.tasks().len();
        let (r, m) = patch_both(
            reviewed,
            bindings::TaskPatch {
                status: Some("done".into()),
                sub_status: Some("none".into()),
                ..blank_task_patch()
            },
        )
        .await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real.tasks().len() == before + 1);
        // `created_at` is each side's own clock at the respawn, so it is the
        // one field left out.
        let successor = |rows: &SharedRows| {
            rows.tasks()
                .into_iter()
                .max_by_key(|t| t.id.0)
                .map(|t| TaskShape {
                    created_at: chrono::DateTime::<chrono::Utc>::MIN_UTC,
                    ..task_shape(&t)
                })
        };
        assert_eq!(successor(&rows_real), successor(&rows_mem));
        let landed = successor(&rows_real).unwrap();
        assert_eq!(landed.schedule.as_deref(), Some(YEARLY));
        assert!(landed.next_due.is_some(), "the successor is armed");

        // -- RequestDispatch: a schedule-origin request needs a due task -----------
        both_refuse(
            real.request_dispatch(armed, "host-a".into(), "schedule".into(), String::new())
                .await
                .unwrap(),
            mem.request_dispatch(armed, "host-a".into(), "schedule".into(), String::new())
                .await
                .unwrap(),
        );
        same_task(armed);

        // -- Review 2026-10-23 (task #98505) ---------------------------------------
        // A refused cron in an auto-grouping epic leaves no repo sub-epic.
        let grouped = bindings::Epic {
            group_by_repo: true,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root = real.create_epic(grouped.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root)).is_some());
        assert_eq!(mem.create_epic(grouped).await.unwrap().0, root);
        let epics_before = rows_mem.epics().len();
        let never = bindings::Task {
            repo_path: "/x/alpha".into(),
            epic_id: root,
            ..mine("0 0 31 2 *")
        };
        assert!(real.create_task(never.clone()).await.is_err());
        assert!(mem.create_task(never).await.is_err());
        assert_eq!(
            rows_mem.epics().len(),
            epics_before,
            "no sub-epic left behind"
        );
        assert_eq!(rows_real.epics().len(), rows_mem.epics().len());

        // EnteringBacklogRearmsAPhoenixTask: out of backlog drops the run, a hand
        // move back re-arms it.
        let phoenix = create(bindings::Task {
            phoenix: true,
            ..mine(YEARLY)
        })
        .await;
        wait_for!(rows_real.task(phoenix).is_some());
        let (r, m) = patch_both(phoenix, to_review.clone()).await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(phoenix)
            .is_some_and(|t| t.next_due.is_none()));
        same_task(phoenix);
        let to_backlog = bindings::TaskPatch {
            status: Some("backlog".into()),
            sub_status: Some("none".into()),
            ..blank_task_patch()
        };
        let (r, m) = patch_both(phoenix, to_backlog.clone()).await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(phoenix)
            .is_some_and(|t| t.next_due.is_some()));
        same_task(phoenix);

        // ClosingCancelsThePendingRun: a close while a user-origin request holds
        // the claim spends the one-off, so a move back to backlog does not fire.
        let claimed = create(bindings::Task {
            repo_path: "/x/alpha".into(),
            ..mine(YEARLY)
        })
        .await;
        wait_for!(rows_real.task(claimed).is_some());
        let (r, m) = (
            real.request_dispatch(claimed, "host-a".into(), "user".into(), String::new())
                .await
                .unwrap(),
            mem.request_dispatch(claimed, "host-a".into(), "user".into(), String::new())
                .await
                .unwrap(),
        );
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(claimed)
            .is_some_and(|t| t.status == TaskStatus::Running));
        same_task(claimed);
        assert!(
            rows_real.task(claimed).unwrap().next_due.is_some(),
            "the claim keeps it"
        );
        let (r, m) = (
            real.close_task(claimed, "host-a".into()).await.unwrap(),
            mem.close_task(claimed, "host-a".into()).await.unwrap(),
        );
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(claimed)
            .is_some_and(|t| t.next_due.is_none()));
        same_task(claimed);
        let (r, m) = patch_both(claimed, to_backlog).await;
        assert!(r.won() && m.won(), "{r:?} / {m:?}");
        wait_for!(rows_real
            .task(claimed)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        same_task(claimed);
        assert_eq!(
            rows_real.task(claimed).unwrap().next_due,
            None,
            "a one-off is spent"
        );
    });
}
