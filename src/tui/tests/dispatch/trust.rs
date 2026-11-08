use super::*;

#[test]
fn dispatch_space_on_backlog_task_emits_trust_check_command() {
    // The trust check (`is_repo_trusted`, a `~/.claude.json` read) is file
    // I/O and must not run inline on the key-handling path — it is deferred
    // to a Command the runtime executes via spawn_blocking. handle_key must
    // not decide dispatch-vs-confirm-trust itself; it only emits the check.
    let mut app = make_app();
    // Navigate to task 1 (Backlog)
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::CheckTrustAndDispatch { id: TaskId(1), .. })
        )),
        "expected CheckTrustAndDispatch command, got {cmds:?}"
    );
    // No dispatch and no mode change happen synchronously — both depend on
    // the check command's result, which only the runtime can produce.
    assert!(
        cmds.iter()
            .all(|c| !matches!(c, Command::Task(TaskCommand::DispatchAgent { .. }))),
        "should not dispatch before the trust check runs"
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_y_in_trust_mode_emits_trust_and_dispatch() {
    let mut app = make_app();
    // Enter confirm mode directly to avoid depending on is_repo_trusted state
    app.input.mode = InputMode::ConfirmTrustRepo {
        task_id: TaskId(1),
        mode: DispatchMode::Dispatch,
    };
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('y'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::TrustAndDispatch { .. }))),
        "expected TrustAndDispatch command"
    );
}

#[test]
fn confirm_n_in_trust_mode_returns_to_normal() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmTrustRepo {
        task_id: TaskId(1),
        mode: DispatchMode::Dispatch,
    };
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        cmds.iter().all(|c| !matches!(c, Command::Task(_))),
        "expected no task command on cancel"
    );
}

#[test]
fn trust_and_dispatch_message_emits_trust_command() {
    let mut app = make_app();
    let cmds = without_usage(app.update(Message::Task(TaskMessage::TrustAndDispatch {
        id: TaskId(1),
        mode: DispatchMode::Dispatch,
    })));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::TrustAndDispatch { .. }))),
        "expected TrustAndDispatch command from message handler"
    );
}

#[test]
fn trust_check_untrusted_enters_confirm_trust_mode() {
    // What the runtime sends back when CheckTrustAndDispatch's spawn_blocking
    // read of ~/.claude.json finds the repo untrusted.
    let mut app = make_app();
    let cmds = without_usage(app.update(Message::Task(TaskMessage::TrustCheckUntrusted {
        id: TaskId(1),
        mode: DispatchMode::Dispatch,
        repo_path: "/repo".to_string(),
    })));
    assert!(cmds.is_empty());
    assert!(
        matches!(
            app.input.mode,
            InputMode::ConfirmTrustRepo {
                task_id: TaskId(1),
                ..
            }
        ),
        "expected ConfirmTrustRepo mode, got {:?}",
        app.input.mode
    );
    let msg = app.status.message.as_deref().expect("status set");
    assert!(msg.contains("/repo"), "got: {msg}");
}

#[test]
fn trust_check_untrusted_ignored_if_already_dispatching() {
    // Guards the async gap: if the task was dispatched (or otherwise no
    // longer a pending Backlog dispatch) by the time the check result
    // arrives, don't clobber whatever mode the UI is in now.
    let mut app = make_app();
    app.mark_dispatching(TaskId(1));
    app.input.mode = InputMode::Help;
    let cmds = without_usage(app.update(Message::Task(TaskMessage::TrustCheckUntrusted {
        id: TaskId(1),
        mode: DispatchMode::Dispatch,
        repo_path: "/repo".to_string(),
    })));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Help);
}

#[test]
fn confirm_y_in_trust_mode_emits_trust_and_quick_dispatch() {
    let mut app = make_app();
    let draft = TaskDraft {
        title: DEFAULT_QUICK_TASK_TITLE.to_string(),
        repo_path: "/my/repo".to_string(),
        ..Default::default()
    };
    app.input.mode = InputMode::ConfirmTrustRepoQuickDispatch {
        draft: draft.clone(),
        epic_id: None,
    };
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('y'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::TrustAndQuickDispatch {
                draft: d,
                epic_id: None,
            }) if d.repo_path == draft.repo_path
        )),
        "expected TrustAndQuickDispatch command, got {cmds:?}"
    );
}

#[test]
fn confirm_n_in_trust_mode_for_quick_dispatch_returns_to_normal() {
    let mut app = make_app();
    let draft = TaskDraft {
        title: DEFAULT_QUICK_TASK_TITLE.to_string(),
        repo_path: "/my/repo".to_string(),
        ..Default::default()
    };
    app.input.mode = InputMode::ConfirmTrustRepoQuickDispatch {
        draft,
        epic_id: None,
    };
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        cmds.iter().all(|c| !matches!(c, Command::Task(_))),
        "expected no task command on cancel"
    );
}

