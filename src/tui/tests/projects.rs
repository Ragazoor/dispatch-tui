use crossterm::event::{KeyCode, KeyModifiers};

use crate::models::{Project, Task, TaskStatus};
use crate::tui::types::{Command, InputMode};
use crate::tui::{App, Message};

use super::helpers::{
    buffer_contains, make_app, make_app_with_archived_task, make_key, make_task, render_to_buffer,
    TEST_TIMEOUT,
};

fn make_task_with_project(id: i64, status: TaskStatus, project_id: i64) -> Task {
    Task {
        project_id,
        ..make_task(id, status)
    }
}

#[test]
fn project_matches_hides_tasks_from_other_project() {
    let t1 = make_task_with_project(1, TaskStatus::Backlog, 1);
    let t2 = make_task_with_project(2, TaskStatus::Backlog, 2);

    let app = App::new(vec![t1, t2], 1, TEST_TIMEOUT);
    // active_project = 1 → only t1 should appear in Backlog
    let visible = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(
        visible.len(),
        1,
        "Expected 1 item for project 1, got {}",
        visible.len()
    );

    // Switch to project 2 → only t2
    let mut app = app;
    app.update(Message::SelectProject(2));
    let visible = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(
        visible.len(),
        1,
        "Expected 1 item for project 2, got {}",
        visible.len()
    );
}

#[test]
fn archived_tasks_filtered_by_active_project() {
    let t = make_task_with_project(1, TaskStatus::Archived, 2);
    let app = App::new(vec![t], 1, TEST_TIMEOUT);
    // active_project = 1, task is in project 2 → archived_tasks returns empty
    assert_eq!(app.archived_tasks().len(), 0);
}

#[test]
fn select_project_clamps_cursor() {
    // Project 1 has 3 tasks in Backlog; project 2 has 1.
    // Cursor is at row 2 (third item in Backlog). Switching to project 2
    // should clamp cursor to row 0.
    let tasks = vec![
        make_task_with_project(1, TaskStatus::Backlog, 1),
        make_task_with_project(2, TaskStatus::Backlog, 1),
        make_task_with_project(3, TaskStatus::Backlog, 1),
        make_task_with_project(4, TaskStatus::Backlog, 2),
    ];
    let mut app = App::new(tasks, 1, TEST_TIMEOUT);
    // Move cursor to row 2 in column 1 (Backlog)
    app.selection_mut().set_row(1, 2);
    app.update_anchor_from_current();

    app.update(Message::SelectProject(2));

    let row = app.selected_row()[0];
    assert_eq!(
        row, 0,
        "Cursor should be clamped to 0 after project switch, got {row}"
    );
}

// ---------------------------------------------------------------------------
// Input handling tests (Task 8)
// ---------------------------------------------------------------------------

fn two_project_app() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.update(Message::ProjectsUpdated(vec![
        Project {
            id: 1,
            name: "Default".to_string(),
            sort_order: 0,
            is_default: true,
        },
        Project {
            id: 2,
            name: "Backend".to_string(),
            sort_order: 1,
            is_default: false,
        },
    ]));
    app
}

#[test]
fn h_from_backlog_opens_projects_panel() {
    let mut app = two_project_app();
    assert_eq!(app.selected_column(), 1);
    assert!(!app.projects_panel_visible());

    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible());
}

#[test]
fn left_from_backlog_opens_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Left));
    assert!(app.projects_panel_visible());
}

#[test]
fn h_not_at_column0_does_not_open_panel() {
    let mut app = two_project_app();
    // Move to column 2 (Running)
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 2);
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(!app.projects_panel_visible());
}

#[test]
fn h_in_epic_view_does_not_open_projects_panel() {
    use crate::models::EpicId;
    let mut app = two_project_app();
    app.board.epics = vec![super::helpers::make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    assert!(matches!(
        app.view_mode(),
        crate::tui::types::ViewMode::Epic { .. }
    ));

    // In Epic view at column 0, h should NOT open the projects panel — it
    // stays in Epic view and navigates columns (no-op at column 0).
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(!app.projects_panel_visible());
    assert!(matches!(
        app.view_mode(),
        crate::tui::types::ViewMode::Epic { .. }
    ));
}

#[test]
fn esc_closes_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible());

    app.handle_key(make_key(KeyCode::Esc));
    assert!(!app.projects_panel_visible());
}

