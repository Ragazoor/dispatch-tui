use super::*;

/// What a dispatch request row looks like once both sides wrote it: everything
/// but the ids and the module-clock timestamps, which differ by construction.
#[derive(Debug, PartialEq)]
struct RequestShape {
    task_id: TaskId,
    host: String,
    requested_by: String,
    mode: dispatch_tui::models::DispatchMode,
    origin: dispatch_tui::models::DispatchOrigin,
    state: dispatch_tui::models::DispatchRequestState,
    reason: Option<String>,
    worktree: Option<String>,
    has_claimed_at: bool,
    has_finished_at: bool,
    deadline_after_requested: chrono::Duration,
}

fn request_shape(r: &dispatch_tui::models::DispatchRequest) -> RequestShape {
    RequestShape {
        task_id: r.task_id,
        host: r.host.clone(),
        requested_by: r.requested_by.clone(),
        mode: r.mode,
        origin: r.origin,
        state: r.state,
        reason: r.reason.clone(),
        worktree: r.worktree.clone(),
        has_claimed_at: r.claimed_at.is_some(),
        has_finished_at: r.finished_at.is_some(),
        deadline_after_requested: r.deadline - r.requested_at,
    }
}

/// store-split M4 (`store-split.allium`: `RequestDispatch` and the rules after
/// it). The in-memory caller serves dispatch requests the way the real module
/// does: the claim-and-row transaction, the derived mode, the host claim, both
/// reports, the conditional release, the restart rule, the task-delete rule,
/// and the refusals — on the same calls, with the same outcome on both sides.
///
/// The module's scheduled backstop (`expire_dispatch_request`) is not driven
/// here: it fires 70s after the request and no client may call it. The double's
/// stand-in is pinned by `src/sync/memory_caller/tests/dispatch_requests.rs`.
#[test]
fn memory_caller_serves_dispatch_requests_like_the_real_module() {
    use dispatch_tui::models::DispatchRequestId;

    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-dispatch-requests");
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
        // "host-a" is this board's host: the subscription delivers its
        // dispatch requests (sync.allium: SubscribeToThisHostsDispatchRequests).
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
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
        let same_task = |id: TaskId| {
            assert_eq!(
                rows_real.task(id).map(|t| task_shape(&t)),
                rows_mem.task(id).map(|t| task_shape(&t)),
                "task {id:?}"
            );
        };
        let same_request = |real_id: DispatchRequestId, mem_id: DispatchRequestId| {
            assert_eq!(
                rows_real
                    .dispatch_request(real_id)
                    .map(|r| request_shape(&r)),
                rows_mem.dispatch_request(mem_id).map(|r| request_shape(&r)),
                "request real {real_id:?} / memory {mem_id:?}"
            );
        };
        let applied_id = |outcome: ReducerOutcome| match outcome {
            ReducerOutcome::Applied(ids) if ids.len() == 1 => DispatchRequestId(ids[0]),
            other => panic!("expected one read-back request id, got {other:?}"),
        };
        let both_refuse = |r: ReducerOutcome, m: ReducerOutcome| match (r, m) {
            (ReducerOutcome::Refused(_), ReducerOutcome::Refused(_)) => {}
            other => panic!("both sides must refuse, got {other:?}"),
        };

        let mine = |tag: &str| bindings::Task {
            created_by: identity.clone(),
            owner: identity.clone(),
            tag: tag.into(),
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

        // -- D11: both sides refuse a host the caller does not own ----------------
        // host-a is this connection's own; their-host never got a row, because
        // a NEW host may not be registered ownerless, so nobody may act for it.
        // (Naming another identity as owner is itself refused:
        // `core.allium: LocalHostOwnerIsWrittenOnce`, checked below.)
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        both_refuse(
            real.register_host("their-host".into(), String::new(), String::new())
                .await
                .unwrap(),
            mem.register_host("their-host".into(), String::new(), String::new())
                .await
                .unwrap(),
        );
        // LocalHostOwnerIsWrittenOnce: neither side lets a caller name another
        // identity as owner, nor disown its own host.
        both_refuse(
            real.register_host("gift".into(), String::new(), "beef".into())
                .await
                .unwrap(),
            mem.register_host("gift".into(), String::new(), "beef".into())
                .await
                .unwrap(),
        );
        both_refuse(
            real.register_host("host-a".into(), String::new(), String::new())
                .await
                .unwrap(),
            mem.register_host("host-a".into(), String::new(), String::new())
                .await
                .unwrap(),
        );
        let theirs = create(mine("")).await;
        wait_for!(rows_real.task(theirs).is_some());
        both_refuse(
            real.request_dispatch(theirs, "their-host".into(), "user".into(), String::new())
                .await
                .unwrap(),
            mem.request_dispatch(theirs, "their-host".into(), "user".into(), String::new())
                .await
                .unwrap(),
        );

        // -- RequestDispatch: claim + row, derived mode --------------------------
        let research = create(mine("research")).await;
        wait_for!(rows_real.task(research).is_some());
        let r_req = applied_id(
            real.request_dispatch(research, "host-a".into(), "mcp".into(), String::new())
                .await
                .unwrap(),
        );
        let m_req = applied_id(
            mem.request_dispatch(research, "host-a".into(), "mcp".into(), String::new())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.dispatch_request(r_req).is_some());
        wait_for!(rows_real
            .task(research)
            .is_some_and(|t| t.status == TaskStatus::Running));
        same_request(r_req, m_req);
        same_task(research);
        assert_eq!(
            rows_mem.dispatch_request(m_req).unwrap().mode,
            dispatch_tui::models::DispatchMode::Research
        );

        // A second request while one is open: refused on both sides.
        both_refuse(
            real.request_dispatch(research, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
            mem.request_dispatch(research, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
        );
        // An unknown origin: refused on both sides.
        let spare = create(mine("")).await;
        wait_for!(rows_real.task(spare).is_some());
        both_refuse(
            real.request_dispatch(spare, "host-a".into(), "cron".into(), String::new())
                .await
                .unwrap(),
            mem.request_dispatch(spare, "host-a".into(), "cron".into(), String::new())
                .await
                .unwrap(),
        );

        // -- HostClaimsDispatch --------------------------------------------------
        both_refuse(
            real.claim_dispatch_request(r_req, "host-b".into())
                .await
                .unwrap(),
            mem.claim_dispatch_request(m_req, "host-b".into())
                .await
                .unwrap(),
        );
        assert!(real
            .claim_dispatch_request(r_req, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .claim_dispatch_request(m_req, "host-a".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .dispatch_request(r_req)
            .is_some_and(|r| r.claimed_at.is_some()));
        same_request(r_req, m_req);
        both_refuse(
            real.claim_dispatch_request(r_req, "host-a".into())
                .await
                .unwrap(),
            mem.claim_dispatch_request(m_req, "host-a".into())
                .await
                .unwrap(),
        );

        // -- HostReportsDispatched -----------------------------------------------
        let report = |id| {
            (
                id,
                "host-a".to_string(),
                "/repo/.worktrees/x".to_string(),
                "task-x".to_string(),
            )
        };
        let (id, who, wt, win) = report(r_req);
        assert!(real
            .report_dispatch_provisioned(id, who, wt, win, false)
            .await
            .unwrap()
            .won());
        let (id, who, wt, win) = report(m_req);
        assert!(mem
            .report_dispatch_provisioned(id, who, wt, win, false)
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .task(research)
            .is_some_and(|t| t.worktree.is_some()));
        same_request(r_req, m_req);
        same_task(research);

        // -- HostReportsDispatchFailed + ReleaseClaimOfFailedRequest ---------------
        let failing = create(mine("")).await;
        wait_for!(rows_real.task(failing).is_some());
        let r_fail = applied_id(
            real.request_dispatch(failing, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
        );
        let m_fail = applied_id(
            mem.request_dispatch(failing, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.dispatch_request(r_fail).is_some());
        real.claim_dispatch_request(r_fail, "host-a".into())
            .await
            .unwrap();
        mem.claim_dispatch_request(m_fail, "host-a".into())
            .await
            .unwrap();
        assert!(real
            .report_dispatch_failed(r_fail, "host-a".into(), "worktree add failed".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_dispatch_failed(m_fail, "host-a".into(), "worktree add failed".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .task(failing)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        same_request(r_fail, m_fail);
        same_task(failing);

        // -- HostRestartFailsInterrupted ------------------------------------------
        let interrupted = create(mine("")).await;
        let waiting = create(mine("")).await;
        wait_for!(rows_real.task(waiting).is_some());
        let mut pairs = Vec::new();
        for task in [interrupted, waiting] {
            let r = applied_id(
                real.request_dispatch(task, "host-a".into(), "user".into(), "dispatch".into())
                    .await
                    .unwrap(),
            );
            let m = applied_id(
                mem.request_dispatch(task, "host-a".into(), "user".into(), "dispatch".into())
                    .await
                    .unwrap(),
            );
            wait_for!(rows_real.dispatch_request(r).is_some());
            pairs.push((r, m));
        }
        real.claim_dispatch_request(pairs[0].0, "host-a".into())
            .await
            .unwrap();
        mem.claim_dispatch_request(pairs[0].1, "host-a".into())
            .await
            .unwrap();
        real.fail_interrupted_dispatch_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.fail_interrupted_dispatch_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(interrupted)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        for (r, m) in &pairs {
            same_request(*r, *m);
        }
        same_task(interrupted);
        same_task(waiting);

        // -- DeletingATaskFailsItsOpenRequest, then DeletingATaskDeletesItsRequestRows -----------------------------------
        // Only a done task may be deleted (`tasks.allium: DeleteTask`): moved
        // to done by hand while its request is still open.
        let done = || bindings::TaskPatch {
            status: Some("done".into()),
            sub_status: Some("none".into()),
            ..blank_task_patch()
        };
        real.patch_task(waiting, done())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.patch_task(waiting, done())
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(waiting)
            .is_some_and(|t| t.status == TaskStatus::Done));
        real.delete_task(waiting).await.unwrap().applied().unwrap();
        mem.delete_task(waiting).await.unwrap().applied().unwrap();
        // `DeletingATaskDeletesItsRequestRows`: failed in the same call, then
        // gone with the task.
        wait_for!(rows_real.dispatch_request(pairs[1].0).is_none());
        assert!(rows_mem.dispatch_request(pairs[1].1).is_none());
    });
}
