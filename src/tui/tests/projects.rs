use crossterm::event::{KeyCode, KeyModifiers};

use crate::models::{Project, ProjectId, Task, TaskStatus};
use crate::tui::types::{Command, InputMode};
use crate::tui::{App, Message};

use super::helpers::{
    buffer_contains, make_app, make_app_with_archived_task, make_key, make_task, render_to_buffer,
    TEST_TIMEOUT,
};

fn make_task_with_project(id: i64, status: TaskStatus, project_id: ProjectId) -> Task {
    Task {
        project_id,
        ..make_task(id, status)
    }
}

#[test]
fn project_matches_hides_tasks_from_other_project() {
    let t1 = make_task_with_project(1, TaskStatus::Backlog, ProjectId(1));
    let t2 = make_task_with_project(2, TaskStatus::Backlog, ProjectId(2));

    let app = App::new(vec![t1, t2], ProjectId(1), TEST_TIMEOUT);
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
    app.update(Message::SelectProject(ProjectId(2)));
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
    let t = make_task_with_project(1, TaskStatus::Archived, ProjectId(2));
    let app = App::new(vec![t], ProjectId(1), TEST_TIMEOUT);
    // active_project = 1, task is in project 2 → archived_tasks returns empty
    assert_eq!(app.archived_tasks().len(), 0);
}

#[test]
fn select_project_clamps_cursor() {
    // Project 1 has 3 tasks in Backlog; project 2 has 1.
    // Cursor is at row 2 (third item in Backlog). Switching to project 2
    // should clamp cursor to row 0.
    let tasks = vec![
        make_task_with_project(1, TaskStatus::Backlog, ProjectId(1)),
        make_task_with_project(2, TaskStatus::Backlog, ProjectId(1)),
        make_task_with_project(3, TaskStatus::Backlog, ProjectId(1)),
        make_task_with_project(4, TaskStatus::Backlog, ProjectId(2)),
    ];
    let mut app = App::new(tasks, ProjectId(1), TEST_TIMEOUT);
    // Move cursor to row 2 in column 1 (Backlog)
    app.selection_mut().set_row(1, 2);
    app.update_anchor_from_current();

    app.update(Message::SelectProject(ProjectId(2)));

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
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], ProjectId(1), TEST_TIMEOUT);
    app.update(Message::ProjectsUpdated(vec![
        Project {
            id: ProjectId(1),
            name: "Default".to_string(),
            sort_order: 0,
            is_default: true,
        },
        Project {
            id: ProjectId(2),
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
    assert_eq!(
        app.active_project(),
        ProjectId(2),
        "j should auto-select Backend (id=2)"
    );
}

#[test]
fn k_moves_cursor_up_in_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.selected_project_row(), 0);
    assert_eq!(
        app.active_project(),
        ProjectId(1),
        "k should auto-select Default (id=1)"
    );
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
fn enter_closes_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Enter));
    assert!(
        !app.projects_panel_visible(),
        "Enter should close the panel"
    );
    assert_eq!(app.selected_column(), 1, "focus should return to Backlog");
    assert_eq!(
        app.active_project(),
        ProjectId(2),
        "active project should still be Backend"
    );
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
            editing_id: Some(ProjectId(1))
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
        InputMode::ConfirmDeleteProject1 { id: ProjectId(2) }
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
        .any(|c| matches!(c, Command::RenameProject { id: ProjectId(1), name } if name == "Renamed")));
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
        InputMode::ConfirmDeleteProject2 { id: ProjectId(2), .. }
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
        .any(|c| matches!(c, Command::DeleteProject { id: ProjectId(2) })));
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
fn shift_j_cursor_follows_moved_project_down() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(
        app.selected_project_row(),
        0,
        "precondition: cursor at row 0 (Default)"
    );

    app.update(Message::ProjectsUpdated(vec![
        Project {
            id: ProjectId(2),
            name: "Backend".to_string(),
            sort_order: 0,
            is_default: false,
        },
        Project {
            id: ProjectId(1),
            name: "Default".to_string(),
            sort_order: 1,
            is_default: true,
        },
    ]));
    app.update(Message::FollowProject(ProjectId(1)));

    assert_eq!(
        app.selected_project_row(),
        1,
        "cursor should follow Default project to its new index 1"
    );
}

