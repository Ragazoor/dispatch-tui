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
fn start_repo_filter_enters_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.update(Message::StartRepoFilter);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn toggle_repo_filter_adds_and_removes() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.update(Message::ToggleRepoFilter("/repo-a".to_string()));
    assert!(app.filter.repos.contains("/repo-a"));
    assert!(!app.filter.repos.contains("/repo-b"));

    app.update(Message::ToggleRepoFilter("/repo-a".to_string()));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn toggle_all_repo_filter_selects_all_then_clears() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    // Toggle all on
    app.update(Message::ToggleAllRepoFilter);
    assert_eq!(app.filter.repos.len(), 2);

    // Toggle all off
    app.update(Message::ToggleAllRepoFilter);
    assert!(app.filter.repos.is_empty());
}

#[test]
fn close_repo_filter_returns_to_normal() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.update(Message::CloseRepoFilter);
    assert_eq!(app.input.mode, InputMode::Normal);
    // Should emit PersistStringSetting
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistStringSetting { .. })));
}

#[test]
fn flattened_board_respects_repo_filter() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    let mut in_repo = make_task(1, TaskStatus::Backlog);
    in_repo.epic_id = Some(EpicId(10));
    in_repo.repo_path = "/included".to_string();
    let mut out_repo = make_task(2, TaskStatus::Backlog);
    out_repo.epic_id = Some(EpicId(10));
    out_repo.repo_path = "/excluded".to_string();
    app.board.tasks = vec![in_repo, out_repo];
    app.board.flattened = true;
    app.filter.repos = vec!["/included".to_string()].into_iter().collect();

    let visible = app.tasks_for_current_view();
    let ids: std::collections::HashSet<_> = visible.iter().map(|t| t.id).collect();
    assert!(ids.contains(&TaskId(1)));
    assert!(!ids.contains(&TaskId(2)));
}

#[test]
fn repo_filter_empty_shows_all_tasks() {
    let app = make_app();
    // repo_filter is empty by default => all tasks visible
    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 4); // tasks 1,2,3,4 (Done tasks are visible, only Archived are excluded)
}

#[test]
fn repo_filter_hides_non_matching_tasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.repos.insert("/repo-a".to_string());

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(1));
}

#[test]
fn repo_filter_applies_to_epics_in_column_items() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let now = chrono::Utc::now();
    app.board.epics = vec![
        Epic {
            id: EpicId(1),
            title: "A".into(),
            description: "".into(),
            repo_path: "/repo-a".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        },
        Epic {
            id: EpicId(2),
            title: "B".into(),
            description: "".into(),
            repo_path: "/repo-b".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        },
    ];
    app.filter.repos.insert("/repo-a".to_string());

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 1); // only epic A
}

#[test]
fn f_key_opens_repo_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn repo_filter_number_key_toggles_repo() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('1')));
    assert!(app.filter.repos.contains("/repo-a"));

    app.handle_key(make_key(KeyCode::Char('1')));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn repo_filter_a_key_toggles_all() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.filter.repos.len(), 2);
}

#[test]
fn repo_filter_enter_closes() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistStringSetting { .. })));
}

#[test]
fn repo_filter_esc_closes() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn repo_filter_out_of_range_number_ignored() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('5')));
    assert!(app.filter.repos.is_empty());
}

#[test]
fn repo_filter_exclude_hides_matching_tasks() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(2));
}

#[test]
fn repo_filter_exclude_empty_shows_all() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.mode = RepoFilterMode::Exclude;

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 2);
}

#[test]
fn repo_filter_exclude_applies_to_epics() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let now = chrono::Utc::now();
    app.board.epics = vec![
        Epic {
            id: EpicId(1),
            title: "A".into(),
            description: "".into(),
            repo_path: "/repo-a".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        },
        Epic {
            id: EpicId(2),
            title: "B".into(),
            description: "".into(),
            repo_path: "/repo-b".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        },
    ];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 1);
    match &items[0] {
        ColumnItem::Epic(e) => assert_eq!(e.id, EpicId(2)),
        _ => panic!("Expected epic"),
    }
}

#[test]
fn close_repo_filter_persists_mode() {
    let mut app = make_app();
    app.filter.mode = RepoFilterMode::Exclude;
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.update(Message::CloseRepoFilter);
    assert!(cmds.iter().any(|c| matches!(c,
        Command::PersistStringSetting { key, value } if key == "repo_filter_mode" && value == "exclude"
    )));
}

#[test]
fn repo_filter_overlay_shows_mode_in_title() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.filter.mode = RepoFilterMode::Exclude;
    app.input.mode = InputMode::RepoFilter;

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(
        buffer_contains(&buf, "exclude"),
        "Expected 'exclude' in overlay title"
    );
}

