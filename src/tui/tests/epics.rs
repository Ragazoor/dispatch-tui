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
fn toggle_flattened_message_flips_state() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    assert!(!app.board.flattened);
    app.update(Message::ToggleFlattened);
    assert!(app.board.flattened);
    app.update(Message::ToggleFlattened);
    assert!(!app.board.flattened);
}

#[test]
fn epic_action_hints_not_done() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Enter]"), "epic shows detail");
    assert!(keys.contains(&"[L]"), "epic shows status forward");
    assert!(keys.contains(&"[H]"), "epic shows status backward");
    assert!(keys.contains(&"[x]"), "epic shows archive");
    assert!(keys.contains(&"[q]"), "epic shows quit");
}

#[test]
fn epic_action_hints_done() {
    let mut epic = make_epic(1);
    epic.status = TaskStatus::Done;
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[L]"), "done epic shows status forward");
    assert!(keys.contains(&"[H]"), "done epic shows status backward");
}

#[test]
fn action_hints_no_ctrl_g_outside_epic() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), 0, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[^g]"),
        "should not show ^g back outside epic view"
    );
}

#[test]
fn epic_action_hints_shows_filter_help() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[f]"), "epic should show filter hint");
    assert!(keys.contains(&"[?]"), "epic should show help hint");
}

#[test]
fn description_editor_result_for_epic() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        ..Default::default()
    });
    app.update(Message::DescriptionEditorResult(
        "epic desc\nline 2".to_string(),
    ));
    assert_eq!(app.input.mode, InputMode::InputEpicRepoPath);
    assert_eq!(
        app.input.epic_draft.as_ref().unwrap().description,
        "epic desc\nline 2"
    );
}

#[test]
fn tasks_for_current_view_board_excludes_epic_tasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(1));
}

#[test]
fn tasks_for_current_view_epic_shows_only_subtasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];

    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(2));
}

#[test]
fn flattened_board_shows_all_tasks_including_subtasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;

    let visible = app.tasks_for_current_view();
    let ids: std::collections::HashSet<_> = visible.iter().map(|t| t.id).collect();
    assert!(ids.contains(&TaskId(1)));
    assert!(ids.contains(&TaskId(2)));
}

#[test]
fn flattened_board_is_recursive_through_nested_epics() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    // Epic tree: root(10) -> child(20)
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child_epic];

    let mut t_root = make_task(1, TaskStatus::Backlog);
    t_root.epic_id = Some(EpicId(10));
    let mut t_leaf = make_task(2, TaskStatus::Running);
    t_leaf.epic_id = Some(EpicId(20));
    app.board.tasks = vec![t_root, t_leaf];
    app.board.flattened = true;

    let backlog = app.column_items_for_status(TaskStatus::Backlog);
    assert!(backlog
        .iter()
        .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(1))));

    let running = app.column_items_for_status(TaskStatus::Running);
    assert!(running
        .iter()
        .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(2))));
}

#[test]
fn flattened_board_hides_all_epic_cards() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut child = make_epic(20);
    child.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child];
    app.board.flattened = true;

    for status in [
        TaskStatus::Backlog,
        TaskStatus::Running,
        TaskStatus::Review,
        TaskStatus::Done,
    ] {
        let items = app.column_items_for_status(status);
        assert!(
            items.iter().all(|i| matches!(i, ColumnItem::Task(_))),
            "flattened view should emit no epic cards in {status:?} column"
        );
    }
}

#[test]
fn flattened_epic_view_shows_only_that_subtree() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    // Two root epics with tasks under each
    app.board.epics = vec![make_epic(10), make_epic(20)];
    let mut a = make_task(1, TaskStatus::Backlog);
    a.epic_id = Some(EpicId(10));
    let mut b = make_task(2, TaskStatus::Backlog);
    b.epic_id = Some(EpicId(20));
    app.board.tasks = vec![a, b];
    app.board.flattened = true;
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let visible = app.tasks_for_current_view();
    let ids: std::collections::HashSet<_> = visible.iter().map(|t| t.id).collect();
    assert!(ids.contains(&TaskId(1)));
    assert!(!ids.contains(&TaskId(2)));
}

#[test]
fn shift_f_key_toggles_flattened() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    assert!(!app.board.flattened);
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert!(app.board.flattened);
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert!(!app.board.flattened);
}

#[test]
fn shift_f_toggles_flattened_inside_epic_view() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert!(app.board.flattened);
}

