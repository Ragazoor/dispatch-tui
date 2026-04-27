#![allow(unused_imports)]

use super::*;
use crate::models::{
    DispatchMode, Epic, EpicId, SubStatus, TaskId, TaskStatus, TaskTag, DEFAULT_QUICK_TASK_TITLE,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    Terminal,
};
use std::time::{Duration, Instant};

#[test]
fn split_pane_opened_resets_focused_to_true() {
    let mut app = make_app();
    // Simulate having lost focus before entering split
    app.board.split.focused = false;

    let _cmds = app.update(Message::SplitPaneOpened {
        pane_id: "pane1".to_string(),
        task_id: None,
    });
    assert!(app.split_active());
    assert!(app.split_focused());
}

#[test]
fn split_pane_closed_resets_focused_to_true() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("pane1".to_string());
    app.board.split.focused = false;

    let _cmds = app.update(Message::SplitPaneClosed);
    assert!(!app.split_active());
    assert!(app.split_focused());
}

#[test]
fn toggle_split_mode_emits_enter_command() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EnterSplitMode));
}

#[test]
fn toggle_split_mode_emits_exit_command() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = None;
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::ExitSplitMode { pane_id, restore_window } if pane_id == "%42" && restore_window.is_none())
    );
}

#[test]
fn toggle_split_exit_restores_pinned_task_window() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::ExitSplitMode { pane_id, restore_window } if pane_id == "%42" && restore_window.as_deref() == Some("task-3"))
    );
}

#[test]
fn g_in_split_mode_on_already_pinned_task_does_nothing() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4)); // same task already pinned
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(
        cmds.is_empty(),
        "G on already-pinned task must not emit commands"
    );
}

#[test]
fn g_in_split_mode_emits_swap_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    // No pinned task — different from already-pinned case
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::SwapSplitPane {
            task_id,
            new_window,
            ..
        } if *task_id == TaskId(4) && new_window == "task-4"
    ));
}

#[test]
fn g_outside_split_mode_is_noop() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // split NOT active
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(cmds.is_empty(), "G outside split mode must be a no-op");
}

#[test]
fn g_in_split_mode_on_task_without_window_shows_status() {
    let task = make_task(4, TaskStatus::Running); // no tmux_window
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.selection_mut().set_column(1);
    let _cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(
        app.status
            .message
            .as_deref()
            .unwrap_or("")
            .contains("No agent session"),
        "G on windowless task must show a status message"
    );
}

#[test]
fn g_in_split_mode_emits_jump_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::JumpToTmux { window } if window == "task-4"
    ));
}

#[test]
fn g_without_split_mode_emits_jump_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(
        &cmds[0],
        Command::JumpToTmux { window } if window == "task-4"
    ));
}

#[test]
fn g_on_pinned_split_task_emits_focus_split_pane() {
    // When the selected task IS the pinned split-pane task, its standalone
    // window no longer exists — [g] must focus the right pane instead.
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::FocusSplitPane { pane_id } if pane_id == "%42"),
        "expected FocusSplitPane {{pane_id: \"%42\"}}, got {:?}",
        cmds
    );
}

#[test]
fn g_on_non_pinned_task_in_split_mode_still_jumps_to_window() {
    // When split is active but the selected task is NOT the pinned one,
    // [g] should still emit JumpToTmux for the selected task's window.
    let mut task1 = make_task(3, TaskStatus::Running);
    task1.tmux_window = Some("task-3".to_string());
    let mut task2 = make_task(4, TaskStatus::Running);
    task2.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task1, task2], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3)); // task3 is pinned, not task4
                                                      // Navigate to Running column and select task4 (row 1, second in column)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::JumpToTmux { window } if window == "task-4"),
        "expected JumpToTmux for non-pinned task, got {:?}",
        cmds
    );
}

#[test]
fn split_pane_opened_updates_state() {
    let mut app = make_app();
    assert!(!app.board.split.active);
    app.update(Message::SplitPaneOpened {
        pane_id: "%42".to_string(),
        task_id: Some(TaskId(3)),
    });
    assert!(app.board.split.active);
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%42"));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
}

#[test]
fn split_pane_closed_resets_state() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    app.update(Message::SplitPaneClosed);
    assert!(!app.board.split.active);
    assert!(app.board.split.right_pane_id.is_none());
    assert!(app.board.split.pinned_task_id.is_none());
}

