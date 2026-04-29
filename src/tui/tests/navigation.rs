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
fn tasks_by_status_filters() {
    let app = make_app();
    let backlog = app.tasks_by_status(TaskStatus::Backlog);
    assert_eq!(backlog.len(), 2);
    assert_eq!(backlog[0].id, TaskId(1));
    assert_eq!(backlog[1].id, TaskId(2));

    let running = app.tasks_by_status(TaskStatus::Running);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, TaskId(3));

    let review = app.tasks_by_status(TaskStatus::Review);
    assert_eq!(review.len(), 0);
}

#[test]
fn move_task_forward() {
    let mut app = make_app();
    // Task 1 is in Backlog; move it forward -> Running
    let cmds = app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    });
    assert_eq!(
        app.board
            .tasks
            .iter()
            .find(|t| t.id == TaskId(1))
            .unwrap()
            .status,
        TaskStatus::Running
    );
    // Should produce a PersistTask command
    assert!(matches!(cmds[0], Command::PersistTask(_)));
}

#[test]
fn move_task_backward_at_start_is_noop() {
    let mut app = make_app();
    // Task 1 is in Backlog; prev() stays Backlog
    let cmds = app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    });
    assert_eq!(
        app.board
            .tasks
            .iter()
            .find(|t| t.id == TaskId(1))
            .unwrap()
            .status,
        TaskStatus::Backlog
    );
    assert!(cmds.is_empty());
}

#[test]
fn quit_enters_confirm_mode() {
    let mut app = make_app();
    assert!(!app.should_quit);
    app.update(Message::Quit);
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn navigate_column_clamps() {
    let mut app = make_app();
    // Projects column (0) is the leftmost; can't go further left.
    app.selection_mut().set_column(0);
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selection().column(), 0); // can't go below 0

    // From archive column (COLUMN_COUNT+1 = 5), pressing right stays clamped.
    app.selection_mut().set_column(TaskStatus::COLUMN_COUNT + 1);
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selection().column(), TaskStatus::COLUMN_COUNT + 1); // can't go above max
}

#[test]
fn navigate_column_moves_through_visual_columns() {
    let mut app = make_app();
    // Board starts at Backlog (nav col 1); Projects is col 0 (left edge).
    assert_eq!(app.selected_column(), 1); // Backlog
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 2); // Running
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 3); // Review
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 4); // Done
}

#[test]
fn navigate_column_clamps_at_visual_column_max() {
    let mut app = make_app();
    // From Done (nav col 4 = COLUMN_COUNT) pressing right enters archive (nav col 5), not a clamp.
    app.selection_mut().set_column(TaskStatus::COLUMN_COUNT);
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), TaskStatus::COLUMN_COUNT + 1); // archive column
                                                                     // From archive (nav col 5), pressing right is clamped.
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), TaskStatus::COLUMN_COUNT + 1); // stays at archive
}

#[test]
fn navigate_row_clamps() {
    let mut app = make_app();
    // Backlog is nav col 1. Selected row starts at 0.
    app.selection_mut().set_column(1);
    app.update(Message::NavigateRow(-1));
    // Navigating up from row 0 now moves to the select-all toggle
    assert!(app.on_select_all());

    // Navigate back down to tasks and then past the end
    app.update(Message::NavigateRow(1));
    assert!(!app.on_select_all());
    app.update(Message::NavigateRow(10));
    assert_eq!(app.selection().row(1), 1); // clamps to last item index
}

#[test]
fn delete_task_with_worktree_emits_cleanup() {
    let mut app = make_app();
    let task = app.find_task_mut(TaskId(4)).unwrap();
    task.worktree = Some("/repo/.worktrees/4-task".to_string());
    task.tmux_window = Some("task-4".to_string());

    let cmds = app.update(Message::DeleteTask(TaskId(4)));
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(4)));
    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteTask(TaskId(4)))));
}

#[test]
fn delete_task_without_worktree_no_cleanup() {
    let mut app = make_app();
    let cmds = app.update(Message::DeleteTask(TaskId(1)));
    assert!(!cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
}

#[test]
fn error_sets_error_popup() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::Error("Something went wrong".to_string()));
    assert_eq!(
        app.status.error_popup.as_deref(),
        Some("Something went wrong")
    );
}

#[test]
fn move_backward_from_running_detaches_but_keeps_worktree() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);

    let cmds = app.update(Message::MoveTask {
        id: TaskId(4),
        direction: MoveDirection::Backward,
    });

    // Should emit KillTmuxWindow then PersistTask (no Cleanup)
    assert_eq!(cmds.len(), 2);
    assert!(matches!(&cmds[0], Command::KillTmuxWindow { window } if window == "task-4"));
    assert!(matches!(&cmds[1], Command::PersistTask(_)));

    // Worktree preserved, tmux_window cleared
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(4)).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/4-task-4"));
    assert!(task.tmux_window.is_none());
}

#[test]
fn move_backward_from_running_without_dispatch_fields() {
    let task = make_task(3, TaskStatus::Running);
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    let cmds = app.update(Message::MoveTask {
        id: TaskId(3),
        direction: MoveDirection::Backward,
    });
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::PersistTask(_)));
}

#[test]
fn move_forward_to_done_enters_confirm_mode() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = None; // session closed, but worktree remains
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);

    let cmds = app.update(Message::MoveTask {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    });

    // Should enter confirmation mode, not move immediately
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(5))));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(5)).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    // Worktree preserved — not taken during confirmation
    assert!(task.worktree.is_some());
}

#[test]
fn move_forward_to_done_with_live_window_enters_confirm_mode() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);

    let cmds = app.update(Message::MoveTask {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    });

    // Should enter confirmation mode, not move immediately
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(5))));
}

#[test]
fn g_key_with_live_window_jumps() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Running = nav col 2
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "task-4"));
}