#[test]
fn h_in_projects_panel_is_noop() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible(), "precondition: panel open");

    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(
        app.projects_panel_visible(),
        "h in projects panel should be a no-op, panel should remain open"
    );
}

#[test]
fn left_in_projects_panel_is_noop() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible(), "precondition: panel open");

    app.handle_key(make_key(KeyCode::Left));
    assert!(
        app.projects_panel_visible(),
        "Left in projects panel should be a no-op, panel should remain open"
    );
}

#[test]
fn j_moves_cursor_down_in_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.selected_project_row(), 0);

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.selected_project_row(), 1);
}

#[test]
fn k_moves_cursor_up_in_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    // Start at row 0, j moves to 1, k moves back to 0
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.selected_project_row(), 0);
}

#[test]
fn k_does_not_underflow_at_row0() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.selected_project_row(), 0);
}

#[test]
fn l_closes_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible());

    app.handle_key(make_key(KeyCode::Char('l')));
    assert!(!app.projects_panel_visible());
    assert_eq!(app.selected_column(), 1, "focus should return to Backlog");
}

#[test]
fn right_closes_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible());

    app.handle_key(make_key(KeyCode::Right));
    assert!(!app.projects_panel_visible());
    assert_eq!(app.selected_column(), 1, "focus should return to Backlog");
}

#[test]
fn enter_selects_project_and_stays_in_col0() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    // Navigate to row 1 (Backend, id=2)
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Enter));
    // Enter activates the project but focus stays at col 0 per spec
    assert!(app.projects_panel_visible());
    assert_eq!(app.active_project(), 2);
}

#[test]
fn n_in_panel_enters_input_project_name_mode() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert!(matches!(
        app.mode(),
        InputMode::InputProjectName { editing_id: None }
    ));
    assert!(app.input_buffer().is_empty());
}

#[test]
fn r_in_panel_enters_rename_mode_with_buffer() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    // Row 0 = Default project (id=1)
    app.handle_key(make_key(KeyCode::Char('r')));
    assert!(matches!(
        app.mode(),
        InputMode::InputProjectName {
            editing_id: Some(1)
        }
    ));
    assert_eq!(app.input_buffer(), "Default");
}

#[test]
fn d_in_panel_enters_confirm_delete_non_default() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    // Navigate to row 1 (Backend, not default)
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(
        app.mode(),
        InputMode::ConfirmDeleteProject1 { id: 2 }
    ));
}

#[test]
fn d_on_default_project_does_nothing() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    // Row 0 = Default (is_default=true)
    app.handle_key(make_key(KeyCode::Char('d')));
    // Mode should still be Normal — default project cannot be deleted
    assert!(matches!(app.mode(), InputMode::Normal));
}

#[test]
fn input_project_name_enter_creates_project() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('n')));
    // Type "MyProj"
    for c in "MyProj".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.mode(), InputMode::Normal));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CreateProject { name } if name == "MyProj")));
}

#[test]
fn input_project_name_enter_renames_project() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('r')));
    // Buffer starts with "Default"; clear it and type "Renamed"
    // backspace 7 times
    for _ in 0..7 {
        app.handle_key(make_key(KeyCode::Backspace));
    }
    for c in "Renamed".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.mode(), InputMode::Normal));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::RenameProject { id: 1, name } if name == "Renamed")));
}

#[test]
fn input_project_name_esc_resets_mode() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('n')));
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.mode(), InputMode::Normal));
}

#[test]
fn input_project_name_empty_enter_does_not_create() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('n')));
    // Buffer empty → Enter should not emit CreateProject
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.mode(), InputMode::Normal));
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CreateProject { .. })));
}

