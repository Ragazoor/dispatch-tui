use super::*;
use crate::models::{test_tmux_window, SubStatus, TaskId, TaskStatus};
use crossterm::event::KeyCode;

#[test]
fn split_pane_opened_resets_focused_to_true() {
    let mut app = make_app();
    // Simulate having lost focus before entering split
    app.board.split.focused = false;
    // An entry in flight, not a bare pane report: claiming focus is the
    // entry's settle, and `SwapOpened` deliberately does not do it.
    app.board.split.in_flight = Some(InFlight::entry());

    let _cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::EntryOpened {
            pane_id: "pane1".to_string(),
            task_id: None,
        },
    ));
    assert!(app.split_active());
    assert!(app.split_focused());
}

#[test]
fn split_pane_closed_resets_focused_to_true() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("pane1".to_string());
    app.board.split.focused = false;

    let _cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(!app.split_active());
    assert!(app.split_focused());
}

#[test]
fn toggle_split_mode_emits_enter_command() {
    let mut app = make_app();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('s'))));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::Split(crate::tui::commands::SplitCommand::Enter)
    ));
}

#[test]
fn toggle_split_mode_emits_exit_command() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = None;
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('s'))));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Split(crate::tui::commands::SplitCommand::Exit { pane_id, restore_window }) if pane_id == "%42" && restore_window.is_none())
    );
}

#[test]
fn toggle_split_exit_restores_pinned_task_window() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-3"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('s'))));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Split(crate::tui::commands::SplitCommand::Exit { pane_id, restore_window }) if pane_id == "%42" && restore_window.as_ref().map(|w| w.as_str()) == Some("task-3"))
    );
}

#[test]
fn capital_s_is_inert_outside_split_mode() {
    // [S] was retired; Space now owns the swap. The key must have no arm at
    // all — no commands, no status hint, no mode change.
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2);
    let mode_before = app.input.mode.clone();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert!(cmds.is_empty(), "S must be inert, got {cmds:?}");
    assert!(
        app.status.message.is_none(),
        "S must not show a hint, got {:?}",
        app.status.message
    );
    assert_eq!(app.input.mode, mode_before);
}

#[test]
fn capital_s_is_inert_in_split_mode() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.selection_mut().set_column(2);
    let mode_before = app.input.mode.clone();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert!(
        cmds.is_empty(),
        "S must be inert in split mode too, got {cmds:?}"
    );
    assert_eq!(app.input.mode, mode_before);
}

#[test]
fn space_in_split_mode_emits_swap_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    // No pinned task — different from already-pinned case
    app.selection_mut().set_column(2);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::Split(crate::tui::commands::SplitCommand::Swap {
            task_id,
            new_window,
            ..
        }) if *task_id == TaskId(4) && new_window == "task-4"
    ));
}

#[test]
fn space_in_split_mode_never_jumps_to_a_window() {
    // The whole point of the rebinding: with the pane open, Space brings the
    // agent to the board rather than taking the user away to its window.
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.selection_mut().set_column(2);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::JumpToTmux { .. })
        )),
        "Space must not jump while split is active, got {cmds:?}"
    );
}

#[test]
fn space_in_split_mode_on_backlog_task_still_dispatches() {
    // Split mode overrides only the jump branch. A task with no window has
    // nothing to swap in, so the status routing is untouched.
    let task = make_task(4, TaskStatus::Backlog);
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.selection_mut().set_column(1);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::CheckTrustAndDispatch { id, .. })
                if *id == TaskId(4)
        )),
        "Space on a Backlog card must still dispatch in split mode, got {cmds:?}"
    );
}

#[test]
fn space_without_split_mode_emits_jump_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::JumpToTmux { window }) if window == "task-4"
    )));
}

#[test]
fn space_on_pinned_split_task_emits_focus_split_pane() {
    // When the selected task IS the pinned split-pane task, its standalone
    // window no longer exists — [space] must focus the right pane instead.
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));
    app.selection_mut().set_column(2); // Running column
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::FocusPane { pane_id }) if pane_id == "%42")),
        "expected Split(FocusPane {{pane_id: \"%42\"}}), got {:?}",
        cmds
    );
    // Priority 1 must win over the split-mode swap branch below it.
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::Swap { .. })
        )),
        "the pinned task must not be swapped in again, got {cmds:?}"
    );
}

#[test]
fn space_on_non_pinned_task_in_split_mode_swaps_it_in() {
    // When split is active but the selected task is NOT the pinned one,
    // [space] swaps that task's window into the pane, replacing the one
    // currently shown — it does not jump to the standalone window.
    let mut task1 = make_task(3, TaskStatus::Running);
    task1.tmux_window = Some(test_tmux_window("task-3"));
    let mut task2 = make_task(4, TaskStatus::Running);
    task2.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task1, task2]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3)); // task3 is pinned, not task4
                                                      // Navigate to Running column and select task4 (row 1, second in column)
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 1);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::Swap {
                task_id,
                new_window,
                old_task,
                ..
            }) if *task_id == TaskId(4)
                && new_window == "task-4"
                && old_task.as_ref().map(|(w, wt)| (w.as_str(), wt.as_str()))
                    == Some(("task-3", "/repo/.worktrees/3-task-3"))
        )),
        "expected Swap for non-pinned task, got {cmds:?}"
    );
}

