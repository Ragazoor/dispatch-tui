//! The ConfirmDone half of `DeleteKeyRouting` (tasks.allium): `x` on a task
//! that is not yet Done always routes to ConfirmDone, never to a delete
//! confirmation — whatever the selection shape. The delete half (`x` on a
//! Done task or a qualifying epic) is covered by `delete.rs`.
use super::*;
use crate::models::{test_tmux_window, TaskId, TaskStatus};
use crossterm::event::KeyCode;

#[test]
fn x_key_on_backlog_task_enters_confirm_done_not_archive() {
    let mut app = make_app();
    // make_app() starts at Backlog (nav col 1) — no need to set_column.
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('x'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    assert_eq!(app.select.pending_done, vec![TaskId(1)]);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Backlog,
        "task should not move until confirmed"
    );
}

#[test]
fn confirm_x_on_backlog_task_moves_to_done_not_archived() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('x')));
    app.handle_key(make_key(KeyCode::Char('y')));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
}

#[test]
fn x_key_on_running_task_moves_to_done_and_preserves_worktree() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/wt/1-test".to_string());
    task.tmux_window = Some(test_tmux_window("dev:1-test"));
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running (nav col 2)

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    assert_eq!(app.select.pending_done, vec![TaskId(1)]);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('y'))));

    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(
        task.worktree.as_deref(),
        Some("/wt/1-test"),
        "moving to Done must not remove the worktree"
    );
    assert!(task.tmux_window.is_none(), "tmux window should be killed");
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::Cleanup { .. })
        )),
        "moving to Done must not emit worktree Cleanup"
    );
}

#[test]
fn x_key_on_review_task_enters_confirm_done_not_archive() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)]);
    app.selection_mut().set_column(3); // Review (nav col 3)
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('x'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    assert_eq!(app.select.pending_done, vec![TaskId(1)]);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "task should not move until confirmed"
    );
}

#[test]
fn confirm_x_on_review_task_y_moves_to_done_not_archived() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)]);
    app.selection_mut().set_column(3); // Review (nav col 3)
    app.handle_key(make_key(KeyCode::Char('x')));
    app.handle_key(make_key(KeyCode::Char('y')));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
}

#[test]
fn x_key_on_all_review_selection_enters_confirm_done_not_archive() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Review),
        make_task(2, TaskStatus::Review),
    ]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    app.handle_key(make_key(KeyCode::Char('x')));

    assert_eq!(
        app.input.mode,
        InputMode::ConfirmDone,
        "expected ConfirmDone, got {:?}",
        app.input.mode
    );
    app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Done
    );
    assert_eq!(
        app.board.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Done
    );
}

#[test]
fn x_key_on_mixed_status_selection_moves_non_done_to_done() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Done),
    ]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmDone,
        "expected ConfirmDone, got {:?}",
        app.input.mode
    );
    app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Done
    );
    assert_eq!(
        app.board.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Done,
        "the already-Done task in the selection is left untouched"
    );
}

#[test]
fn confirm_done_n_cancels_and_leaves_task_in_place() {
    let mut app = make_app();
    // make_app() starts at Backlog (nav col 1).
    app.handle_key(make_key(KeyCode::Char('x')));
    let _ = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
}