#[test]
fn load_filter_preset_replaces_repo_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.filter.repos.insert("/repo-a".to_string());

    let preset_repos: HashSet<String> = ["/repo-b".to_string()].into_iter().collect();
    app.filter.presets = vec![("backend".to_string(), preset_repos, RepoFilterMode::Include)];

    app.update(Message::LoadFilterPreset("backend".to_string()));
    assert!(app.filter.repos.contains("/repo-b"));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn cancel_preset_input_returns_to_repo_filter() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "draft".to_string();
    app.update(Message::CancelPresetInput);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(app.input.buffer.is_empty());
}

#[test]
fn repo_filter_s_key_starts_save_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Char('s')));
    assert_eq!(app.input.mode, InputMode::InputPresetName);
}

#[test]
fn repo_filter_x_key_starts_delete_preset() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo".to_string()].into_iter().collect();
    app.filter.presets = vec![("test".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmDeletePreset);
}

#[test]
fn repo_filter_shift_a_loads_first_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    let repos: HashSet<String> = ["/repo-b".to_string()].into_iter().collect();
    app.filter.presets = vec![("backend".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert!(app.filter.repos.contains("/repo-b"));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn repo_filter_overlay_shows_presets() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("frontend".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::RepoFilter;

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(buffer_contains(&buf, "A"), "Expected preset letter A");
    assert!(
        buffer_contains(&buf, "frontend"),
        "Expected preset name 'frontend'"
    );
}

#[test]
fn repo_filter_overlay_shows_name_input() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "myfilter".to_string();

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(buffer_contains(&buf, "Name:"), "Expected name input prompt");
    assert!(buffer_contains(&buf, "myfilter"), "Expected buffer content");
}

#[test]
fn repo_filter_overlay_shows_delete_help() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string()];
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("test".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::ConfirmDeletePreset;

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(
        buffer_contains(&buf, "delete preset"),
        "Expected delete help text"
    );
}

#[test]
fn handle_key_repo_filter_toggle() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('1')));
    assert!(app.filter.repos.contains("/repo"));
}

#[test]
fn handle_key_repo_filter_close_enter() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_repo_filter_close_esc() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_repo_filter_close_q() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn repo_filter_j_moves_cursor_down() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn repo_filter_space_toggles_cursor_repo() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 1; // pointing at /repo-b
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        app.filter.repos.contains("/repo-b"),
        "cursor repo should be toggled"
    );
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn render_status_bar_repo_filter() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Filter repos"),
        "RepoFilter mode should show 'Filter repos'"
    );
}

#[test]
fn render_repo_filter_overlay_shows_title() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Repo Filter"),
        "repo filter overlay should show 'Repo Filter' title"
    );
}

#[test]
fn render_repo_filter_overlay_shows_repos() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/alpha".to_string(), "/repo/beta".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "/repo/alpha"),
        "repo filter overlay should show '/repo/alpha'"
    );
    assert!(
        buffer_contains(&buf, "/repo/beta"),
        "repo filter overlay should show '/repo/beta'"
    );
}

#[test]
fn render_repo_filter_overlay_shows_include_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "include"),
        "repo filter overlay should show 'include' as default mode"
    );
}

#[test]
fn render_repo_filter_overlay_shows_navigate_hint() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "navigate"),
        "repo filter overlay should show 'navigate' hint"
    );
}

#[test]
fn render_repo_filter_input_preset_name() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string()];
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "my-preset".to_string();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Name:"),
        "preset name input should show 'Name:' label"
    );
    assert!(
        buffer_contains(&buf, "my-preset"),
        "preset name input should show the buffer content 'my-preset'"
    );
}

#[test]
fn render_repo_filter_confirm_delete_preset() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string()];
    app.input.mode = InputMode::ConfirmDeletePreset;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "delete preset"),
        "confirm delete mode should show 'delete preset' text"
    );
}

#[test]
fn start_delete_repo_path_enters_confirm_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.update(Message::StartDeleteRepoPath);
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn start_delete_repo_path_no_repos_is_noop() {
    let mut app = make_app();
    app.board.repo_paths = vec![];
    app.input.mode = InputMode::RepoFilter;
    app.update(Message::StartDeleteRepoPath);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn confirm_delete_repo_path_emits_command() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 1;
    let cmds = app.update(Message::DeleteRepoPath("/repo-b".to_string()));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteRepoPath(p) if p == "/repo-b")));
}