#[test]
fn shift_k_cursor_follows_moved_project_up() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.selected_project_row(), 1);

    app.update(Message::ProjectsUpdated(vec![
        Project {
            id: ProjectId(2),
            name: "Backend".to_string(),
            sort_order: 0,
            is_default: false,
        },
        Project {
            id: ProjectId(1),
            name: "Default".to_string(),
            sort_order: 1,
            is_default: true,
        },
    ]));
    app.update(Message::FollowProject(ProjectId(2)));

    assert_eq!(
        app.selected_project_row(),
        0,
        "cursor should follow Backend project to its new index 0"
    );
}

#[test]
fn follow_project_unknown_id_does_not_panic() {
    let mut app = two_project_app();
    app.update(Message::FollowProject(ProjectId(99)));
    assert_eq!(app.selected_project_row(), 0);
}

#[test]
fn follow_project_at_boundary_does_not_move_cursor() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.selected_project_row(), 0);
    app.update(Message::FollowProject(ProjectId(1)));
    assert_eq!(app.selected_project_row(), 0, "cursor should stay at 0");
}

#[test]
fn follow_project_updates_list_state_and_selection_in_sync() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.update(Message::ProjectsUpdated(vec![
        Project {
            id: ProjectId(2),
            name: "Backend".to_string(),
            sort_order: 0,
            is_default: false,
        },
        Project {
            id: ProjectId(1),
            name: "Default".to_string(),
            sort_order: 1,
            is_default: true,
        },
    ]));
    app.update(Message::FollowProject(ProjectId(2)));

    assert_eq!(app.selected_project_row(), 0, "selection row should be 0");
    assert_eq!(
        app.selected_project().map(|p| p.id),
        Some(ProjectId(2)),
        "selected_project() should return Backend (id=2) after FollowProject"
    );
}

#[test]
fn g_closes_projects_panel() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('g')));
    assert!(!app.projects_panel_visible(), "g should close the panel");
    assert_eq!(app.selected_column(), 1, "focus should return to Backlog");
    assert_eq!(
        app.active_project(),
        ProjectId(2),
        "active project should still be Backend"
    );
}

#[test]
fn g_in_empty_projects_panel_closes_panel() {
    let mut app = App::new(vec![], ProjectId(1), TEST_TIMEOUT);
    app.update(Message::ProjectsUpdated(vec![]));
    app.update(Message::NavigateColumn(-1));
    assert!(app.projects_panel_visible(), "precondition: panel open");
    app.handle_key(make_key(KeyCode::Char('g')));
    assert!(
        !app.projects_panel_visible(),
        "g should close panel even with no projects"
    );
}

#[test]
fn shift_j_emits_reorder_project_down() {
    use crossterm::event::KeyEvent;
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ReorderProject { id: ProjectId(1), delta: 1 })));
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
        .any(|c| matches!(c, Command::ReorderProject { id: ProjectId(2), delta: -1 })));
}

// ---------------------------------------------------------------------------
// Inline Projects column rendering tests (Task 7)
// ---------------------------------------------------------------------------

fn make_app_with_default_project() -> App {
    let mut app = make_app();
    app.board.projects.push(Project {
        id: ProjectId(1),
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
    assert!(
        app.projects_panel_visible(),
        "precondition: projects panel open"
    );

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
    let cmds = app.update(Message::SelectProject(ProjectId(2)));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::PersistStringSetting { key, value }
            if key == "last_project" && value == "2"
        )),
        "expected PersistStringSetting(last_project=2) but got {cmds:?}"
    );
}

#[test]
fn q_from_board_opens_projects_panel() {
    let mut app = two_project_app();
    assert_eq!(app.selected_column(), 1);
    assert!(!app.projects_panel_visible());

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(
        app.projects_panel_visible(),
        "q should navigate to projects panel"
    );
    assert!(!app.should_quit(), "q should not quit yet");
}

#[test]
fn q_from_projects_panel_triggers_quit_prompt() {
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('h')));
    assert!(app.projects_panel_visible());

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(
        matches!(app.mode(), crate::tui::types::InputMode::ConfirmQuit),
        "q from projects panel should enter ConfirmQuit mode"
    );
}

// ---------------------------------------------------------------------------
// Default project catch-all filter
// ---------------------------------------------------------------------------

