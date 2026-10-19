use super::*;
use crate::models::{TaskId, TaskStatus};
use crate::tui::{Message, ViewMode};
use crossterm::event::KeyCode;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_app_with_task() -> App {
    let mut app = App::new(vec![]);
    app.board.tasks.push({
        let mut t = make_task(1, TaskStatus::Backlog);
        t.description = "line one\nline two\nline three".to_string();
        t.repo_path = "/repo/path".to_string();
        t
    });
    app.board.view_mode = ViewMode::Board(crate::tui::BoardSelection::new_for_board());
    app.selection_mut().set_column(1);
    app
}

// ── lifecycle ────────────────────────────────────────────────────────────────

#[test]
fn open_task_detail_from_board_transitions_view_mode() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    assert!(
        matches!(&app.board.view_mode, ViewMode::TaskDetail { task_id, scroll, zoomed, .. }
            if *task_id == TaskId(1) && *scroll == 0 && !zoomed)
    );
}

#[test]
fn open_task_detail_stores_previous_board_mode() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::TaskDetail { previous, .. } if matches!(previous.as_ref(), ViewMode::Board(_))
    ));
}

#[test]
fn close_task_detail_restores_board_mode() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::CloseDetail,
    ));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn open_task_detail_from_epic_stores_epic_mode() {
    let mut app = App::new(vec![]);
    app.board.view_mode = ViewMode::Epic {
        epic_id: crate::models::EpicId(42),
        selection: crate::tui::BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(crate::tui::BoardSelection::new_for_board())),
    };
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(99)),
    ));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::TaskDetail { previous, .. } if matches!(previous.as_ref(), ViewMode::Epic { .. })
    ));
}

#[test]
fn close_task_detail_from_epic_restores_epic_mode() {
    let mut app = App::new(vec![]);
    app.board.view_mode = ViewMode::Epic {
        epic_id: crate::models::EpicId(42),
        selection: crate::tui::BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(crate::tui::BoardSelection::new_for_board())),
    };
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(99)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::CloseDetail,
    ));
    assert!(matches!(&app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id.0 == 42));
}

// ── scroll ───────────────────────────────────────────────────────────────────

#[test]
fn j_key_increments_scroll_in_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    if let ViewMode::TaskDetail {
        ref mut max_scroll, ..
    } = app.board.view_mode
    {
        *max_scroll = 10;
    }
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 1));
}

#[test]
fn k_key_at_zero_stays_at_zero() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 0));
}

#[test]
fn j_key_clamped_at_max_scroll() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    if let ViewMode::TaskDetail {
        ref mut scroll,
        ref mut max_scroll,
        ..
    } = app.board.view_mode
    {
        *scroll = 3;
        *max_scroll = 3;
    }
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 3));
}

#[test]
fn down_key_increments_scroll() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    if let ViewMode::TaskDetail {
        ref mut max_scroll, ..
    } = app.board.view_mode
    {
        *max_scroll = 10;
    }
    app.handle_key(make_key(KeyCode::Down));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 1));
}

#[test]
fn k_key_decrements_scroll_from_nonzero() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    if let ViewMode::TaskDetail {
        ref mut scroll,
        ref mut max_scroll,
        ..
    } = app.board.view_mode
    {
        *scroll = 3;
        *max_scroll = 5;
    }
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 2));
}

#[test]
fn up_key_decrements_scroll() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    if let ViewMode::TaskDetail {
        ref mut scroll,
        ref mut max_scroll,
        ..
    } = app.board.view_mode
    {
        *scroll = 2;
        *max_scroll = 5;
    }
    app.handle_key(make_key(KeyCode::Up));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 1));
}

// ── zoom ─────────────────────────────────────────────────────────────────────

#[test]
fn z_key_toggles_zoomed_on() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { zoomed, .. } if *zoomed));
}

#[test]
fn z_key_toggles_zoomed_off() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Char('z')));
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { zoomed, .. } if !zoomed));
}

