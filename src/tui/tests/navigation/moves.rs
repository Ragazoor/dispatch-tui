use super::*;

#[test]
fn tasks_by_status_filters() {
    let app = make_app();
    let backlog = app.view().tasks_by_status(TaskStatus::Backlog);
    assert_eq!(backlog.len(), 2);
    assert_eq!(backlog[0].id, TaskId(1));
    assert_eq!(backlog[1].id, TaskId(2));

    let running = app.view().tasks_by_status(TaskStatus::Running);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, TaskId(3));

    let review = app.view().tasks_by_status(TaskStatus::Review);
    assert_eq!(review.len(), 0);
}

#[test]
fn move_task_forward() {
    let mut app = make_app();
    // Task 1 is in Backlog; move it forward -> Running
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    }));
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
    assert!(matches!(
        cmds[0],
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    ));
}

#[test]
fn move_task_backward_at_start_is_noop() {
    let mut app = make_app();
    // Task 1 is in Backlog; prev() stays Backlog
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    }));
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
    app.update(Message::System(crate::tui::messages::SystemMessage::Quit));
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn pressing_q_on_board_enters_confirm_quit() {
    let mut app = make_app();
    assert_eq!(app.input.mode, InputMode::Normal);
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn navigate_column_clamps() {
    let mut app = make_app();
    // Backlog column (1) is the leftmost; navigating further left is a no-op.
    app.selection_mut().set_column(1);
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selection().column(), 1); // can't go below Backlog

    // Done (COLUMN_COUNT = 4) is the rightmost; navigating further right is a
    // no-op — there is no fifth, edge column any more (board-layout.allium).
    app.selection_mut().set_column(TaskStatus::COLUMN_COUNT);
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selection().column(), TaskStatus::COLUMN_COUNT); // can't go above Done
}

#[test]
fn navigate_column_moves_through_the_nav_columns() {
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
fn navigate_column_always_lands_on_first_row_and_resets_scroll() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
        make_task(4, TaskStatus::Running),
        make_task(5, TaskStatus::Running),
    ]);
    // Move to the last row in Backlog (nav col 1) and simulate it having
    // scrolled down, then leave and return.
    app.selection_mut().set_column(1);
    app.update(Message::NavigateRow(1));
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 2, "precondition: parked on row 2");
    *app.selection_mut().list_states[0].offset_mut() = 5;

    app.update(Message::NavigateColumn(1)); // -> Running
    app.update(Message::NavigateColumn(-1)); // back -> Backlog

    assert_eq!(app.selection().column(), 1, "precondition: back in Backlog");
    assert_eq!(
        app.selection().row(1),
        0,
        "switching back into a column must default to the first card, not the sticky row"
    );
    assert_eq!(
        app.selection().list_states[0].offset(),
        0,
        "re-entering a column must scroll it back to the top"
    );
    assert!(!app.on_select_all());
}

#[test]
fn navigate_column_noop_at_edge_preserves_row_and_scroll() {
    let mut app = make_app();
    app.selection_mut().set_column(1); // leftmost
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 1);
    *app.selection_mut().list_states[0].offset_mut() = 3;

    // h/Left at the leftmost column is a clamped no-op — must not reset cursor/scroll.
    app.update(Message::NavigateColumn(-1));

    assert_eq!(app.selection().row(1), 1, "no-op must not reset row");
    assert_eq!(
        app.selection().list_states[0].offset(),
        3,
        "no-op must not reset scroll"
    );
}

#[test]
fn navigate_column_into_empty_column_preserves_select_all() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    app.selection_mut().set_column(1);
    app.selection_mut().on_select_all = true;

    // Running (nav col 2) is empty; switching there must not clobber on_select_all
    // for an empty destination (mirrors the `[`/`]` empty-column no-op semantics).
    app.update(Message::NavigateColumn(1));

    assert_eq!(app.selection().column(), 2);
    assert!(
        app.on_select_all(),
        "on_select_all must be left untouched when the destination column is empty"
    );
    assert_eq!(app.selection().row(2), 0);
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
fn navigate_row_clamps_to_task_count_in_flat_mode_with_epic_headers() {
    // In flat mode, column_items_for_status inserts unselectable EpicHeader
    // separators between epic groups, so its length overcounts selectable rows.
    // With 2 Running tasks under epic 10 it returns
    //   [SubstatusLabel, EpicHeader(epic10), Task(t1), Task(t2)]  — len = 4,
    // while column_item_count returns 2 (tasks only, since flattened=true).
    // Navigation must clamp to task_count-1 (row 1), not len()-1 (row 3).
    let mut t1 = make_task(1, TaskStatus::Running);
    t1.epic_id = Some(EpicId(10));
    let mut t2 = make_task(2, TaskStatus::Running);
    t2.epic_id = Some(EpicId(10));

    let mut app = App::new(vec![t1, t2]);
    app.board.epics = vec![make_epic(10)];
    app.filter.lens = crate::tui::types::Lens::Flat;
    app.selection_mut().set_column(2); // Running = nav col 2

    // Navigate far past the end — should clamp to last selectable task (row 1).
    app.update(Message::NavigateRow(10));
    assert_eq!(
        app.selection().row(2),
        1,
        "should clamp to task count - 1, not column_items.len() - 1"
    );

    // Navigate back up from row 0 — should enter the select-all toggle row,
    // same as in non-flat mode (the boundary logic is mode-independent).
    app.update(Message::NavigateRow(-10)); // back to row 0
    assert!(!app.on_select_all());
    app.update(Message::NavigateRow(-1)); // one step up from row 0
    assert!(
        app.on_select_all(),
        "NavigateRow(-1) from row 0 in flat mode should land on select-all toggle"
    );
}

