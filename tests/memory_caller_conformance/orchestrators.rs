use super::*;

/// The switch and the ready mark, as the module and the twin keep them: the
/// part of a row that is not a clock stamp.
fn switch_of(rows: &SharedRows, epic: EpicId) -> Option<(String, i64)> {
    rows.epic_orchestration(epic)
        .map(|o| (o.start_rule, o.max_parallel))
}

fn mark_of(rows: &SharedRows, task: TaskId) -> Option<(String, String)> {
    rows.ready_mark(task).map(|m| (m.marked_by, m.agent_name))
}

/// `orchestrators.allium: SwitchOrchestratorOn`, `ChangeOrchestratorSettings`,
/// `SwitchOrchestratorOff`; `epics.allium: ReparentingKeepsOneOrchestratorOnAPath`,
/// `DeleteEpic`; `tasks.allium: MarkTaskReady`, `ClearReadyMark`,
/// `LeavingBacklogDropsTheReadyMark`. Both sides switch, refuse in the same
/// words, and end the switch and the mark on the same writes.
#[test]
fn memory_caller_keeps_orchestrators_and_ready_marks_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-orchestrators");
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
            .with_identity_rules();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        register_profiles(&real, &mem, &accepted.identity).await;
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

        // One call on both sides: the same verdict, refusals in the same words.
        macro_rules! both {
            ($call:ident ( $($arg:expr),* )) => {{
                let r = real.$call($($arg.clone()),*).await.unwrap();
                let m = mem.$call($($arg.clone()),*).await.unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(_), ReducerOutcome::Applied(_)) => None,
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words");
                        Some(r)
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }};
        }

        let epic = |title: &str, parent: i64| bindings::Epic {
            title: title.into(),
            parent_epic_id: parent,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let make_epic = |title: &'static str, parent: EpicId| {
            let row = epic(title, parent.0);
            let real = &real;
            let mem = &mem;
            async move {
                let id = real.create_epic(row.clone()).await.unwrap();
                assert_eq!(id, mem.create_epic(row).await.unwrap());
                id
            }
        };
        let root = make_epic("Root", EpicId(0)).await;
        let sub = make_epic("Sub", root).await;
        let other = make_epic("Other", EpicId(0)).await;
        let doomed = make_epic("Doomed", EpicId(0)).await;
        wait_for!(rows_real.epic(doomed).is_some());

        let none_rule: Option<String> = None;
        let none_max: Option<i64> = None;

        // Switch on, with the defaults, and the refusals.
        assert_eq!(
            both!(switch_orchestrator_on(sub, none_rule, none_max)),
            None
        );
        let below = both!(switch_orchestrator_on(root, none_rule, none_max))
            .expect("an orchestrator below refuses");
        assert!(below.contains("below this one"), "{below}");
        assert!(both!(switch_orchestrator_on(sub, none_rule, none_max)).is_some());
        assert!(both!(switch_orchestrator_on(other, none_rule, Some(0_i64))).is_some());
        assert!(both!(switch_orchestrator_on(
            other,
            Some("sometimes".to_string()),
            none_max
        ))
        .is_some());
        assert_eq!(switch_of(&rows_mem, sub), Some(("marked_only".into(), 0)));
        wait_for!(switch_of(&rows_real, sub) == Some(("marked_only".into(), 0)));

        // Settings change only what is named.
        assert_eq!(
            both!(change_orchestrator_settings(
                sub,
                Some("everything".to_string()),
                Some(4_i64)
            )),
            None
        );
        assert!(both!(change_orchestrator_settings(other, none_rule, Some(2_i64))).is_some());
        assert_eq!(switch_of(&rows_mem, sub), Some(("everything".into(), 4)));
        wait_for!(switch_of(&rows_real, sub) == Some(("everything".into(), 4)));

        // The ready mark.
        let task_row = bindings::Task {
            created_by: identity.clone(),
            ..blank_task_in_epic(other.0)
        };
        let task = real.create_task(task_row.clone()).await.unwrap();
        assert_eq!(task, mem.create_task(task_row).await.unwrap());
        wait_for!(rows_real.task(task).is_some());

        assert!(both!(mark_task_ready(task, "agent".to_string(), String::new())).is_some());
        assert!(both!(mark_task_ready(
            task,
            "operator".to_string(),
            "folke3".to_string()
        ))
        .is_some());
        assert!(both!(mark_task_ready(
            TaskId(9_999_999),
            "operator".to_string(),
            String::new()
        ))
        .is_some());
        assert_eq!(
            both!(mark_task_ready(
                task,
                "agent".to_string(),
                "folke3".to_string()
            )),
            None
        );
        assert_eq!(
            both!(mark_task_ready(task, "operator".to_string(), String::new())),
            None
        );
        let marked = Some(("agent".to_string(), "folke3".to_string()));
        assert_eq!(mark_of(&rows_mem, task), marked, "the first marker stays");
        wait_for!(mark_of(&rows_real, task) == marked);

        assert_eq!(both!(clear_ready_mark(task)), None);
        assert_eq!(
            both!(clear_ready_mark(task)),
            None,
            "clearing twice is no error"
        );
        assert_eq!(mark_of(&rows_mem, task), None);
        wait_for!(mark_of(&rows_real, task).is_none());

        // Leaving backlog drops the mark; a task outside backlog is refused.
        assert_eq!(
            both!(mark_task_ready(task, "operator".to_string(), String::new())),
            None
        );
        wait_for!(mark_of(&rows_real, task).is_some());
        let to_review = bindings::TaskPatch {
            status: Some("review".into()),
            ..blank_task_patch()
        };
        assert_eq!(both!(patch_task(task, to_review)), None);
        assert_eq!(mark_of(&rows_mem, task), None);
        wait_for!(mark_of(&rows_real, task).is_none());
        let refused = both!(mark_task_ready(task, "operator".to_string(), String::new()))
            .expect("a review task is refused");
        assert_eq!(refused, "only a backlog task can be marked ready");

        // A re-parent under an orchestrated epic switches the moved switch off.
        assert_eq!(
            both!(switch_orchestrator_on(other, none_rule, none_max)),
            None
        );
        wait_for!(switch_of(&rows_real, other).is_some());
        let under_sub = bindings::EpicPatch {
            parent_epic_id: Some(sub.0),
            ..blank_epic_patch()
        };
        assert_eq!(both!(patch_epic(other, under_sub)), None);
        assert_eq!(switch_of(&rows_mem, other), None);
        wait_for!(switch_of(&rows_real, other).is_none());
        assert!(switch_of(&rows_mem, sub).is_some(), "the one above stays");

        // Switch off.
        assert_eq!(both!(switch_orchestrator_off(sub)), None);
        let off_again = both!(switch_orchestrator_off(sub)).expect("nothing to switch off");
        assert_eq!(off_again, format!("epic {} has no orchestrator", sub.0));
        assert_eq!(switch_of(&rows_mem, sub), None);
        wait_for!(switch_of(&rows_real, sub).is_none());

        // A delete takes the switch with the epic.
        assert_eq!(
            both!(switch_orchestrator_on(doomed, none_rule, none_max)),
            None
        );
        wait_for!(switch_of(&rows_real, doomed).is_some());
        assert_eq!(both!(delete_epic(doomed)), None);
        assert_eq!(switch_of(&rows_mem, doomed), None);
        wait_for!(switch_of(&rows_real, doomed).is_none() && rows_real.epic(doomed).is_none());
    });
}

