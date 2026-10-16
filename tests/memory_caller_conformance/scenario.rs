use super::*;

/// The orchestration-fidelity scenario: create two epics and a task inside
/// the first, claim and release the task (including the worktree-attached
/// refusal), patch the epic, move the task to the second epic and back
/// (`set_task_epic`, exercising the epic-chain recalculation on both the
/// source and destination), round-trip repo config, round-trip a
/// subscription, then delete the task and both epics — asserting after every
/// step that the real store and `MemoryReducerCaller` agree.
#[test]
fn memory_caller_matches_the_real_reducers() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-conformance");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let mem = MemoryReducerCaller::new(rows_mem.clone(), clock);

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
        // `owner_board` must be non-empty hex (it becomes a SQL literal
        // matched against `owner`/`created_by`); this scenario never relies on
        // those two subscriptions, so any valid-looking hex string does.
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![1, 2],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        // The host-naming reducers below act for "host-a" and "host-b", which
        // D11 lets only their owner do (`store-split.allium`): register both
        // as this connection's own, on both sides.
        let mem = mem.with_sender(accepted.identity.clone());
        for host in ["host-a", "host-b"] {
            for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
                caller
                    .register_host(host.into(), String::new(), accepted.identity.clone())
                    .await
                    .unwrap()
                    .applied()
                    .unwrap();
            }
        }

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

        let compare_task = |id: i64| {
            assert_eq!(
                rows_real.task(TaskId(id)).map(|t| task_shape(&t)),
                rows_mem.task(TaskId(id)).map(|t| task_shape(&t)),
                "task {id}"
            );
        };
        let compare_epic = |id: i64| {
            assert_eq!(
                rows_real.epic(EpicId(id)).map(|e| epic_shape(&e)),
                rows_mem.epic(EpicId(id)).map(|e| epic_shape(&e)),
                "epic {id}"
            );
        };
        // `owner` is a bindings/DB-only column `crate::models::Task` does not
        // carry (see `TaskShape`'s doc comment), so this reads it directly:
        // over SQL on the real side, off `MemoryReducerCaller`'s own table on
        // the fake side. The one field `set_task_epic` mutates natively that
        // `compare_task` cannot see.
        let compare_owner = |id: i64| {
            assert_eq!(
                column(
                    &instance,
                    &format!("SELECT owner FROM tasks WHERE id = {id}")
                ),
                mem.task_owner(id).unwrap_or_default(),
                "task {id} owner"
            );
        };
        let compare_learning = |id: LearningId| {
            assert_eq!(
                rows_real.learning(id).as_ref().map(learning_shape),
                rows_mem.learning(id).as_ref().map(learning_shape),
                "learning {id:?}"
            );
        };

        // -- create_epic (two: the task's home, and a destination to move it to) --
        let epic_id_real = real.create_epic(blank_epic()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(epic_id_real)).is_some());
        let epic_id_mem = mem.create_epic(blank_epic()).await.unwrap().0;
        assert_eq!(epic_id_real, epic_id_mem, "generated epic id");
        compare_epic(epic_id_real);

        let epic2_id_real = real.create_epic(blank_epic()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(epic2_id_real)).is_some());
        let epic2_id_mem = mem.create_epic(blank_epic()).await.unwrap().0;
        assert_eq!(epic2_id_real, epic2_id_mem, "generated second epic id");
        compare_epic(epic2_id_real);

        // -- create_task (in the epic, backlog) ----------------------------------
        let task_id_real = real
            .create_task(blank_task_in_epic(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.task(task_id_real).is_some());
        let task_id_mem = mem
            .create_task(blank_task_in_epic(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(task_id_real, task_id_mem, "generated task id");
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);

        // -- patch_epic -------------------------------------------------------------
        real.patch_epic(
            EpicId(epic_id_real),
            bindings::EpicPatch {
                title: Some("renamed conformance epic".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .epic(EpicId(epic_id_real))
            .is_some_and(|e| e.title == "renamed conformance epic"));
        mem.patch_epic(
            EpicId(epic_id_mem),
            bindings::EpicPatch {
                title: Some("renamed conformance epic".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
        compare_epic(epic_id_real);

        // -- set_task_epic: move the (backlog) task to the second epic --------------
        real.set_task_epic(task_id_real, Some(EpicId(epic2_id_real)), String::new())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.epic_id == Some(EpicId(epic2_id_real))));
        mem.set_task_epic(task_id_mem, Some(EpicId(epic2_id_mem)), String::new())
            .await
            .unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);
        compare_epic(epic2_id_real);
        compare_owner(task_id_real.0);

        // Move it back, so the delete section below only has one epic's worth
        // of tasks to clean up.
        real.set_task_epic(task_id_real, Some(EpicId(epic_id_real)), String::new())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.epic_id == Some(EpicId(epic_id_real))));
        mem.set_task_epic(task_id_mem, Some(EpicId(epic_id_mem)), String::new())
            .await
            .unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);
        compare_epic(epic2_id_real);
        compare_owner(task_id_real.0);

        // -- repo configuration ---------------------------------------------------
        real.save_repo_path("/repo".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        wait_for!(!rows_real.repo_paths().is_empty());
        mem.save_repo_path("/repo".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        assert_eq!(rows_real.repo_paths(), rows_mem.repo_paths(), "repo_paths");

        real.set_verify_command("/repo".into(), "cargo test".into())
            .await
            .unwrap();
        wait_for!(rows_real.verify_command("/repo").as_deref() == Some("cargo test"));
        mem.set_verify_command("/repo".into(), "cargo test".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.verify_command("/repo"),
            rows_mem.verify_command("/repo"),
            "verify_command"
        );

        real.record_base_branch("/repo".into(), "main".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        wait_for!(!rows_real.base_branches().is_empty());
        mem.record_base_branch("/repo".into(), "main".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        assert_eq!(
            rows_real.base_branches(),
            rows_mem.base_branches(),
            "base_branches"
        );

        real.delete_repo_path("/repo".into()).await.unwrap();
        wait_for!(rows_real.repo_paths().is_empty());
        mem.delete_repo_path("/repo".into()).await.unwrap();
        assert!(rows_mem.repo_paths().is_empty());

        // -- subscriptions ----------------------------------------------------------
        // The connection's OWN identity, not an arbitrary hex string: this
        // connection's subscription asks for `subscriptions WHERE subscriber =
        // '{accepted.identity}'` (`subscription_queries`), so a row filed
        // under any other subscriber would never arrive here to compare.
        let subscriber = accepted.identity.as_str();
        real.subscribe_to_epic(subscriber.into(), EpicId(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.subscribed_epics(subscriber) == vec![EpicId(epic_id_real)]);
        mem.subscribe_to_epic(subscriber.into(), EpicId(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(
            rows_real.subscribed_epics(subscriber),
            rows_mem.subscribed_epics(subscriber),
            "subscribed_epics after subscribe"
        );

        real.unsubscribe_from_epic(subscriber.into(), EpicId(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.subscribed_epics(subscriber).is_empty());
        mem.unsubscribe_from_epic(subscriber.into(), EpicId(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(
            rows_real.subscribed_epics(subscriber),
            rows_mem.subscribed_epics(subscriber),
            "subscribed_epics after unsubscribe"
        );

        // -- settings ---------------------------------------------------------------
        // Scoped by HOST (`docs/specs/settings.allium`), not by owner — "host-a"
        // is the same host this scenario's subscription already asks
        // `settings WHERE host = 'host-a'` for (`subscription_queries`), so a
        // save lands in `rows_real` without a second subscribe.
        real.save_setting("host-a".into(), "theme".into(), "dark".into())
            .await
            .unwrap();
        wait_for!(rows_real.setting("theme").as_deref() == Some("dark"));
        mem.save_setting("host-a".into(), "theme".into(), "dark".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.setting("theme"),
            rows_mem.setting("theme"),
            "setting after save"
        );

        // A second save of the same (host, key) upserts rather than adding a
        // row (`SaveSetting`'s `@guidance`).
        real.save_setting("host-a".into(), "theme".into(), "light".into())
            .await
            .unwrap();
        wait_for!(rows_real.setting("theme").as_deref() == Some("light"));
        mem.save_setting("host-a".into(), "theme".into(), "light".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.setting("theme"),
            rows_mem.setting("theme"),
            "setting after re-save"
        );

        // An empty host is refused on both sides, and the value already saved
        // above is left untouched.
        let real_empty_host = real
            .save_setting(String::new(), "theme".into(), "purple".into())
            .await
            .unwrap();
        let mem_empty_host = mem
            .save_setting(String::new(), "theme".into(), "purple".into())
            .await
            .unwrap();
        assert!(!real_empty_host.won(), "real must refuse an empty host");
        assert!(!mem_empty_host.won(), "mem must refuse an empty host");
        assert_eq!(
            rows_real.setting("theme"),
            rows_mem.setting("theme"),
            "setting after refused save"
        );

        // A host this connection does not own is refused the same way, for a
        // save, a clear and both poll-owner reducers (D11, store-split.allium).
        let real_foreign = [
            real.save_setting("not-mine".into(), "theme".into(), "x".into())
                .await
                .unwrap(),
            real.clear_setting("not-mine".into(), "theme".into())
                .await
                .unwrap(),
            real.claim_poll_owner(PollScopeId::Epic(EpicId(1)), "not-mine".into())
                .await
                .unwrap(),
            real.override_poll_owner(PollScopeId::Epic(EpicId(1)), "not-mine".into())
                .await
                .unwrap(),
        ];
        let mem_foreign = [
            mem.save_setting("not-mine".into(), "theme".into(), "x".into())
                .await
                .unwrap(),
            mem.clear_setting("not-mine".into(), "theme".into())
                .await
                .unwrap(),
            mem.claim_poll_owner(PollScopeId::Epic(EpicId(1)), "not-mine".into())
                .await
                .unwrap(),
            mem.override_poll_owner(PollScopeId::Epic(EpicId(1)), "not-mine".into())
                .await
                .unwrap(),
        ];
        for (real_out, mem_out) in real_foreign.iter().zip(&mem_foreign) {
            assert!(!real_out.won(), "real must refuse a host it does not own");
            assert!(!mem_out.won(), "mem must refuse a host it does not own");
        }
        assert_eq!(rows_real.setting("theme"), rows_mem.setting("theme"));

        // Clearing a key that was never set is a no-op, not a refusal
        // (`ClearSetting`'s `@guidance`).
        let real_noop_clear = real
            .clear_setting("host-a".into(), "never-set".into())
            .await
            .unwrap();
        let mem_noop_clear = mem
            .clear_setting("host-a".into(), "never-set".into())
            .await
            .unwrap();
        assert!(real_noop_clear.won(), "clearing an absent key is a no-op");
        assert!(mem_noop_clear.won(), "clearing an absent key is a no-op");

        real.clear_setting("host-a".into(), "theme".into())
            .await
            .unwrap();
        wait_for!(rows_real.setting("theme").is_none());
        mem.clear_setting("host-a".into(), "theme".into())
            .await
            .unwrap();
        assert!(rows_mem.setting("theme").is_none());

        // Reused below and by the final delete section: task #4971's
        // `delete_task`/`batch_delete` `requires: task.status = done` guard
        // means every task this scenario deletes must be marked done first.
        let mark_done = || bindings::TaskPatch {
            status: Some("done".into()),
            ..blank_task_patch()
        };

        // -- usage events (usage) ---------------------------------------------------
        let cap = 3i64;
        let usage_event = |actor: &str| bindings::UsageEvent {
            id: 0,
            recorded_at: "2026-01-01 00:00:00.000".into(),
            category: "tool".into(),
            action: "used".into(),
            detail: None,
            actor: actor.into(),
        };
        for _ in 0..5 {
            real.record_usage_event(usage_event("conformance"), cap)
                .await
                .unwrap();
        }
        wait_for!(
            rows_real
                .usage_summary(&dispatch_tui::store::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>()
                == cap
        );
        for _ in 0..5 {
            mem.record_usage_event(usage_event("conformance"), cap)
                .await
                .unwrap();
        }
        assert_eq!(
            rows_real
                .usage_summary(&dispatch_tui::store::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>(),
            rows_mem
                .usage_summary(&dispatch_tui::store::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>(),
            "usage_events after prune"
        );
        let refused_real = real
            .record_usage_event(usage_event("conformance"), 0)
            .await
            .unwrap();
        let refused_mem = mem
            .record_usage_event(usage_event("conformance"), 0)
            .await
            .unwrap();
        assert_eq!(refused_real.won(), refused_mem.won(), "non-positive cap");

        // -- learnings, retrievals and verdicts (learnings, task #5003) -------------
        let helped_real = real.create_learning(blank_learning()).await.unwrap();
        wait_for!(rows_real.learning(helped_real).is_some());
        let helped_mem = mem.create_learning(blank_learning()).await.unwrap();
        assert_eq!(helped_real, helped_mem, "generated learning id");
        compare_learning(helped_real);

        let wrong_real = real.create_learning(blank_learning()).await.unwrap();
        wait_for!(rows_real.learning(wrong_real).is_some());
        let wrong_mem = mem.create_learning(blank_learning()).await.unwrap();
        assert_eq!(wrong_real, wrong_mem, "generated second learning id");

        // `ApprovedLearningsHaveScopeRef` (`docs/specs/learnings.allium`),
        // enforced server-side by both `create_learning`s.
        let bad_scope_real = real
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: None,
                ..blank_learning()
            })
            .await;
        let bad_scope_mem = mem
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: None,
                ..blank_learning()
            })
            .await;
        assert_eq!(
            bad_scope_real.is_err(),
            bad_scope_mem.is_err(),
            "scope-ref validation"
        );

        let revise = || bindings::LearningPatch {
            status: None,
            summary: Some("revised".into()),
            embedding: None,
        };
        real.patch_learning(helped_real, revise()).await.unwrap();
        wait_for!(rows_real
            .learning(helped_real)
            .is_some_and(|l| l.summary == "revised"));
        mem.patch_learning(helped_mem, revise()).await.unwrap();
        compare_learning(helped_real);

        real.record_learning_retrieval(task_id_real, helped_real, RetrievalSource::QueryLearnings)
            .await
            .unwrap();
        wait_for!(!rows_real.retrievals_for_task(task_id_real).is_empty());
        mem.record_learning_retrieval(task_id_mem, helped_mem, RetrievalSource::QueryLearnings)
            .await
            .unwrap();
        assert_eq!(
            rows_real.retrievals_for_task(task_id_real).len(),
            rows_mem.retrievals_for_task(task_id_mem).len(),
            "retrievals after record_learning_retrieval"
        );

        let verdicts = |helped_id: LearningId, wrong_id: LearningId| {
            vec![
                (helped_id, LearningVerdict::Helped),
                (wrong_id, LearningVerdict::Wrong),
            ]
        };
        real.apply_learning_verdicts(verdicts(helped_real, wrong_real))
            .await
            .unwrap();
        wait_for!(rows_real
            .learning(helped_real)
            .is_some_and(|l| l.upvote_count == 1));
        mem.apply_learning_verdicts(verdicts(helped_mem, wrong_mem))
            .await
            .unwrap();
        compare_learning(helped_real);
        compare_learning(wrong_real);

        // (An unknown verdict string is unrepresentable through the typed
        // `LearningVerdict`; the module's own tests cover that refusal.)

        // -- rescope_epic_learnings ---------------------------------------------------
        let epic_scoped_real = real
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: Some(epic_id_real.to_string()),
                ..blank_learning()
            })
            .await
            .unwrap();
        wait_for!(rows_real.learning(epic_scoped_real).is_some());
        let epic_scoped_mem = mem
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: Some(epic_id_mem.to_string()),
                ..blank_learning()
            })
            .await
            .unwrap();
        assert_eq!(
            epic_scoped_real, epic_scoped_mem,
            "generated epic-scoped learning id"
        );
        real.rescope_epic_learnings(EpicId(epic_id_real), EpicId(epic2_id_real))
            .await
            .unwrap();
        wait_for!(rows_real
            .learning(epic_scoped_real)
            .is_some_and(|l| l.scope_ref == Some(epic2_id_real.to_string())));
        mem.rescope_epic_learnings(EpicId(epic_id_mem), EpicId(epic2_id_mem))
            .await
            .unwrap();
        compare_learning(epic_scoped_real);

        // -- archive_stale_learnings --------------------------------------------------
        // `blank_learning`'s fixed, already-stale `created_at`/`updated_at`
        // makes this deterministic rather than waiting on wall-clock time.
        let stale_real = real.create_learning(blank_learning()).await.unwrap();
        wait_for!(rows_real.learning(stale_real).is_some());
        let stale_mem = mem.create_learning(blank_learning()).await.unwrap();
        assert_eq!(stale_real, stale_mem, "generated stale learning id");
        real.archive_stale_learnings(ts("2025-01-01 00:00:00.000"))
            .await
            .unwrap();
        wait_for!(rows_real
            .learning(stale_real)
            .is_some_and(|l| l.status == dispatch_tui::models::LearningStatus::Archived));
        mem.archive_stale_learnings(ts("2025-01-01 00:00:00.000"))
            .await
            .unwrap();
        compare_learning(stale_real);

        // -- delete_learning + its retrieval cascade ---------------------------------
        real.delete_learning(wrong_real).await.unwrap();
        wait_for!(rows_real.learning(wrong_real).is_none());
        mem.delete_learning(wrong_mem).await.unwrap();
        assert!(rows_mem.learning(wrong_mem).is_none());

        // A missing id is REFUSED, unlike most reducers' silent no-op.
        let missing_real = real.delete_learning(wrong_real).await.unwrap();
        let missing_mem = mem.delete_learning(wrong_mem).await.unwrap();
        assert_eq!(missing_real.won(), missing_mem.won(), "double delete");

        // -- agent session state: subagents + hooks (agent_state) -------------------
        let run = || bindings::TaskPatch {
            status: Some("running".into()),
            sub_status: Some("active".into()),
            ..blank_task_patch()
        };
        real.patch_task(task_id_real, run()).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Running));
        mem.patch_task(task_id_mem, run()).await.unwrap();
        compare_task(task_id_real.0);

        let real_live = real
            .subagent_start(
                task_id_real,
                "agent-a".into(),
                "session-1".into(),
                ts("2026-01-01 00:00:00.000"),
            )
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == real_live));
        let mem_live = mem
            .subagent_start(
                task_id_mem,
                "agent-a".into(),
                "session-1".into(),
                ts("2026-01-01 00:00:00.000"),
            )
            .await
            .unwrap();
        assert_eq!(real_live, mem_live, "subagent_start live count");
        compare_task(task_id_real.0);

        real.record_pre_tool_use(
            task_id_real,
            SubStatus::Active,
            ts("2026-01-01 00:00:01.000"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.last_pre_tool_use_at.is_some()));
        mem.record_pre_tool_use(
            task_id_mem,
            SubStatus::Active,
            ts("2026-01-01 00:00:01.000"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        real.record_notification(
            task_id_real,
            NotificationWrite::Raise,
            ts("2026-01-01 00:00:02.000"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::NeedsInput));
        mem.record_notification(
            task_id_mem,
            NotificationWrite::Raise,
            ts("2026-01-01 00:00:02.000"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        // A Stop while a subagent is live defers rather than flips.
        let real_deferred = real
            .try_record_stop(task_id_real, ts("2026-01-01 00:00:03.000"))
            .await
            .unwrap();
        wait_for!(rows_real.task(task_id_real).is_some_and(|t| t.stop_pending));
        let mem_deferred = mem
            .try_record_stop(task_id_mem, ts("2026-01-01 00:00:03.000"))
            .await
            .unwrap();
        assert_eq!(
            real_deferred, mem_deferred,
            "try_record_stop deferred outcome"
        );
        compare_task(task_id_real.0);

        real.record_user_prompt_submit(
            task_id_real,
            ts("2026-01-01 00:00:04.000"),
            ts("2026-01-01 00:00:04.500"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| !t.stop_pending));
        mem.record_user_prompt_submit(
            task_id_mem,
            ts("2026-01-01 00:00:04.000"),
            ts("2026-01-01 00:00:04.500"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        let real_stop = real
            .subagent_stop(task_id_real, "agent-a".into(), "session-1".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == 0));
        let mem_stop = mem
            .subagent_stop(task_id_mem, "agent-a".into(), "session-1".into())
            .await
            .unwrap();
        assert_eq!(real_stop.live, mem_stop.live, "subagent_stop live count");
        assert_eq!(
            real_stop.is_review, mem_stop.is_review,
            "subagent_stop is_review"
        );
        compare_task(task_id_real.0);

        // subagent_clear / subagent_clear_and_void_pending_stop, over a fresh
        // subagent so there is something live to drain.
        real.subagent_start(
            task_id_real,
            "agent-b".into(),
            "session-2".into(),
            ts("2026-01-01 00:00:05.000"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == 1));
        mem.subagent_start(
            task_id_mem,
            "agent-b".into(),
            "session-2".into(),
            ts("2026-01-01 00:00:05.000"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        let real_cleared = real.subagent_clear(task_id_real).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == 0));
        let mem_cleared = mem.subagent_clear(task_id_mem).await.unwrap();
        assert_eq!(
            real_cleared.live, mem_cleared.live,
            "subagent_clear live count"
        );
        compare_task(task_id_real.0);

        real.subagent_clear_and_void_pending_stop(task_id_real)
            .await
            .unwrap();
        mem.subagent_clear_and_void_pending_stop(task_id_mem)
            .await
            .unwrap();
        compare_task(task_id_real.0);

        // mark_pr_learnings_gate_shown: no board-visible field (bindings-only,
        // per `TaskShape`'s doc comment), so only the outcome parity matters —
        // the write itself is already committed by the time `.await` returns.
        real.mark_pr_learnings_gate_shown(task_id_real, ts("2026-01-01 00:00:06.000"))
            .await
            .unwrap();
        let real_gate_repeat = real
            .mark_pr_learnings_gate_shown(task_id_real, ts("2026-01-01 00:00:07.000"))
            .await
            .unwrap();
        mem.mark_pr_learnings_gate_shown(task_id_mem, ts("2026-01-01 00:00:06.000"))
            .await
            .unwrap();
        let mem_gate_repeat = mem
            .mark_pr_learnings_gate_shown(task_id_mem, ts("2026-01-01 00:00:07.000"))
            .await
            .unwrap();
        assert_eq!(
            real_gate_repeat.won(),
            mem_gate_repeat.won(),
            "repeat PR-learnings-gate outcome"
        );

        // batch_patch_sub_status: no recalculation, ignores a missing id.
        let updates = |task_id: TaskId| {
            vec![
                (task_id, SubStatus::Stale),
                (TaskId(999_999), SubStatus::Active),
            ]
        };
        real.batch_patch_sub_status(updates(task_id_real))
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::Stale));
        mem.batch_patch_sub_status(updates(task_id_mem))
            .await
            .unwrap();
        compare_task(task_id_real.0);

        // -- work packages (`work-packages.allium`) -----------------------------
        // `updated_at` is the store's own clock on a patch, so it is left out
        // of the comparison, on `SameEndState`'s reasoning.
        let package_shapes = |rows: &SharedRows| -> Vec<(i64, String, String, Option<String>)> {
            rows.work_packages()
                .into_iter()
                .map(|p| (p.id.0, p.title, p.description, p.created_by))
                .collect()
        };
        let package_row = || bindings::WorkPackage {
            id: 0,
            title: "  Window 1 ".into(),
            description: "the plan".into(),
            created_by: accepted.identity.clone(),
            created_at: "2026-01-01 00:00:00.000".into(),
            updated_at: "2026-01-01 00:00:00.000".into(),
        };
        let package_real = real.create_work_package(package_row()).await.unwrap();
        wait_for!(!rows_real.work_packages().is_empty());
        let package_mem = mem.create_work_package(package_row()).await.unwrap();
        assert_eq!(package_real, package_mem, "generated work package id");
        assert_eq!(package_shapes(&rows_real), package_shapes(&rows_mem));

        let blank = bindings::WorkPackage {
            title: "  ".into(),
            ..package_row()
        };
        assert!(real.create_work_package(blank.clone()).await.is_err());
        assert!(mem.create_work_package(blank).await.is_err());

        let task_item = dispatch_tui::models::WorkPackageItem::Task(task_id_real);
        let epic_item = dispatch_tui::models::WorkPackageItem::Epic(EpicId(epic_id_real));
        for item in [task_item, epic_item, task_item] {
            let r = real
                .link_work_package_item(package_real, item)
                .await
                .unwrap();
            let m = mem.link_work_package_item(package_mem, item).await.unwrap();
            assert_eq!(r.won(), m.won(), "link {item:?}");
        }
        wait_for!(rows_real.work_package_links().len() == 2);
        assert_eq!(
            rows_real.work_package_links(),
            rows_mem.work_package_links(),
            "links after linking"
        );

        let unknown = dispatch_tui::models::WorkPackageItem::Task(TaskId(987_654));
        let r = real
            .link_work_package_item(package_real, unknown)
            .await
            .unwrap();
        let m = mem
            .link_work_package_item(package_mem, unknown)
            .await
            .unwrap();
        assert!(!r.won(), "a link to an unknown task is refused");
        assert_eq!(r.won(), m.won(), "link to an unknown task");

        for _ in 0..2 {
            let r = real
                .unlink_work_package_item(package_real, epic_item)
                .await
                .unwrap();
            let m = mem
                .unlink_work_package_item(package_mem, epic_item)
                .await
                .unwrap();
            assert_eq!(r.won(), m.won(), "unlink");
        }
        wait_for!(rows_real.work_package_links().len() == 1);
        assert_eq!(
            rows_real.work_package_links(),
            rows_mem.work_package_links()
        );

        let rename = || bindings::WorkPackagePatch {
            title: Some(" Window 2".into()),
            description: None,
        };
        real.patch_work_package(package_real, rename())
            .await
            .unwrap();
        wait_for!(rows_real
            .work_packages()
            .first()
            .is_some_and(|p| p.title == "Window 2"));
        mem.patch_work_package(package_mem, rename()).await.unwrap();
        assert_eq!(package_shapes(&rows_real), package_shapes(&rows_mem));

        let missing = dispatch_tui::models::WorkPackageId(987_654);
        let r = real.patch_work_package(missing, rename()).await.unwrap();
        let m = mem.patch_work_package(missing, rename()).await.unwrap();
        assert!(!r.won(), "patching a missing package is refused");
        assert_eq!(r.won(), m.won(), "patch of a missing package");

        // -- poll ownership (agent_state) ----------------------------------------
        real.claim_poll_owner(PollScopeId::Task(task_id_real), "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .poll_owner(PollScopeId::Task(task_id_real))
            .is_some());
        mem.claim_poll_owner(PollScopeId::Task(task_id_mem), "host-a".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.poll_owner(PollScopeId::Task(task_id_real)),
            rows_mem.poll_owner(PollScopeId::Task(task_id_mem)),
            "poll_owner after claim"
        );

        // A second claim leaves the existing owner alone.
        real.claim_poll_owner(PollScopeId::Task(task_id_real), "host-b".into())
            .await
            .unwrap();
        mem.claim_poll_owner(PollScopeId::Task(task_id_mem), "host-b".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.poll_owner(PollScopeId::Task(task_id_real)),
            rows_mem.poll_owner(PollScopeId::Task(task_id_mem)),
            "poll_owner unchanged after a second claim"
        );

        real.override_poll_owner(PollScopeId::Task(task_id_real), "host-b".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .poll_owner(PollScopeId::Task(task_id_real))
            .is_some_and(|p| p.host == "host-b"));
        mem.override_poll_owner(PollScopeId::Task(task_id_mem), "host-b".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.poll_owner(PollScopeId::Task(task_id_real)),
            rows_mem.poll_owner(PollScopeId::Task(task_id_mem)),
            "poll_owner after override"
        );

        // -- host registry (agent_state) -----------------------------------------
        // No board-visible reader on either side (see `SharedRows`'s own
        // comment on `hosts`), so only outcome parity is comparable here.
        let real_host = real
            .register_host("conformance-host".into(), "label".into(), String::new())
            .await
            .unwrap();
        let mem_host = mem
            .register_host("conformance-host".into(), "label".into(), String::new())
            .await
            .unwrap();
        assert_eq!(real_host.won(), mem_host.won(), "register_host");
        let real_host_empty = real
            .register_host(String::new(), "label".into(), String::new())
            .await
            .unwrap();
        let mem_host_empty = mem
            .register_host(String::new(), "label".into(), String::new())
            .await
            .unwrap();
        assert_eq!(
            real_host_empty.won(),
            mem_host_empty.won(),
            "register_host with an empty id"
        );

        // -- feed ingestion + repo-group/managed-role epics (agent_state) --------
        // `title` (not just `feed_command`) must differ from every other epic
        // this scenario creates: `create_epic`'s id read-back matches on
        // `(title, parent_epic_id, created_at, created_by)`
        // (`matches_created_epic`, src/sync/sdk_connector.rs) — NOT on
        // `feed_command` — so an otherwise-`blank_epic()` fixture would
        // collide with `epic_id_real`/`epic2_id_real` and read back the
        // wrong id.
        //
        // `created_by` must be this connection's OWN identity, not the
        // literal `"tester"` `blank_epic()` defaults to: unlike
        // `epic_id_real`/`epic2_id_real` (visible only because the test
        // pre-declares them as followed epics, `SubscriptionRequest::new`'s
        // `vec![1, 2]` above), a brand-new top-level epic has no such
        // pre-declared coverage — `own_creations`
        // (`SELECT * FROM epics WHERE created_by = '{owner}'`) is what makes
        // it visible for the id read-back at all.
        // A child of `epic_id_real`, not top-level: nesting it here means its
        // own child tasks fall under the followed-epic subtree subscription,
        // not just `own_creations`.
        let feed_epic = |parent_epic_id: i64| bindings::Epic {
            title: "feed epic".into(),
            feed_command: "some-command".into(),
            parent_epic_id,
            created_by: accepted.identity.clone(),
            ..blank_epic()
        };
        let feed_epic_id_real = real.create_epic(feed_epic(epic_id_real)).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(feed_epic_id_real)).is_some());
        let feed_epic_id_mem = mem.create_epic(feed_epic(epic_id_mem)).await.unwrap().0;
        assert_eq!(feed_epic_id_real, feed_epic_id_mem, "feed epic id");

        let feed_item = |external_id: &str| bindings::FeedIngestItem {
            item: bindings::FeedTaskUpsertItem {
                external_id: external_id.into(),
                title: "feed task".into(),
                description: String::new(),
                repo_path: "/repo".into(),
                status: "backlog".into(),
                sub_status: "none".into(),
                base_branch: "main".into(),
                tag: String::new(),
                labels: "[]".into(),
                sort_order: None,
                url: String::new(),
                url_type: String::new(),
                wrap_up_mode: String::new(),
            },
            signals: Vec::new(),
        };
        // `created_by` is this connection's own identity here too: a feed
        // task's home epic (`feed_epic_id_real`) is not a followed epic, so
        // only `own_creations` (`SELECT * FROM tasks WHERE created_by =
        // '{owner}'`) makes the inserted rows arrive on this subscription at
        // all.
        real.ingest_feed(
            EpicId(feed_epic_id_real),
            vec![feed_item("feed-a"), feed_item("feed-b")],
            false,
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len() == 2);
        mem.ingest_feed(
            EpicId(feed_epic_id_mem),
            vec![feed_item("feed-a"), feed_item("feed-b")],
            false,
            "conformance".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len(),
            rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem)).len(),
            "feed task count after first upsert"
        );

        // Non-additive: absent items are removed.
        real.ingest_feed(
            EpicId(feed_epic_id_real),
            vec![feed_item("feed-a")],
            false,
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len() == 1);
        mem.ingest_feed(
            EpicId(feed_epic_id_mem),
            vec![feed_item("feed-a")],
            false,
            "conformance".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len(),
            rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem)).len(),
            "feed task count after stale-delete"
        );

        // An empty emission reconciles the feed epic down to nothing.
        real.ingest_feed(
            EpicId(feed_epic_id_real),
            vec![],
            false,
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .tasks_for_epic(EpicId(feed_epic_id_real))
            .is_empty());
        mem.ingest_feed(
            EpicId(feed_epic_id_mem),
            vec![],
            false,
            "conformance".into(),
        )
        .await
        .unwrap();
        assert!(rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem)).is_empty());

        // -- retirement on delete_task (agent_state) -----------------------------
        // Retirement only fires from the guarded `delete_task`, never from a
        // feed's own stale-reconciliation delete — see the module's own
        // `delete_stale_feed_tasks_in_epic` vs. `delete_task_side_effects`.
        real.ingest_feed(
            EpicId(feed_epic_id_real),
            vec![feed_item("retire-me")],
            false,
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(!rows_real
            .tasks_for_epic(EpicId(feed_epic_id_real))
            .is_empty());
        mem.ingest_feed(
            EpicId(feed_epic_id_mem),
            vec![feed_item("retire-me")],
            false,
            "conformance".into(),
        )
        .await
        .unwrap();
        let retire_task_real = rows_real.tasks_for_epic(EpicId(feed_epic_id_real))[0].id;
        let retire_task_mem = rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem))[0].id;
        real.patch_task(retire_task_real, mark_done())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(retire_task_real)
            .is_some_and(|t| t.status == TaskStatus::Done));
        mem.patch_task(retire_task_mem, mark_done()).await.unwrap();
        real.delete_task(retire_task_real).await.unwrap();
        wait_for!(rows_real.task(retire_task_real).is_none());
        mem.delete_task(retire_task_mem).await.unwrap();
        assert_eq!(
            rows_real.retired_without_task(EpicId(feed_epic_id_real), &["retire-me".to_string()]),
            rows_mem.retired_without_task(EpicId(feed_epic_id_mem), &["retire-me".to_string()]),
            "retired_without_task after delete_task"
        );

        // -- respawn_phoenix_successor (agent_state) -----------------------------
        // Each of these three tasks needs its own distinct `title`: both
        // `create_task` and `respawn_phoenix_successor`'s id read-back match
        // on `(title, repo_path, owner, epic_id, created_at, created_by)`
        // (`matches_create`), and `blank_task_in_epic(epic_id_real)`'s
        // defaults would otherwise collide with `task_id_real` and each
        // other, all three sharing `epic_id_real`.
        let real_predecessor = real
            .create_task(bindings::Task {
                title: "phoenix predecessor".into(),
                status: "done".into(),
                phoenix: true,
                ..blank_task_in_epic(epic_id_real)
            })
            .await
            .unwrap();
        wait_for!(rows_real.task(real_predecessor).is_some());
        let mem_predecessor = mem
            .create_task(bindings::Task {
                title: "phoenix predecessor".into(),
                status: "done".into(),
                phoenix: true,
                ..blank_task_in_epic(epic_id_mem)
            })
            .await
            .unwrap();
        assert_eq!(real_predecessor, mem_predecessor, "phoenix predecessor id");

        let phoenix_successor = |epic_id: i64| bindings::Task {
            title: "phoenix successor".into(),
            ..blank_task_in_epic(epic_id)
        };
        let real_successor = real
            .respawn_phoenix_successor(real_predecessor, phoenix_successor(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.task(real_successor).is_some());
        let mem_successor = mem
            .respawn_phoenix_successor(mem_predecessor, phoenix_successor(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(real_successor, mem_successor, "phoenix successor id");
        wait_for!(rows_real.task(real_predecessor).is_some_and(|t| !t.phoenix));
        compare_task(real_predecessor.0);
        compare_task(real_successor.0);

        // Clean up the extra fixtures this section created, so the delete
        // section below only has the original task/epics' worth to close out.
        for (real_id, mem_id) in [
            (real_predecessor, mem_predecessor),
            (real_successor, mem_successor),
        ] {
            real.patch_task(real_id, mark_done()).await.unwrap();
            wait_for!(rows_real
                .task(real_id)
                .is_some_and(|t| t.status == TaskStatus::Done));
            mem.patch_task(mem_id, mark_done()).await.unwrap();
            real.delete_task(real_id).await.unwrap();
            wait_for!(rows_real.task(real_id).is_none());
            mem.delete_task(mem_id).await.unwrap();
        }
        real.delete_epic(EpicId(feed_epic_id_real)).await.unwrap();
        wait_for!(rows_real.epic(EpicId(feed_epic_id_real)).is_none());
        mem.delete_epic(EpicId(feed_epic_id_mem)).await.unwrap();

        // -- delete_task's learnings cascade (learnings, task #5003) ----------------
        // `delete_task_side_effects`'s learnings half: a sourced learning is
        // detached (`SET NULL`) rather than deleted, and every retrieval
        // recorded against the doomed task is dropped.
        let sourced_real = real
            .create_learning(bindings::Learning {
                source_task_id: Some(task_id_real.0),
                ..blank_learning()
            })
            .await
            .unwrap();
        wait_for!(rows_real.learning(sourced_real).is_some());
        let sourced_mem = mem
            .create_learning(bindings::Learning {
                source_task_id: Some(task_id_mem.0),
                ..blank_learning()
            })
            .await
            .unwrap();
        assert_eq!(sourced_real, sourced_mem, "generated sourced learning id");
        real.record_learning_retrieval(task_id_real, sourced_real, RetrievalSource::QueryLearnings)
            .await
            .unwrap();
        wait_for!(!rows_real.retrievals_for_task(task_id_real).is_empty());
        mem.record_learning_retrieval(task_id_mem, sourced_mem, RetrievalSource::QueryLearnings)
            .await
            .unwrap();

        // -- delete_task, then delete_epic ---------------------------------------
        // task #4971 added `delete_task`'s `requires: task.status = done` guard
        // (spacetime/module/src/lib.rs::delete_task) — this scenario predates
        // that guard, so the task must be marked done first or the real side
        // refuses the delete while the (guard-unaware) mem side does not.
        real.patch_task(task_id_real, mark_done()).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Done));
        mem.patch_task(task_id_mem, mark_done()).await.unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);

        real.delete_task(task_id_real).await.unwrap();
        wait_for!(rows_real.task(task_id_real).is_none());
        mem.delete_task(task_id_mem).await.unwrap();
        assert!(rows_mem.task(task_id_mem).is_none());
        compare_epic(epic_id_real);

        // `work-packages.allium: DeletingAnItemRemovesItsLinks` — the task was
        // the package's last link.
        wait_for!(rows_real.work_package_links().is_empty());
        assert!(rows_mem.work_package_links().is_empty());
        real.delete_work_package(package_real).await.unwrap();
        wait_for!(rows_real.work_packages().is_empty());
        mem.delete_work_package(package_mem).await.unwrap();
        assert!(rows_mem.work_packages().is_empty());

        wait_for!(rows_real
            .learning(sourced_real)
            .is_some_and(|l| l.source_task_id.is_none()));
        compare_learning(sourced_real);
        assert!(rows_real.retrievals_for_task(task_id_real).is_empty());
        assert!(rows_mem.retrievals_for_task(task_id_mem).is_empty());

        real.delete_epic(EpicId(epic_id_real)).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic_id_real)).is_none());
        mem.delete_epic(EpicId(epic_id_mem)).await.unwrap();
        assert!(rows_mem.epic(EpicId(epic_id_mem)).is_none());

        real.delete_epic(EpicId(epic2_id_real)).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic2_id_real)).is_none());
        mem.delete_epic(EpicId(epic2_id_mem)).await.unwrap();
        assert!(rows_mem.epic(EpicId(epic2_id_mem)).is_none());
    });
}
