#![allow(unused_imports)]

use super::*;
use crate::models::{TaskId, TaskStatus};
use crate::tui::{Message, ViewMode};
use crossterm::event::KeyCode;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_app_with_task() -> App {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
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
    app.update(Message::OpenTaskDetail(1));
    assert!(
        matches!(&app.board.view_mode, ViewMode::TaskDetail { task_id, scroll, zoomed, .. }
            if *task_id == 1 && *scroll == 0 && !zoomed)
    );
}

#[test]
fn open_task_detail_stores_previous_board_mode() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::TaskDetail { previous, .. } if matches!(previous.as_ref(), ViewMode::Board(_))
    ));
}

#[test]
fn close_task_detail_restores_board_mode() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.update(Message::CloseTaskDetail);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn open_task_detail_from_epic_stores_epic_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.view_mode = ViewMode::Epic {
        epic_id: crate::models::EpicId(42),
        selection: crate::tui::BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(crate::tui::BoardSelection::new_for_board())),
    };
    app.update(Message::OpenTaskDetail(99));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::TaskDetail { previous, .. } if matches!(previous.as_ref(), ViewMode::Epic { .. })
    ));
}

#[test]
fn close_task_detail_from_epic_restores_epic_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.view_mode = ViewMode::Epic {
        epic_id: crate::models::EpicId(42),
        selection: crate::tui::BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(crate::tui::BoardSelection::new_for_board())),
    };
    app.update(Message::OpenTaskDetail(99));
    app.update(Message::CloseTaskDetail);
    assert!(matches!(&app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id.0 == 42));
}

// ── scroll ───────────────────────────────────────────────────────────────────

#[test]
fn j_key_increments_scroll_in_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    if let ViewMode::TaskDetail { ref mut max_scroll, .. } = app.board.view_mode {
        *max_scroll = 10;
    }
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 1));
}

#[test]
fn k_key_at_zero_stays_at_zero() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 0));
}

#[test]
fn j_key_clamped_at_max_scroll() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    if let ViewMode::TaskDetail { ref mut scroll, ref mut max_scroll, .. } = app.board.view_mode {
        *scroll = 3;
        *max_scroll = 3;
    }
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 3));
}

#[test]
fn down_key_increments_scroll() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    if let ViewMode::TaskDetail { ref mut max_scroll, .. } = app.board.view_mode {
        *max_scroll = 10;
    }
    app.handle_key(make_key(KeyCode::Down));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 1));
}

// ── zoom ─────────────────────────────────────────────────────────────────────

#[test]
fn z_key_toggles_zoomed_on() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { zoomed, .. } if *zoomed));
}

#[test]
fn z_key_toggles_zoomed_off() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Char('z')));
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { zoomed, .. } if !zoomed));
}

// ── close keys ───────────────────────────────────────────────────────────────

#[test]
fn q_closes_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn esc_closes_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn enter_closes_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

// ── key inertness ────────────────────────────────────────────────────────────

#[test]
fn n_key_is_inert_in_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
    assert!(matches!(app.input.mode, crate::tui::InputMode::Normal));
}

#[test]
fn d_key_is_inert_in_task_detail() {
    let mut app = make_app_with_task();
    app.update(Message::OpenTaskDetail(1));
    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
}

// ── empty description ────────────────────────────────────────────────────────

#[test]
fn j_key_inert_when_no_description_and_max_scroll_zero() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.tasks.push(make_task(1, TaskStatus::Backlog)); // empty description
    app.update(Message::OpenTaskDetail(1));
    // max_scroll defaults to 0
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(&app.board.view_mode, ViewMode::TaskDetail { scroll, .. } if *scroll == 0));
}