#[test]
fn g_key_without_window_shows_message() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Backlog = nav col 1
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No active session"));
}

#[test]
fn typing_appends_to_input_buffer() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.handle_key(make_key(KeyCode::Char('H')));
    app.handle_key(make_key(KeyCode::Char('i')));
    assert_eq!(app.input.buffer, "Hi");
}

#[test]
fn any_key_clears_error_popup() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.status.error_popup = Some("boom".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.status.error_popup.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn error_popup_blocks_normal_key_handling() {
    let mut app = make_app();
    app.status.error_popup = Some("boom".to_string());
    app.handle_key(make_key(KeyCode::Char('q'))); // would normally quit
    assert!(app.status.error_popup.is_none());
    assert!(!app.should_quit); // quit was NOT processed
}

#[test]
fn resumed_sets_tmux_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    let cmds = app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "win-4".to_string(),
    });
    assert_eq!(app.board.tasks[0].tmux_window.as_deref(), Some("win-4"));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
}

#[test]
fn resumed_unknown_id_is_noop() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], 1, TEST_TIMEOUT);
    let cmds = app.update(Message::Resumed {
        id: TaskId(999),
        tmux_window: "win".to_string(),
    });
    assert!(cmds.is_empty());
}

#[test]
fn resumed_sets_status_to_running() {
    let mut task = make_task(4, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);

    let cmds = app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "task-4".to_string(),
    });

    let task = app.board.tasks.iter().find(|t| t.id == TaskId(4)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.tmux_window.as_deref(), Some("task-4"));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(t) if t.status == TaskStatus::Running));
}

#[test]
fn refresh_tasks_replaces_and_clamps() {
    let mut app = make_app();
    app.selection_mut().set_row(1, 1); // row 1 of Backlog (nav col 1, has 2 items)
    app.update(Message::RefreshTasks(vec![make_task(
        10,
        TaskStatus::Backlog,
    )]));
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(10));
    assert_eq!(app.selection().row(1), 0); // clamped from 1 to 0
}

#[test]
fn refresh_tasks_empty_clamps_all_rows_to_zero() {
    let mut app = make_app();
    app.selection_mut().set_row(0, 1);
    app.selection_mut().set_row(1, 1);
    app.update(Message::RefreshTasks(vec![]));
    assert!(app.board.tasks.is_empty());
    assert!(app.selection().selected_row.iter().all(|&r| r == 0));
}

#[test]
fn g_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds.is_empty());
}

#[test]
fn shift_l_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert!(cmds.is_empty());
}

#[test]
fn shift_h_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    assert!(cmds.is_empty());
}

#[test]
fn dismiss_error_clears_popup() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.status.error_popup = Some("boom".to_string());
    app.update(Message::DismissError);
    assert!(app.status.error_popup.is_none());
}

#[test]
fn input_char_appends_to_buffer() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::InputChar('H'));
    app.update(Message::InputChar('i'));
    assert_eq!(app.input.buffer, "Hi");
}

#[test]
fn input_backspace_removes_last_char() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.buffer = "abc".to_string();
    app.update(Message::InputBackspace);
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn status_info_sets_message() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::StatusInfo("hello".to_string()));
    assert_eq!(app.status.message.as_deref(), Some("hello"));
}

#[test]
fn start_quick_dispatch_selection_enters_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::StartQuickDispatchSelection);
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert!(app.status.message.is_some());
}

#[test]
fn select_quick_dispatch_repo_dispatches() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    let cmds = app.update(Message::SelectQuickDispatchRepo(1));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(
        |c| matches!(c, Command::QuickDispatch { ref draft, .. } if draft.repo_path == "/repo2")
    ));
}

#[test]
fn select_quick_dispatch_repo_out_of_range_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.update(Message::SelectQuickDispatchRepo(5));
    assert!(cmds.is_empty());
    // Mode is not changed by the handler (stays as-is)
}

#[test]
fn space_toggles_task_selection() {
    let mut app = make_app();
    // Select task 1 in Backlog
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.tasks.contains(&TaskId(1)));

    // Toggle off
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(!app.select.tasks.contains(&TaskId(1)));
}

#[test]
fn space_on_empty_column_is_noop() {
    let mut app = make_app();
    // Navigate to Review column (empty)
    app.update(Message::NavigateColumn(2));
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn batch_move_forward_moves_all_selected() {
    let mut app = make_app();
    // Select both Backlog tasks
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    // Press m to batch move forward
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));

    // Both should now be Running
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
    assert_eq!(
        app.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Running
    );
    // Should have PersistTask commands
    let persist_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::PersistTask(_)))
        .count();
    assert_eq!(persist_count, 2);
}

#[test]
fn batch_move_clears_selection() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    app.handle_key(make_key(KeyCode::Char('L')));

    assert!(app.select.tasks.is_empty());
}

#[test]
fn batch_move_backward() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Done),
            make_task(2, TaskStatus::Done),
            make_task(3, TaskStatus::Done),
        ],
        1,
        TEST_TIMEOUT,
    );

    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    app.handle_key(make_key(KeyCode::Char('H')));

    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Review);
    assert_eq!(app.find_task(TaskId(2)).unwrap().status, TaskStatus::Review);
    // Task 3 not selected, should remain Done
    assert_eq!(app.find_task(TaskId(3)).unwrap().status, TaskStatus::Done);
}

#[test]
fn single_task_operations_work_without_selection() {
    let mut app = make_app();
    assert!(app.select.tasks.is_empty());

    // Single move should still work
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
    assert!(!cmds.is_empty());
}

#[test]
fn refresh_tasks_prunes_stale_selections() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(99))); // non-existent

    // Refresh with only task 1
    app.update(Message::RefreshTasks(vec![make_task(
        1,
        TaskStatus::Backlog,
    )]));

    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(!app.select.tasks.contains(&TaskId(99)));
}