// ── close keys ───────────────────────────────────────────────────────────────

#[test]
fn q_closes_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn esc_closes_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn enter_closes_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

// ── key inertness ────────────────────────────────────────────────────────────

#[test]
fn n_key_is_inert_in_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
    assert!(matches!(app.input.mode, crate::tui::InputMode::Normal));
}

#[test]
fn d_key_is_inert_in_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
}

// ── empty description ────────────────────────────────────────────────────────

#[test]
fn j_key_inert_when_no_description_and_max_scroll_zero() {
    let mut app = App::new(vec![]);
    app.board.tasks.push(make_task(1, TaskStatus::Backlog)); // empty description
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    // max_scroll defaults to 0
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 0));
}

// ── scroll clamping on render ────────────────────────────────────────────────

#[test]
fn zoom_after_scrolling_to_end_keeps_last_line_visible() {
    let mut app = make_app_with_task();
    app.board.tasks[0].description = (1..=40)
        .map(|i| format!("row-{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    // Small terminal: render, then scroll to the end.
    render_to_buffer(&mut app, 100, 30);
    if let ViewMode::TaskDetail {
        scroll, max_scroll, ..
    } = &mut app.board.view_mode
    {
        *scroll = *max_scroll;
    }
    // Zoom grows the overlay, so the range shrinks under the stored scroll.
    app.handle_key(make_key(KeyCode::Char('z')));
    let buf = render_to_buffer(&mut app, 100, 30);
    let text = (0..30)
        .map(|y| buffer_line(&buf, y))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("row-40"),
        "last line must stay visible:\n{text}"
    );
    if let ViewMode::TaskDetail {
        scroll, max_scroll, ..
    } = &app.board.view_mode
    {
        assert!(
            scroll <= max_scroll,
            "scroll {scroll} past max {max_scroll}"
        );
    }
}

#[test]
fn last_line_sits_on_bottom_body_row_at_max_scroll() {
    let mut app = make_app_with_task();
    app.board.tasks[0].description = (1..=40)
        .map(|i| format!("row-{i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    render_to_buffer(&mut app, 100, 30);
    if let ViewMode::TaskDetail {
        scroll, max_scroll, ..
    } = &mut app.board.view_mode
    {
        *scroll = *max_scroll;
    }
    let buf = render_to_buffer(&mut app, 100, 30);
    let lines: Vec<String> = (0..30).map(|y| buffer_line(&buf, y)).collect();
    let last = lines
        .iter()
        .position(|l| l.contains("row-40"))
        .expect("last line visible");
    let border = lines
        .iter()
        .position(|l| l.contains("j/k scroll"))
        .expect("bottom border with hints");
    assert_eq!(
        last + 1,
        border,
        "last line must sit directly above the bottom border:\n{}",
        lines.join("\n")
    );
}

#[test]
fn short_terminal_renders_task_detail_without_panic() {
    let mut app = make_app_with_task();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    for h in 0..8 {
        render_to_buffer(&mut app, 40, h);
    }
}

// ── title (board-visuals.allium: CardLeadsWithTheAgentName) ─────────────────

fn detail_title_row(app: &mut App) -> String {
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    let buf = render_to_buffer(app, 100, 30);
    (0..30)
        .map(|y| buffer_line(&buf, y))
        .find(|l| l.contains("Task #1"))
        .expect("the popup's title row")
}

#[test]
fn task_detail_title_names_the_agent_and_the_id() {
    let mut app = make_app_with_task();
    app.board.tasks[0].agent_name = Some("paivi".into());

    let row = detail_title_row(&mut app);

    assert!(row.contains(" paivi · Task #1 "), "{row:?}");
}

#[test]
fn task_detail_title_without_an_agent_names_only_the_id() {
    let mut app = make_app_with_task();

    let row = detail_title_row(&mut app);

    assert!(row.contains(" Task #1 "), "{row:?}");
    assert!(!row.contains('·'), "{row:?}");
}
