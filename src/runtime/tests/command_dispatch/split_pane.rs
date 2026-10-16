use super::*;

// --- SplitCommand ------------------------------------------------------

#[tokio::test]
async fn dispatch_split_enter_opens_an_unowned_pane() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1\n"), // current_pane_id
        MockProcessRunner::ok_with_stdout(b"%2\n"), // split-window
    ]))
    .await;

    let extra = h.dispatch(Command::Split(SplitCommand::Enter)).await;

    assert!(extra.is_empty(), "Split arms queue no follow-on commands");
    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Split(crate::tui::messages::SplitMessage::EntryOpened { task_id: None, .. })
        ),
        "a bare Enter opens a pane owned by no task, got: {msg:?}"
    );
}

#[tokio::test]
async fn dispatch_split_enter_with_task_joins_that_task_window() {
    let mut h = harness(
        MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%1\n"), // current_pane_id
            MockProcessRunner::ok_with_stdout(b"%1 \n"), // companion_pane_ids: none
            MockProcessRunner::ok(),                    // join-pane
        ])
        .with_windows(&["task-1"]),
    )
    .await;

    h.dispatch(Command::Split(SplitCommand::EnterWithTask {
        task_id: TaskId(1),
        window: test_tmux_window("task-1"),
    }))
    .await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Split(crate::tui::messages::SplitMessage::EntryOpened {
                task_id: Some(TaskId(1)),
                ..
            })
        ),
        "the pane must come back owned by the task it was opened for, got: {msg:?}"
    );
    assert!(
        h.mock
            .flattened_calls()
            .iter()
            .any(|c| c.contains("join-pane")),
        "the task's window is joined in, not split afresh: {:?}",
        h.mock.flattened_calls()
    );
}

#[tokio::test]
async fn dispatch_split_exit_with_a_window_to_restore_breaks_the_pane_out() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // break-pane
    ]))
    .await;

    h.dispatch(Command::Split(SplitCommand::Exit {
        pane_id: "%2".into(),
        restore_window: Some(test_tmux_window("task-1")),
    }))
    .await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Split(crate::tui::messages::SplitMessage::PaneClosed)
        ),
        "Expected PaneClosed, got: {msg:?}"
    );
    // The distinction the arm carries: a pane with a window to go back to is
    // broken out, never killed — killing it would take the agent with it.
    let calls = h.mock.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("break-pane")),
        "expected break-pane, got: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("kill-pane")),
        "a restorable pane must not be killed, got: {calls:?}"
    );
}

#[tokio::test]
async fn dispatch_split_exit_without_a_window_kills_the_pane() {
    let mut h = harness(MockProcessRunner::new(vec![MockProcessRunner::ok()])).await;

    h.dispatch(Command::Split(SplitCommand::Exit {
        pane_id: "%2".into(),
        restore_window: None,
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Split(crate::tui::messages::SplitMessage::PaneClosed)
    ));
    assert!(
        h.mock
            .flattened_calls()
            .iter()
            .any(|c| c.contains("kill-pane")),
        "a pane with nowhere to go back to is killed, got: {:?}",
        h.mock.flattened_calls()
    );
}

#[tokio::test]
async fn dispatch_split_swap_hands_the_pane_to_the_incoming_task() {
    let mut h = harness(
        MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // swap-pane
            MockProcessRunner::ok(), // kill-window (no outgoing task)
        ])
        .with_windows(&["task-1"]),
    )
    .await;

    h.dispatch(Command::Split(SplitCommand::Swap {
        task_id: TaskId(1),
        new_window: test_tmux_window("task-1"),
        old_pane_id: "%2".into(),
        old_task: None,
    }))
    .await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Split(crate::tui::messages::SplitMessage::SwapOpened {
                task_id: TaskId(1),
                ..
            })
        ),
        "the swapped-in pane must be reported as the new task's, got: {msg:?}"
    );
    assert!(
        h.mock
            .flattened_calls()
            .iter()
            .any(|c| c.contains("swap-pane")),
        "got: {:?}",
        h.mock.flattened_calls()
    );
}

/// No message on success, so the runner announcement is the whole signal.
#[tokio::test]
async fn dispatch_split_focus_pane_selects_it() {
    let mut h = harness(MockProcessRunner::new(vec![MockProcessRunner::ok()])).await;

    h.dispatch(Command::Split(SplitCommand::FocusPane {
        pane_id: "%2".into(),
    }))
    .await;

    let call = h.await_call("select-pane").await;
    assert!(call.contains("%2"), "the focused pane must be ours: {call}");
}

#[tokio::test]
async fn dispatch_split_check_pane_exists_reports_a_pane_that_is_gone() {
    // A *successful* tmux call whose listing no longer holds %2 — real tmux
    // exits 0 for an unknown pane.
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1\n%7\n"),
    ]))
    .await;

    h.dispatch(Command::Split(SplitCommand::CheckPaneExists {
        pane_id: "%2".into(),
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Split(crate::tui::messages::SplitMessage::PaneClosed)
    ));
}

#[tokio::test]
async fn dispatch_split_respawn_pane_reports_one_it_could_not_revive() {
    let mut h = harness(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "no such pane",
    )]))
    .await;

    h.dispatch(Command::Split(SplitCommand::RespawnPane {
        pane_id: "%2".into(),
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Split(crate::tui::messages::SplitMessage::PaneClosed)
    ));
    // `PaneClosed` alone does not discriminate this arm from
    // `CheckPaneExists` — both report it on a failing tmux call. The call
    // itself must be the respawn attempt, not a pane-existence query.
    let calls = h.mock.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("respawn-pane")),
        "expected a respawn-pane call, got: {calls:?}"
    );
}