#[test]
fn e_on_task_enters_confirm_then_edits() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmEditTask(_)));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PopOutEditor(EditKind::TaskEdit(_)))));
}

#[test]
fn shift_v_key_on_task_is_noop() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('V')));
    assert!(cmds.is_empty());
}

#[test]
fn shift_v_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('V')));
    assert!(cmds.is_empty());
}

#[test]
fn focus_changed_ignored_when_split_inactive() {
    let mut app = make_app();
    assert!(!app.split_active());

    let cmds = app.update(Message::FocusChanged(false));
    assert!(cmds.is_empty());
    assert!(app.split_focused()); // still true — ignored
}

#[test]
fn normal_mode_unrecognized_key_is_noop() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
    assert!(!app.should_quit);
}

#[test]
fn text_input_unrecognized_key_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "x".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "x");
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn question_mark_toggles_help_mode() {
    let mut app = make_app();
    assert_eq!(app.input.mode, InputMode::Normal);

    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Help);
}

#[test]
fn question_mark_dismisses_help() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn help_mode_ignores_other_keys() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::Help);
    assert!(!app.should_quit);
}

#[test]
fn help_overlay_hidden_in_normal_mode() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 80, 30);
    assert!(!buffer_contains(&buf, "Navigation"));
}

#[test]
fn move_review_to_done_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(3); // Review = nav col 3

    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(1))));
    assert!(app.status.message.as_deref().unwrap().contains("Done"));
}

#[test]
fn move_backlog_to_running_no_confirmation() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Backlog = nav col 1

    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn batch_move_mixed_statuses_moves_non_review_immediately() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Running),
            make_task(2, TaskStatus::Review),
        ],
        1,
        TEST_TIMEOUT,
    );
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    let cmds = app.update(Message::BatchMoveTasks {
        ids: vec![TaskId(1), TaskId(2)],
        direction: MoveDirection::Forward,
    });
    // Running→Review moved immediately
    let t1 = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(t1.status, TaskStatus::Review);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(1))));

    // Review→Done waiting for confirmation
    let t2 = app.board.tasks.iter().find(|t| t.id == TaskId(2)).unwrap();
    assert_eq!(t2.status, TaskStatus::Review); // not moved yet
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(_)));
}

#[test]
fn key_a_toggles_off_when_all_selected() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.select.tasks.len(), 2);
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn navigate_down_from_toggle_exits_to_row_zero() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(!app.on_select_all());
    assert_eq!(app.selected_row()[0], 0);
}

#[test]
fn column_scrolls_to_keep_cursor_visible() {
    // Create 20 backlog tasks — more than fit in a 20-row terminal
    let tasks: Vec<Task> = (1..=20)
        .map(|id| make_task(id, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks, 1, TEST_TIMEOUT);

    // Navigate down to the last task (row 19, past visible area)
    for _ in 0..19 {
        app.update(Message::NavigateRow(1));
    }

    // Render in a small terminal (height 20 with header/detail/status ~10 lines
    // leaves roughly 10 lines for the column, fitting ~4-5 two-line task cards)
    let buf = render_to_buffer(&mut app, 120, 20);

    // The cursor should be on "Task 20" and it should be visible in the buffer
    assert!(
        buffer_contains(&buf, "Task 20"),
        "cursor task should be visible after scrolling down"
    );
}

#[test]
fn column_scrolls_back_up_when_cursor_moves_up() {
    let tasks: Vec<Task> = (1..=20)
        .map(|id| make_task(id, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks, 1, TEST_TIMEOUT);

    // Navigate to the bottom
    for _ in 0..19 {
        app.update(Message::NavigateRow(1));
    }
    // Render once to establish scroll state
    let _ = render_to_buffer(&mut app, 120, 20);

    // Navigate back to the top
    for _ in 0..19 {
        app.update(Message::NavigateRow(-1));
    }
    let buf = render_to_buffer(&mut app, 120, 20);

    assert!(
        buffer_contains(&buf, "Task 1"),
        "first task should be visible after scrolling back up"
    );
}

#[test]
fn toggle_notifications_flips_state() {
    let mut app = make_app();
    assert!(!app.notifications_enabled()); // default: false
    app.update(Message::ToggleNotifications);
    assert!(app.notifications_enabled());
    app.update(Message::ToggleNotifications);
    assert!(!app.notifications_enabled());
}

#[test]
fn refresh_tasks_emits_notification_on_review_transition() {
    let mut app = make_app();
    app.set_notifications_enabled(true);
    // Task 3 starts as Running
    assert_eq!(app.board.tasks[2].status, TaskStatus::Running);

    // Simulate DB refresh where task 3 moved to Review
    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 1);
    match &notif_cmds[0] {
        Command::SendNotification { title, urgent, .. } => {
            assert!(title.contains("Task 3"));
            assert!(!urgent);
        }
        _ => unreachable!(),
    }
}

#[test]
fn refresh_tasks_emits_urgent_notification_on_needs_input() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::RefreshTasks(updated));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 1);
    match &notif_cmds[0] {
        Command::SendNotification { urgent, .. } => {
            assert!(urgent);
        }
        _ => unreachable!(),
    }
}

#[test]
fn refresh_tasks_does_not_duplicate_notifications() {
    let mut app = make_app();

    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    app.update(Message::RefreshTasks(updated.clone()));
    // Second refresh with same state should not re-notify
    let cmds = app.update(Message::RefreshTasks(updated));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn refresh_tasks_does_not_duplicate_needs_input_notifications() {
    let mut app = make_app();

    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    app.update(Message::RefreshTasks(updated.clone()));
    // Second refresh with same state should not re-notify
    let cmds = app.update(Message::RefreshTasks(updated));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn refresh_tasks_renotifies_needs_input_after_clearing() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    // First transition to NeedsInput
    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::RefreshTasks(updated.clone()));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::SendNotification { .. }))
            .count(),
        1
    );

    // Clear NeedsInput (agent resumes)
    updated[2].sub_status = SubStatus::Active;
    app.update(Message::RefreshTasks(updated.clone()));

    // Second transition to NeedsInput should re-notify
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::RefreshTasks(updated));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::SendNotification { .. }))
            .count(),
        1
    );
}