#[test]
fn split_pane_opened_updates_state() {
    let mut app = make_app();
    assert!(!app.board.split.active);
    app.board.split.in_flight = Some(InFlight::entry());
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EntryOpened {
            pane_id: "%42".to_string(),
            task_id: Some(TaskId(3)),
        },
    ));
    assert!(app.board.split.active);
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%42"));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
}

/// `SplitPaneEntrySettles` and `SplitPaneSwapSettles` are each gated on their
/// own rearrangement being in flight, so a pane report belonging to neither
/// must touch nothing at all — the same shape as
/// `an_enter_failure_with_no_entry_in_flight_still_reports` below, minus the
/// report, because a success nobody asked for has nothing to say.
///
/// Unreachable from the runtime, which only reports a pane after the board has
/// marked the rearrangement in flight. Asserted because that guard is what
/// makes it unreachable, and a guard nothing exercises is a guard a later edit
/// can drop without noticing.
#[test]
fn a_pane_report_with_no_rearrangement_in_flight_changes_nothing() {
    for message in [
        crate::tui::messages::SplitMessage::EntryOpened {
            pane_id: "%42".to_string(),
            task_id: Some(TaskId(3)),
        },
        crate::tui::messages::SplitMessage::SwapOpened {
            pane_id: "%42".to_string(),
            task_id: TaskId(3),
        },
    ] {
        let mut app = make_app();
        let cmds = app.update(Message::Split(message.clone()));
        assert!(cmds.is_empty(), "expected no commands for {message:?}");
        assert!(!app.board.split.active, "{message:?}");
        assert_eq!(app.board.split.right_pane_id, None, "{message:?}");
        assert_eq!(app.board.split.pinned_task_id, None, "{message:?}");
    }
}

/// `SwapSplitPane` and `PinTaskInSplitPane` both require nothing in flight,
/// and `DeferSwapWhileSwapInFlight` holds a request only during a *swap*. A
/// request arriving during an ENTRY therefore matches no rule and must do
/// nothing — in particular it must not start a swap, which would overwrite the
/// entry and leave the entry's own report settling the swap instead.
///
/// Unreachable in practice: a swap request is only raised while split mode is
/// active, and an entry runs while it is not. Asserted because the parallel
/// booleans this replaced had no way to express the refusal at all — the
/// in-flight check was a check on the *swap* flag, so an entry fell straight
/// through it.
#[test]
fn a_swap_request_during_an_entry_does_nothing() {
    let mut app = app_in_split_mode(3);
    app.board.split.in_flight = Some(InFlight::entry());
    let cmds = app.update(Message::Split(crate::tui::messages::SplitMessage::Swap(
        TaskId(4),
    )));
    assert!(cmds.is_empty(), "expected no commands, got {cmds:?}");
    assert!(
        entry_in_flight(&app.board.split),
        "the entry must survive the refused swap"
    );
    assert_eq!(pending_swap(&app.board.split), None);
}

/// The other half of the same guard: a report is matched against the
/// rearrangement it names, not against whichever one happens to be in flight.
///
/// Before the two success variants existed, one `PaneOpened` ran both settles
/// back to back and let each one's flag decide which was addressed — so a
/// report could only ever be attributed by guessing. Unreachable too (an entry
/// and a swap never overlap), and asserted for the same reason.
#[test]
fn a_swap_report_does_not_settle_an_entry() {
    let mut app = make_app();
    app.board.split.in_flight = Some(InFlight::entry());
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::SwapOpened {
            pane_id: "%42".to_string(),
            task_id: TaskId(3),
        },
    ));
    assert!(cmds.is_empty(), "expected no commands, got {cmds:?}");
    assert!(
        entry_in_flight(&app.board.split),
        "the entry must still be waiting for its own report"
    );
    assert!(!app.board.split.active);
}

/// split-pane.allium's `RefuseExitSplitModeOntoLiveWindow`: a refused exit
/// changes nothing, and "pressing [s] again re-attempts the exit" is only true
/// if the board still knows which pane to break out. Issuing the Exit command
/// must therefore not consume `right_pane_id` — only `PaneClosed`, the
/// confirmation that tmux actually did it, clears the split state.
///
/// Consuming it optimistically wedged split mode for the rest of the session:
/// with the id gone, a second [s] returns no command at all, a swap bails on
/// its missing pane id, and the liveness poll that would emit `PaneClosed` is
/// never raised — leaving split mode active over a live orphan pane.
#[test]
fn a_toggle_that_exits_split_mode_keeps_the_pane_id_until_tmux_confirms() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-3"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));

    // Compared through Debug so a field added later is covered without also
    // adding Clone/PartialEq to a production type for one test's benefit.
    let before = format!("{:?}", app.board.split);
    let cmds = app.handle_toggle_split_mode();

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::Exit { pane_id, .. })
                if pane_id == "%42"
        )),
        "the exit must carry the pane id, got: {cmds:?}"
    );
    // The whole state, not just the pane id: the next field added to the exit
    // path will not carry the `clone()`-not-`take()` comment, and a refused
    // exit has to leave every part of the split untouched for the re-attempt
    // to mean anything.
    assert_eq!(
        format!("{:?}", app.board.split),
        before,
        "issuing the exit must not mutate SplitState — only PaneClosed may"
    );
}

