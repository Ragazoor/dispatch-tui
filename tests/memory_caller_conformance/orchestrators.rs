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
        assert_eq!(both!(start_orchestrator_session(id, host, false)), None);
        let refused = both!(start_orchestrator_session(id, host, false)).expect("already live");
        assert!(refused.contains("already running"), "{refused}");
        assert_eq!(both!(end_orchestrator_session(id, host, true)), None);
        assert_eq!(both!(start_orchestrator_session(id, host, false)), None);
        let restarted = Some((host.clone(), window.clone(), true, false, false));
        assert_eq!(orchestrator_of(&rows_mem, epic), restarted);
        wait_for!(orchestrator_of(&rows_real, epic) == restarted);
        assert_eq!(both!(end_orchestrator_session(id, host, true)), None);
        let refused = both!(start_orchestrator_session(id, host, false)).expect("died twice");
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
        assert_eq!(both!(start_orchestrator_session(id, host, false)), None);
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

/// What both sides keep of an epic's memory: the log as (kind, task, question,
/// text), the notes as (text, done) and the items as (task, about, open).
type Memory = (
    Vec<(String, i64, String, String)>,
    Vec<(String, bool)>,
    Vec<(i64, String, bool)>,
);

fn memory_of(rows: &SharedRows, epic: EpicId) -> Memory {
    let log = rows
        .orchestrator_log()
        .into_iter()
        .filter(|e| e.epic_id == epic.0)
        .map(|e| (e.kind, e.task_id, e.question, e.text))
        .collect();
    let notes = rows
        .epic_notes()
        .into_iter()
        .filter(|n| n.epic_id == epic.0)
        .map(|n| (n.text, n.done))
        .collect();
    let items = rows
        .waiting_items()
        .into_iter()
        .filter(|i| i.epic_id == epic.0)
        .map(|i| (i.task_id, i.about, i.resolved_at.is_empty()))
        .collect();
    (log, notes, items)
}