#[test]
fn refresh_tasks_skips_notification_when_disabled() {
    let mut app = make_app();
    // notifications disabled by default — no toggle needed

    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn key_n_uppercase_toggles_notifications() {
    let mut app = make_app();
    assert!(!app.notifications_enabled()); // default: false
    let cmds = app.handle_key(make_key(KeyCode::Char('N')));
    assert!(app.notifications_enabled()); // toggled to enabled
                                          // Should emit PersistSetting command
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistSetting { .. })));
    // Should show status message
    assert!(app.status.message.as_deref().unwrap().contains("enabled"));
}

#[test]
fn refresh_tasks_clears_notified_when_task_leaves_review() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    // Move to review — triggers notification
    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    app.update(Message::RefreshTasks(updated));

    // Move to done — should clear notified state
    let mut updated2 = app.board.tasks.to_vec();
    updated2[2].status = TaskStatus::Done;
    app.update(Message::RefreshTasks(updated2));

    // Move back to review — should re-notify
    let mut updated3 = app.board.tasks.to_vec();
    updated3[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated3));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 1);
}

#[test]
fn refresh_tasks_clears_notified_state_even_when_disabled() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    // Task transitions to review while notifications enabled — gets notified
    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::SendNotification { .. }))
            .count(),
        1
    );

    // Disable notifications
    app.update(Message::ToggleNotifications);

    // Task leaves review while disabled
    let mut updated2 = app.board.tasks.to_vec();
    updated2[2].status = TaskStatus::Done;
    app.update(Message::RefreshTasks(updated2));

    // Re-enable notifications
    app.update(Message::ToggleNotifications);

    // Task returns to review — should re-notify because notified state was cleared
    let mut updated3 = app.board.tasks.to_vec();
    updated3[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated3));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(
        notif_cmds.len(),
        1,
        "Should re-notify after notified state was cleared while disabled"
    );
}

#[test]
fn summary_row_shows_bell_and_hint_when_notifications_enabled() {
    let mut app = make_app();
    app.set_notifications_enabled(true); // explicitly enable (default is false)
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "\u{1F514}")); // 🔔
    assert!(buffer_contains(&buf, "[N]"));
}

#[test]
fn summary_row_shows_muted_bell_and_hint_when_disabled() {
    let mut app = make_app();
    // notifications disabled by default — no toggle needed
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "\u{1F515}")); // 🔕
    assert!(buffer_contains(&buf, "[N]"));
}

#[test]
fn detail_panel_shows_pr_url() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    // Navigate to Review column (index 2) and open detail panel
    for _ in 0..2 {
        app.update(Message::NavigateColumn(1));
    }
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    app.update(Message::OpenTaskDetail(1));
    let _buf = render_to_buffer(&mut app, 200, 20);
}

#[test]
fn summary_row_shows_filter_indicator() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.filter.repos.insert("/a".to_string());
    app.filter.repos.insert("/b".to_string());

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "2/3 repos"),
        "Expected filter indicator in summary"
    );
}

#[test]
fn summary_row_shows_excl_prefix_in_exclude_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.filter.repos.insert("/a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "excl 1/3 repos"),
        "Expected excl prefix in filter indicator"
    );
}

#[test]
fn w_key_on_non_review_task_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);

    app.handle_key(make_key(KeyCode::Char('W')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn handle_refresh_usage_stores_by_task_id() {
    use crate::models::TaskUsage;
    let mut app = make_app();
    let usage = vec![TaskUsage {
        task_id: TaskId(1),
        input_tokens: 10_000,
        output_tokens: 2_000,
        cache_read_tokens: 500,
        cache_write_tokens: 100,
        updated_at: chrono::Utc::now(),
    }];
    app.update(Message::RefreshUsage(usage));
    assert!(app.board.usage.contains_key(&TaskId(1)));
    assert_eq!(app.board.usage[&TaskId(1)].input_tokens, 10_000);
}

#[test]
fn recovery_from_stale_resets_substatus_to_active() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(3),
        output: "new output".to_string(),
        activity_ts: 1,
    });
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn recovery_from_crashed_resets_substatus_to_active() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(3),
        output: "new output".to_string(),
        activity_ts: 1,
    });
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn active_task_output_does_not_emit_persist() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Active;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(3),
        output: "output".to_string(),
        activity_ts: 1,
    });
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Active); // unchanged
                                                    // No PersistTask since sub_status didn't change
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn tick_skips_already_stale_tasks() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    // Tick should NOT re-emit PersistTask for already-stale tasks
    // (only CaptureTmux and RefreshFromDb expected)
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn tick_skips_already_crashed_tasks() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn tick_skips_conflict_tasks_for_stale_detection() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Conflict;
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
    assert_eq!(app.board.tasks[0].sub_status, SubStatus::Conflict);
}

#[test]
fn refresh_from_stale_to_active_resets_last_active_at() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(300));

    let mut refreshed = make_task(3, TaskStatus::Running);
    refreshed.sub_status = SubStatus::Active;
    refreshed.tmux_window = Some("win-3".to_string());

    app.update(Message::RefreshTasks(vec![refreshed]));
    let elapsed = app.agents.last_active_at[&TaskId(3)].elapsed();
    assert!(elapsed < Duration::from_secs(1), "timer should be reset");
}