#[test]
fn cancel_delete_repo_path_returns_to_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn delete_repo_path_removes_from_active_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.repos.insert("/repo-b".to_string());
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.update(Message::DeleteRepoPath("/repo-a".to_string()));
    assert!(!app.filter.repos.contains("/repo-a"));
    assert!(app.filter.repos.contains("/repo-b"));
}

#[test]
fn delete_repo_path_clamps_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.repo_cursor = 1;
    // Simulate the path being removed (RepoPathsUpdated would do this in practice)
    app.update(Message::RepoPathsUpdated(vec!["/repo-a".to_string()]));
    assert!(
        app.input.repo_cursor < app.board.repo_paths.len(),
        "cursor should be clamped after repo list shrinks"
    );
}

#[test]
fn backspace_in_repo_filter_starts_delete() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn delete_key_in_repo_filter_starts_delete() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Delete));
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn y_in_confirm_delete_repo_path_confirms() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 0;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteRepoPath(p) if p == "/repo-a")));
}

#[test]
fn n_in_confirm_delete_repo_path_cancels() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn handle_key_normal_start_repo_filter() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(*app.mode(), InputMode::RepoFilter);
}

#[test]
fn handle_key_repo_filter_a_toggles_all() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('a')));
    // Should toggle all repos in filter
    assert!(!app.filter.repos.is_empty());
}

#[test]
fn handle_key_repo_filter_space_toggles_cursor_item() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.filter.repos.contains("/repo"));
}

#[test]
fn handle_key_repo_filter_j_moves_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn handle_key_repo_filter_k_moves_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 1;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn handle_key_repo_filter_backspace_starts_delete_repo_path() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(*app.mode(), InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn handle_key_repo_filter_s_starts_save_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('s')));
    assert_eq!(*app.mode(), InputMode::InputPresetName);
}

#[test]
fn handle_key_repo_filter_x_starts_delete_preset() {
    let mut app = make_app();
    app.filter.presets = vec![(
        "preset-a".to_string(),
        std::collections::HashSet::new(),
        RepoFilterMode::Include,
    )];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(*app.mode(), InputMode::ConfirmDeletePreset);
}

#[test]
fn handle_key_repo_filter_uppercase_loads_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.filter.presets = vec![(
        "preset-a".to_string(),
        {
            let mut s = std::collections::HashSet::new();
            s.insert("/repo".to_string());
            s
        },
        RepoFilterMode::Include,
    )];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('A')));
    assert!(app.filter.repos.contains("/repo"));
}

#[test]
fn handle_key_repo_filter_uppercase_out_of_range_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    // No presets
    let cmds = app.handle_key(make_key(KeyCode::Char('A')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_repo_filter_unknown_key_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_confirm_delete_repo_path_y_deletes() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds.iter().any(|c| matches!(c, Command::DeleteRepoPath(_))));
}

#[test]
fn handle_key_confirm_delete_repo_path_other_cancels() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;

    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::RepoFilter);
}

#[test]
fn handle_key_confirm_delete_repo_path_uppercase_y() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert!(cmds.iter().any(|c| matches!(c, Command::DeleteRepoPath(_))));
}

/// RepoFilter mode routes correctly.
#[test]
fn handle_key_repo_filter_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    // Esc closes the filter (may emit refresh commands)
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDeleteRepoPath mode routes correctly.
#[test]
fn handle_key_confirm_delete_repo_path_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    // Any non-y key returns to RepoFilter
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
#[ignore = "requires a real TTY and interactive editor session; run manually to verify"]
fn buffered_editor_keystrokes_do_not_leak_into_repo_picker() {}

// ---------------------------------------------------------------------------
// Epic-in-epic: TUI navigation tests
// ---------------------------------------------------------------------------

#[test]
fn enter_sub_epic_from_epic_view_nests_parent() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(1), make_epic(2)];
    // Start in Epic view for epic 1
    app.update(Message::EnterEpic(EpicId(1)));
    app.selection_mut().set_column(2);

    // Enter sub-epic 2 from within epic 1
    app.update(Message::EnterEpic(EpicId(2)));

    match &app.board.view_mode {
        ViewMode::Epic {
            epic_id, parent, ..
        } => {
            assert_eq!(*epic_id, EpicId(2), "should be in sub-epic 2");
            match parent.as_ref() {
                ViewMode::Epic {
                    epic_id: parent_id, ..
                } => {
                    assert_eq!(*parent_id, EpicId(1), "parent should be epic 1");
                }
                _ => panic!("Expected parent to be ViewMode::Epic"),
            }
        }
        _ => panic!("Expected ViewMode::Epic"),
    }
}
