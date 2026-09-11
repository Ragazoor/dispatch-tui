use super::*;
use crate::models::{test_tmux_window, SubStatus, TaskId, TaskStatus};
use crossterm::event::KeyCode;

#[test]
fn split_pane_opened_resets_focused_to_true() {
    let mut app = make_app();
    // Simulate having lost focus before entering split
    app.board.split.focused = false;

    let _cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneOpened {
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
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneOpened {
            pane_id: "%42".to_string(),
            task_id: Some(TaskId(3)),
        },
    ));
    assert!(app.board.split.active);
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%42"));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
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

    // Enter confirm quit, then confirm with 'y'
    app.input.mode = InputMode::ConfirmQuit;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));

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

    app.input.mode = InputMode::ConfirmQuit;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));

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
    assert!(app.board.split.swap_in_flight);
    assert!(app.board.split.pending_swap.is_none());
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
    assert_eq!(app.board.split.pending_swap, Some(TaskId(5)));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
}

#[test]
fn a_further_swap_replaces_the_held_one() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    swap(&mut app, 3);
    assert_eq!(app.board.split.pending_swap, Some(TaskId(3)));
}

#[test]
fn settling_a_swap_replays_the_held_one() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneOpened {
            pane_id: "%77".to_string(),
            task_id: Some(TaskId(4)),
        },
    ));
    // The settle assigns both halves of the new occupant's identity...
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(4)));
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%77"));
    // ...and only then is the held swap started, against the settled state.
    assert_eq!(swap_target(&cmds), Some(TaskId(5)));
    assert!(app.board.split.swap_in_flight);
    assert!(app.board.split.pending_swap.is_none());
}

#[test]
fn settling_a_swap_with_nothing_held_starts_nothing() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneOpened {
            pane_id: "%77".to_string(),
            task_id: Some(TaskId(4)),
        },
    ));
    assert_eq!(swap_target(&cmds), None);
    assert!(!app.board.split.swap_in_flight);
}

#[test]
fn a_failed_swap_settles_and_replays_the_held_one() {
    // Settling on failure is load-bearing: a swap_in_flight left set would
    // wedge the board out of swapping for the rest of the session.
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::SwapFailed {
            error: "Swap failed: rename window failed".to_string(),
        },
    ));
    assert_eq!(swap_target(&cmds), Some(TaskId(5)));
    assert!(app.board.split.pending_swap.is_none());
}

#[test]
fn a_failed_swap_leaves_the_previous_task_pinned_and_reports_it() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::SwapFailed {
            error: "Swap failed: rename window failed".to_string(),
        },
    ));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%42"));
    assert!(!app.board.split.swap_in_flight);
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
    let cmds = app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneOpened {
            pane_id: "%77".to_string(),
            task_id: Some(TaskId(4)),
        },
    ));
    assert_eq!(swap_target(&cmds), None);
    assert!(!app.board.split.swap_in_flight);
    assert!(app.board.split.pending_swap.is_none());
}

#[test]
fn a_swap_with_no_split_pane_to_swap_into_is_not_started() {
    // Nothing downstream would report back, so marking a swap in flight here
    // would wedge every later swap.
    let mut app = app_in_split_mode(3);
    app.board.split.right_pane_id = None;
    let cmds = swap(&mut app, 4);
    assert_eq!(swap_target(&cmds), None);
    assert!(!app.board.split.swap_in_flight);
    assert!(app.board.split.pending_swap.is_none());
}

#[test]
fn leaving_split_mode_clears_the_swap_serialisation_state() {
    let mut app = app_in_split_mode(3);
    swap(&mut app, 4);
    swap(&mut app, 5);
    app.update(Message::Split(
        crate::tui::messages::SplitMessage::PaneClosed,
    ));
    assert!(!app.board.split.swap_in_flight);
    assert!(app.board.split.pending_swap.is_none());
}
