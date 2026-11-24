//! The board lens (`board-layout.allium`, "Board Lens"). Its order and the
//! orchestrator view are in `orchestrator_view.rs`.

use super::*;
use crate::tui::types::Lens;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

#[test]
fn shift_a_and_shift_f_no_longer_change_the_lens() {
    let mut app = App::new(vec![]);
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert_eq!(app.filter.lens, Lens::Flat);
}

#[test]
fn tab_cycles_the_lens_inside_an_epic_view_and_the_lens_survives_leaving_it() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
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