#[test]
fn tick_captures_non_pinned_tasks_in_split_mode() {
    let mut task3 = make_task(3, TaskStatus::Running);
    task3.tmux_window = Some("task-3".to_string());
    let mut task4 = make_task(4, TaskStatus::Running);
    task4.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task3, task4], TEST_TIMEOUT);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    let cmds = app.update(Message::Tick);

    // Task 3 (not pinned) should still get captured
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(3), .. })));
    // Task 4 (pinned in split) should NOT
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(4), .. })));
}

#[test]
fn toggle_split_with_selected_tmux_task_emits_enter_with_task() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::EnterSplitModeWithTask { task_id, window }
            if *task_id == TaskId(3) && window == "task-3"
    ));
}

#[test]
fn toggle_split_without_tmux_task_emits_plain_enter() {
    let task = make_task(3, TaskStatus::Running);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column, task has no tmux_window
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EnterSplitMode));
}

#[test]
fn toggle_split_no_selection_emits_plain_enter() {
    // make_app has tasks but default selection is on Backlog column — task 1 has no tmux_window
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EnterSplitMode));
}

#[test]
fn handle_key_normal_toggle_split_mode() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert!(cmds.iter().any(|c| matches!(c, Command::EnterSplitMode)));
}

#[test]
fn confirm_quit_with_active_split_emits_exit_split_mode() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

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
            Command::ExitSplitMode {
                pane_id,
                restore_window: Some(w),
            } if pane_id == "%42" && w == "task-3"
        )),
        "should emit ExitSplitMode to restore task window before quitting"
    );
}

#[test]
fn finish_complete_respawns_split_pane_for_pinned_task() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
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
fn finish_complete_no_respawn_for_non_pinned_task() {
    let mut app = App::new(
        vec![
            {
                let mut t = make_task(1, TaskStatus::Review);
                t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
                t.tmux_window = Some("task-1".to_string());
                t
            },
            {
                let mut t = make_task(2, TaskStatus::Running);
                t.tmux_window = Some("task-2".to_string());
                t
            },
        ],
        TEST_TIMEOUT,
    );
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(2));

    let cmds = app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { .. })),
        "should NOT respawn when a different task finishes"
    );
    assert_eq!(
        app.board.split.pinned_task_id,
        Some(TaskId(2)),
        "pinned task should be unchanged"
    );
}

#[test]
fn finish_complete_no_respawn_without_split() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    // split is NOT active (default)

    let cmds = app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { .. })),
        "should NOT respawn when split mode is inactive"
    );
}

#[test]
fn pr_merged_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task's PR is merged"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn confirm_done_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));
    app.input.mode = InputMode::ConfirmDone(TaskId(1));

    let cmds = app.update(Message::ConfirmDone);

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task is confirmed done"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn archive_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Done);
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::ArchiveTask(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task is archived"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn retry_resume_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.sub_status = SubStatus::Crashed;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));
    app.input.mode = InputMode::ConfirmRetry(TaskId(1));

    let cmds = app.update(Message::RetryResume(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task is retried"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn pr_created_does_not_respawn_split_pane() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::PrCreated {
        id: TaskId(1),
        pr_url: "https://github.com/org/repo/pull/42".to_string(),
    });

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { .. })),
        "should NOT respawn — agent keeps running after PR creation"
    );
    assert_eq!(
        app.board.split.pinned_task_id,
        Some(TaskId(1)),
        "pinned task should remain"
    );
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
            Command::ExitSplitMode {
                pane_id,
                restore_window: None,
            } if pane_id == "%99"
        )),
        "should emit ExitSplitMode with no restore_window for empty split"
    );
}

#[test]
fn epic_wrap_up_respawns_split_pane_only_once() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(2));
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));
    app.update(Message::EpicWrapUpRebase);

    // First task completes — this is the pinned one
    let cmds1 = app.update(Message::FinishComplete(TaskId(2)));
    let respawn_count_1 = cmds1
        .iter()
        .filter(|c| matches!(c, Command::RespawnSplitPane { .. }))
        .count();
    assert_eq!(respawn_count_1, 1, "should respawn once for pinned task");
    assert_eq!(app.board.split.pinned_task_id, None);

    // Second task completes — no longer pinned
    let cmds2 = app.update(Message::FinishComplete(TaskId(1)));
    let respawn_count_2 = cmds2
        .iter()
        .filter(|c| matches!(c, Command::RespawnSplitPane { .. }))
        .count();
    assert_eq!(respawn_count_2, 0, "should NOT respawn for non-pinned task");
}