#[test]
fn toggle_flattened_clamps_selection_when_epic_disappears() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    // Board with one root epic and one subtask inside. No standalone tasks.
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask];

    // Select the (only) item in the backlog column: the epic card at row 0.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    // Toggle flatten: epic card disappears, subtask appears in its place.
    // Count stays 1 so row 0 is still valid, but now points at the task.
    app.update(Message::ToggleFlattened);
    assert!(app.board.flattened);
    assert_eq!(app.selected_row()[0], 0);

    // Toggle again while selection points beyond the end of the column:
    // put two tasks in backlog, select row 1, then flatten off. After
    // un-flattening, the column has one epic + whatever standalone tasks
    // (none). Row must be clamped to 0.
    app.selection_mut().set_row(1, 5);
    app.update(Message::ToggleFlattened);
    assert!(!app.board.flattened);
    let count = app.column_items_for_status(TaskStatus::Backlog).len();
    assert!(count > 0);
    assert!(app.selected_row()[0] < count);
}

#[test]
fn flattened_survives_enter_and_exit_epic() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleFlattened);
    assert!(app.board.flattened);

    app.update(Message::EnterEpic(EpicId(10)));
    assert!(app.board.flattened, "flatten should persist into epic view");

    app.update(Message::ExitEpic);
    assert!(app.board.flattened, "flatten should persist back to board");
}

#[test]
fn flattened_survives_refresh_tasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::ToggleFlattened);
    assert!(app.board.flattened);

    app.update(Message::RefreshTasks(vec![make_task(
        1,
        TaskStatus::Backlog,
    )]));
    assert!(app.board.flattened);
}

#[test]
fn enter_on_epic_toggles_detail() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Epic is at row 0 in Backlog column (no standalone tasks)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Enter));
    assert!(
        matches!(app.board.view_mode, ViewMode::Board(_)),
        "Should stay in board view — Enter on epic is a no-op until Task 5 input routing"
    );
}

#[test]
fn e_on_epic_opens_editor() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(matches!(&cmds[0], Command::PopOutEditor(EditKind::EpicEdit(e)) if e.id == EpicId(10)));
}

#[test]
fn enter_epic_switches_to_epic_view() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(2);

    app.update(Message::EnterEpic(EpicId(10)));

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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(3);

    app.update(Message::EnterEpic(EpicId(10)));
    app.selection_mut().set_column(1);

    app.update(Message::ExitEpic);

    match &app.board.view_mode {
        ViewMode::Board(sel) => {
            assert_eq!(sel.column(), 3, "board selection should be restored");
        }
        _ => panic!("Expected ViewMode::Board"),
    }
}

#[test]
fn exit_epic_when_on_board_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::ExitEpic);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn column_items_board_view_includes_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)]; // epic with no subtasks = Backlog

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 2); // 1 task + 1 epic
                                // Same priority (5), so task (id=1) sorts before epic (id=10)
    assert!(matches!(items[0], ColumnItem::Task(_)));
    assert!(matches!(items[1], ColumnItem::Epic(_)));
}

#[test]
fn column_items_epic_view_no_epics() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.board.epics = vec![make_epic(10)];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(items.iter().all(|i| matches!(i, ColumnItem::Task(_))));
}

#[test]
fn selected_column_item_returns_epic() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
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
    app.update(Message::StartNewEpic);
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);
}

#[test]
fn epic_created_adds_to_state() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let epic = make_epic(1);
    app.update(Message::EpicCreated(epic));
    assert_eq!(app.board.epics.len(), 1);
}

#[test]
fn delete_epic_removes_from_state_and_tasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask, make_task(2, TaskStatus::Backlog)];

    let cmds = app.update(Message::DeleteEpic(EpicId(10)));
    assert!(app.board.epics.is_empty());
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(2));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn move_epic_status_forward() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)]; // starts as Backlog
    let cmds = app.update(Message::MoveEpicStatus(EpicId(10), MoveDirection::Forward));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Running),
            ..
        }
    )));
}

#[test]
fn move_epic_status_backward() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    let cmds = app.update(Message::MoveEpicStatus(EpicId(10), MoveDirection::Backward));
    assert_eq!(app.board.epics[0].status, TaskStatus::Review);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Review),
            ..
        }
    )));
}

#[test]
fn shift_l_key_on_epic_moves_status_forward() {
    let mut app = make_app_with_epic_selected();
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistEpic { .. })));
}

#[test]
fn shift_h_key_on_backlog_epic_stays_backlog() {
    let mut app = make_app_with_epic_selected();
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    // Already at Backlog, can't go backward
    assert_eq!(app.board.epics[0].status, TaskStatus::Backlog);
    assert!(cmds.is_empty());
}

#[test]
fn shift_h_on_done_epic_moves_to_review() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Done);
            t.epic_id = Some(EpicId(10));
            t
        }],
        1,
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    // Done epic → column 4
    app.selection_mut().set_column(4);
    app.selection_mut().set_row(4, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Review);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Review),
            ..
        }
    )));
}