/// `orchestrators.allium`: an orchestrated epic's memory (its log, notes and
/// waiting items) and the rules that write and end it, on both sides alike:
/// the orchestrator's start and its entry, answers, items and their resolve,
/// notes, the summary, the session entries, the down item, a finished task
/// resolving its items, switch-off resolving the rest, and the epic's delete
/// removing it all.
#[test]
fn memory_caller_keeps_an_orchestrators_memory_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-orchestrator-memory");
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
                    (ReducerOutcome::Applied(r), ReducerOutcome::Applied(m)) => Ok((r, m)),
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
        let task_row = bindings::Task {
            created_by: identity.clone(),
            ..blank_task_in_epic(epic.0)
        };
        let first = create_task_both(&real, &mem, &rows_real, &rows_mem, task_row.clone()).await;
        let second = create_task_both(&real, &mem, &rows_real, &rows_mem, task_row).await;
        let host = "host-a".to_string();
        let none_rule: Option<String> = None;
        let none_max: Option<i64> = None;
        both!(switch_orchestrator_on(epic, none_rule, none_max)).expect("switch on");
        both!(ensure_orchestrator(epic, host)).expect("ensure");
        wait_for!(!rows_real.orchestrators().is_empty());
        let (o_real, o_mem) = (
            rows_real.orchestrators()[0].clone(),
            rows_mem.orchestrators()[0].clone(),
        );
        assert_eq!(o_real.id, o_mem.id);
        let id = o_real.id;

        // Each side's orchestrator has a name of its own, so a write that
        // names it is made on each side with its own name.
        macro_rules! each {
            ($call:ident ( $($arg:expr),* )) => {{
                let r = real.$call(o_real.name.clone(), $($arg.clone()),*).await.unwrap();
                let m = mem.$call(o_mem.name.clone(), $($arg.clone()),*).await.unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(r), ReducerOutcome::Applied(m)) => Ok((r, m)),
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words");
                        Err(r)
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }};
        }

        // The session's start and end write their entries; a second death in
        // a row raises one epic-wide item, however often it is asked.
        both!(start_orchestrator_session(id, host, true)).expect("start");
        both!(end_orchestrator_session(id, host, false)).expect("end");
        both!(raise_orchestrator_down_item(id, host)).expect("not down: nothing");
        for _ in 0..2 {
            both!(start_orchestrator_session(id, host, false)).expect("start");
            both!(end_orchestrator_session(id, host, true)).expect("died");
        }
        both!(raise_orchestrator_down_item(id, host)).expect("down");
        both!(raise_orchestrator_down_item(id, host)).expect("down, asked again");

        // A start of a task outside the epic is refused on both sides.
        let outside_epic = bindings::Epic {
            title: "Outside".into(),
            created_by: identity.clone(),
            ..blank_epic()
        };
        let other = real.create_epic(outside_epic.clone()).await.unwrap();
        assert_eq!(other, mem.create_epic(outside_epic).await.unwrap());
        wait_for!(rows_real.epic(other).is_some());
        let outside = create_task_both(
            &real,
            &mem,
            &rows_real,
            &rows_mem,
            bindings::Task {
                created_by: identity.clone(),
                ..blank_task_in_epic(other.0)
            },
        )
        .await;
        let wanted = format!("task {} is not in epic {}", outside.0, epic.0);
        for (caller, name) in [
            (&real as &dyn ReducerCaller, o_real.name.clone()),
            (&mem as &dyn ReducerCaller, o_mem.name.clone()),
        ] {
            match caller
                .orchestrator_request_dispatch(outside, host.clone(), name, "r".into())
                .await
                .unwrap()
            {
                ReducerOutcome::Refused(why) => assert!(why.contains(&wanted), "{why}"),
                other => panic!("an outside start must be refused: {other:?}"),
            }
        }

        // A start writes its reason in the request's transaction; a blank
        // reason is refused before anything is written.
        let blank = String::from(" ");
        let reason = String::from("first step of the plan");
        let r = real
            .orchestrator_request_dispatch(first, host.clone(), o_real.name.clone(), blank.clone())
            .await
            .unwrap();
        let m = mem
            .orchestrator_request_dispatch(first, host.clone(), o_mem.name.clone(), blank)
            .await
            .unwrap();
        assert_eq!(r, m);
        assert_eq!(
            r,
            ReducerOutcome::Refused("the reason must not be empty".into())
        );
        let r = real
            .orchestrator_request_dispatch(first, host.clone(), o_real.name.clone(), reason.clone())
            .await
            .unwrap();
        let m = mem
            .orchestrator_request_dispatch(first, host.clone(), o_mem.name.clone(), reason)
            .await
            .unwrap();
        assert!(r.won() && m.won(), "{r:?} {m:?}");

        let (question, answer, why) = (
            "rebase or pr?".to_string(),
            "rebase".to_string(),
            "the epic says so".to_string(),
        );
        each!(record_orchestrator_answer(first, question, answer, why)).expect("answer");
        let (about, tried, rec) = (
            "  may I drop the column?  ".to_string(),
            "read the migration".to_string(),
            "keep it".to_string(),
        );
        let (r_item, m_item) =
            each!(raise_waiting_item(second.0, about, tried, rec)).expect("raise");
        assert_eq!(r_item, m_item, "item ids are assigned alike");
        each!(raise_waiting_item(0_i64, about, tried, rec)).expect("raise an epic-wide item");
        let note = "v3 is next".to_string();
        let (r_note, m_note) = each!(write_epic_note(note)).expect("note");
        assert_eq!(r_note, m_note, "note ids are assigned alike");
        let (no_text, done): (Option<String>, Option<bool>) = (None, Some(true));
        each!(change_epic_note(r_note[0], no_text, done)).expect("tick");
        let (no_text, no_done): (Option<String>, Option<bool>) = (None, None);
        let refused = each!(change_epic_note(r_note[0], no_text, no_done)).expect_err("nothing");
        assert!(refused.contains("needs its text"), "{refused}");
        let stale = "a stale fact".to_string();
        let (r_stale, m_stale) = each!(write_epic_note(stale)).expect("a second note");
        assert_eq!(r_stale, m_stale);
        each!(remove_epic_note(r_stale[0])).expect("remove");
        let summary = "done".to_string();
        let refused = each!(record_orchestrator_summary(summary)).expect_err("not done");
        assert!(refused.contains("is not done"), "{refused}");

        both!(resolve_waiting_item(r_item[0])).expect("resolve");
        both!(resolve_waiting_item(r_item[0])).expect("resolve again changes nothing");

        // A finished task resolves its items; switch-off resolves the rest.
        let done_patch = bindings::TaskPatch {
            status: Some("done".into()),
            ..blank_task_patch()
        };
        both!(patch_task(second, done_patch)).expect("second done");

        let expected = memory_of(&rows_mem, epic);
        assert_eq!(expected.0.len(), 11, "{:?}", expected.0);
        assert_eq!(expected.1.len(), 1, "{:?}", expected.1);
        assert_eq!(expected.2.len(), 3, "{:?}", expected.2);
        wait_for!(memory_of(&rows_real, epic) == expected);

        both!(switch_orchestrator_off(epic)).expect("off");
        assert!(memory_of(&rows_mem, epic).2.iter().all(|i| !i.2));
        wait_for!(memory_of(&rows_real, epic) == memory_of(&rows_mem, epic));

        // The memory goes with the epic. Its tasks are finished first.
        both!(patch_task(first, done_patch)).expect("first done");
        both!(delete_task(first)).expect("delete first");
        both!(delete_task(second)).expect("delete second");
        both!(delete_epic(epic)).expect("delete epic");
        let gone = (vec![], vec![], vec![]);
        assert_eq!(memory_of(&rows_mem, epic), gone);
        wait_for!(memory_of(&rows_real, epic) == gone);
    });
}
