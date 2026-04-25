use ratatui::buffer::Buffer;

use super::super::App;
use super::{
    make_app, make_key, make_review_board_app, make_security_board_app, render_to_buffer,
    TEST_TIMEOUT,
};
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
fn snapshot_review_board_reviewer_mode() {
    let mut app = make_review_board_app();
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_review_board_with_running_agent() {
    let mut app = make_review_board_app();
    app.review.review_agents.insert(
        crate::models::PrRef::new("acme/app".to_string(), 1),
        super::super::types::ReviewAgentHandle {
            tmux_window: "review:pr-1".to_string(),
            worktree: "/repo/.worktrees/review-1".to_string(),
            status: crate::models::ReviewAgentStatus::Reviewing,
        },
    );
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_security_board() {
    let mut app = make_security_board_app();
    let rendered = render_to_string(&mut app, 120, 40);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_security_board_severity_order() {
    use super::super::types::Message;
    use crate::tui::types::SecurityBoardMode;
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    app.update(Message::SwitchSecurityBoardMode(SecurityBoardMode::Alerts));
    // Loaded in reverse severity order — Critical should render at the top.
    app.update(Message::SecurityAlertsLoaded(vec![
        super::make_security_alert(1, "org/alpha", crate::models::AlertSeverity::Low),
        super::make_security_alert(2, "org/beta", crate::models::AlertSeverity::Medium),
        super::make_security_alert(3, "org/gamma", crate::models::AlertSeverity::High),
        super::make_security_alert(4, "org/delta", crate::models::AlertSeverity::Critical),
    ]));
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
    }
}

#[test]
fn snapshot_tab_bar_with_feed_epics_board_active() {
    let mut app = App::new(vec![], super::TEST_TIMEOUT);
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
    let mut app = App::new(vec![], super::TEST_TIMEOUT);
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
