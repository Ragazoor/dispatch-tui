//! `schedules.allium: FireDueTask` — the board's tick asks for a dispatch of
//! every due backlog task it can see. Whether the task's repo exists on this
//! machine is the runtime's check (`exec_fire_scheduled`), not the tick's.

use chrono::{Duration, Utc};

use super::helpers::{make_app, make_task};
use crate::models::{DispatchRequestId, TaskId, TaskStatus};
use crate::tui::commands::TaskCommand;
use crate::tui::Command;

fn fired(cmds: &[Command]) -> Vec<TaskId> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Task(TaskCommand::FireScheduled { due }) => {
                Some(due.iter().map(|(id, _)| *id).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn with_next_due(id: i64, status: TaskStatus, offset: Duration) -> crate::models::Task {
    crate::models::Task {
        schedule: Some("0 9 * * 1".into()),
        next_due: Some(Utc::now() + offset),
        ..make_task(id, status)
    }
}

#[test]
fn a_due_backlog_task_is_fired() {
    let mut app = make_app();
    app.board.tasks.push(with_next_due(
        10,
        TaskStatus::Backlog,
        -Duration::minutes(1),
    ));
    assert_eq!(fired(&app.handle_tick()), vec![TaskId(10)]);
}

#[test]
fn a_task_not_yet_due_or_not_in_backlog_or_unscheduled_is_not_fired() {
    let mut app = make_app();
    app.board
        .tasks
        .push(with_next_due(10, TaskStatus::Backlog, Duration::days(1)));
    app.board
        .tasks
        .push(with_next_due(11, TaskStatus::Review, -Duration::minutes(1)));
    // make_app's own backlog tasks have no schedule at all.
    assert!(fired(&app.handle_tick()).is_empty());
}

#[test]
fn a_task_with_an_open_request_is_not_fired_again() {
    let mut app = make_app();
    app.board.tasks.push(with_next_due(
        10,
        TaskStatus::Backlog,
        -Duration::minutes(1),
    ));
    app.dispatch_requests
        .open
        .insert(TaskId(10), DispatchRequestId(1));
    assert!(fired(&app.handle_tick()).is_empty());
}

/// A fire takes a reducer round trip to move the task out of backlog. The
/// ticks in between do not ask again.
#[test]
fn a_fired_task_is_not_fired_again_on_the_next_tick() {
    let mut app = make_app();
    app.board.tasks.push(with_next_due(
        10,
        TaskStatus::Backlog,
        -Duration::minutes(1),
    ));
    assert_eq!(fired(&app.handle_tick()), vec![TaskId(10)]);
    assert!(fired(&app.handle_tick()).is_empty());
}
