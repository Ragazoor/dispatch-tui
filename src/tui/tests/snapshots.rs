use ratatui::buffer::Buffer;

use super::super::App;
use super::{make_app, make_key, make_task, render_to_buffer, TEST_TIMEOUT};
use crate::models::TaskStatus;
use crossterm::event::KeyCode;

fn buffer_to_string(buf: &Buffer) -> String {
    let area = buf.area();
    let mut lines = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut line = String::with_capacity(area.width as usize * 3);
        for x in area.left()..area.right() {
            line.push_str(buf[(x, y)].symbol());
        }
        line.truncate(line.trim_end().len());
        lines.push(line);
    }
    lines.join("\n")
}

fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
    buffer_to_string(&render_to_buffer(app, width, height))
}

#[test]
fn snapshot_empty_kanban_board() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_kanban_with_tasks() {
    let mut app = make_app();
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_help_overlay() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('?')));
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_input_title_form() {
    use super::super::types::{InputMode, TaskDraft};
    let mut app = make_app();
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "My new task".to_string();
    app.input.task_draft = Some(TaskDraft::default());
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_input_repo_path_form() {
    use super::super::types::{InputMode, TaskDraft};
    use crate::models::TaskTag;
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo/alpha".to_string(), "/repo/beta".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer = String::new();
    app.input.task_draft = Some(TaskDraft {
        title: "My new task".to_string(),
        description: "A description".to_string(),
        tag: Some(TaskTag::Feature),
        ..TaskDraft::default()
    });
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_quick_dispatch_form() {
    use super::super::types::InputMode;
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo/alpha".to_string(), "/repo/beta".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_confirm_retry_form() {
    use super::super::types::InputMode;
    use crate::models::TaskId;
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmRetry(TaskId(1));
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_input_epic_title_form() {
    use super::super::types::{EpicDraft, InputMode};
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "My new epic".to_string();
    app.input.epic_draft = Some(EpicDraft::default());
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

fn make_feed_epic(id: i64, title: &str, sort_order: i64) -> crate::models::Epic {
    let now = chrono::Utc::now();
    crate::models::Epic {
        id: crate::models::EpicId(id),
        title: title.to_string(),
        description: String::new(),
        repo_path: "/repo".to_string(),
        status: crate::models::TaskStatus::Backlog,
        plan_path: None,
        sort_order: Some(sort_order),
        auto_dispatch: false,
        parent_epic_id: None,
        feed_command: Some(format!("feed-{title}")),
        feed_interval_secs: Some(30),
        created_at: now,
        updated_at: now,
        project_id: 1,
    }
}

#[test]
fn snapshot_tab_bar_with_feed_epics_board_active() {
    let mut app = App::new(vec![], 1, super::TEST_TIMEOUT);
    app.board.epics = vec![
        make_feed_epic(1, "My Feed", -2),
        make_feed_epic(2, "Another Feed", -1),
    ];
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tab_bar_with_feed_epics_feed_active() {
    use super::super::types::Message;
    let mut app = App::new(vec![], 1, super::TEST_TIMEOUT);
    app.board.epics = vec![
        make_feed_epic(1, "My Feed", -2),
        make_feed_epic(2, "Another Feed", -1),
    ];
    // Enter the first feed epic view to make its tab active
    let feed_epic_id = app
        .epics()
        .iter()
        .find(|e| e.feed_command.is_some())
        .unwrap()
        .id;
    app.update(Message::EnterEpic(feed_epic_id));
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_kanban_with_projects_focused() {
    use super::super::types::Message;
    use super::make_app;
    let mut app = make_app();
    // Add a project so the Projects column has content
    app.board.projects.push(crate::models::Project {
        id: 1,
        name: "Default".to_string(),
        is_default: true,
        sort_order: 0,
    });
    // Navigate to Projects (col 0) — make_app starts at col 1 (Backlog)
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selected_column(), 0);
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_kanban_with_archive_focused() {
    use super::super::types::Message;
    use super::make_app_with_archived_task;
    let mut app = make_app_with_archived_task();
    // Navigate to Archive (col 5 = COLUMN_COUNT + 1) — make_app starts at col 1 (Backlog)
    for _ in 0..4 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(
        app.selected_column(),
        crate::models::TaskStatus::COLUMN_COUNT + 1
    );
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_task_detail_overlay_peek() {
    use crate::tui::Message;
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut task = make_task(1, TaskStatus::Backlog);
    task.description = "First line of description.\nSecond line.\nThird line.".to_string();
    task.repo_path = "/repo/my-project".to_string();
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    app.board.tasks.push(task);
    app.update(Message::OpenTaskDetail(1));
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_task_detail_overlay_zoomed() {
    use crate::tui::{Message, ViewMode};
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut task = make_task(1, TaskStatus::Backlog);
    task.description = "First line of description.\nSecond line.\nThird line.".to_string();
    task.repo_path = "/repo/my-project".to_string();
    app.board.tasks.push(task);
    app.update(Message::OpenTaskDetail(1));
    if let ViewMode::TaskDetail { ref mut zoomed, .. } = app.board.view_mode {
        *zoomed = true;
    }
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_task_detail_overlay_empty_optional_fields() {
    use crate::tui::Message;
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let mut task = make_task(1, TaskStatus::Backlog);
    task.description = "Just a description.".to_string();
    task.repo_path = "/repo/path".to_string();
    // pr_url, plan_path, epic_id all None (default from make_task)
    app.board.tasks.push(task);
    app.update(Message::OpenTaskDetail(1));
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_input_project_name_create() {
    use super::super::types::InputMode;
    let mut app = make_app();
    app.input.mode = InputMode::InputProjectName { editing_id: None };
    app.input.buffer = "My Project".to_string();
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_input_project_name_rename() {
    use super::super::types::InputMode;
    let mut app = make_app();
    app.input.mode = InputMode::InputProjectName {
        editing_id: Some(1),
    };
    app.input.buffer = "Renamed Project".to_string();
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_confirm_delete_project1() {
    use super::super::types::InputMode;
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteProject1 { id: 2 };
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_confirm_delete_project2() {
    use super::super::types::InputMode;
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteProject2 {
        id: 2,
        item_count: 3,
    };
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_proposed_learnings_overlay() {
    use crate::models::{Learning, LearningKind, LearningScope, LearningStatus};
    use chrono::Utc;

    let mut app = make_app();

    let now = Utc::now();
    let learnings = vec![
        Learning {
            id: 1,
            kind: LearningKind::Preference,
            summary: "Prefer concise responses over verbose ones".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec!["style".to_string()],
            status: LearningStatus::Proposed,
            source_task_id: None,
            confirmed_count: 0,
            last_confirmed_at: None,
            created_at: now,
            updated_at: now,
        },
        Learning {
            id: 2,
            kind: LearningKind::Pitfall,
            summary: "tokio::spawn needs explicit error logging".to_string(),
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo".to_string()),
            tags: vec!["async".to_string()],
            status: LearningStatus::Proposed,
            source_task_id: None,
            confirmed_count: 0,
            last_confirmed_at: None,
            created_at: now,
            updated_at: now,
        },
        Learning {
            id: 3,
            kind: LearningKind::Convention,
            summary: "Use LearningService not raw db for learnings".to_string(),
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo".to_string()),
            tags: vec![],
            status: LearningStatus::Proposed,
            source_task_id: None,
            confirmed_count: 0,
            last_confirmed_at: None,
            created_at: now,
            updated_at: now,
        },
    ];
    app.update(crate::tui::Message::ShowProposedLearnings(learnings));

    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_proposed_learnings_empty_state() {
    let mut app = make_app();
    app.update(crate::tui::Message::ShowProposedLearnings(vec![]));
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}
