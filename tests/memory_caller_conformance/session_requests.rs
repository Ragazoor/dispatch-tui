use super::*;

/// store-split M5 (`store-split.allium`: `RequestWrapUp` and the rules after
/// it). The in-memory caller serves wrap-up and exit-session requests the way
/// the real module does: the request and its refusals, the claim that clears a
/// Conflict, both reports with the module setting the conflict, the token
/// check, the close (terminal write, epic rollup) and the epic chain in one
/// transaction, the restart rule and the task-delete rule — on the same
/// calls, with the same outcome on both sides.
///
/// The module's scheduled backstop (`expire_session_request`) is not driven
/// here, as for M4's: no client may call it. The double's stand-in is pinned
/// by `src/sync/memory_caller/tests/session_requests.rs`.
#[test]
fn memory_caller_serves_session_requests_like_the_real_module() {
    use dispatch_tui::models::SessionRequestId;

    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-session-requests");
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
        // sync.allium: SubscribeToThisHostsSessionRequests — and the dispatch
        // requests the chain writes, which the same host asks for.
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
        let same_session = |real_id: SessionRequestId, mem_id: SessionRequestId| {
            assert_eq!(
                rows_real
                    .session_request(real_id)
                    .map(|r| session_shape(&r)),
                rows_mem.session_request(mem_id).map(|r| session_shape(&r)),
                "session request real {real_id:?} / memory {mem_id:?}"
            );
        };
        let applied_id = |outcome: ReducerOutcome| match outcome {
            ReducerOutcome::Applied(ids) if ids.len() == 1 => SessionRequestId(ids[0]),
            other => panic!("expected one read-back request id, got {other:?}"),
        };
        let both_refuse = |r: ReducerOutcome, m: ReducerOutcome| match (r, m) {
            (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                assert_eq!(r, m, "both sides must refuse in the same words");
            }
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
        let mine = |epic_id: i64, tag: &str| bindings::Task {
            created_by: identity.clone(),
            // A task in an epic carries no owner (`OwnerTracksUserBoardTask`).
            owner: if epic_id == 0 {
                identity.clone()
            } else {
                String::new()
            },
            tag: tag.into(),
            ..blank_task_in_epic(epic_id)
        };
        // A task running on host-a: the state every wrap-up starts from. Patched
        // in directly, not dispatched: a dispatch finishing in the same
        // millisecond as the wrap-up would void the token (W3) on one side and
        // not the other.
        let dispatched = |task: TaskId| {
            let real = &real;
            let mem = &mem;
            let instance = &instance;
            async move {
                for caller in [real as &dyn ReducerCaller, mem as &dyn ReducerCaller] {
                    caller
                        .patch_task(
                            task,
                            bindings::TaskPatch {
                                status: Some("running".into()),
                                sub_status: Some("active".into()),
                                worktree: Some(format!("/repo/.worktrees/{}-t", task.0)),
                                host: Some("host-a".into()),
                                ..blank_task_patch()
                            },
                        )
                        .await
                        .unwrap()
                        .applied()
                        .unwrap();
                }
                // The window is the current agent's since agents A4, and an
                // agent comes from the backfill: a worktree and none.
                let backfilled = instance.call("backfill_agents", &[]);
                assert!(backfilled.status.success(), "{}", describe(&backfilled));
                mem.backfill_agents().unwrap().applied().unwrap();
                for caller in [real as &dyn ReducerCaller, mem as &dyn ReducerCaller] {
                    caller
                        .patch_task(
                            task,
                            bindings::TaskPatch {
                                tmux_window: Some(format!("task-{}", task.0)),
                                ..blank_task_patch()
                            },
                        )
                        .await
                        .unwrap()
                        .applied()
                        .unwrap();
                }
            }
        };

        // -- RequestWrapUp: refusals first --------------------------------------
        let backlog = create(mine(0, "")).await;
        wait_for!(rows_real.task(backlog).is_some());
        both_refuse(
            real.request_wrap_up(backlog, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
            mem.request_wrap_up(backlog, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let reviewer = create(mine(0, "pr-review")).await;
        wait_for!(rows_real.task(reviewer).is_some());
        dispatched(reviewer).await;
        wait_for!(rows_real
            .task(reviewer)
            .is_some_and(|t| t.worktree.is_some()));
        both_refuse(
            real.request_wrap_up(reviewer, "host-a".into(), "done".into())
                .await
                .unwrap(),
            mem.request_wrap_up(reviewer, "host-a".into(), "done".into())
                .await
                .unwrap(),
        );
        both_refuse(
            real.request_wrap_up(reviewer, "host-a".into(), "squash".into())
                .await
                .unwrap(),
            mem.request_wrap_up(reviewer, "host-a".into(), "squash".into())
                .await
                .unwrap(),
        );

        // -- A rebase: request, claim, conflict ----------------------------------
        let epic = {
            let row = bindings::Epic {
                created_by: identity.clone(),
                auto_dispatch: true,
                ..blank_epic()
            };
            let r = real.create_epic(row.clone()).await.unwrap();
            let m = mem.create_epic(row).await.unwrap();
            assert_eq!(r, m);
            r
        };
        let task = create(mine(epic.0, "")).await;
        let next = create(mine(epic.0, "")).await;
        wait_for!(rows_real.task(next).is_some());
        dispatched(task).await;
        wait_for!(rows_real.task(task).is_some_and(|t| t.worktree.is_some()));
        let r_wrap = applied_id(
            real.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_wrap = applied_id(
            mem.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.session_request(r_wrap).is_some());
        same_session(r_wrap, m_wrap);
        same_task(task);
        both_refuse(
            real.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
            mem.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        // A claim from the wrong host is refused, the right one wins once.
        assert!(!real
            .claim_session_request(r_wrap, "host-b".into())
            .await
            .unwrap()
            .won());
        assert!(real
            .claim_session_request(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .claim_session_request(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .session_request(r_wrap)
            .is_some_and(|r| r.claimed_at.is_some()));
        same_session(r_wrap, m_wrap);
        assert!(!real
            .claim_session_request(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(!mem
            .claim_session_request(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        // A conflict report: the MODULE sets the sub_status.
        assert!(real
            .report_session_request_failed(
                r_wrap,
                "host-a".into(),
                "Rebase conflict on b in a.rs".into(),
                true
            )
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_failed(
                m_wrap,
                "host-a".into(),
                "Rebase conflict on b in a.rs".into(),
                true
            )
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::Conflict));
        same_session(r_wrap, m_wrap);
        same_task(task);

        // -- A second rebase succeeds: the claim cleared the Conflict ---------------
        let r_wrap = applied_id(
            real.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_wrap = applied_id(
            mem.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        real.claim_session_request(r_wrap, "host-a".into())
            .await
            .unwrap();
        mem.claim_session_request(m_wrap, "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status != dispatch_tui::models::SubStatus::Conflict));
        same_task(task);
        assert!(real
            .report_session_request_done(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_done(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        // A repeated done report is accepted on both sides.
        assert!(real
            .report_session_request_done(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_done(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .session_request(r_wrap)
            .is_some_and(|r| r.finished_at.is_some()));
        same_session(r_wrap, m_wrap);

        // -- RequestExitSession: refusals leave the token valid -----------------------
        both_refuse(
            real.request_exit_session(
                task,
                SessionRequestId(r_wrap.0 + 50),
                "rebase".into(),
                String::new(),
            )
            .await
            .unwrap(),
            mem.request_exit_session(
                task,
                SessionRequestId(m_wrap.0 + 50),
                "rebase".into(),
                String::new(),
            )
            .await
            .unwrap(),
        );
        both_refuse(
            real.request_exit_session(task, r_wrap, "pr".into(), String::new())
                .await
                .unwrap(),
            mem.request_exit_session(task, m_wrap, "pr".into(), String::new())
                .await
                .unwrap(),
        );
        same_task(task);

        // -- The close, the token and the chain in one transaction ---------------------
        let r_exit = applied_id(
            real.request_exit_session(task, r_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
        );
        let m_exit = applied_id(
            mem.request_exit_session(task, m_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
        );
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.status == TaskStatus::Done));
        wait_for!(rows_real.session_request(r_exit).is_some());
        wait_for!(rows_real
            .task(next)
            .is_some_and(|t| t.status == TaskStatus::Running));
        same_session(r_exit, m_exit);
        same_task(task);
        same_task(next);
        assert!(rows_real.session_request(r_exit).unwrap().chained_task == Some(next));
        assert!(rows_mem.session_request(m_exit).unwrap().chained_task == Some(next));
        assert_eq!(
            rows_real.epic(epic).map(|e| epic_shape(&e)),
            rows_mem.epic(epic).map(|e| epic_shape(&e)),
        );
        // The token is spent.
        both_refuse(
            real.request_exit_session(task, r_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
            mem.request_exit_session(task, m_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
        );

        // -- The teardown reports, and a failed one does not revert the close ---------------
        assert!(real
            .claim_session_request(r_exit, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .claim_session_request(m_exit, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(real
            .report_session_request_failed(
                r_exit,
                "host-a".into(),
                "tmux kill-window failed".into(),
                false
            )
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_failed(
                m_exit,
                "host-a".into(),
                "tmux kill-window failed".into(),
                false
            )
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .session_request(r_exit)
            .is_some_and(|r| r.reason.is_some()));
        same_session(r_exit, m_exit);
        same_task(task);

        // -- A pr close ----------------------------------------------------------------------
        let pr_task = create(mine(0, "")).await;
        wait_for!(rows_real.task(pr_task).is_some());
        dispatched(pr_task).await;
        let r_pr = applied_id(
            real.request_wrap_up(pr_task, "host-a".into(), "pr".into())
                .await
                .unwrap(),
        );
        let m_pr = applied_id(
            mem.request_wrap_up(pr_task, "host-a".into(), "pr".into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.session_request(r_pr).is_some());
        same_session(r_pr, m_pr);
        let url = "https://github.com/o/r/pull/9";
        both_refuse(
            real.request_exit_session(pr_task, r_pr, "pr".into(), String::new())
                .await
                .unwrap(),
            mem.request_exit_session(pr_task, m_pr, "pr".into(), String::new())
                .await
                .unwrap(),
        );
        let r_close = applied_id(
            real.request_exit_session(pr_task, r_pr, "pr".into(), url.into())
                .await
                .unwrap(),
        );
        let m_close = applied_id(
            mem.request_exit_session(pr_task, m_pr, "pr".into(), url.into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real
            .task(pr_task)
            .is_some_and(|t| t.status == TaskStatus::Review));
        wait_for!(rows_real.session_request(r_close).is_some());
        same_session(r_close, m_close);
        same_task(pr_task);

        // -- HostRestartFailsInterruptedSessionRequests --------------------------------------
        let running = create(mine(0, "")).await;
        wait_for!(rows_real.task(running).is_some());
        dispatched(running).await;
        let r_run = applied_id(
            real.request_wrap_up(running, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_run = applied_id(
            mem.request_wrap_up(running, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        real.claim_session_request(r_run, "host-a".into())
            .await
            .unwrap();
        mem.claim_session_request(m_run, "host-a".into())
            .await
            .unwrap();
        real.fail_interrupted_session_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.fail_interrupted_session_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .session_request(r_run)
            .is_some_and(|r| r.reason.is_some()));
        same_session(r_run, m_run);
        assert_eq!(
            rows_mem.session_request(m_run).unwrap().reason.as_deref(),
            Some("host restarted")
        );

        // -- DeletingATaskFailsItsOpenSessionRequest -----------------------------------------
        let doomed = create(mine(0, "")).await;
        wait_for!(rows_real.task(doomed).is_some());
        dispatched(doomed).await;
        let r_doom = applied_id(
            real.request_wrap_up(doomed, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_doom = applied_id(
            mem.request_wrap_up(doomed, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        // A task is deleted only from Done (`tasks.allium: DeleteTask`).
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .patch_task(
                    doomed,
                    bindings::TaskPatch {
                        status: Some("done".into()),
                        sub_status: Some("none".into()),
                        ..blank_task_patch()
                    },
                )
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        real.delete_task(doomed).await.unwrap().applied().unwrap();
        mem.delete_task(doomed).await.unwrap().applied().unwrap();
        wait_for!(rows_real
            .session_request(r_doom)
            .is_some_and(|r| r.reason.is_some()));
        same_session(r_doom, m_doom);
        assert_eq!(
            rows_mem.session_request(m_doom).unwrap().reason.as_deref(),
            Some("task deleted")
        );
    });
}
