//! `tasks.allium: ClearBuildOutputOfDoneTask` — which Done tasks the board's
//! tick hands to the host for clearing, and how often.

use super::*;

use crate::models::{TaskId, TaskStatus};
use crate::tui::commands::TaskCommand;
use crate::tui::messages::SystemMessage;

const HOST: &str = "this-machine";

/// A task in `status` holding `worktree` on `host`.
fn held(id: i64, status: TaskStatus, worktree: &str, host: &str) -> Task {
    Task {
        worktree: Some(worktree.to_string()),
        host: Some(host.to_string()),
        tmux_window: None,
        ..make_task(id, status)
    }
}

fn app_with(tasks: Vec<Task>) -> App {
    let mut app = App::new(tasks);
    app.set_local_host_id(HOST.to_string());
    app
}

/// The worktrees the tick asked to clear, in the order it named them.
fn cleared_on_tick(app: &mut App) -> Vec<(TaskId, String)> {
    app.update(Message::System(SystemMessage::Tick))
        .into_iter()
        .filter_map(|c| match c {
            Command::Task(TaskCommand::ClearBuildOutput { worktrees }) => Some(worktrees),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn a_done_task_held_here_is_cleared() {
    let mut app = app_with(vec![held(1, TaskStatus::Done, "/wt/1", HOST)]);

    assert_eq!(
        cleared_on_tick(&mut app),
        vec![(TaskId(1), "/wt/1".to_string())]
    );
}

#[test]
fn a_task_is_cleared_once_per_board_run() {
    let mut app = app_with(vec![held(1, TaskStatus::Done, "/wt/1", HOST)]);

    assert_eq!(cleared_on_tick(&mut app).len(), 1);
    assert!(cleared_on_tick(&mut app).is_empty());
}

#[test]
fn a_task_not_done_is_not_cleared() {
    let mut app = app_with(vec![
        held(1, TaskStatus::Running, "/wt/1", HOST),
        held(2, TaskStatus::Review, "/wt/2", HOST),
        held(3, TaskStatus::Backlog, "/wt/3", HOST),
    ]);

    assert!(cleared_on_tick(&mut app).is_empty());
}

#[test]
fn a_done_task_held_by_another_host_is_not_cleared() {
    let mut app = app_with(vec![held(1, TaskStatus::Done, "/wt/1", "other-machine")]);

    assert!(cleared_on_tick(&mut app).is_empty());
}

#[test]
fn nothing_is_cleared_before_the_host_id_is_known() {
    let mut app = App::new(vec![held(1, TaskStatus::Done, "/wt/1", HOST)]);

    assert!(cleared_on_tick(&mut app).is_empty());
}

#[test]
fn a_done_task_without_a_worktree_is_not_cleared() {
    let mut app = app_with(vec![make_task(1, TaskStatus::Done)]);

    assert!(cleared_on_tick(&mut app).is_empty());
}

#[test]
fn a_worktree_another_task_still_works_in_is_skipped_until_it_is_done() {
    let mut app = app_with(vec![
        held(1, TaskStatus::Done, "/wt/shared", HOST),
        held(2, TaskStatus::Running, "/wt/shared", HOST),
    ]);

    assert!(cleared_on_tick(&mut app).is_empty());

    app.board.tasks[1].status = TaskStatus::Done;
    let cleared = cleared_on_tick(&mut app);
    assert!(
        cleared.contains(&(TaskId(1), "/wt/shared".to_string())),
        "task 1 was not marked cleared while skipped, got {cleared:?}"
    );
}

#[test]
fn a_task_that_leaves_done_and_returns_is_cleared_again() {
    let mut app = app_with(vec![held(1, TaskStatus::Done, "/wt/1", HOST)]);
    assert_eq!(cleared_on_tick(&mut app).len(), 1);

    app.board.tasks[0].status = TaskStatus::Running;
    assert!(cleared_on_tick(&mut app).is_empty());

    app.board.tasks[0].status = TaskStatus::Done;
    assert_eq!(
        cleared_on_tick(&mut app),
        vec![(TaskId(1), "/wt/1".to_string())]
    );
}