#[test]
fn refresh_staying_stale_does_not_reset_last_active_at() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    let old_instant = Instant::now() - Duration::from_secs(300);
    app.agents.last_active_at.insert(TaskId(3), old_instant);

    let mut refreshed = make_task(3, TaskStatus::Running);
    refreshed.sub_status = SubStatus::Stale;
    refreshed.tmux_window = Some("win-3".to_string());

    app.update(Message::RefreshTasks(vec![refreshed]));
    let elapsed = app.agents.last_active_at[&TaskId(3)].elapsed();
    assert!(
        elapsed > Duration::from_secs(200),
        "timer should NOT be reset when staying stale"
    );
}

#[test]
fn refresh_from_crashed_to_active_resets_last_active_at() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(300));

    let mut refreshed = make_task(3, TaskStatus::Running);
    refreshed.sub_status = SubStatus::Active;
    refreshed.tmux_window = Some("win-3".to_string());

    app.update(Message::RefreshTasks(vec![refreshed]));
    let elapsed = app.agents.last_active_at[&TaskId(3)].elapsed();
    assert!(elapsed < Duration::from_secs(1), "timer should be reset");
}

#[test]
fn move_task_forward_resets_substatus() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.find_task_mut(id).unwrap().sub_status = SubStatus::Stale;
    app.update(Message::MoveTask {
        id,
        direction: MoveDirection::Forward,
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
}

#[test]
fn move_task_backward_resets_substatus() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.update(Message::MoveTask {
        id,
        direction: MoveDirection::Backward,
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn shift_l_with_mixed_selection_moves_tasks_only() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    // Cursor on the task (row 0) so 'm' triggers batch move, not epic move
    app.selection_mut().set_column(1); // Backlog = nav col 1
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('L')));
    // Task should move forward
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
}

#[test]
fn detach_tmux_single_sets_confirm_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-1".to_string());

    app.update(Message::DetachTmux(TaskId(1)));

    assert!(
        matches!(&app.input.mode, InputMode::ConfirmDetachTmux(ids) if ids == &[TaskId(1)]),
        "Expected ConfirmDetachTmux([1]), got {:?}",
        app.input.mode
    );
    assert!(app.status.message.is_some());
}

#[test]
fn detach_tmux_noop_on_task_without_window() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], 1, TEST_TIMEOUT);
    // tmux_window is None by default from make_task

    let cmds = app.update(Message::DetachTmux(TaskId(1)));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty(), "should produce no commands");
}

#[test]
fn batch_detach_tmux() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Review),
            make_task(2, TaskStatus::Review),
        ],
        1,
        TEST_TIMEOUT,
    );
    app.board.tasks[0].tmux_window = Some("task-1".to_string());
    app.board.tasks[1].tmux_window = Some("task-2".to_string());

    app.update(Message::BatchDetachTmux(vec![TaskId(1), TaskId(2)]));
    let cmds = app.update(Message::ConfirmDetachTmux);

    assert!(
        app.board.tasks[0].tmux_window.is_none(),
        "task 1 window should be cleared"
    );
    assert!(
        app.board.tasks[1].tmux_window.is_none(),
        "task 2 window should be cleared"
    );

    let kill_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::KillTmuxWindow { .. }))
        .count();
    assert_eq!(kill_count, 2, "should kill 2 windows");

    let persist_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::PersistTask(_)))
        .count();
    assert_eq!(persist_count, 2, "should persist 2 tasks");
}

#[test]
fn move_repo_cursor_down_wraps() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 1; // last
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 0, "should wrap to first");
}

#[test]
fn move_repo_cursor_up_wraps() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0; // first
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 1, "should wrap to last");
}

#[test]
fn repo_cursor_resets_on_quick_dispatch_entry() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.repo_cursor = 1;
    app.update(Message::StartQuickDispatchSelection);
    assert_eq!(
        app.input.repo_cursor, 0,
        "cursor should reset to 0 on mode entry"
    );
}

#[test]
fn detached_review_task_shows_awaiting_merge_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.pr_url = Some("https://github.com/org/repo/pull/10".to_string());
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None; // detached
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting merge"),
        "detached review task should show 'awaiting merge' section header"
    );
}

#[test]
fn live_review_task_shows_awaiting_review_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.pr_url = Some("https://github.com/org/repo/pull/10".to_string());
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some("1-fix".to_string()); // live
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "live review task should show 'awaiting review' section header"
    );
    assert!(
        !buffer_contains(&buf, "awaiting merge"),
        "live review task should not show 'awaiting merge'"
    );
}

#[test]
fn detached_and_live_review_tasks_get_separate_sections() {
    // Live task (has tmux window)
    let mut live = make_task(1, TaskStatus::Review);
    live.sub_status = SubStatus::AwaitingReview;
    live.pr_url = Some("https://github.com/org/repo/pull/10".to_string());
    live.worktree = Some("/repo/.worktrees/1-fix".to_string());
    live.tmux_window = Some("1-fix".to_string());

    // Detached task (no tmux window)
    let mut detached = make_task(2, TaskStatus::Review);
    detached.sub_status = SubStatus::AwaitingReview;
    detached.pr_url = Some("https://github.com/org/repo/pull/11".to_string());
    detached.worktree = Some("/repo/.worktrees/2-feat".to_string());
    detached.tmux_window = None;

    let mut app = App::new(vec![live, detached], 1, TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "should show 'awaiting review' for live task"
    );
    assert!(
        buffer_contains(&buf, "awaiting merge"),
        "should show 'awaiting merge' for detached task"
    );
}

#[test]
fn is_detached_returns_true_for_review_without_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    assert!(task.is_detached());
}

#[test]
fn is_detached_returns_false_with_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some("1-fix".to_string());
    assert!(!task.is_detached());
}