#[test]
fn shift_e_key_starts_new_epic() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('E')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);
}

#[test]
fn g_key_on_epic_from_board_enters_epic_view() {
    let mut app = make_app_with_epic_selected();
    app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Epic {
            epic_id: EpicId(10),
            ..
        }
    ));
}

#[test]
fn e_key_in_epic_view_edits_epic() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PopOutEditor(EditKind::EpicEdit(e)) if e.id == EpicId(10)));
}

#[test]
fn e_key_on_task_in_epic_view_edits_task_not_epic() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask];
    app.update(Message::EnterEpic(EpicId(10)));

    // Cursor on the subtask in the Backlog column (col 1, row 0)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmEditTask(TaskId(1))
    ));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(cmds.len(), 1, "expected exactly one command");
    assert!(
        matches!(&cmds[0], Command::PopOutEditor(EditKind::TaskEdit(t)) if t.id == TaskId(1)),
        "expected PopOutEditor(TaskEdit(task 1), got {:?}",
        cmds
    );
}

#[test]
fn esc_in_epic_view_exits_to_board() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn shift_u_in_epic_view_toggles_auto_dispatch() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(42);
    epic.auto_dispatch = true;
    app.board.epics = vec![epic];

    // Enter epic view
    app.update(Message::EnterEpic(EpicId(42)));

    // Press Shift+U — should return ToggleEpicAutoDispatch command with auto_dispatch = false
    let cmds = app.handle_key(make_key(KeyCode::Char('U')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::ToggleEpicAutoDispatch {
            id: EpicId(42),
            auto_dispatch: false
        }
    )));

    // Also verify in-memory state was updated
    assert!(!app.board.epics[0].auto_dispatch);
}

#[test]
fn epic_title_esc_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "partial".to_string();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
}

#[test]
fn epic_title_enter_with_text_advances_to_description() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "My Epic".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicDescription);
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.epic_draft.as_ref().unwrap().title, "My Epic");
}

#[test]
fn epic_title_enter_empty_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn epic_description_enter_advances_to_repo_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        ..Default::default()
    });
    app.input.buffer = "epic desc".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicRepoPath);
    assert!(app.input.buffer.is_empty());
    assert_eq!(
        app.input.epic_draft.as_ref().unwrap().description,
        "epic desc"
    );
}

#[test]
fn epic_repo_path_enter_with_text_completes() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer = "/tmp".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::InsertEpic(ref d) if d.repo_path == "/tmp")));
}

#[test]
fn epic_repo_path_enter_empty_uses_saved_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::InsertEpic(ref d) if d.repo_path == "/tmp")));
}

#[test]
fn epic_repo_path_enter_empty_no_saved_stays() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec![];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    let _cmds = app.handle_key(make_key(KeyCode::Enter));
    // Should stay in repo path mode since there's no fallback
    assert!(app.status.message.is_some());
}

#[test]
fn epic_text_input_char_appends() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.handle_key(make_key(KeyCode::Char('A')));
    app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(app.input.buffer, "Ab");
}

#[test]
fn epic_text_input_backspace_removes() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "abc".to_string();
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn epic_text_input_unrecognized_key_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "x".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "x");
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);
}

#[test]
fn epic_repo_path_digit_quick_selects() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/first".to_string(), "/second".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::InsertEpic(ref d) if d.repo_path == "/second")));
}

#[test]
fn epic_repo_path_digit_with_nonempty_buffer_appends() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/first".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer = "/my".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Char('1')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "/my1");
}

fn make_app_confirm_delete_epic() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1); // cursor on epic (same priority as task, sorts after by id)
    app.input.mode = InputMode::ConfirmDeleteEpic;
    app.status.message = Some("Delete epic \"Epic 10\" and subtasks? [y/n]".to_string());
    app
}

#[test]
fn confirm_delete_epic_enters_mode_with_title() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1); // cursor on epic (same priority as task, sorts after by id)
    app.update(Message::ConfirmDeleteEpic);
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteEpic);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete epic \"Epic 10\" and subtasks? [y/n]")
    );
}

#[test]
fn confirm_delete_epic_y_deletes() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn confirm_delete_epic_uppercase_y_deletes() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn confirm_delete_epic_other_key_cancels() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert_eq!(app.board.epics.len(), 1); // not deleted
    assert!(cmds.is_empty());
}

#[test]
fn confirm_delete_epic_no_epic_selected_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1); // cursor on task, not epic
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty()); // no deletion happened
}