#[test]
fn confirm_delete_project1_y_transitions_to_confirm2() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('d')));
    app.handle_key(make_key(KeyCode::Char('y')));
    assert!(matches!(
        app.mode(),
        InputMode::ConfirmDeleteProject2 { id: 2, .. }
    ));
}

#[test]
fn confirm_delete_project1_n_resets_mode() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('d')));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert!(matches!(app.mode(), InputMode::Normal));
}

#[test]
fn confirm_delete_project2_y_emits_delete_command() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('d')));
    app.handle_key(make_key(KeyCode::Char('y')));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(matches!(app.mode(), InputMode::Normal));
    assert!(!app.projects_panel_visible());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteProject { id: 2 })));
}

#[test]
fn confirm_delete_project2_n_resets_mode() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('d')));
    app.handle_key(make_key(KeyCode::Char('y')));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert!(matches!(app.mode(), InputMode::Normal));
}

#[test]
fn shift_j_emits_reorder_project_down() {
    use crossterm::event::KeyEvent;
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ReorderProject { id: 1, delta: 1 })));
}

#[test]
fn shift_k_emits_reorder_project_up() {
    use crossterm::event::KeyEvent;
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ReorderProject { id: 2, delta: -1 })));
}

// ---------------------------------------------------------------------------
// Inline Projects column rendering tests (Task 7)
// ---------------------------------------------------------------------------

fn make_app_with_default_project() -> App {
    let mut app = make_app();
    app.board.projects.push(Project {
        id: 1,
        name: "Default".to_string(),
        is_default: true,
        sort_order: 0,
    });
    app
}

#[test]
fn projects_column_renders_project_cards_when_focused() {
    let mut app = make_app_with_default_project();
    // Navigate from col 1 (Backlog) left to col 0 (Projects)
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 0);
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "Default"),
        "expected Default project card in buffer"
    );
}

#[test]
fn projects_column_shows_task_count() {
    let mut app = make_app_with_default_project();
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 0);
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "tasks"),
        "expected task count in project card"
    );
}

#[test]
fn selecting_project_keeps_focus_in_col_0() {
    let mut app = make_app_with_default_project();
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 0);
    let project_id = app.projects()[0].id;
    app.update(Message::SelectProject(project_id));
    assert_eq!(
        app.selected_column(),
        0,
        "focus should stay in Projects column after SelectProject"
    );
}

#[test]
fn refresh_preserves_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible(), "precondition: projects panel open");

    let tasks = app.board.tasks.clone();
    app.update(Message::RefreshTasks(tasks));

    assert!(
        app.projects_panel_visible(),
        "projects panel should stay open after refresh, but cursor moved to col {}",
        app.selected_column()
    );
}

#[test]
fn l_in_archive_is_noop() {
    let mut app = make_app_with_archived_task();
    app.update(Message::NavigateColumn(1));
    app.update(Message::NavigateColumn(1));
    app.update(Message::NavigateColumn(1));
    app.update(Message::NavigateColumn(1));
    assert!(app.show_archived(), "precondition: archive column open");

    app.handle_key(make_key(KeyCode::Char('l')));
    assert!(
        app.show_archived(),
        "l in archive should be a no-op, column should remain open"
    );
}

#[test]
fn refresh_preserves_archive_column() {
    let mut app = make_app_with_archived_task();
    app.update(Message::NavigateColumn(1));
    app.update(Message::NavigateColumn(1));
    app.update(Message::NavigateColumn(1));
    app.update(Message::NavigateColumn(1));
    assert!(app.show_archived(), "precondition: archive column open");

    let tasks = app.board.tasks.clone();
    app.update(Message::RefreshTasks(tasks));

    assert!(
        app.show_archived(),
        "archive column should stay open after refresh, but cursor moved to col {}",
        app.selected_column()
    );
}

#[test]
fn select_project_emits_persist_string_setting() {
    let mut app = two_project_app();
    let cmds = app.update(Message::SelectProject(2));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::PersistStringSetting { key, value }
            if key == "last_project" && value == "2"
        )),
        "expected PersistStringSetting(last_project=2) but got {cmds:?}"
    );
}