fn app_with_default_and_custom_projects() -> App {
    let t1 = make_task_with_project(1, TaskStatus::Backlog, ProjectId(1)); // Default
    let t2 = make_task_with_project(2, TaskStatus::Backlog, ProjectId(2)); // Custom
    let mut app = App::new(vec![t1, t2], ProjectId(1), TEST_TIMEOUT);
    app.update(Message::ProjectsUpdated(vec![
        Project {
            id: ProjectId(1),
            name: "Default".to_string(),
            sort_order: 0,
            is_default: true,
        },
        Project {
            id: ProjectId(2),
            name: "Custom".to_string(),
            sort_order: 1,
            is_default: false,
        },
    ]));
    app
}

#[test]
fn default_project_shows_all_tasks() {
    let app = app_with_default_and_custom_projects();
    // active_project = 1 (Default) → both tasks should appear
    let visible = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(
        visible.len(),
        2,
        "Default project should show all tasks regardless of project_id"
    );
}

#[test]
fn non_default_project_shows_only_its_own_tasks() {
    let mut app = app_with_default_and_custom_projects();
    app.update(Message::SelectProject(ProjectId(2)));
    let visible = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(
        visible.len(),
        1,
        "Custom project should show only its own tasks"
    );
}

#[test]
fn default_project_shows_archived_from_all_projects() {
    let mut app = app_with_default_and_custom_projects();
    app.board.tasks = vec![
        make_task_with_project(1, TaskStatus::Archived, ProjectId(1)),
        make_task_with_project(2, TaskStatus::Archived, ProjectId(2)),
    ];
    assert_eq!(
        app.archived_tasks().len(),
        2,
        "Default project should show archived tasks from all projects"
    );
}

#[test]
fn q_then_q_full_flow() {
    // First q opens the projects panel; second q enters ConfirmQuit
    let mut app = two_project_app();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(
        app.projects_panel_visible(),
        "first q should open projects panel"
    );

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(
        matches!(app.mode(), crate::tui::types::InputMode::ConfirmQuit),
        "second q should enter ConfirmQuit"
    );
}

#[test]
fn q_in_epic_view_exits_epic_not_projects_panel() {
    use crate::models::EpicId;
    let mut app = two_project_app();
    app.board.epics = vec![super::helpers::make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    assert!(matches!(
        app.view_mode(),
        crate::tui::types::ViewMode::Epic { .. }
    ));

    app.handle_key(make_key(KeyCode::Char('q')));
    // Should exit the epic, not open the projects panel
    assert!(!app.projects_panel_visible());
    assert!(matches!(
        app.view_mode(),
        crate::tui::types::ViewMode::Board(_)
    ));
}

#[test]
fn opening_projects_panel_positions_cursor_on_active_project() {
    // Two projects: Default (id=1, idx=0), Backend (id=2, idx=1).
    // Active project is Backend. Opening the panel should start at row 1.
    let mut app = two_project_app();

    // Select Backend via j while in the panel.
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.active_project(), 2);
    app.handle_key(make_key(KeyCode::Char('l'))); // close panel

    // Simulate stale cursor (as happens after app load from persisted settings).
    app.selection_mut().set_row(0, 0);
    app.projects_panel.list_state.select(Some(0));

    // Re-open: cursor must jump to Backend's row (1).
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(
        app.selected_project_row(),
        1,
        "cursor should be on Backend (row 1), not Default (row 0)"
    );
}

#[test]
fn opening_projects_panel_with_default_project_active_starts_at_row0() {
    // Default project (id=1) is at row 0. Opening the panel should start there
    // even if the cursor was left at a different row.
    let mut app = two_project_app();
    // active_project is Default (id=1) from two_project_app().

    // Simulate stale cursor at row 1.
    app.selection_mut().set_row(0, 1);
    app.projects_panel.list_state.select(Some(1));

    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(
        app.selected_project_row(),
        0,
        "cursor should be on Default (row 0)"
    );
}

#[test]
fn opening_projects_panel_first_time_with_non_default_active_project() {
    // Simulates app startup where active_project was restored from persisted
    // settings but the panel has never been opened (projects_row defaults to 0).
    // SelectProject updates active_project and list_state but not projects_row,
    // so the cursor is stale until the panel is opened.
    let mut app = two_project_app();
    app.update(Message::SelectProject(2)); // Backend, idx=1; projects_row stays 0

    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(
        app.selected_project_row(),
        1,
        "first open should land on Backend (row 1), not Default (row 0)"
    );
}
