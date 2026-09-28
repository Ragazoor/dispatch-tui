//! The `x` key as a permanent-delete gesture (task #4971).
//!
//! tasks.allium: DeleteTask, BatchDelete and the DeleteKeyRouting guidance;
//! epics.allium: ConfirmDeleteEpic, DeleteEpic. `x` on a Done task or a fully
//! done epic deletes it after a y/n confirmation; `x` on anything unfinished
//! still completes it (ConfirmDone). There is no archived status to fall back
//! to — a finished task either stays in Done or is gone.
//!
//! These tests drive the TUI through keys only and observe the board and the
//! emitted commands, so they do not depend on how the confirmation modes and
//! messages are named internally. The ConfirmDone half of the routing (`x` on
//! a non-Done task) is covered by the `x_key_on_*_enters_confirm_done_*` tests
//! in `confirm_done.rs`; `x_on_a_mixed_task_selection_deletes_nothing` below adds
//! the no-delete half of it.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{test_tmux_window, EpicId, TaskId, TaskStatus};
use crate::tui::commands::{CleanupFollowUp, EpicCommand, TaskCommand};
use crossterm::event::KeyCode;

fn press(app: &mut App, c: char) -> Vec<Command> {
    app.handle_key(make_key(KeyCode::Char(c)))
}

fn task_in(id: i64, status: TaskStatus, epic: i64) -> Task {
    let mut t = make_task(id, status);
    t.epic_id = Some(EpicId(epic));
    t
}

fn deleted_task_ids(cmds: &[Command]) -> Vec<TaskId> {
    let mut ids: Vec<TaskId> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::Task(TaskCommand::Delete(id)) => Some(*id),
            Command::Task(TaskCommand::Cleanup {
                id,
                follow_up: CleanupFollowUp::DeleteRow,
                ..
            }) => Some(*id),
            _ => None,
        })
        .collect();
    // tasks.allium: BatchDelete's atomic call — a multi-item selection emits
    // ONE `BatchDelete` command carrying every id, rather than `Delete`/
    // `Cleanup{follow_up: DeleteRow}` issued once per item.
    for c in cmds {
        if let Command::Task(TaskCommand::BatchDelete { task_ids, .. }) = c {
            ids.extend(task_ids.iter().copied());
        }
    }
    ids
}

fn deleted_epic_ids(cmds: &[Command]) -> Vec<EpicId> {
    let mut ids: Vec<EpicId> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::Epic(EpicCommand::Delete(id)) => Some(*id),
            _ => None,
        })
        .collect();
    for c in cmds {
        if let Command::Task(TaskCommand::BatchDelete { epic_ids, .. }) = c {
            ids.extend(epic_ids.iter().copied());
        }
    }
    ids
}

fn status_message(app: &App) -> String {
    app.status.message.clone().unwrap_or_default()
}

fn assert_selected_epic(app: &App, id: i64) {
    assert!(
        matches!(app.selected_column_item(), Some(ColumnItem::Epic(e)) if e.id == EpicId(id)),
        "fixture: the cursor must be on epic {id}"
    );
}

// --- Single task ------------------------------------------------------------

#[test]
fn x_on_a_done_task_asks_to_delete_it() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    app.selection_mut().set_column(4); // Done (nav col 4)

    let cmds = without_usage(press(&mut app, 'x'));

    assert!(cmds.is_empty(), "nothing happens before confirmation");
    assert_ne!(app.input.mode, InputMode::Normal, "a y/n prompt is shown");
    assert_ne!(
        app.input.mode,
        InputMode::ConfirmDone,
        "a Done task is deleted, not completed again"
    );
    let msg = status_message(&app);
    assert!(
        msg.contains("Delete") && msg.contains("[y/n]"),
        "the prompt names the delete, got {msg:?}"
    );
    assert!(app.find_task(TaskId(1)).is_some());
}

#[test]
fn confirming_delete_of_a_done_task_removes_it_from_the_board() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    app.selection_mut().set_column(4);

    press(&mut app, 'x');
    let cmds = press(&mut app, 'y');

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        app.find_task(TaskId(1)).is_none(),
        "the task leaves the board outright — there is no archived state"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Delete(TaskId(1))))),
        "a task owning neither worktree nor window is deleted immediately, got {cmds:?}"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Persist(_)))),
        "deleting persists no status change, got {cmds:?}"
    );
}