/// An orchestrator row as the module and the twin keep it, without its id,
/// name and clock stamps: (host, window, restarted_once, session_died,
/// has a turn).
fn orchestrator_of(rows: &SharedRows, epic: EpicId) -> Option<(String, String, bool, bool, bool)> {
    rows.orchestrators()
        .into_iter()
        .find(|o| o.epic_id == epic.0)
        .map(|o| {
            (
                o.host,
                o.tmux_window,
                o.restarted_once,
                o.session_died,
                !o.last_turn_at.is_empty(),
            )
        })
}

/// `orchestrators.allium: BoardRunsItsOrchestrators`,
/// `StartOrchestratorSession`, `OrchestratorWindowGone`,
/// `OrchestratorTurnClearsTheRestartFlag`, `EndOrchestratorSession`,
/// `SwitchOrchestratorOff`. Both sides create one row per epic and host,
/// spend and give back the one restart alike, refuse in the same words, and
/// remove the row with the switch.
#[test]
fn memory_caller_runs_orchestrator_sessions_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-orchestrator-sessions");
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
            .with_identity_rules();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        register_profiles(&real, &mem, &accepted.identity).await;
        let identity = accepted.identity.clone();
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
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
        macro_rules! both {
            ($call:ident ( $($arg:expr),* )) => {{
                let r = real.$call($($arg.clone()),*).await.unwrap();
                let m = mem.$call($($arg.clone()),*).await.unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(_), ReducerOutcome::Applied(_)) => None,
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words");
                        Some(r)
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }};
        }

        let row = bindings::Epic {
            title: "E".into(),
            created_by: identity.clone(),
            ..blank_epic()
        };
        let epic = real.create_epic(row.clone()).await.unwrap();
        assert_eq!(epic, mem.create_epic(row).await.unwrap());
        wait_for!(rows_real.epic(epic).is_some());
        let host = "host-a".to_string();

        let refused = both!(ensure_orchestrator(epic, host)).expect("no switch");
        assert_eq!(refused, format!("epic {} has no orchestrator", epic.0));

        let none_rule: Option<String> = None;
        let none_max: Option<i64> = None;
        assert_eq!(
            both!(switch_orchestrator_on(epic, none_rule, none_max)),
            None
        );
        assert_eq!(both!(ensure_orchestrator(epic, host)), None);
        assert_eq!(
            both!(ensure_orchestrator(epic, host)),
            None,
            "a second ensure is no error"
        );
        wait_for!(orchestrator_of(&rows_real, epic).is_some());
        let fresh = Some((host.clone(), String::new(), false, false, false));
        assert_eq!(orchestrator_of(&rows_mem, epic), fresh);
        assert_eq!(orchestrator_of(&rows_real, epic), fresh);
        assert_eq!(rows_mem.orchestrators().len(), 1);

        let id_of = |rows: &SharedRows| rows.orchestrators()[0].id;
        let (id_real, id_mem) = (id_of(&rows_real), id_of(&rows_mem));
        assert_eq!(id_real, id_mem, "ids are assigned alike");
        let id = id_real;
        let window = format!("orch-{}", epic.0);

        // Start, die, the one restart, die again: the budget is spent.
        assert_eq!(both!(start_orchestrator_session(id, host)), None);
        let refused = both!(start_orchestrator_session(id, host)).expect("already live");
        assert!(refused.contains("already running"), "{refused}");
        assert_eq!(both!(end_orchestrator_session(id, host, true)), None);
        assert_eq!(both!(start_orchestrator_session(id, host)), None);
        let restarted = Some((host.clone(), window.clone(), true, false, false));
        assert_eq!(orchestrator_of(&rows_mem, epic), restarted);
        wait_for!(orchestrator_of(&rows_real, epic) == restarted);
        assert_eq!(both!(end_orchestrator_session(id, host, true)), None);
        let refused = both!(start_orchestrator_session(id, host)).expect("died twice");
        assert!(refused.contains("died twice in a row"), "{refused}");

        // A turn gives the restart back.
        let name_real = rows_real.orchestrators()[0].name.clone();
        let name_mem = rows_mem.orchestrators()[0].name.clone();
        assert!(real
            .orchestrator_turn_completed(name_real)
            .await
            .unwrap()
            .applied()
            .is_ok());
        assert!(mem
            .orchestrator_turn_completed(name_mem)
            .await
            .unwrap()
            .applied()
            .is_ok());
        let after_turn = Some((host.clone(), String::new(), false, true, true));
        assert_eq!(orchestrator_of(&rows_mem, epic), after_turn);
        wait_for!(orchestrator_of(&rows_real, epic) == after_turn);
        assert_eq!(both!(start_orchestrator_session(id, host)), None);
        assert_eq!(both!(end_orchestrator_session(id, host, false)), None);
        let refused = both!(end_orchestrator_session(id, host, false)).expect("not running");
        assert!(refused.contains("is not running"), "{refused}");
        let refused = both!(orchestrator_turn_completed("nobody".to_string())).expect("unknown");
        assert_eq!(refused, "no orchestrator is named nobody");

        // The switch takes the row with it.
        assert_eq!(both!(switch_orchestrator_off(epic)), None);
        assert!(rows_mem.orchestrators().is_empty());
        wait_for!(rows_real.orchestrators().is_empty());
    });
}