/// Deleting a task that owns a worktree is GATED on the removal: the row is
/// dropped by the cleanup's success path, so the delete must NOT be emitted
/// beside the cleanup (`WorktreeReleaseIsGated` in docs/specs/tasks.allium).
/// Emitting both is what let a failed removal drop the row anyway and strand
/// the directory with nothing referencing it.
#[test]
fn delete_task_with_worktree_gates_the_row_delete_on_cleanup() {
    let mut app = make_app();
    let task = app.find_task_mut(TaskId(4)).unwrap();
    task.worktree = Some("/repo/.worktrees/4-task".to_string());
    task.tmux_window = Some(test_tmux_window("task-4"));

    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Delete(
        TaskId(4),
    )));
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(4)));
    let follow_up = cmds
        .iter()
        .find_map(|c| match c {
            Command::Task(crate::tui::commands::TaskCommand::Cleanup { follow_up, .. }) => {
                Some(*follow_up)
            }
            _ => None,
        })
        .expect("delete of a task with a worktree must emit Cleanup");
    assert_eq!(
        follow_up,
        crate::tui::commands::CleanupFollowUp::DeleteRow,
        "the cleanup owns the row delete"
    );
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::Delete(TaskId(4)))
        )),
        "the row must not be deleted before the worktree removal has succeeded"
    );
}

/// The other half: with nothing to release, the delete is immediate.
#[test]
fn delete_task_without_worktree_deletes_the_row_immediately() {
    let mut app = make_app();
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Delete(
        TaskId(1),
    )));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Delete(TaskId(1)))
    )));
}

#[test]
fn delete_task_without_worktree_no_cleanup() {
    let mut app = make_app();
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Delete(
        TaskId(1),
    )));
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Cleanup { .. })
    )));
}

#[test]
fn error_sets_error_popup() {
    let mut app = App::new(vec![]);
    app.update(Message::System(crate::tui::messages::SystemMessage::Error(
        "Something went wrong".to_string(),
    )));
    assert_eq!(
        app.status.error_popup.as_deref(),
        Some("Something went wrong")
    );
}

#[test]
fn move_backward_from_running_detaches_but_keeps_worktree() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);

    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(4),
        direction: MoveDirection::Backward,
    }));

    // Should emit KillTmuxWindow then PersistTask (no Cleanup)
    assert_eq!(cmds.len(), 2);
    assert!(
        matches!(&cmds[0], Command::Task(crate::tui::commands::TaskCommand::KillTmuxWindow { window }) if window == "task-4")
    );
    assert!(matches!(
        &cmds[1],
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    ));

    // Worktree preserved, tmux_window cleared
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(4)).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/4-task-4"));
    assert!(task.tmux_window.is_none());
}

#[test]
fn move_backward_from_running_without_dispatch_fields() {
    let mut task = make_task(3, TaskStatus::Running);
    task.worktree = None;
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(3),
        direction: MoveDirection::Backward,
    }));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        cmds[0],
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    ));
}

#[test]
fn move_forward_to_done_enters_confirm_mode() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = None; // session closed, but worktree remains
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    }));

    // Should enter confirmation mode, not move immediately
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(5)).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    // Worktree preserved — not taken during confirmation
    assert!(task.worktree.is_some());
}

/// The single-task Done prompt records its task in `select.pending_done`, the
/// same place the batch prompt uses — that is why `InputMode::ConfirmDone`
/// carries no payload.
#[test]
fn single_move_to_done_records_the_task_in_pending_done() {
    let mut app = App::new(vec![make_task(5, TaskStatus::Review)]);

    app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    }));

    assert_eq!(app.select.pending_done, vec![TaskId(5)]);
}

#[test]
fn move_forward_to_done_with_live_window_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(5, TaskStatus::Review)]);

    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    }));

    // Should enter confirmation mode, not move immediately
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    assert_eq!(app.select.pending_done, vec![TaskId(5)]);
}

#[test]
fn space_key_with_live_window_jumps() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.selection_mut().set_column(2); // Running = nav col 2
    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        cmds.iter().any(|c| matches!(c, Command::Task(crate::tui::commands::TaskCommand::JumpToTmux { window }) if window == "task-4"))
    );
}

#[test]
fn space_key_without_window_dispatches_backlog() {
    // Space on a windowless Backlog task now activates it (dispatch), routing
    // through the repo-trust gate — a CheckTrustAndDispatch command — rather
    // than showing "No active session" or deciding trust synchronously.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selection_mut().set_column(1); // Backlog = nav col 1
    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::CheckTrustAndDispatch {
                id: TaskId(1),
                ..
            })
        )),
        "expected CheckTrustAndDispatch command, got {cmds:?}"
    );
}