#[test]
fn split_pane_closed_resets_state() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(!app.board.split.active);
    assert!(app.board.split.right_pane_id.is_none());
    assert!(app.board.split.pinned_task_id.is_none());
}

#[test]
fn tick_checks_window_for_non_pinned_tasks_in_split_mode() {
    let mut task3 = make_task(3, TaskStatus::Running);
    task3.tmux_window = Some(test_tmux_window("task-3"));
    let mut task4 = make_task(4, TaskStatus::Running);
    task4.tmux_window = Some(test_tmux_window("task-4"));
    let mut app = App::new(vec![task3, task4]);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));

    // Task 3 (not pinned) must appear in the batch; task 4 (pinned) must not.
    let check_included = |id: TaskId| {
        cmds.iter().any(|c| {
            if let Command::Task(crate::tui::commands::TaskCommand::BatchCheckWindows { windows }) =
                c
            {
                windows.iter().any(|(wid, _)| *wid == id)
            } else {
                false
            }
        })
    };
    assert!(
        check_included(TaskId(3)),
        "task 3 (not pinned) must be in the batch"
    );
    assert!(
        !check_included(TaskId(4)),
        "task 4 (pinned) must NOT be in the batch"
    );
}

#[test]
fn toggle_split_with_selected_tmux_task_emits_enter_with_task() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-3"));
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('s'))));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::Split(crate::tui::commands::SplitCommand::EnterWithTask { task_id, window })
            if *task_id == TaskId(3) && window == "task-3"
    ));
}

#[test]
fn toggle_split_without_tmux_task_emits_plain_enter() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column, task has no tmux_window
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('s'))));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::Split(crate::tui::commands::SplitCommand::Enter)
    ));
}

#[test]
fn toggle_split_no_selection_emits_plain_enter() {
    // make_app has tasks but default selection is on Backlog column — task 1 has no tmux_window
    let mut app = make_app();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('s'))));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::Split(crate::tui::commands::SplitCommand::Enter)
    ));
}

#[test]
fn handle_key_normal_toggle_split_mode() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('s')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::Enter))));
}

#[test]
fn confirm_quit_with_active_split_emits_exit_split_mode() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-3"));
    let mut app = App::new(vec![task]);

    // Set up active split with a pinned task
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));

    let cmds = confirm_quit(&mut app);

    assert!(app.should_quit);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::Exit {
                pane_id,
                restore_window: Some(w),
            }) if pane_id == "%42" && w == "task-3"
        )),
        "should emit Split(Exit) to restore task window before quitting"
    );
}

// `TaskMessage::FinishComplete` no longer exists — the TUI wrap-up entry
// point (`W`) is gone, so tmux_window can no longer be cleared by a
// board-driven finish. `ConfirmDone` (Review -> Done, e.g. via `L`) is one
// of the surviving paths that clears tmux_window and exercises the same
// `SplitPaneRespawnOnWindowCleared` rule (docs/specs/split-pane.allium).

#[test]
fn confirm_done_respawns_split_pane_for_pinned_task() {
    let mut app = App::new(vec![{
        let mut t = make_task(1, TaskStatus::Review);
        t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
        t.tmux_window = Some(test_tmux_window("task-1"));
        t
    }]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    }));
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::ConfirmDone,
    ));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::RespawnPane { pane_id }) if pane_id == "%5")),
        "should emit RespawnSplitPane for the pinned pane"
    );
    assert_eq!(
        app.board.split.pinned_task_id, None,
        "pinned_task_id should be cleared"
    );
    assert!(app.board.split.active, "split mode should remain active");
    assert_eq!(
        app.board.split.right_pane_id.as_deref(),
        Some("%5"),
        "pane_id should be preserved"
    );
}

#[test]
fn confirm_done_no_respawn_for_non_pinned_task() {
    let mut app = App::new(vec![
        {
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some(test_tmux_window("task-1"));
            t
        },
        {
            let mut t = make_task(2, TaskStatus::Running);
            t.tmux_window = Some(test_tmux_window("task-2"));
            t
        },
    ]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(2));

    app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    }));
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::ConfirmDone,
    ));

    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::RespawnPane { .. })
        )),
        "should NOT respawn when a different task finishes"
    );
    assert_eq!(
        app.board.split.pinned_task_id,
        Some(TaskId(2)),
        "pinned task should be unchanged"
    );
}

#[test]
fn confirm_done_no_respawn_without_split() {
    let mut app = App::new(vec![{
        let mut t = make_task(1, TaskStatus::Review);
        t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
        t.tmux_window = Some(test_tmux_window("task-1"));
        t
    }]);
    // split is NOT active (default)

    app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    }));
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::ConfirmDone,
    ));

    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::RespawnPane { .. })
        )),
        "should NOT respawn when split mode is inactive"
    );
}