/// WorktreeReleaseIsGated: the row delete is the teardown's follow-up, never
/// a sibling of it, so a failed removal leaves the row (and its worktree
/// pointer) for a retry.
#[test]
fn confirming_delete_of_a_done_task_with_a_worktree_gates_the_row_on_teardown() {
    let mut task = make_task(1, TaskStatus::Done);
    task.worktree = Some("/wt/1-test".to_string());
    task.tmux_window = Some(test_tmux_window("dev:1-test"));
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(4);

    press(&mut app, 'x');
    let cmds = press(&mut app, 'y');

    let cleanup = cmds
        .iter()
        .find_map(|c| match c {
            Command::Task(TaskCommand::Cleanup {
                id,
                worktree,
                follow_up,
                ..
            }) if *id == TaskId(1) => Some((worktree.clone(), *follow_up)),
            _ => None,
        })
        .expect("deleting a task that owns a worktree tears it down");
    assert_eq!(cleanup.0.as_deref(), Some("/wt/1-test"));
    assert_eq!(cleanup.1, CleanupFollowUp::DeleteRow);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Delete(_)))),
        "the delete is the teardown's follow-up, never a sibling, got {cmds:?}"
    );
}

#[test]
fn declining_the_delete_leaves_the_done_task_in_place() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    app.selection_mut().set_column(4);

    press(&mut app, 'x');
    let cmds = without_usage(press(&mut app, 'n'));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty(), "declining emits nothing, got {cmds:?}");
    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Done);
}

/// The target is the task under the cursor when `x` was pressed. A refresh
/// that drifts the cursor before `y` must not redirect a permanent delete.
#[test]
fn delete_targets_the_task_at_x_press_not_at_y_press() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Done),
        make_task(2, TaskStatus::Done),
        make_task(3, TaskStatus::Done),
    ]);
    app.selection_mut().set_column(4);
    app.update(Message::NavigateRow(1));
    let target = app.selected_task().unwrap().id;

    press(&mut app, 'x');

    // The target vanishes in a background refresh (deleted elsewhere); the
    // cursor clamps onto another Done task.
    let refreshed: Vec<Task> = [1, 2, 3]
        .into_iter()
        .filter(|id| TaskId(*id) != target)
        .map(|id| make_task(id, TaskStatus::Done))
        .collect();
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        refreshed,
    )));
    let drifted_onto = app.selected_task().unwrap().id;
    assert_ne!(drifted_onto, target, "fixture: the cursor drifted");

    let cmds = press(&mut app, 'y');

    assert!(
        app.find_task(drifted_onto).is_some(),
        "the task the cursor drifted onto must NOT be deleted"
    );
    assert!(
        !deleted_task_ids(&cmds).contains(&drifted_onto),
        "no delete may be issued for the drifted-onto task, got {cmds:?}"
    );
}

// --- Multi-selection of tasks -----------------------------------------------

#[test]
fn x_on_an_all_done_task_selection_deletes_them_all_after_confirmation() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Done),
        make_task(2, TaskStatus::Done),
        make_task(3, TaskStatus::Backlog),
    ]);
    for id in [1, 2] {
        app.update(Message::Task(
            crate::tui::messages::TaskMessage::ToggleSelect(TaskId(id)),
        ));
    }

    press(&mut app, 'x');
    assert_ne!(
        app.input.mode,
        InputMode::Normal,
        "a count-based y/n prompt"
    );
    assert_ne!(app.input.mode, InputMode::ConfirmDone);
    assert!(status_message(&app).contains("Delete"));

    let cmds = press(&mut app, 'y');

    assert!(app.find_task(TaskId(1)).is_none());
    assert!(app.find_task(TaskId(2)).is_none());
    assert!(
        app.find_task(TaskId(3)).is_some(),
        "unselected task untouched"
    );
    let mut deleted = deleted_task_ids(&cmds);
    deleted.sort_by_key(|t| t.0);
    assert_eq!(deleted, vec![TaskId(1), TaskId(2)]);
    assert!(
        app.select.tasks.is_empty(),
        "selection cleared after the batch"
    );
}