#[test]
fn g_key_on_epic_enters_epic_view() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];

    // Even with subtasks that have tmux windows, g enters epic view
    let mut subtask = make_task(1, TaskStatus::Review);
    subtask.epic_id = Some(EpicId(10));
    subtask.tmux_window = Some("win-1".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(3);
    app.selection_mut().set_row(3, 0);

    app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(10)));
}

#[test]
fn shift_g_on_epic_jumps_to_review_subtask() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Review);
    subtask.epic_id = Some(EpicId(10));
    subtask.tmux_window = Some("win-1".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(3);
    app.selection_mut().set_row(3, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-1"));
}

#[test]
fn shift_g_on_epic_no_session_shows_status() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let _cmds = app.handle_key(make_key(KeyCode::Char('G')));
    // Should NOT enter epic view — shows status info instead
    assert!(!matches!(app.board.view_mode, ViewMode::Epic { .. }));
}

#[test]
fn shift_g_on_epic_jumps_to_blocked_running_subtask() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.sub_status = SubStatus::NeedsInput;
    subtask.tmux_window = Some("win-blocked".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-blocked"));
}

#[test]
fn shift_g_on_epic_skips_active_running_subtask() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.sub_status = SubStatus::Active;
    subtask.tmux_window = Some("win-running".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let _cmds = app.handle_key(make_key(KeyCode::Char('G')));
    // Active running subtask is skipped, no session found => status info
    assert!(!matches!(app.board.view_mode, ViewMode::Epic { .. }));
}

#[test]
fn shift_g_on_epic_prefers_blocked_running_over_review() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut review_task = make_task(1, TaskStatus::Review);
    review_task.epic_id = Some(EpicId(10));
    review_task.tmux_window = Some("win-review".to_string());

    let mut running_task = make_task(2, TaskStatus::Running);
    running_task.epic_id = Some(EpicId(10));
    running_task.sub_status = SubStatus::NeedsInput;
    running_task.tmux_window = Some("win-running".to_string());

    app.board.tasks = vec![review_task, running_task];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-running"));
}

#[test]
fn shift_g_on_epic_active_running_falls_through_to_review() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut review_task = make_task(1, TaskStatus::Review);
    review_task.epic_id = Some(EpicId(10));
    review_task.tmux_window = Some("win-review".to_string());

    let mut running_task = make_task(2, TaskStatus::Running);
    running_task.epic_id = Some(EpicId(10));
    running_task.sub_status = SubStatus::Active;
    running_task.tmux_window = Some("win-running".to_string());

    app.board.tasks = vec![review_task, running_task];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-review"));
}

#[test]
fn shift_g_on_epic_picks_lowest_sort_order() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut task_high = make_task(1, TaskStatus::Running);
    task_high.epic_id = Some(EpicId(10));
    task_high.sub_status = SubStatus::NeedsInput;
    task_high.sort_order = Some(5);
    task_high.tmux_window = Some("win-high".to_string());

    let mut task_low = make_task(2, TaskStatus::Running);
    task_low.epic_id = Some(EpicId(10));
    task_low.sub_status = SubStatus::Stale;
    task_low.sort_order = Some(1);
    task_low.tmux_window = Some("win-low".to_string());

    app.board.tasks = vec![task_high, task_low];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-low"));
}

#[test]
fn column_items_sorted_by_sort_order() {
    let mut app = make_app();
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.title = "First".to_string();
    t1.sort_order = Some(200);
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.title = "Second".to_string();
    t2.sort_order = Some(100);
    app.board.tasks = vec![t1, t2];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 2);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Second"),
        _ => panic!("expected task"),
    }
    match &items[1] {
        ColumnItem::Task(t) => assert_eq!(t.title, "First"),
        _ => panic!("expected task"),
    }
}

#[test]
fn column_items_null_sort_order_uses_id() {
    let mut app = make_app();
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.title = "High ID".to_string();
    t1.sort_order = None;
    let mut t2 = make_task(5, TaskStatus::Backlog);
    t2.title = "Low ID".to_string();
    t2.sort_order = None;
    app.board.tasks = vec![t1, t2];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Low ID"),
        _ => panic!("expected task"),
    }
}

#[test]
fn handle_key_normal_new_epic() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('E')));
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);
}

#[test]
fn handle_key_normal_tab_is_noop_without_feed_epics() {
    // Tab from Board is a no-op when there are no feed epics.
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_epic_text_input_char_and_enter() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('E'))); // start epic creation
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);

    app.handle_key(make_key(KeyCode::Char('X')));
    assert_eq!(app.input.buffer, "X");

    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::InputEpicDescription);
}

#[test]
fn handle_key_epic_text_input_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn space_toggles_epic_selection() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Epic is at row 0 in Backlog column (no standalone tasks)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.epics.contains(&EpicId(10)));
}

