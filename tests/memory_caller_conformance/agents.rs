use super::*;

/// What an agent row looks like once both sides wrote it: everything but the
/// id, the name and the module-clock timestamps, which differ by construction.
/// The names are checked apart: both sides pick from the same list, but each
/// seeds the picker from its own clock.
#[derive(Debug, PartialEq)]
struct AgentShape {
    task_id: TaskId,
    host: String,
    start_mode: dispatch_tui::models::DispatchMode,
    state: dispatch_tui::models::AgentState,
    end_reason: Option<dispatch_tui::models::AgentEndReason>,
    tmux_window: Option<String>,
    has_ended_at: bool,
}

fn agent_shapes(rows: &SharedRows) -> Vec<AgentShape> {
    rows.agents()
        .iter()
        .map(|a| AgentShape {
            task_id: a.task_id,
            host: a.host.clone(),
            start_mode: a.start_mode,
            state: a.state,
            end_reason: a.end_reason,
            tmux_window: a.tmux_window.as_ref().map(|w| w.as_str().to_string()),
            has_ended_at: a.ended_at.is_some(),
        })
        .collect()
}

/// agents A1 (`agents.allium`). The in-memory caller starts, mirrors, ends and
/// deletes agents the way the real module does, on the same calls: the claim
/// starts one, the provisioned report and a window clear mirror the window, a
/// failed launch and a worktree clear end one, a relaunch replaces one, and a
/// delete takes every agent of the task.
#[test]
fn memory_caller_serves_agents_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-agents");
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
        let same_agents = || {
            assert_eq!(agent_shapes(&rows_real), agent_shapes(&rows_mem));
            for rows in [&rows_real, &rows_mem] {
                let names: Vec<String> = rows.agents().into_iter().map(|a| a.name).collect();
                let unique: std::collections::HashSet<&String> = names.iter().collect();
                assert_eq!(unique.len(), names.len(), "names repeat: {names:?}");
                for name in &names {
                    assert!(
                        dispatch_spacetime_module::AGENT_NAMES.contains(&name.as_str()),
                        "{name}"
                    );
                }
            }
        };
        let request_id = |outcome: ReducerOutcome| match outcome {
            ReducerOutcome::Applied(ids) if ids.len() == 1 => {
                dispatch_tui::models::DispatchRequestId(ids[0])
            }
            other => panic!("expected one read-back request id, got {other:?}"),
        };
        let patch_both = |task: TaskId, patch: bindings::TaskPatch| {
            let real = &real;
            let mem = &mem;
            async move {
                real.patch_task(task, patch.clone())
                    .await
                    .unwrap()
                    .applied()
                    .unwrap();
                mem.patch_task(task, patch)
                    .await
                    .unwrap()
                    .applied()
                    .unwrap();
            }
        };

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let mine = || bindings::Task {
            created_by: identity.clone(),
            owner: identity.clone(),
            ..blank_task_in_epic(0)
        };
        let task = real.create_task(mine()).await.unwrap();
        assert_eq!(task, mem.create_task(mine()).await.unwrap());
        let failing = real.create_task(mine()).await.unwrap();
        assert_eq!(failing, mem.create_task(mine()).await.unwrap());
        wait_for!(rows_real.task(failing).is_some());

        // -- StartAgentOnFreshLaunch, then the provisioned window ------------------
        let dispatch = |task: TaskId, mode: &'static str| {
            let real = &real;
            let mem = &mem;
            async move {
                let r = request_id(
                    real.request_dispatch(task, "host-a".into(), "user".into(), mode.into())
                        .await
                        .unwrap(),
                );
                let m = request_id(
                    mem.request_dispatch(task, "host-a".into(), "user".into(), mode.into())
                        .await
                        .unwrap(),
                );
                assert!(real
                    .claim_dispatch_request(r, "host-a".into())
                    .await
                    .unwrap()
                    .won());
                assert!(mem
                    .claim_dispatch_request(m, "host-a".into())
                    .await
                    .unwrap()
                    .won());
                (r, m)
            }
        };
        let (r, m) = dispatch(task, "research").await;
        wait_for!(rows_real.current_agent(task).is_some());
        same_agents();
        for (caller, id) in [
            (&real as &dyn ReducerCaller, r),
            (&mem as &dyn ReducerCaller, m),
        ] {
            assert!(caller
                .report_dispatch_provisioned(
                    id,
                    "host-a".into(),
                    "/repo/.worktrees/x".into(),
                    "task-x".into(),
                    false,
                )
                .await
                .unwrap()
                .won());
        }
        wait_for!(rows_real
            .current_agent(task)
            .is_some_and(|a| a.tmux_window.is_some()));
        same_agents();

        // -- A pause: the window clears, the agent stays current --------------------
        patch_both(
            task,
            bindings::TaskPatch {
                tmux_window: Some(String::new()),
                ..blank_task_patch()
            },
        )
        .await;
        wait_for!(rows_real
            .current_agent(task)
            .is_some_and(|a| a.tmux_window.is_none()));
        same_agents();

        // -- EndAgentWhenLaunchFails --------------------------------------------------
        let (r, m) = dispatch(failing, "").await;
        for (caller, id) in [
            (&real as &dyn ReducerCaller, r),
            (&mem as &dyn ReducerCaller, m),
        ] {
            assert!(caller
                .report_dispatch_failed(id, "host-a".into(), "boom".into())
                .await
                .unwrap()
                .won());
        }
        wait_for!(rows_real
            .agents()
            .iter()
            .any(|a| a.task_id == failing && !a.is_current()));
        same_agents();

        // -- EndAgentWhenWorktreeGoes (RetryFresh's pointer clear), the relaunch ---
        patch_both(
            task,
            bindings::TaskPatch {
                worktree: Some(String::new()),
                tmux_window: Some(String::new()),
                host: Some(String::new()),
                ..blank_task_patch()
            },
        )
        .await;
        wait_for!(rows_real.current_agent(task).is_none());
        same_agents();
        patch_both(
            task,
            bindings::TaskPatch {
                status: Some("backlog".into()),
                sub_status: Some("none".into()),
                ..blank_task_patch()
            },
        )
        .await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        dispatch(task, "").await;
        wait_for!(rows_real.current_agent(task).is_some());
        same_agents();

        // -- BackfillCurrentAgents: nothing to do on either side -----------------------
        let backfilled = instance.call("backfill_agents", &[]);
        assert!(backfilled.status.success(), "{}", describe(&backfilled));
        mem.backfill_agents().unwrap().applied().unwrap();
        same_agents();

        // -- AgentsGoWithTheirTask ------------------------------------------------------
        patch_both(
            task,
            bindings::TaskPatch {
                status: Some("done".into()),
                sub_status: Some("none".into()),
                ..blank_task_patch()
            },
        )
        .await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.status == TaskStatus::Done));
        real.delete_task(task).await.unwrap().applied().unwrap();
        mem.delete_task(task).await.unwrap().applied().unwrap();
        wait_for!(rows_real.agents().iter().all(|a| a.task_id != task));
        same_agents();
        assert_eq!(
            mem.handed_out_agent_names().len(),
            3,
            "deleted agents keep their names"
        );
    });
}