/// DeleteKeyRouting: a tasks-only selection with a non-Done task routes to
/// ConfirmDone over the non-Done ones and deletes nothing — not even the
/// already-Done tasks in it.
#[test]
fn x_on_a_mixed_task_selection_deletes_nothing() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Done),
    ]);
    for id in [1, 2] {
        app.update(Message::Task(
            crate::tui::messages::TaskMessage::ToggleSelect(TaskId(id)),
        ));
    }

    press(&mut app, 'x');
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    let cmds = press(&mut app, 'y');

    assert!(deleted_task_ids(&cmds).is_empty(), "got {cmds:?}");
    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Done);
    assert_eq!(app.find_task(TaskId(2)).unwrap().status, TaskStatus::Done);
}

// --- Single epic --------------------------------------------------------------

/// Epic 10 holds a Done task directly and a sub-epic 20 holding another Done
/// task. `extra` is added under the sub-epic, for the refusal cases.
fn app_with_nested_epic(extra: Option<Task>) -> App {
    let mut tasks = vec![
        task_in(1, TaskStatus::Done, 10),
        task_in(2, TaskStatus::Done, 20),
    ];
    tasks.extend(extra);
    let mut app = App::new(tasks);
    let mut parent = make_epic(10);
    parent.status = TaskStatus::Done;
    let mut child = make_epic(20);
    child.parent_epic_id = Some(EpicId(10));
    child.status = TaskStatus::Done;
    app.board.epics = vec![parent, child];
    app
}

#[test]
fn x_on_an_epic_whose_whole_subtree_is_done_asks_to_delete_it() {
    let mut app = app_with_nested_epic(None);
    app.selection_mut().set_column(4);
    app.selection_mut().set_row(4, 0);
    assert_selected_epic(&app, 10);

    let cmds = without_usage(press(&mut app, 'x'));

    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteEpic);
    assert!(
        status_message(&app).contains("Delete epic"),
        "got {:?}",
        status_message(&app)
    );
}

#[test]
fn confirming_an_epic_delete_removes_its_whole_subtree() {
    let mut app = app_with_nested_epic(None);
    app.selection_mut().set_column(4);
    app.selection_mut().set_row(4, 0);
    assert_selected_epic(&app, 10);

    press(&mut app, 'x');
    let cmds = press(&mut app, 'y');

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.epics.is_empty(), "epic and sub-epic are gone");
    assert!(app.board.tasks.is_empty(), "and every task in the subtree");
    assert_eq!(deleted_epic_ids(&cmds), vec![EpicId(10)]);
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Epic(EpicCommand::Persist { .. }) | Command::Task(TaskCommand::Persist(_))
        )),
        "a delete persists no status, got {cmds:?}"
    );
}

/// ConfirmDeleteEpic's guard is the WHOLE subtree, at any depth: a running
/// task two levels down refuses the delete even though the epic's direct
/// task is done.
#[test]
fn x_on_an_epic_with_an_unfinished_task_deep_in_its_subtree_refuses() {
    let mut app = app_with_nested_epic(Some(task_in(3, TaskStatus::Running, 20)));
    app.selection_mut().set_column(4);
    app.selection_mut().set_row(4, 0);
    assert_selected_epic(&app, 10);

    let cmds = without_usage(press(&mut app, 'x'));

    assert!(cmds.is_empty(), "got {cmds:?}");
    assert_eq!(app.input.mode, InputMode::Normal, "no prompt is offered");
    assert!(
        status_message(&app).to_lowercase().contains("delete"),
        "the refusal is explained, got {:?}",
        status_message(&app)
    );
    assert_eq!(app.board.epics.len(), 2);
    assert_eq!(app.board.tasks.len(), 3);
}

#[test]
fn x_on_an_epic_with_no_tasks_at_all_asks_to_delete_it() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1); // an empty epic sits in Backlog
    app.selection_mut().set_row(1, 0);
    assert_selected_epic(&app, 10);

    press(&mut app, 'x');
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteEpic);
    let cmds = press(&mut app, 'y');
    assert!(app.board.epics.is_empty());
    assert_eq!(deleted_epic_ids(&cmds), vec![EpicId(10)]);
}

// --- Batch with epics (BatchDelete) -------------------------------------------