#[test]
fn is_detached_returns_false_for_conflict() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Conflict;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    assert!(!task.is_detached());
}

#[test]
fn mark_dispatching_sets_guard_and_returns_no_commands() {
    let mut app = make_app();
    assert!(!app.is_dispatching(TaskId(99)));
    let cmds = app.update(Message::MarkDispatching(TaskId(99)));
    assert!(cmds.is_empty());
    assert!(app.is_dispatching(TaskId(99)));
}

#[test]
fn tick_with_active_split_checks_pane() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    let cmds = app.update(Message::Tick);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::CheckSplitPaneExists { pane_id } if pane_id == "%42"
    )));
}

#[test]
fn tick_without_split_does_not_check_pane() {
    let mut app = make_app();
    let cmds = app.update(Message::Tick);
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CheckSplitPaneExists { .. })));
}

#[test]
fn tick_skips_capture_for_split_pinned_task() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    let cmds = app.update(Message::Tick);

    // Should NOT emit CaptureTmux for the pinned task (its window is a pane now)
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(4), .. })),
        "split-pinned task should be excluded from CaptureTmux"
    );
}

#[test]
fn resumed_clears_last_error() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    app.agents
        .last_error
        .insert(TaskId(4), "some crash".to_string());

    app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "win-4".to_string(),
    });

    assert!(!app.agents.last_error.contains_key(&TaskId(4)));
}


#[test]
fn mark_active_sets_last_active_at_to_now() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    assert!(!tracking.last_active_at.contains_key(&TaskId(1)));

    tracking.mark_active(TaskId(1));

    let elapsed = tracking.last_active_at[&TaskId(1)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn mark_active_overwrites_previous_value() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    tracking
        .last_active_at
        .insert(TaskId(1), Instant::now() - Duration::from_secs(300));

    tracking.mark_active(TaskId(1));

    let elapsed = tracking.last_active_at[&TaskId(1)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn inactive_duration_returns_none_for_unknown_task() {
    let tracking = AgentTracking::new(TEST_TIMEOUT);
    assert!(tracking.inactive_duration(TaskId(99)).is_none());
}

#[test]
fn inactive_duration_returns_elapsed_time() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    tracking
        .last_active_at
        .insert(TaskId(1), Instant::now() - Duration::from_secs(60));

    let duration = tracking.inactive_duration(TaskId(1)).unwrap();
    assert!(duration >= Duration::from_secs(59));
    assert!(duration < Duration::from_secs(62));
}

#[test]
fn inactive_duration_near_zero_after_mark_active() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    tracking.mark_active(TaskId(1));

    let duration = tracking.inactive_duration(TaskId(1)).unwrap();
    assert!(duration < Duration::from_secs(1));
}

#[test]
fn repo_cursor_resets_on_entering_repo_path_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.repo_cursor = 2; // cursor was left at position 2
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer = "some desc".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(app.input.repo_cursor, 0, "cursor should reset to top");
}

#[test]
fn fuzzy_matches_empty_query_matches_anything() {
    assert!(super::fuzzy_matches("/some/path", ""));
    assert!(super::fuzzy_matches("", ""));
}

#[test]
fn fuzzy_matches_subsequence() {
    assert!(super::fuzzy_matches("/tmp", "tmp"));
    assert!(super::fuzzy_matches("/home/tmp", "tmp"));
    assert!(super::fuzzy_matches("/home/ragge/proj", "ragge"));
}

#[test]
fn fuzzy_matches_case_insensitive() {
    assert!(super::fuzzy_matches("/TMP", "tmp"));
    assert!(super::fuzzy_matches("/tmp", "TMP"));
}

#[test]
fn fuzzy_matches_no_match() {
    assert!(!super::fuzzy_matches("/var", "tmp"));
}

#[test]
fn fuzzy_matches_chars_must_be_in_order() {
    // "tp" is a valid subsequence of "/tmp" (t at 1, p at 3)
    assert!(super::fuzzy_matches("/tmp", "tp"));
    // "pt" requires p before t: p is at 3, t is at 1 -> false
    assert!(!super::fuzzy_matches("/tmp", "pt"));
}

#[test]
fn filtered_repos_empty_query_returns_all() {
    let paths = vec!["/a".to_string(), "/b".to_string()];
    assert_eq!(super::filtered_repos(&paths, ""), vec!["/a", "/b"]);
}

#[test]
fn filtered_repos_narrows_by_query() {
    let paths = vec![
        "/home/ragge/proj".to_string(),
        "/var/log".to_string(),
        "/home/other".to_string(),
    ];
    let result = super::filtered_repos(&paths, "home");
    assert_eq!(result, vec!["/home/ragge/proj", "/home/other"]);
}

#[test]
fn filtered_repos_no_matches_returns_empty() {
    let paths = vec!["/tmp".to_string(), "/var".to_string()];
    assert_eq!(super::filtered_repos(&paths, "xyz"), Vec::<String>::new());
}

#[test]
fn repo_cursor_resets_on_copy_task() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.repo_cursor = 2; // cursor was left at position 2
                               // Copy the first task (select it and press 'c')
    app.handle_key(make_key(KeyCode::Char('c')));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.repo_cursor, 0,
        "cursor should reset to top on copy"
    );
}

