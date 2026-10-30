//! The board lens: Tab / BackTab cycle everything → flat → flat_active
//! (`board-layout.allium`, "Board Lens").

use super::*;
use crate::tui::types::Lens;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn back_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
}

#[test]
fn the_board_starts_on_the_everything_lens() {
    let app = App::new(vec![]);
    assert_eq!(app.filter.lens, Lens::Everything);
    assert!(!app.filter.lens.only_active());
    assert!(!app.filter.lens.flattened());
}

#[test]
fn tab_cycles_the_lens_forward_and_wraps() {
    let mut app = App::new(vec![]);
    let mut seen = vec![];
    for _ in 0..3 {
        app.handle_key(tab());
        seen.push(app.filter.lens);
    }
    assert_eq!(seen, vec![Lens::Flat, Lens::FlatActive, Lens::Everything]);
}

#[test]
fn back_tab_cycles_the_lens_backward_and_wraps() {
    let mut app = App::new(vec![]);
    let mut seen = vec![];
    for _ in 0..3 {
        app.handle_key(back_tab());
        seen.push(app.filter.lens);
    }
    assert_eq!(seen, vec![Lens::FlatActive, Lens::Flat, Lens::Everything]);
}

#[test]
fn each_lens_derives_both_switches() {
    let cases = [
        (Lens::Everything, false, false),
        (Lens::Flat, false, true),
        (Lens::FlatActive, true, true),
    ];
    for (lens, only_active, flattened) in cases {
        assert_eq!(lens.only_active(), only_active, "{lens:?}");
        assert_eq!(lens.flattened(), flattened, "{lens:?}");
    }
}

#[test]
fn shift_a_and_shift_f_no_longer_change_the_lens() {
    let mut app = App::new(vec![]);
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert_eq!(app.filter.lens, Lens::Everything);
}

#[test]
fn tab_cycles_the_lens_inside_an_epic_view_and_the_lens_survives_leaving_it() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    app.handle_key(tab());
    app.handle_key(tab());
    assert_eq!(app.filter.lens, Lens::FlatActive);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));
    assert_eq!(app.filter.lens, Lens::FlatActive);
}

#[test]
fn the_flat_active_lens_hides_tasks_without_a_session() {
    let mut idle = make_task(1, TaskStatus::Running);
    idle.tmux_window = None;
    let mut live = make_task(2, TaskStatus::Running);
    live.tmux_window = Some(crate::models::test_tmux_window("task-2"));
    let mut app = App::new(vec![idle, live]);
    cycle_lens_to(&mut app, Lens::FlatActive);
    let ids: Vec<_> = app
        .view()
        .tasks_for_current_view()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, vec![TaskId(2)]);
}