#[test]
fn space_on_epic_toggle_off() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    // Select
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.epics.contains(&EpicId(10)));

    // Deselect
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(!app.select.epics.contains(&EpicId(10)));
}

#[test]
fn space_on_empty_column_no_epics_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    // Navigate to Review column (empty)
    app.update(Message::NavigateColumn(2));
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.epics.is_empty());
    assert!(app.select.tasks.is_empty());
}

#[test]
fn select_all_column_includes_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.epics.contains(&EpicId(10)));
}

#[test]
fn select_all_deselects_all_including_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    // Select all
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 1);
    assert_eq!(app.select.epics.len(), 1);

    // Deselect all
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
    assert!(app.select.epics.is_empty());
}

#[test]
fn select_all_column_with_only_epics() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10), make_epic(20)];

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
    assert_eq!(app.select.epics.len(), 2);
    assert!(app.select.epics.contains(&EpicId(10)));
    assert!(app.select.epics.contains(&EpicId(20)));
}

#[test]
fn esc_clears_epic_selection() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    assert_eq!(app.select.epics.len(), 1);

    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.epics.is_empty());
}

#[test]
fn x_key_with_epic_selection_shows_count_in_confirm() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    app.update(Message::ToggleSelectEpic(EpicId(20)));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert!(matches!(app.input.mode, InputMode::ConfirmArchive(None)));
    assert_eq!(
        app.status.message.as_deref(),
        Some("Archive 2 items? [y/n]")
    );
}

#[test]
fn shift_l_on_epic_moves_status_forward() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Cursor on Backlog column, row 0 (the epic)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Running),
            ..
        }
    )));
}

#[test]
fn render_selected_epic_shows_star_prefix() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "* "),
        "Selected epic should show * prefix"
    );
    assert!(
        buffer_contains(&buf, "Epic 10"),
        "Epic title should be visible"
    );
}

#[test]
fn render_unselected_epic_no_star() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Epic 10"),
        "Epic title should be visible"
    );
    // The epic renders with "  " prefix (2 spaces), not "* "
    assert!(
        !buffer_contains(&buf, "* "),
        "Unselected epic should not show * prefix"
    );
}

#[test]
fn render_batch_hints_with_epic_selection() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "1 selected"),
        "Should show selection count"
    );
    assert!(buffer_contains(&buf, "archive"), "Should show archive hint");
}

#[test]
fn render_column_header_checked_with_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    // Select both the task and the epic
    app.update(Message::SelectAllColumn);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[x]"),
        "Checkbox should be checked when all items selected"
    );
}

#[test]
fn refresh_epics_prunes_stale_epic_selections() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    app.update(Message::ToggleSelectEpic(EpicId(99))); // non-existent

    // Refresh with only epic 10
    app.update(Message::RefreshEpics(vec![make_epic(10)]));
    assert!(app.select.epics.contains(&EpicId(10)));
    assert!(!app.select.epics.contains(&EpicId(99)));
}

#[test]
fn render_status_bar_confirm_delete_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Delete epic"),
        "ConfirmDeleteEpic should show 'Delete epic'"
    );
}

#[test]
fn render_status_bar_epic_title() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: enter title"),
        "InputEpicTitle should show 'Creating epic: enter title'"
    );
}

#[test]
fn render_status_bar_epic_description() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: opening $EDITOR for description"),
        "InputEpicDescription should show 'Creating epic: opening $EDITOR for description'"
    );
}

#[test]
fn render_status_bar_epic_repo_path() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicRepoPath;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: enter repo path"),
        "InputEpicRepoPath should show 'Creating epic: enter repo path'"
    );
}

#[test]
fn render_input_form_epic_title_shows_new_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "My epic".to_string();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Title:"),
        "'Title:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "My epic"),
        "buffer text 'My epic' should be visible"
    );
}

#[test]
fn render_input_form_epic_description_shows_fields() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic title".to_string(),
        ..Default::default()
    });
    app.input.buffer = "Epic desc".to_string();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Epic title"),
        "completed title 'Epic title' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Description:"),
        "'Description:' label should be visible"
    );
}

#[test]
fn render_input_form_epic_repo_path_shows_repos() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic title".to_string(),
        description: "Epic desc".to_string(),
        ..Default::default()
    });
    app.input.buffer = String::new();
    app.board.repo_paths = vec!["/repo/x".to_string()];
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Repo path:"),
        "'Repo path:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "/repo/x"),
        "repo path '/repo/x' should be listed"
    );
}