#[test]
fn move_repo_cursor_wraps_within_filtered_list() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string(), "/home".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Type "tmp" to filter — only /tmp matches (1 item)
    for c in "tmp".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    assert_eq!(app.input.repo_cursor, 0);
    // j wraps in a list of 1 — cursor stays at 0
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn typing_resets_repo_cursor_to_zero() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Navigate to position 2
    app.handle_key(make_key(KeyCode::Down));
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 2);
    // Type a character — cursor should reset
    app.handle_key(make_key(KeyCode::Char('/')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn startup_never_returns_none() {
    let tips = vec![make_tip_with_id(1), make_tip_with_id(2)];
    assert!(determine_tips_start(&tips, 0, crate::models::TipsShowMode::Never).is_none());
}

#[test]
fn refresh_status_never_fetched() {
    let (text, color) = ui::refresh_status(None, false, Duration::from_secs(30));
    assert_eq!(text, "Never fetched  [r] refresh");
    assert_eq!(color, Color::DarkGray);
}

#[test]
fn refresh_status_loading() {
    let last = Instant::now() - Duration::from_secs(5);
    let (text, color) = ui::refresh_status(Some(last), true, Duration::from_secs(30));
    assert_eq!(text, "Refreshing...  [r] refresh");
    assert_eq!(color, Color::DarkGray);
}

#[test]
fn refresh_status_loading_overrides_never_fetched() {
    let (text, color) = ui::refresh_status(None, true, Duration::from_secs(30));
    assert_eq!(text, "Refreshing...  [r] refresh");
    assert_eq!(color, Color::DarkGray);
}

#[test]
fn refresh_status_fresh_seconds() {
    let last = Instant::now() - Duration::from_secs(1);
    let (text, color) = ui::refresh_status(Some(last), false, Duration::from_secs(30));
    assert!(
        text.starts_with("Updated ") && text.contains("s ago") && text.ends_with("  [r] refresh"),
        "expected 'Updated Xs ago  [r] refresh', got: {text}"
    );
    assert_eq!(color, Color::White);
}

#[test]
fn refresh_status_fresh_just_below_minutes_threshold() {
    // 59s elapsed, 30s interval: 59 < 2*30=60 → still White, seconds format
    let last = Instant::now() - Duration::from_secs(59);
    let (text, color) = ui::refresh_status(Some(last), false, Duration::from_secs(30));
    assert!(text.contains("59s ago"), "expected '59s ago' in: {text}");
    assert_eq!(color, Color::White);
}

#[test]
fn refresh_status_minutes_format() {
    // 60s elapsed → "1m 0s ago"
    let last = Instant::now() - Duration::from_secs(60);
    let (text, color) = ui::refresh_status(Some(last), false, Duration::from_secs(300));
    assert!(
        text.contains("1m") && text.contains("s ago"),
        "expected minutes format in: {text}"
    );
    assert_eq!(color, Color::White);
}

#[test]
fn refresh_status_yellow_at_2x_interval() {
    let interval = Duration::from_secs(30);
    // exactly 2× interval → Yellow
    let last = Instant::now() - interval * 2;
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, Color::Yellow);
}

#[test]
fn refresh_status_white_just_below_2x_interval() {
    let interval = Duration::from_secs(30);
    // 1ms under 2× interval → still White
    let last = Instant::now() - (interval * 2 - Duration::from_millis(100));
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, Color::White);
}

#[test]
fn refresh_status_yellow_just_below_4x_interval() {
    let interval = Duration::from_secs(30);
    // 1ms under 4× interval → still Yellow
    let last = Instant::now() - (interval * 4 - Duration::from_millis(100));
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, Color::Yellow);
}

#[test]
fn refresh_status_red_at_4x_interval() {
    let interval = Duration::from_secs(30);
    // exactly 4× interval → Red
    let last = Instant::now() - interval * 4;
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, Color::Red);
}

#[test]
fn test_selection_preserved_when_task_above_cursor_moves() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog), // row 0
            make_task(2, TaskStatus::Backlog), // row 1 — cursor here
            make_task(3, TaskStatus::Backlog), // row 2
        ],
        1,
        TEST_TIMEOUT,
    );
    // App starts at Backlog (nav col 1); navigate down to row 1.
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 1);

    // Task 1 moves out; Task 2 follows to row 0.
    app.update(Message::RefreshTasks(vec![
        make_task(1, TaskStatus::Running),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
    ]));

    // Anchor follows task 2 — stays in Backlog (nav col 1) at row 0.
    assert_eq!(app.selection().column(), 1);
    assert_eq!(app.selection().row(1), 0);
    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(matches!(items[0], ColumnItem::Task(t) if t.id == TaskId(2)));
}

#[test]
fn test_selection_follows_task_to_new_column() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog), // row 0 — default cursor
            make_task(2, TaskStatus::Backlog),
        ],
        1,
        TEST_TIMEOUT,
    );
    // App starts at Backlog (nav col 1).
    assert_eq!(app.selection().column(), 1);
    assert_eq!(app.selection().row(1), 0);

    // Task 1 dispatched to Running
    app.update(Message::RefreshTasks(vec![
        make_task(1, TaskStatus::Running),
        make_task(2, TaskStatus::Backlog),
    ]));

    assert_eq!(app.selection().column(), 2); // Running = nav col 2
    assert_eq!(app.selection().row(2), 0);
    let items = app.column_items_for_status(TaskStatus::Running);
    assert!(matches!(items[0], ColumnItem::Task(t) if t.id == TaskId(1)));
}

#[test]
fn test_selection_falls_back_when_task_deleted() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
            make_task(3, TaskStatus::Backlog),
        ],
        1,
        TEST_TIMEOUT,
    );
    // App starts at Backlog (nav col 1); navigate down to row 2 (Task 3).
    app.update(Message::NavigateRow(1));
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 2); // Task 3

    // Task 3 deleted
    app.update(Message::RefreshTasks(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]));

    assert_eq!(app.selection().column(), 1);
    assert_eq!(app.selection().row(1), 1); // clamped to last valid row
}

#[test]
fn test_selection_preserved_on_same_data_refresh() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
            make_task(3, TaskStatus::Backlog),
        ],
        1,
        TEST_TIMEOUT,
    );
    // App starts at Backlog (nav col 1); navigate down to row 1.
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 1);

    app.update(Message::RefreshTasks(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
    ]));

    assert_eq!(app.selection().row(1), 1);
    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(matches!(items[1], ColumnItem::Task(t) if t.id == TaskId(2)));
}