#[test]
fn pr_merged_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some(test_tmux_window("task-1"));
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::Pr(crate::tui::messages::PrMessage::Merged(
        TaskId(1),
    )));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::RespawnPane { pane_id }) if pane_id == "%5")),
        "should respawn split pane when pinned task's PR is merged"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn confirm_done_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));
    app.prompt_move_to_done(vec![TaskId(1)]);

    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::ConfirmDone,
    ));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::RespawnPane { pane_id }) if pane_id == "%5")),
        "should respawn split pane when pinned task is confirmed done"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn archive_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Done);
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Archive(
        TaskId(1),
    )));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::RespawnPane { pane_id }) if pane_id == "%5")),
        "should respawn split pane when pinned task is archived"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn retry_resume_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-1"));
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.sub_status = SubStatus::Crashed;
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));
    app.input.mode = InputMode::ConfirmRetry(TaskId(1));

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::RetryResume(TaskId(1)),
    ));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Split(crate::tui::commands::SplitCommand::RespawnPane { pane_id }) if pane_id == "%5")),
        "should respawn split pane when pinned task is retried"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn confirm_quit_with_split_no_pinned_task_kills_pane() {
    let mut app = make_app();

    // Split active but no pinned task (empty split)
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%99".to_string());
    app.board.split.pinned_task_id = None;

    let cmds = confirm_quit(&mut app);

    assert!(app.should_quit);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::Exit {
                pane_id,
                restore_window: None,
            }) if pane_id == "%99"
        )),
        "should emit Split(Exit) with no restore_window for empty split"
    );
}

// ---------------------------------------------------------------------------
// Swap serialisation (docs/specs/split-pane.allium:
// DeferSwapWhileSwapInFlight, SplitPaneSwapSettles)
// ---------------------------------------------------------------------------

/// Three Running tasks with windows, split mode active with `pinned` pinned.
fn app_in_split_mode(pinned: i64) -> App {
    let tasks = [3, 4, 5]
        .into_iter()
        .map(|id| {
            let mut t = make_task(id, TaskStatus::Running);
            t.tmux_window = Some(test_tmux_window(&format!("task-{id}")));
            t
        })
        .collect();
    let mut app = App::new(tasks);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(pinned));
    app
}

fn swap(app: &mut App, id: i64) -> Vec<Command> {
    app.update(Message::Split(crate::tui::messages::SplitMessage::Swap(
        TaskId(id),
    )))
}

/// The successful report a swap settles on. Mirrors `entry_settles` on the
/// entry side — the two are the same message, told apart by which
/// rearrangement is in flight.
/// How many exits a settle issued. A held toggle and a held quit must exit
/// once between them, not twice.
fn exit_count(cmds: &[Command]) -> usize {
    cmds.iter()
        .filter(|c| {
            matches!(
                c,
                Command::Split(crate::tui::commands::SplitCommand::Exit { .. })
            )
        })
        .count()
}

fn swap_settles(app: &mut App, id: i64) -> Vec<Command> {
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::SwapOpened {
            pane_id: "%77".to_string(),
            task_id: TaskId(id),
        },
    ))
}

/// The failing report a swap settles on. The pane keeps showing whatever it
/// showed before, but the swap settles all the same.
fn swap_fails(app: &mut App) -> Vec<Command> {
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::SwapFailed {
            error: "Swap failed: rename window failed".to_string(),
        },
    ))
}

fn swap_target(cmds: &[Command]) -> Option<TaskId> {
    cmds.iter().find_map(|c| match c {
        Command::Split(crate::tui::commands::SplitCommand::Swap { task_id, .. }) => Some(*task_id),
        _ => None,
    })
}

#[test]
fn the_first_swap_marks_a_swap_in_flight() {
    let mut app = app_in_split_mode(3);
    let cmds = swap(&mut app, 4);
    assert_eq!(swap_target(&cmds), Some(TaskId(4)));
    assert!(swap_in_flight(&app.board.split));
    assert_eq!(pending_swap(&app.board.split), None);
}

#[test]
fn a_swap_while_one_is_in_flight_is_held_not_started() {
    // The whole defect: the second swap would read pinned_task_id and
    // right_pane_id, neither of which has moved yet, and rename a window to a
    // name the first swap's rename just took.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    let cmds = swap(&mut app, 5);
    assert_eq!(
        swap_target(&cmds),
        None,
        "a second swap must not reach tmux while one is in flight"
    );
    assert_eq!(pending_swap(&app.board.split), Some(TaskId(5)));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
}

#[test]
fn a_further_swap_replaces_the_held_one() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    swap(&mut app, 3);
    assert_eq!(pending_swap(&app.board.split), Some(TaskId(3)));
}

#[test]
fn settling_a_swap_replays_the_held_one() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    let cmds = swap_settles(&mut app, 4);
    // The settle assigns both halves of the new occupant's identity...
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(4)));
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%77"));
    // ...and only then is the held swap started, against the settled state.
    assert_eq!(swap_target(&cmds), Some(TaskId(5)));
    assert!(swap_in_flight(&app.board.split));
    assert_eq!(pending_swap(&app.board.split), None);
}

#[test]
fn settling_a_swap_with_nothing_held_starts_nothing() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    let cmds = swap_settles(&mut app, 4);
    assert_eq!(swap_target(&cmds), None);
    assert!(!swap_in_flight(&app.board.split));
}