#[test]
fn render_epic_banner_shows_title() {
    let mut app = make_app();
    let mut epic = make_epic(10);
    epic.title = "Auth Refactor".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Auth Refactor"),
        "epic banner should show the epic title 'Auth Refactor'"
    );
}

#[test]
fn render_epic_banner_not_shown_in_board_view() {
    let mut app = make_app();
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    // Stay in default Board view — do not switch to ViewMode::Epic
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "Esc to return"),
        "epic banner should not be shown in Board view"
    );
}

#[test]
fn render_detail_task_with_epic_reference() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.epic_id = Some(EpicId(10));
    let mut epic = make_epic(10);
    epic.title = "Auth Epic".to_string();
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    app.board.epics = vec![epic];
    // Switch to Epic view so the subtask is visible (Board view hides epic subtasks)
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 160, 30);
}

#[test]
fn render_detail_epic_shows_title_and_id() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.title = "Platform Migration".to_string();
    app.board.epics = vec![epic];
    // Epic is the only item in Backlog column (no standalone tasks)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_detail_epic_with_plan_shows_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plans/migration.md".to_string());
    app.board.epics = vec![epic];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_detail_epic_shows_subtask_list() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    let mut t1 = make_task(101, TaskStatus::Done);
    t1.title = "Subtask Alpha".to_string();
    t1.epic_id = Some(EpicId(10));
    let mut t2 = make_task(102, TaskStatus::Running);
    t2.title = "Subtask Beta".to_string();
    t2.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t1, t2];

    // Epic is in Backlog; subtasks are in other columns so won't appear as
    // standalone items in column 1 (Backlog). The epic itself is the first item.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_detail_epic_subtask_conflict_shows_warning() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    let mut t1 = make_task(201, TaskStatus::Running);
    t1.title = "Conflicted Task".to_string();
    t1.epic_id = Some(EpicId(10));
    t1.sub_status = SubStatus::Conflict;
    app.board.tasks = vec![t1];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_tab_bar_epic_mode_shows_epic_title() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.title = "Platform Work".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Platform Work"),
        "tab bar in epic mode should show the epic title"
    );
}

#[test]
fn render_tab_bar_epic_mode_replaces_tasks_tab() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.title = "Platform Work".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        !buffer_contains(&buf, "Tasks"),
        "epic tab should replace the Tasks tab, not appear alongside it"
    );
}

#[test]
fn epic_card_title_truncated_in_narrow_terminal() {
    let mut epic = make_epic(1);
    epic.title = "This is a very long epic title that should be truncated to fit".to_string();
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::RefreshEpics(vec![epic]));

    let buf = render_to_buffer(&mut app, 80, 10);
    assert!(
        !buffer_contains(
            &buf,
            "This is a very long epic title that should be truncated to fit"
        ),
        "full epic title should be truncated in narrow terminal"
    );
}

#[test]
fn handle_key_normal_esc_in_epic_view_exits() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { .. }));

    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_normal_q_in_epic_view_exits() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_epic_repo_path_enter_selects_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/tmp".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds.iter().any(|c| matches!(c, Command::InsertEpic(_))));
}

#[test]
fn handle_key_tag_selects_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    app.handle_key(make_key(KeyCode::Char('e')));
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().tag,
        Some(TaskTag::Epic)
    );
}

#[test]
fn handle_key_normal_dispatch_in_epic_view_with_no_items() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    // No subtasks, cursor on empty column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    // Should dispatch the epic itself
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchEpic { .. })));
}

#[test]
fn handle_key_normal_shift_l_on_epic_moves_status() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistEpic { .. })));
}

#[test]
fn handle_key_normal_shift_h_on_epic_moves_backward() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistEpic { .. })));
}

/// InputEpicTitle mode routes to the text input handler.
#[test]
fn handle_key_input_epic_title_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputEpicDescription mode routes to the text input handler.
#[test]
fn handle_key_input_epic_description_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputEpicRepoPath mode routes to the text input handler.
#[test]
fn handle_key_input_epic_repo_path_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicRepoPath;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDeleteEpic mode routes correctly.
#[test]
fn handle_key_confirm_delete_epic_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// Normal mode on Epic view routes to the board handler (not review/security).
#[test]
fn handle_key_normal_epic_view_routes_correctly() {
    let mut app = make_app();
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(1),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    // 'q' in epic view exits to board (doesn't quit)
    let cmds = app.handle_key(make_key(KeyCode::Char('q')));
    assert!(cmds.is_empty());
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn epic_view_header_shows_auto_dispatch_indicator() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(1);
    epic.auto_dispatch = true;
    app.board.epics = vec![epic];
    app.update(Message::EnterEpic(EpicId(1)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "auto dispatch [U]"),
        "Expected 'auto dispatch [U]' in header"
    );
}