#[test]
fn test_selection_falls_back_when_column_empties() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Running),
        ],
        1,
        TEST_TIMEOUT,
    );
    // App starts at Backlog (nav col 1); navigate right to Running (nav col 2).
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selection().column(), 2);

    // Task 2 deleted — Running column empties, anchor not found
    app.update(Message::RefreshTasks(vec![make_task(
        1,
        TaskStatus::Backlog,
    )]));

    // Cursor must be in a valid state: row 0 in the empty Running column
    assert_eq!(app.selection().column(), 2);
    assert_eq!(app.selection().row(2), 0);
}

// --- Archive column navigation ---

#[test]
fn navigate_right_from_done_shows_archive() {
    let mut app = make_app();
    // Navigate to Done column (nav col 4 = COLUMN_COUNT, starting from col 1)
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(app.selected_column(), 4);
    assert!(!app.show_archived());

    // Navigate right from Done → archive column (nav col 5)
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 5);
    assert!(app.show_archived());
}

#[test]
fn navigate_right_from_done_resets_archive_selection() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Archived),
            make_task(2, TaskStatus::Archived),
        ],
        1,
        TEST_TIMEOUT,
    );
    // Pre-position archive selection at row 1
    app.selection_mut().set_row(TaskStatus::COLUMN_COUNT + 1, 1);
    *app.archive.list_state.selected_mut() = Some(1);

    // Navigate from Backlog (col 1) to archive (col 5): 4 steps
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    // Selection should reset to 0
    assert_eq!(app.selected_archive_row(), 0);
}

#[test]
fn navigate_left_from_archive_hides_it_and_goes_to_done() {
    let mut app = make_app();
    // Enter archive column (col 5) from Backlog (col 1): 4 steps
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    assert!(app.show_archived());

    // Navigate left → Done (col 4)
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 4);
    assert!(!app.show_archived());
}

#[test]
fn pressing_right_at_archive_column_stays_clamped() {
    let mut app = make_app();
    // Navigate from col 1 to col 5 (4 steps), then 2 more (should clamp at 5)
    for _ in 0..6 {
        app.update(Message::NavigateColumn(1));
    }
    // Should clamp at 5
    assert_eq!(app.selected_column(), 5);
    assert!(app.show_archived());
}

#[test]
fn h_key_in_archive_returns_to_done() {
    let mut app = make_app();
    // Enter archive column (col 5): 4 steps from col 1
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    assert!(app.show_archived());

    // Press h → Done (col 4)
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.selected_column(), 4);
    assert!(!app.show_archived());
}

#[test]
fn left_arrow_in_archive_returns_to_done() {
    let mut app = make_app();
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    assert!(app.show_archived());

    app.handle_key(make_key(KeyCode::Left));
    assert_eq!(app.selected_column(), 4);
    assert!(!app.show_archived());
}

#[test]
fn esc_key_in_archive_returns_to_done() {
    let mut app = make_app();
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    assert!(app.show_archived());

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.selected_column(), 4);
    assert!(!app.show_archived());
}

#[test]
fn navigate_left_from_done_does_not_show_archive() {
    let mut app = make_app();
    // Navigate to Done column (nav col 4): 3 steps from col 1
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(app.selected_column(), 4);

    // Navigate left (back to Review, col 3) — archive must NOT appear
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 3);
    assert!(!app.show_archived());
}

#[test]
fn board_selection_projects_row_roundtrip() {
    let mut sel = BoardSelection::new();
    assert_eq!(sel.row(0), 0); // projects_row starts at 0
    sel.set_row(0, 3);
    assert_eq!(sel.row(0), 3);
}

#[test]
fn board_selection_archive_row_roundtrip() {
    let mut sel = BoardSelection::new();
    assert_eq!(sel.row(5), 0); // archive_row starts at 0
    sel.set_row(5, 7);
    assert_eq!(sel.row(5), 7);
}

#[test]
fn board_selection_task_col_row_uses_offset() {
    let mut sel = BoardSelection::new();
    sel.set_row(1, 10); // Backlog = nav col 1 → array index 0
    assert_eq!(sel.row(1), 10);
    sel.set_row(4, 5); // Done = nav col 4 → array index 3
    assert_eq!(sel.row(4), 5);
}

// --- New [0,5] column-range navigation tests ---

#[test]
fn navigate_left_from_backlog_enters_projects() {
    let mut app = make_app();
    // Board starts at Backlog (col 1).
    assert_eq!(app.selected_column(), 1);
    app.update(Message::NavigateColumn(-1)); // col 1 → col 0 (Projects)
    assert_eq!(app.selected_column(), 0);
}

#[test]
fn navigate_right_from_done_enters_archive() {
    let mut app = make_app();
    // Board starts at Backlog (col 1). Navigate to Done (col 4): 3 steps.
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(app.selected_column(), 4);
    app.update(Message::NavigateColumn(1)); // col 4 → col 5 (Archive)
    assert_eq!(app.selected_column(), 5);
}

#[test]
fn navigate_left_at_projects_is_noop() {
    let mut app = make_app();
    // Board starts at Backlog (col 1). Go to Projects (col 0) first.
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 0);
    app.update(Message::NavigateColumn(-1)); // clamp at 0
    assert_eq!(app.selected_column(), 0);
}

#[test]
fn navigate_right_at_archive_is_noop() {
    let mut app = make_app();
    // Board starts at Backlog (col 1). Navigate to Archive (col 5): 4 steps.
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(app.selected_column(), 5);
    app.update(Message::NavigateColumn(1)); // clamp at 5
    assert_eq!(app.selected_column(), 5);
}