#[test]
fn trust_check_untrusted_for_quick_dispatch_enters_confirm_mode() {
    // What the runtime sends back when the QuickDispatch command's
    // spawn_blocking read of ~/.claude.json finds the repo untrusted.
    let mut app = make_app();
    let draft = TaskDraft {
        title: DEFAULT_QUICK_TASK_TITLE.to_string(),
        repo_path: "/my/repo".to_string(),
        ..Default::default()
    };
    let cmds = without_usage(app.update(Message::Task(
        TaskMessage::TrustCheckUntrustedForQuickDispatch {
            draft: draft.clone(),
            epic_id: None,
        },
    )));
    assert!(cmds.is_empty());
    assert!(
        matches!(
            &app.input.mode,
            InputMode::ConfirmTrustRepoQuickDispatch { draft: d, epic_id: None }
            if d.repo_path == draft.repo_path
        ),
        "expected ConfirmTrustRepoQuickDispatch mode, got {:?}",
        app.input.mode
    );
    let msg = app.status.message.as_deref().expect("status set");
    assert!(msg.contains("/my/repo"), "got: {msg}");
}

// -- The background trust check and grant report back as messages ------------

fn dispatch_flow() -> crate::tui::messages::TrustFlow {
    crate::tui::messages::TrustFlow::Dispatch {
        id: TaskId(1),
        mode: DispatchMode::Dispatch,
        repo_path: "/repo".into(),
    }
}

fn quick_flow() -> crate::tui::messages::TrustFlow {
    crate::tui::messages::TrustFlow::QuickDispatch {
        draft: TaskDraft {
            title: "Quick".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        epic_id: None,
    }
}

fn trust_checked(flow: crate::tui::messages::TrustFlow, result: Result<bool, String>) -> Message {
    Message::Task(crate::tui::messages::TaskMessage::TrustChecked { flow, result })
}

fn trust_granted(flow: crate::tui::messages::TrustFlow, result: Result<(), String>) -> Message {
    Message::Task(crate::tui::messages::TaskMessage::TrustGranted { flow, result })
}

#[test]
fn a_trusted_check_resumes_a_board_dispatch() {
    let mut app = make_app();
    let cmds = without_usage(app.update(trust_checked(dispatch_flow(), Ok(true))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::DispatchAgent { id: TaskId(1), .. })
        )),
        "expected DispatchAgent, got {cmds:?}"
    );
}

#[test]
fn an_untrusted_check_asks_before_a_board_dispatch() {
    let mut app = make_app();
    let cmds = without_usage(app.update(trust_checked(dispatch_flow(), Ok(false))));
    assert!(cmds.is_empty(), "nothing dispatches before the answer");
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmTrustRepo {
            task_id: TaskId(1),
            ..
        }
    ));
}

#[test]
fn a_trusted_check_resumes_a_quick_dispatch() {
    let mut app = make_app();
    let cmds = without_usage(app.update(trust_checked(quick_flow(), Ok(true))));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::QuickDispatchTrusted { .. }))),
        "expected QuickDispatchTrusted, got {cmds:?}"
    );
}

#[test]
fn an_untrusted_check_asks_before_a_quick_dispatch() {
    let mut app = make_app();
    let cmds = without_usage(app.update(trust_checked(quick_flow(), Ok(false))));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmTrustRepoQuickDispatch { .. }
    ));
}

#[test]
fn a_failed_check_is_reported_and_dispatches_nothing() {
    let mut app = make_app();
    let cmds = without_usage(app.update(trust_checked(dispatch_flow(), Err("io".into()))));
    assert!(cmds.is_empty());
    let status = app.status_message().unwrap_or_default();
    assert!(status.contains("Trust check failed: io"), "got {status:?}");
}

#[test]
fn a_granted_trust_resumes_each_flow() {
    let mut app = make_app();
    let cmds = without_usage(app.update(trust_granted(dispatch_flow(), Ok(()))));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(TaskCommand::DispatchAgent {
            id: TaskId(1),
            mode: Some(DispatchMode::Dispatch)
        })
    )));
    let cmds = without_usage(app.update(trust_granted(quick_flow(), Ok(()))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::QuickDispatchTrusted { .. }))));
}

#[test]
fn a_failed_grant_abandons_the_claim_and_raises_the_error() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    let cmds = without_usage(app.update(trust_granted(dispatch_flow(), Err("bad json".into()))));
    assert!(cmds.is_empty());
    assert!(!app.is_dispatching(TaskId(1)), "the claim is abandoned");
    let err = app.error_popup().unwrap_or_default();
    assert!(
        err.contains("Failed to trust repo: bad json"),
        "got {err:?}"
    );
}