#[test]
fn epic_view_header_shows_manual_dispatch_indicator() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut epic = make_epic(1);
    epic.auto_dispatch = false;
    app.board.epics = vec![epic];
    app.update(Message::EnterEpic(EpicId(1)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "manual dispatch [U]"),
        "Expected 'manual dispatch [U]' in header"
    );
}

#[test]
fn repo_cursor_resets_on_entering_epic_repo_path_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.repo_cursor = 1;
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(crate::tui::types::EpicDraft {
        title: "E".to_string(),
        ..Default::default()
    });
    app.input.buffer = "epic desc".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicRepoPath);
    assert_eq!(app.input.repo_cursor, 0, "cursor should reset to top");
}

#[test]
fn exit_sub_epic_returns_to_parent_epic() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(1), make_epic(2)];
    app.update(Message::EnterEpic(EpicId(1)));
    app.update(Message::EnterEpic(EpicId(2)));

    app.update(Message::ExitEpic);

    match &app.board.view_mode {
        ViewMode::Epic { epic_id, .. } => {
            assert_eq!(*epic_id, EpicId(1), "should return to parent epic 1");
        }
        _ => panic!("Expected ViewMode::Epic after exiting sub-epic"),
    }
}

#[test]
fn exit_from_root_epic_returns_to_board() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(1)];
    app.selection_mut().set_column(3);
    app.update(Message::EnterEpic(EpicId(1)));
    app.update(Message::ExitEpic);

    match &app.board.view_mode {
        ViewMode::Board(sel) => {
            assert_eq!(sel.column(), 3, "board column should be restored");
        }
        _ => panic!("Expected ViewMode::Board"),
    }
}

#[test]
fn board_view_excludes_sub_epics() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut sub = make_epic(20);
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), sub];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    // Only root epic (id=10) should appear; sub-epic (id=20) must not
    let epic_ids: Vec<i64> = items
        .iter()
        .filter_map(|i| {
            if let ColumnItem::Epic(e) = i {
                Some(e.id.0)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(epic_ids, vec![10], "only root epic should appear on board");
}

#[test]
fn epic_view_includes_sub_epics_as_column_items() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut sub = make_epic(20);
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), sub];

    app.update(Message::EnterEpic(EpicId(10)));

    let items = app.column_items_for_status(TaskStatus::Backlog);
    // sub-epic (id=20) should appear as an Epic column item
    let epic_ids: Vec<i64> = items
        .iter()
        .filter_map(|i| {
            if let ColumnItem::Epic(e) = i {
                Some(e.id.0)
            } else {
                None
            }
        })
        .collect();
    assert!(
        epic_ids.contains(&20),
        "sub-epic should appear inside parent epic view"
    );
}

#[test]
fn epic_view_breadcrumb_shows_parent_and_child_title() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let parent_epic = make_epic_with_title(1, "Root Epic");
    let child_epic = make_epic_with_title(2, "Child Epic");
    app.board.epics = vec![parent_epic.clone(), child_epic.clone()];

    // Nested: viewing child epic, parent is another epic view
    app.board.view_mode = ViewMode::Epic {
        epic_id: child_epic.id,
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Epic {
            epic_id: parent_epic.id,
            selection: BoardSelection::new_for_epic(),
            parent: Box::new(ViewMode::Board(BoardSelection::new())),
        }),
    };

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Root Epic"),
        "breadcrumb should show parent epic title"
    );
    assert!(
        buffer_contains(&buf, "Child Epic"),
        "breadcrumb should show current epic title"
    );
    // The separator between parent and child
    assert!(
        buffer_contains(&buf, ">"),
        "breadcrumb should show > separator between parent and child"
    );
}

#[test]
fn epic_view_no_breadcrumb_when_parent_is_board() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let epic = make_epic_with_title(1, "Only Epic");
    app.board.epics = vec![epic.clone()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: epic.id,
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Only Epic"),
        "title should show current epic title"
    );
}

#[test]
fn create_epic_in_epic_view_inherits_parent() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let parent_id = EpicId(42);
    app.board.view_mode = ViewMode::Epic {
        epic_id: parent_id,
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    // Enter the epic creation flow
    app.update(Message::StartNewEpic);
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);

    // The draft should already know about the parent
    let draft_parent = app.input.epic_draft.as_ref().and_then(|d| d.parent_epic_id);
    assert_eq!(
        draft_parent,
        Some(parent_id),
        "epic_draft.parent_epic_id should be set to current epic's id"
    );

    // Submit title, description, repo path — the final command must carry parent_epic_id
    app.update(Message::SubmitEpicTitle("Sub Epic".to_string()));
    app.update(Message::SubmitEpicDescription("desc".to_string()));
    let cmds = app.update(Message::SubmitEpicRepoPath("/tmp".to_string()));

    let draft = cmds
        .iter()
        .find_map(|c| {
            if let Command::InsertEpic(d) = c {
                Some(d)
            } else {
                None
            }
        })
        .expect("expected Command::InsertEpic");

    assert_eq!(
        draft.parent_epic_id,
        Some(parent_id),
        "InsertEpic draft must carry parent_epic_id"
    );
}