#[test]
fn a_failed_swap_settles_and_replays_the_held_one() {
    // Settling on failure is load-bearing: a swap_in_flight left set would
    // wedge the board out of swapping for the rest of the session.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    let cmds = swap_fails(&mut app);
    assert_eq!(swap_target(&cmds), Some(TaskId(5)));
    assert_eq!(pending_swap(&app.board.split), None);
}

#[test]
fn a_failed_swap_leaves_the_previous_task_pinned_and_reports_it() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap_fails(&mut app);
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%42"));
    assert!(!swap_in_flight(&app.board.split));
    assert_eq!(
        app.status.error_popup.as_deref(),
        Some("Swap failed: rename window failed")
    );
}

#[test]
fn a_held_swap_for_the_task_that_became_pinned_is_dropped() {
    // Pressing Space twice on the same task while the first swap runs: the
    // replay is refused by the already-pinned guard, not acted on twice.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 4);
    let cmds = swap_settles(&mut app, 4);
    assert_eq!(swap_target(&cmds), None);
    assert!(!swap_in_flight(&app.board.split));
    assert_eq!(pending_swap(&app.board.split), None);
}

#[test]
fn a_swap_with_no_split_pane_to_swap_into_is_not_started() {
    // Nothing downstream would report back, so marking a swap in flight here
    // would wedge every later swap.
    let mut app = app_in_split_mode(3);
    app.board.split.right_pane_id = None;
    let cmds = swap(&mut app, 4);
    assert_eq!(swap_target(&cmds), None);
    assert!(!swap_in_flight(&app.board.split));
    assert_eq!(pending_swap(&app.board.split), None);
}

#[test]
fn leaving_split_mode_drops_the_held_swap_but_not_the_swap_in_flight() {
    // A close says a pane went away. It does not say the tmux work already
    // under way will not report back, and that report is what clears the flag
    // and performs anything held for it — so the flag survives. The held
    // request does not: it names an occupant the user asked to see, and there
    // is no occupant. See `SplitPaneClosedResets` in
    // `docs/specs/split-pane.allium`.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(swap_in_flight(&app.board.split));
    assert_eq!(pending_swap(&app.board.split), None);
}

// ---------------------------------------------------------------------------
// A quit arriving mid-swap (docs/specs/split-pane.allium:
// HoldQuitWhileRearrangementInFlight, SplitPaneSwapSettles)
// ---------------------------------------------------------------------------

#[test]
fn a_quit_while_a_swap_is_in_flight_is_held_not_acted_on() {
    // The defect: a swap exchanges the panes, then renames the outgoing task's
    // window back to its own name. Quitting in between leaves that window
    // carrying the INCOMING task's name — two windows sharing one, which
    // dispatch.allium's TmuxWindowNamesAreUnique forbids.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    let cmds = confirm_quit(&mut app);
    assert_eq!(exit_pane_id(&cmds), None);
    assert!(!app.should_quit(), "the quit must wait for the swap");
    assert!(pending_quit(&app.board.split));
}

#[test]
fn settling_a_swap_with_a_held_quit_exits_then_quits() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    confirm_quit(&mut app);
    let cmds = swap_settles(&mut app, 4);
    // The exit runs against the settled occupant — task 4 is what the pane
    // holds by now, and it is what gets restored to a standalone window.
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%77"));
    assert!(app.should_quit());
    assert!(!pending_quit(&app.board.split));
    assert!(!swap_in_flight(&app.board.split));
}

#[test]
fn a_failed_swap_still_quits() {
    // The user asked to leave. A failed rearrangement changes what there is to
    // tidy up, never whether the application goes away — and the pane still
    // shows the previous occupant, so there is still an agent to restore.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    confirm_quit(&mut app);
    let cmds = swap_fails(&mut app);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%42"));
    assert!(app.should_quit());
    assert!(!pending_quit(&app.board.split));
}

#[test]
fn a_held_swap_is_dropped_when_a_quit_is_held_too() {
    // Replaying it would move a pane into a board nobody will look at again,
    // and the replayed swap is itself a rearrangement the quit does not wait
    // for — reintroducing the exact race the hold exists to close.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    confirm_quit(&mut app);
    let cmds = swap_settles(&mut app, 4);
    assert_eq!(swap_target(&cmds), None);
    assert_eq!(pending_swap(&app.board.split), None);
    assert!(app.should_quit());
}

#[test]
fn a_late_pane_close_does_not_drop_a_quit_held_for_a_swap() {
    // Clearing swap_in_flight here would silence the settle, and a quit held
    // for that swap would vanish with it — one the user cannot notice, having
    // already asked the application to close.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    confirm_quit(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(pending_quit(&app.board.split));
    assert!(!app.should_quit());
    swap_fails(&mut app);
    assert!(app.should_quit());
}