fn select(app: &mut App, tasks: &[i64], epics: &[i64]) {
    for id in tasks {
        app.update(Message::Task(
            crate::tui::messages::TaskMessage::ToggleSelect(TaskId(*id)),
        ));
    }
    for id in epics {
        app.update(Message::Epic(
            crate::tui::messages::EpicMessage::ToggleSelect(EpicId(*id)),
        ));
    }
}

/// Press x, and y if a prompt was offered — BatchDelete may refuse at either
/// point, and the spec fixes only the outcome.
fn press_x_then_confirm(app: &mut App) -> Vec<Command> {
    let mut cmds = press(app, 'x');
    if app.input.mode != InputMode::Normal {
        cmds.extend(press(app, 'y'));
    }
    cmds
}

#[test]
fn batch_delete_of_done_tasks_and_a_fully_done_epic_deletes_everything() {
    let mut tasks = vec![make_task(5, TaskStatus::Done)];
    tasks.push(task_in(1, TaskStatus::Done, 10));
    let mut app = App::new(tasks);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    select(&mut app, &[5], &[10]);

    let cmds = press_x_then_confirm(&mut app);

    assert!(app.board.epics.is_empty());
    assert!(app.board.tasks.is_empty());
    assert!(deleted_task_ids(&cmds).contains(&TaskId(5)));
    assert_eq!(deleted_epic_ids(&cmds), vec![EpicId(10)]);
    assert!(app.select.tasks.is_empty() && app.select.epics.is_empty());
}

/// "No partial batches": one non-Done task refuses the whole batch, epic
/// included, and the status bar says why.
#[test]
fn batch_delete_with_a_non_done_task_deletes_nothing() {
    let mut tasks = vec![make_task(5, TaskStatus::Backlog)];
    tasks.push(task_in(1, TaskStatus::Done, 10));
    let mut app = App::new(tasks);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    select(&mut app, &[5], &[10]);

    let cmds = press_x_then_confirm(&mut app);

    assert!(deleted_task_ids(&cmds).is_empty(), "got {cmds:?}");
    assert!(deleted_epic_ids(&cmds).is_empty(), "got {cmds:?}");
    assert_eq!(app.board.epics.len(), 1);
    assert_eq!(app.board.tasks.len(), 2);
    assert_eq!(
        app.find_task(TaskId(5)).unwrap().status,
        TaskStatus::Backlog
    );
    assert!(
        !status_message(&app).is_empty(),
        "the refusal names what failed"
    );
}

/// The same all-or-nothing refusal when the failing item is an epic whose
/// subtree still holds unfinished work, at any depth.
#[test]
fn batch_delete_with_an_epic_holding_unfinished_work_deletes_nothing() {
    let mut app = app_with_nested_epic(Some(task_in(3, TaskStatus::Review, 20)));
    app.board.tasks.push(make_task(5, TaskStatus::Done));
    select(&mut app, &[5], &[10]);

    let cmds = press_x_then_confirm(&mut app);

    assert!(deleted_task_ids(&cmds).is_empty(), "got {cmds:?}");
    assert!(deleted_epic_ids(&cmds).is_empty(), "got {cmds:?}");
    assert!(
        app.find_task(TaskId(5)).is_some(),
        "the Done task is spared too"
    );
    assert_eq!(app.board.epics.len(), 2);
    assert!(!status_message(&app).is_empty());
}

// --- Board layout: no Archive edge column -------------------------------------

/// board-layout.allium: "At nav index 4 (Done), l/Right is a no-op (already at
/// rightmost edge)." There is no fifth column to enter.
#[test]
fn l_at_the_done_column_is_a_no_op() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    app.selection_mut().set_column(4);

    press(&mut app, 'l');
    assert_eq!(app.selected_column(), 4);
    app.handle_key(make_key(KeyCode::Right));
    assert_eq!(app.selected_column(), 4);
}

/// h/l moves between nav indices 1–4 only: walking right from Backlog stops
/// at Done however many times it is pressed.
#[test]
fn walking_right_from_backlog_stops_at_done() {
    let mut app = make_app();
    for _ in 0..6 {
        press(&mut app, 'l');
    }
    assert_eq!(app.selected_column(), 4, "nav indices are 1–4 only");
}
