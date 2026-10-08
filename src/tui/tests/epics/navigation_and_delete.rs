use super::*;

#[test]
fn enter_on_epic_opens_its_epic_view() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    // Epic is at row 0 in Backlog column (no standalone tasks)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Enter));
    assert!(
        matches!(app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(10)),
        "Enter on an epic card jumps into the deepest epic holding its work"
    );
}

#[test]
fn e_on_epic_opens_editor() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(
        matches!(&cmds[0], Command::Editor(crate::tui::commands::EditorCommand::PopOut(EditKind::EpicEdit(e))) if e.id == EpicId(10))
    );
}

#[test]
fn enter_epic_switches_to_epic_view() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(2);

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));

    match &app.board.view_mode {
        ViewMode::Epic {
            epic_id, parent, ..
        } => {
            assert_eq!(*epic_id, EpicId(10));
            match parent.as_ref() {
                ViewMode::Board(sel) => assert_eq!(sel.column(), 2, "board column should be saved"),
                _ => panic!("Expected parent to be ViewMode::Board"),
            }
        }
        _ => panic!("Expected ViewMode::Epic"),
    }
}

#[test]
fn exit_epic_restores_board_selection() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(3);

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    app.selection_mut().set_column(1);

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));

    match &app.board.view_mode {
        ViewMode::Board(sel) => {
            assert_eq!(sel.column(), 3, "board selection should be restored");
        }
        _ => panic!("Expected ViewMode::Board"),
    }
}

#[test]
fn exit_epic_when_on_board_is_noop() {
    let mut app = App::new(vec![]);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn column_items_board_view_includes_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)]; // epic with no subtasks = Backlog

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 2); // 1 task + 1 epic
                                // Same priority (5), so task (id=1) sorts before epic (id=10)
    assert!(matches!(items[0], ColumnItem::Task(_)));
    assert!(matches!(items[1], ColumnItem::Epic(_)));
}

#[test]
fn column_items_epic_view_no_epics() {
    let mut app = App::new(vec![]);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.board.epics = vec![make_epic(10)];

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(items.iter().all(|i| matches!(i, ColumnItem::Task(_))));
}

#[test]
fn selected_column_item_returns_epic() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];

    // Same priority (5), task (id=1) at row 0, epic (id=10) at row 1
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);

    match app.selected_column_item() {
        Some(ColumnItem::Epic(e)) => assert_eq!(e.id, EpicId(10)),
        other => panic!("Expected Epic, got {:?}", other),
    }
}

#[test]
fn start_new_epic_sets_input_mode() {
    let mut app = make_app();
    app.update(Message::Epic(crate::tui::messages::EpicMessage::StartNew));
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);
}

#[test]
fn epic_created_adds_to_state() {
    let mut app = App::new(vec![]);
    let epic = make_epic(1);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Created(
        epic,
    )));
    assert_eq!(app.board.epics.len(), 1);
}

#[test]
fn delete_epic_removes_from_state_and_tasks() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask, make_task(2, TaskStatus::Backlog)];

    let cmds = app.update(Message::Epic(crate::tui::messages::EpicMessage::Delete(
        EpicId(10),
    )));
    assert!(app.board.epics.is_empty());
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(2));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Epic(crate::tui::commands::EpicCommand::Delete(id)) if *id == EpicId(10))));
}

/// #4096 on the epic-delete path: the subtree's rows all go, so a subtask that
/// owns only a window must still have it reclaimed — nothing will name it again
/// (`TeardownIsOwedWheneverThereIsSomethingToRelease`).
#[test]
fn delete_epic_tears_down_a_subtask_that_owns_only_a_window() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];

    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.worktree = None;
    subtask.tmux_window = Some(test_tmux_window("task-1"));
    app.board.tasks = vec![subtask];

    let cmds = app.update(Message::Epic(crate::tui::messages::EpicMessage::Delete(
        EpicId(10),
    )));

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::Cleanup {
                worktree: None,
                tmux_window: Some(w),
                ..
            }) if w == "task-1"
        )),
        "the subtask's window must be reclaimed with its row, got: {cmds:?}"
    );
}

#[test]
fn delete_epic_cleans_up_worktrees_of_sub_epic_subtasks() {
    // `EpicCrud::delete_epic` deletes the whole subtree (delete_epic_recursive
    // walks parent_epic_id), so a sub-epic's subtask rows vanish too. The
    // Cleanup commands must cover them, or their worktrees leak with no row
    // left to point at them. See DeleteEpic in docs/specs/epics.allium.
    let mut app = App::new(vec![]);
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child_epic];

    let mut direct = make_task(1, TaskStatus::Running);
    direct.epic_id = Some(EpicId(10));
    direct.worktree = Some("/repo/.worktrees/1-direct".to_string());
    let mut nested = make_task(2, TaskStatus::Running);
    nested.epic_id = Some(EpicId(20));
    nested.worktree = Some("/repo/.worktrees/2-nested".to_string());
    app.board.tasks = vec![direct, nested];

    let cmds = app.update(Message::Epic(crate::tui::messages::EpicMessage::Delete(
        EpicId(10),
    )));

    let cleaned: Vec<&str> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::Task(crate::tui::commands::TaskCommand::Cleanup { worktree, .. }) => {
                worktree.as_deref()
            }
            _ => None,
        })
        .collect();
    assert!(
        cleaned.contains(&"/repo/.worktrees/1-direct"),
        "direct subtask worktree must be cleaned up, got: {cleaned:?}"
    );
    assert!(
        cleaned.contains(&"/repo/.worktrees/2-nested"),
        "sub-epic subtask worktree must be cleaned up, got: {cleaned:?}"
    );
    // The epic delete drops every subtask row in one operation, so a successful
    // teardown has nothing left to write back — asking it to clear a column on a
    // deleted row would only produce a spurious failure. This is the documented
    // exemption from WorktreeReleaseIsGated (docs/specs/tasks.allium).
    let follow_ups: Vec<crate::tui::commands::CleanupFollowUp> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::Task(crate::tui::commands::TaskCommand::Cleanup { follow_up, .. }) => {
                Some(*follow_up)
            }
            _ => None,
        })
        .collect();
    assert!(
        follow_ups
            .iter()
            .all(|f| *f == crate::tui::commands::CleanupFollowUp::Nothing),
        "epic-delete teardowns have no follow-up to apply, got: {follow_ups:?}"
    );
}

#[test]
fn delete_epic_removes_the_whole_subtree_from_the_board() {
    // The DB delete drops every descendant epic and its tasks, so the in-memory
    // board must not keep rows pointing at deleted epics.
    let mut app = App::new(vec![]);
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    let mut grandchild_epic = make_epic(30);
    grandchild_epic.parent_epic_id = Some(EpicId(20));
    app.board.epics = vec![make_epic(10), child_epic, grandchild_epic, make_epic(40)];

    let mut nested = make_task(2, TaskStatus::Running);
    nested.epic_id = Some(EpicId(30));
    let mut unrelated = make_task(3, TaskStatus::Running);
    unrelated.epic_id = Some(EpicId(40));
    app.board.tasks = vec![nested, unrelated];

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Delete(
        EpicId(10),
    )));

    let epic_ids: Vec<EpicId> = app.board.epics.iter().map(|e| e.id).collect();
    assert_eq!(
        epic_ids,
        vec![EpicId(40)],
        "the deleted epic's whole subtree must leave the board"
    );
    let task_ids: Vec<TaskId> = app.board.tasks.iter().map(|t| t.id).collect();
    assert_eq!(
        task_ids,
        vec![TaskId(3)],
        "subtasks of descendant epics must leave the board too"
    );
}
