//! Jumping through nested epics: Enter on a flattened task card goes to its
//! epic, Enter on an epic card goes to the deepest epic holding its work, `q`
//! leaves one level and `Q` leaves them all.
//!
//! See "Epic Navigation" in `docs/specs/epics.allium` and
//! `core/Epic.deepest_epic_with` in `docs/specs/core.allium`.
use super::*;
use crate::models::{EpicId, TaskStatus};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const RUNNING_COL: usize = 2;

fn press(app: &mut App, c: char) {
    let mods = if c.is_ascii_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    app.handle_key(KeyEvent::new(KeyCode::Char(c), mods));
}

fn enter(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

/// Epic 10 > 20 > 30, plus any extra epics the test adds by `(id, parent)`.
fn tree(extra: &[(i64, i64)]) -> Vec<crate::models::Epic> {
    let mut epics = vec![make_epic(10)];
    for &(id, parent) in &[(20, 10), (30, 20)]
        .iter()
        .chain(extra)
        .copied()
        .collect::<Vec<_>>()
    {
        let mut e = make_epic(id);
        e.parent_epic_id = Some(EpicId(parent));
        epics.push(e);
    }
    epics
}

fn running_in(id: i64, epic: Option<i64>) -> Task {
    let mut t = make_task(id, TaskStatus::Running);
    t.epic_id = epic.map(EpicId);
    t
}

/// The epic views showing, innermost first; empty on the board.
fn trail(app: &App) -> Vec<EpicId> {
    let mut out = vec![];
    let mut view = &app.board.view_mode;
    while let ViewMode::Epic {
        epic_id, parent, ..
    } = view
    {
        out.push(*epic_id);
        view = parent;
    }
    out
}

fn on_running_card(app: &mut App) {
    app.selection_mut().set_column(RUNNING_COL);
    app.selection_mut().set_row(RUNNING_COL, 0);
}

#[test]
fn enter_on_a_flattened_task_jumps_to_its_epic_through_every_ancestor() {
    let mut app = App::new(vec![running_in(1, Some(30))]);
    app.board.epics = tree(&[]);
    app.board.flattened = true;
    on_running_card(&mut app);

    enter(&mut app);

    assert_eq!(trail(&app), vec![EpicId(30), EpicId(20), EpicId(10)]);
}

#[test]
fn q_after_a_jump_climbs_one_epic_and_shift_q_returns_to_the_board() {
    let mut app = App::new(vec![running_in(1, Some(30))]);
    app.board.epics = tree(&[]);
    app.board.flattened = true;
    on_running_card(&mut app);
    enter(&mut app);

    press(&mut app, 'q');
    assert_eq!(trail(&app), vec![EpicId(20), EpicId(10)]);

    press(&mut app, 'Q');
    assert!(trail(&app).is_empty());
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn shift_q_on_the_board_does_nothing() {
    let mut app = App::new(vec![]);
    press(&mut app, 'Q');
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn enter_on_a_flattened_epicless_task_does_nothing() {
    let mut app = App::new(vec![running_in(1, None)]);
    app.board.flattened = true;
    on_running_card(&mut app);

    enter(&mut app);

    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn flattened_jump_inside_an_epic_view_starts_below_that_epic() {
    let mut app = App::new(vec![running_in(1, Some(30))]);
    app.board.epics = tree(&[]);
    app.board.flattened = true;
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(20),
    )));
    on_running_card(&mut app);

    enter(&mut app);

    assert_eq!(trail(&app), vec![EpicId(30), EpicId(20)]);
}

#[test]
fn enter_on_an_unflattened_task_still_opens_detail() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    enter(&mut app);

    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
}

#[test]
fn i_opens_detail_on_a_flattened_task() {
    let mut app = App::new(vec![running_in(1, Some(30))]);
    app.board.epics = tree(&[]);
    app.board.flattened = true;
    on_running_card(&mut app);

    press(&mut app, 'i');

    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
}

#[test]
fn enter_on_an_epic_card_jumps_to_the_deepest_epic_holding_the_work() {
    let mut app = App::new(vec![running_in(1, Some(30))]);
    app.board.epics = tree(&[]);
    on_running_card(&mut app);

    enter(&mut app);

    assert_eq!(trail(&app), vec![EpicId(30), EpicId(20), EpicId(10)]);
}

#[test]
fn the_jump_stops_where_two_sub_epics_hold_the_work() {
    let mut app = App::new(vec![running_in(1, Some(30)), running_in(2, Some(40))]);
    app.board.epics = tree(&[(40, 10)]);
    on_running_card(&mut app);

    enter(&mut app);

    assert_eq!(trail(&app), vec![EpicId(10)]);
}

#[test]
fn the_jump_stops_at_an_epic_with_a_direct_task_in_the_column() {
    let mut app = App::new(vec![running_in(1, Some(30)), running_in(2, Some(10))]);
    app.board.epics = tree(&[]);
    on_running_card(&mut app);

    enter(&mut app);

    assert_eq!(trail(&app), vec![EpicId(10)]);
}

#[test]
fn space_on_an_epic_card_still_enters_one_level() {
    let mut app = App::new(vec![running_in(1, Some(30))]);
    app.board.epics = tree(&[]);
    on_running_card(&mut app);

    press(&mut app, ' ');

    assert_eq!(trail(&app), vec![EpicId(10)]);
}