#[test]
fn a_toggle_while_a_swap_is_in_flight_is_held_not_acted_on() {
    // Acted on here it would read `active` — which a pane close may already
    // have cleared under the running swap — and start an entry beside it. Both
    // then settle on the same pane report, the swap's settle consumes it, and
    // the entry never finishes: [s] is dead for the rest of the session.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    let cmds = press_s(&mut app);
    assert!(
        !enter_command(&cmds),
        "no entry may start beside a live swap"
    );
    assert_eq!(exit_pane_id(&cmds), None);
    assert!(pending_toggle(&app.board.split));
    assert!(!entry_in_flight(&app.board.split));
}

#[test]
fn a_close_mid_swap_cannot_start_an_entry() {
    // The premise both specs rest on, enforced rather than assumed: a close
    // clears `active` while the swap runs on, and the toggle hold is what stops
    // an entry starting in that window. See
    // `HoldToggleWhileRearrangementInFlight` in `docs/specs/split-pane.allium`.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(!app.split_active());
    assert!(swap_in_flight(&app.board.split));
    app.selection_mut().set_column(2);
    press_s(&mut app);
    assert!(
        !entry_in_flight(&app.board.split),
        "an entry must never overlap a swap"
    );
    assert!(pending_toggle(&app.board.split));
}

#[test]
fn settling_a_swap_with_a_held_toggle_exits() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    press_s(&mut app);
    let cmds = swap_settles(&mut app, 4);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%77"));
    assert!(!pending_toggle(&app.board.split));
    assert!(!app.should_quit());
}

#[test]
fn a_held_toggle_replayed_after_a_close_mid_swap_does_not_re_enter() {
    // The press was made with the pane open: it meant "close this split". The
    // close already did that, so the replay is a no-op — replaying it as a
    // fresh press would enter instead, the opposite of what was asked.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    press_s(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    let cmds = swap_fails(&mut app);
    assert!(!enter_command(&cmds));
    assert_eq!(exit_pane_id(&cmds), None);
    assert!(!entry_in_flight(&app.board.split));
}

#[test]
fn a_held_swap_is_dropped_when_a_toggle_is_held_too() {
    // Contradictory instructions — "show me this task" and "close this split".
    // Acting on both would start a fresh swap and then break out the very pane
    // it is exchanging, and the swap's own report would afterwards set `active`
    // back to true, re-opening the split the press asked to close.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    press_s(&mut app);
    let cmds = swap_settles(&mut app, 4);
    assert_eq!(swap_target(&cmds), None, "the held swap must be dropped");
    assert_eq!(pending_swap(&app.board.split), None);
    assert!(!swap_in_flight(&app.board.split));
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%77"));
}

#[test]
fn a_held_toggle_is_dropped_when_a_quit_is_held_too() {
    // One exit between them, not two: the quit's own exit already takes the
    // pane away.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    press_s(&mut app);
    confirm_quit(&mut app);
    let cmds = swap_settles(&mut app, 4);
    assert_eq!(
        exit_count(&cmds),
        1,
        "a held toggle and a held quit must exit once between them"
    );
    assert!(app.should_quit());
    assert!(!pending_toggle(&app.board.split));
}

#[test]
fn a_quit_with_no_rearrangement_in_flight_is_acted_on_at_once() {
    let mut app = app_in_split_mode(3);
    let cmds = confirm_quit(&mut app);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%42"));
    assert!(app.should_quit());
    assert!(!pending_quit(&app.board.split));
}

// ---------------------------------------------------------------------------
// Entry serialisation (docs/specs/split-pane.allium:
// HoldToggleWhileRearrangementInFlight, SplitPaneEntrySettles)
// ---------------------------------------------------------------------------

/// `make_app`'s Running task (3, already provisioned with a window) selected in
/// the Running column — the state a press of [s] or a confirmed quit acts on.
fn app_with_running_task_selected() -> App {
    let mut app = make_app();
    app.selection_mut().set_column(2);
    app
}

/// `(entry in flight, held toggle, held quit)`.
///
/// Neither hold belongs to the entry specifically. Both sit on the
/// rearrangement, because both are held for whichever one is in flight — an
/// entry or a swap — and each settle replays them the same way. See
/// `HoldToggleWhileRearrangementInFlight` and
/// `HoldQuitWhileRearrangementInFlight` in `docs/specs/split-pane.allium`.
fn entry_holds(app: &App) -> (bool, bool, bool) {
    (
        entry_in_flight(&app.board.split),
        pending_toggle(&app.board.split),
        pending_quit(&app.board.split),
    )
}

fn entry_in_flight(split: &SplitState) -> bool {
    matches!(
        split.in_flight,
        Some(InFlight {
            kind: Rearrangement::Entry,
            ..
        })
    )
}

fn swap_in_flight(split: &SplitState) -> bool {
    matches!(
        split.in_flight,
        Some(InFlight {
            kind: Rearrangement::Swap { .. },
            ..
        })
    )
}

fn pending_toggle(split: &SplitState) -> bool {
    split.in_flight.as_ref().is_some_and(|f| f.pending_toggle)
}

fn pending_quit(split: &SplitState) -> bool {
    split.in_flight.as_ref().is_some_and(|f| f.pending_quit)
}

/// The swap held for the rearrangement in flight, if any. `None` both when
/// nothing is in flight and when an entry is: only a swap can hold one.
fn pending_swap(split: &SplitState) -> Option<TaskId> {
    match split.in_flight.as_ref()?.kind {
        Rearrangement::Swap { pending_swap } => pending_swap,
        Rearrangement::Entry => None,
    }
}