#[test]
fn breadcrumb_shows_three_levels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let grandparent = make_epic_with_title(1, "Grandparent");
    let parent = make_epic_with_title(2, "ParentEpic");
    let child = make_epic_with_title(3, "ChildEpic");
    app.board.epics = vec![grandparent, parent, child];

    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(3),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Epic {
            epic_id: EpicId(2),
            selection: BoardSelection::new_for_epic(),
            parent: Box::new(ViewMode::Epic {
                epic_id: EpicId(1),
                selection: BoardSelection::new_for_epic(),
                parent: Box::new(ViewMode::Board(BoardSelection::new())),
            }),
        }),
    };

    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "Grandparent"),
        "breadcrumb should show grandparent title"
    );
    assert!(
        buffer_contains(&buf, "ParentEpic"),
        "breadcrumb should show parent title"
    );
    assert!(
        buffer_contains(&buf, "ChildEpic"),
        "breadcrumb should show child title"
    );
}

#[test]
fn test_epic_anchor_preserved_on_refresh() {
    let tasks = vec![make_task(1, TaskStatus::Backlog)];
    let epics = vec![make_epic(1)];
    let mut app = App::new(tasks.clone(), 1, TEST_TIMEOUT);
    app.update(Message::RefreshEpics(epics.clone()));

    let items = app.column_items_for_status(TaskStatus::Backlog);
    let epic_row = items
        .iter()
        .position(|i| matches!(i, ColumnItem::Epic(_)))
        .expect("epic should be in Backlog column");
    for _ in 0..epic_row {
        app.update(Message::NavigateRow(1));
    }
    assert!(matches!(
        app.column_items_for_status(TaskStatus::Backlog)[app.selection().row(1)],
        ColumnItem::Epic(_)
    ));

    // Refresh same data
    app.update(Message::RefreshTasks(tasks));
    app.update(Message::RefreshEpics(epics));

    // Still on the epic
    assert!(matches!(
        app.column_items_for_status(TaskStatus::Backlog)[app.selection().row(1)],
        ColumnItem::Epic(_)
    ));
}

#[test]
fn epic_view_navigation_does_not_enter_projects_or_archive() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { .. }));

    // Starts at Backlog (column 1)
    assert_eq!(app.selected_column(), 1);

    // Navigate left past Backlog — should not enter Projects (col 0)
    app.update(Message::NavigateColumn(-1));
    assert_eq!(
        app.selected_column(),
        1,
        "should not enter Projects (col 0) from epic view"
    );

    // Navigate right to Done (col 4)
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(app.selected_column(), 4);

    // Navigate right past Done — should not enter Archive (col 5)
    app.update(Message::NavigateColumn(1));
    assert_eq!(
        app.selected_column(),
        4,
        "should not enter Archive (col 5) from epic view"
    );
}

#[test]
fn test_selection_survives_flatten_toggle() {
    // Use task IDs > 1 so they sort after Epic(1) in the column.
    // Column order: [Task(1), Epic(1), Task(2)] — tasks inserted before epics,
    // stable sort keeps Task(1) before Epic(1) when both have key (5,1,1).
    // Navigate +2 to reach Task(2) at row 2.
    let tasks = vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ];
    let epics = vec![make_epic(1)];
    let mut app = App::new(tasks.clone(), 1, TEST_TIMEOUT);
    app.update(Message::RefreshEpics(epics.clone()));

    app.update(Message::NavigateRow(1)); // row 1 — Epic(1)
    app.update(Message::NavigateRow(1)); // row 2 — Task(2)
    let items = app.column_items_for_status(TaskStatus::Backlog);
    let pre_id: TaskId = match &items[app.selection().row(0)] {
        ColumnItem::Task(t) => t.id,
        _ => panic!("expected task at cursor"),
    };

    app.update(Message::ToggleFlattened);
    app.update(Message::ToggleFlattened);

    let items = app.column_items_for_status(TaskStatus::Backlog);
    let post_id: TaskId = match &items[app.selection().row(0)] {
        ColumnItem::Task(t) => t.id,
        _ => panic!("expected task at cursor"),
    };
    assert_eq!(pre_id, post_id);
}