/// `store-split.allium: RequestDispatch`'s limit in an orchestrated epic
/// (`orchestrators.allium: running_here`), and its one-winner claim. Both
/// sides stop a burst at the limit with the same words, give the slot back
/// when a request fails, and let only one of two machines start one task.
#[test]
fn memory_caller_holds_starts_to_the_limit_like_the_real_module() {
    use dispatch_tui::models::DispatchRequestId;

    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-start-limit");
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
            .with_identity_rules();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        register_profiles(&real, &mem, &accepted.identity).await;
        let identity = accepted.identity.clone();
        for host in ["host-a", "host-b"] {
            for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
                caller
                    .register_host(host.into(), String::new(), identity.clone())
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
        macro_rules! both {
            ($call:ident ( $($arg:expr),* )) => {{
                let r = real.$call($($arg.clone()),*).await.unwrap();
                let m = mem.$call($($arg.clone()),*).await.unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(r), ReducerOutcome::Applied(m)) => {
                        Ok((r, m))
                    }
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words");
                        Err(r)
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }};
        }

        let row = bindings::Epic {
            title: "E".into(),
            created_by: identity.clone(),
            ..blank_epic()
        };
        let epic = real.create_epic(row.clone()).await.unwrap();
        assert_eq!(epic, mem.create_epic(row).await.unwrap());
        wait_for!(rows_real.epic(epic).is_some());
        let none_rule: Option<String> = None;
        both!(switch_orchestrator_on(epic, none_rule, Some(1_i64))).expect("switch on");

        let task_row = bindings::Task {
            created_by: identity.clone(),
            ..blank_task_in_epic(epic.0)
        };
        let first = create_task_both(&real, &mem, &rows_real, &rows_mem, task_row.clone()).await;
        let second = create_task_both(&real, &mem, &rows_real, &rows_mem, task_row).await;
        let (host_a, host_b, mcp, derive) = (
            "host-a".to_string(),
            "host-b".to_string(),
            "mcp".to_string(),
            String::new(),
        );

        // A burst stops at the limit, on both sides, in the same words.
        let (r_ids, m_ids) =
            both!(request_dispatch(first, host_a, mcp, derive)).expect("the first start");
        let refused = both!(request_dispatch(second, host_a, mcp, derive))
            .expect_err("the epic is at its limit");
        assert_eq!(
            refused,
            format!("epic {} already runs 1 of 1 tasks", epic.0)
        );

        // Another machine has its own slots; but the first claim on a task
        // wins, so host b cannot start the task host a holds.
        assert!(both!(request_dispatch(first, host_b, mcp, derive)).is_err());

        // A failed request gives the slot back.
        let (r_req, m_req) = (DispatchRequestId(r_ids[0]), DispatchRequestId(m_ids[0]));
        wait_for!(rows_real.dispatch_request(r_req).is_some());
        for (caller, req) in [
            (&real as &dyn ReducerCaller, r_req),
            (&mem as &dyn ReducerCaller, m_req),
        ] {
            caller
                .claim_dispatch_request(req, host_a.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
            caller
                .report_dispatch_failed(req, host_a.clone(), "boom".into())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        wait_for!(rows_real
            .dispatch_request(r_req)
            .is_some_and(|r| !r.is_open()));
        both!(request_dispatch(second, host_a, mcp, derive)).expect("the slot is free again");
    });
}