fn press_s(app: &mut App) -> Vec<Command> {
    without_usage(app.handle_key(make_key(KeyCode::Char('s'))))
}

fn enter_command(cmds: &[Command]) -> bool {
    cmds.iter().any(|c| {
        matches!(
            c,
            Command::Split(
                crate::tui::commands::SplitCommand::Enter
                    | crate::tui::commands::SplitCommand::EnterWithTask { .. }
            )
        )
    })
}

fn exit_pane_id(cmds: &[Command]) -> Option<String> {
    cmds.iter().find_map(|c| match c {
        Command::Split(crate::tui::commands::SplitCommand::Exit { pane_id, .. }) => {
            Some(pane_id.clone())
        }
        _ => None,
    })
}

/// The pane the runtime reports back once entry has actually opened one.
fn entry_settles(app: &mut App) -> Vec<Command> {
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EntryOpened {
            pane_id: "%9".to_string(),
            task_id: None,
        },
    ))
}

#[test]
fn the_first_toggle_marks_entry_in_flight() {
    let mut app = make_app();
    let cmds = press_s(&mut app);
    assert!(enter_command(&cmds));
    assert_eq!(entry_holds(&app), (true, false, false));
    // Entry has not finished: the pane does not exist yet.
    assert!(!app.board.split.active);
}

#[test]
fn a_toggle_while_entry_is_in_flight_is_held_not_started() {
    // The whole defect: the second press reads `active` as false, because the
    // pane has not reported back, and opens a second pane the board cannot
    // track.
    let mut app = make_app();
    press_s(&mut app);
    let cmds = press_s(&mut app);
    assert!(
        !enter_command(&cmds),
        "a second press must not open a second pane, got {cmds:?}"
    );
    assert_eq!(entry_holds(&app), (true, true, false));
}

#[test]
fn a_further_toggle_replaces_the_held_one() {
    // Held, not counted: a burst during one entry is a single held toggle.
    let mut app = make_app();
    press_s(&mut app);
    press_s(&mut app);
    let cmds = press_s(&mut app);
    assert!(!enter_command(&cmds));
    assert_eq!(entry_holds(&app), (true, true, false));
}

#[test]
fn settling_an_entry_with_a_held_toggle_exits() {
    // A held press is replayed as the toggle it is, and a toggle against a
    // pane that has just opened exits — so a fast double-tap ends where a
    // slow one does.
    let mut app = make_app();
    press_s(&mut app);
    press_s(&mut app);
    let cmds = entry_settles(&mut app);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%9"));
    assert_eq!(entry_holds(&app), (false, false, false));
}

#[test]
fn settling_an_entry_with_nothing_held_leaves_the_pane_open() {
    let mut app = make_app();
    press_s(&mut app);
    let cmds = entry_settles(&mut app);
    assert_eq!(exit_pane_id(&cmds), None);
    assert!(app.board.split.active);
    assert_eq!(entry_holds(&app), (false, false, false));
}

#[test]
fn a_settled_entry_lets_the_next_press_exit() {
    let mut app = make_app();
    press_s(&mut app);
    entry_settles(&mut app);
    let cmds = press_s(&mut app);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%9"));
}

#[test]
fn a_failed_entry_settles_so_the_next_press_still_works() {
    // Settling on failure is load-bearing: an entry left in flight would
    // wedge [s] for the rest of the session.
    let mut app = make_app();
    press_s(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EnterFailed {
            failure: crate::tui::messages::EnterFailure::NoTmux,
        },
    ));
    assert_eq!(entry_holds(&app), (false, false, false));
    assert!(!app.board.split.active);
    let cmds = press_s(&mut app);
    assert!(enter_command(&cmds));
}

#[test]
fn a_failed_entry_reports_the_tmux_error() {
    let mut app = make_app();
    press_s(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EnterFailed {
            failure: crate::tui::messages::EnterFailure::Failed(
                "Split failed: no space for a new pane".to_string(),
            ),
        },
    ));
    assert_eq!(
        app.status.error_popup.as_deref(),
        Some("Split failed: no space for a new pane")
    );
    assert_eq!(entry_holds(&app), (false, false, false));
    assert!(!app.board.split.active);
}

#[test]
fn a_failed_entry_drops_the_held_toggle() {
    // Nothing opened, so replaying the held press would restart the attempt
    // that just failed and repeat its error rather than undo anything.
    let mut app = make_app();
    press_s(&mut app);
    press_s(&mut app);
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::EnterFailed {
            failure: crate::tui::messages::EnterFailure::NoTmux,
        },
    ));
    assert!(!enter_command(&cmds));
    assert_eq!(exit_pane_id(&cmds), None);
    assert_eq!(entry_holds(&app), (false, false, false));
}

#[test]
fn an_entry_outside_tmux_reports_a_status_hint_not_an_error() {
    let mut app = make_app();
    press_s(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EnterFailed {
            failure: crate::tui::messages::EnterFailure::NoTmux,
        },
    ));
    assert_eq!(
        app.status.message.as_deref(),
        Some("Split mode requires tmux")
    );
    assert!(app.status.error_popup.is_none());
}

#[test]
fn a_swap_settling_does_not_disturb_the_entry_state() {
    // Only one rearrangement can ever be in flight — a swap needs split mode
    // active, an entry needs it inactive — so a swap settling must leave
    // nothing of an entry behind.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap_settles(&mut app, 4);
    assert_eq!(entry_holds(&app), (false, false, false));
    assert!(app.board.split.active);
}

#[test]
fn an_enter_failure_with_no_entry_in_flight_still_reports() {
    // SplitPaneEntrySettles requires an entry in flight, so a settle with none
    // touches no state — but the failure is still reported, because a failure
    // nothing else mentions must not go silent.
    let mut app = make_app();
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EnterFailed {
            failure: crate::tui::messages::EnterFailure::NoTmux,
        },
    ));
    assert_eq!(entry_holds(&app), (false, false, false));
    assert!(!app.should_quit());
    assert_eq!(
        app.status.message.as_deref(),
        Some("Split mode requires tmux")
    );
}

/// Confirm the quit dialog: `q` opens it, `y` confirms.
fn confirm_quit(app: &mut App) -> Vec<Command> {
    app.input.mode = InputMode::ConfirmQuit;
    without_usage(app.handle_key(make_key(KeyCode::Char('y'))))
}

#[test]
fn a_quit_while_entry_is_in_flight_is_held_not_acted_on() {
    // Acted on here it would issue no exit at all — exit is gated on `active`,
    // still false — and dispatch would go away leaving the agent's pane inside
    // the board's own window.
    let mut app = app_with_running_task_selected();
    press_s(&mut app);
    let cmds = confirm_quit(&mut app);
    assert_eq!(exit_pane_id(&cmds), None);
    assert!(!app.should_quit(), "the quit must wait for the entry");
    assert_eq!(entry_holds(&app), (true, false, true));
}

#[test]
fn settling_an_entry_with_a_held_quit_exits_then_quits() {
    let mut app = app_with_running_task_selected();
    press_s(&mut app);
    confirm_quit(&mut app);
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::EntryOpened {
            pane_id: "%9".to_string(),
            task_id: Some(TaskId(3)),
        },
    ));
    // The pinned agent is broken back out to its own window before the board
    // goes away, which is the whole reason the quit waited.
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%9"));
    assert!(app.should_quit());
    assert_eq!(entry_holds(&app), (false, false, false));
}

#[test]
fn a_failed_entry_still_quits() {
    // The user asked to leave. A failed entry changes what there is to tidy
    // up, never whether the application goes away.
    let mut app = make_app();
    press_s(&mut app);
    confirm_quit(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::EnterFailed {
            failure: crate::tui::messages::EnterFailure::NoTmux,
        },
    ));
    assert!(app.should_quit());
    assert_eq!(entry_holds(&app), (false, false, false));
}

#[test]
fn a_held_toggle_and_a_held_quit_exit_once_between_them() {
    let mut app = app_with_running_task_selected();
    press_s(&mut app);
    press_s(&mut app);
    confirm_quit(&mut app);
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::EntryOpened {
            pane_id: "%9".to_string(),
            task_id: Some(TaskId(3)),
        },
    ));
    let exits = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::Split(crate::tui::commands::SplitCommand::Exit { .. })
            )
        })
        .count();
    assert_eq!(exits, 1, "one exit between them, got {cmds:?}");
    assert!(app.should_quit());
}

#[test]
fn quitting_with_no_entry_in_flight_exits_immediately() {
    // The ordinary path must keep working: no entry, no wait.
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("task-3"));
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    let cmds = confirm_quit(&mut app);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%42"));
    assert!(app.should_quit());
}

#[test]
fn a_swap_settling_does_not_claim_tmux_focus() {
    // PinTaskInSplitPane: focus does NOT transfer on a swap. Only an entry
    // settling resets it (SplitPaneEntrySettles).
    let mut app = app_in_split_mode(3);
    app.board.split.focused = false;
    swap(&mut app, 4);
    swap_settles(&mut app, 4);
    assert!(
        !app.split_focused(),
        "a swap must leave the focus border where it was"
    );
}

#[test]
fn a_late_pane_close_does_not_cancel_an_entry_in_flight() {
    // A liveness poll issued while the previous pane was open can land after
    // the user closed it and pressed [s] again. The close is about a pane that
    // is already history; the entry it lands during is not.
    let mut app = make_app();
    press_s(&mut app);
    press_s(&mut app);
    confirm_quit(&mut app);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert_eq!(entry_holds(&app), (true, true, true));
    assert!(!app.should_quit());
    // ...and the entry still settles into the exit-then-quit it was holding.
    let cmds = entry_settles(&mut app);
    assert_eq!(exit_pane_id(&cmds).as_deref(), Some("%9"));
    assert!(app.should_quit());
}

#[test]
fn a_pane_close_with_no_entry_in_flight_resets_everything() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(!app.split_active());
    assert!(app.board.split.right_pane_id.is_none());
    assert!(app.board.split.pinned_task_id.is_none());
    assert_eq!(pending_swap(&app.board.split), None);
    assert!(app.split_focused());
}
