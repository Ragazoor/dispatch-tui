#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::models::{Todo, TodoId};
use crate::tui::messages::TodoMessage;
use crate::tui::types::{Command, Message, ViewMode};
use crate::tui::App;
use chrono::Utc;

fn make_app() -> App {
    App::new(vec![])
}

fn make_todo(id: i64, title: &str, done: bool, sort_order: i64) -> Todo {
    Todo {
        id: TodoId(id),
        title: title.into(),
        done,
        sort_order,
        created_at: Utc::now(),
    }
}

#[test]
fn open_returns_load_command() {
    let mut app = make_app();
    let cmds = app.update(Message::Todo(TodoMessage::Open));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Todo(crate::tui::commands::TodoCommand::Load)
    )));
}

#[test]
fn show_sets_view_mode_with_done_items_last() {
    let mut app = make_app();
    let todos = vec![
        make_todo(1, "open-a", false, 0),
        make_todo(2, "done-b", true, 1),
        make_todo(3, "open-c", false, 2),
    ];
    app.update(Message::Todo(TodoMessage::Show(todos)));
    match &app.board.view_mode {
        ViewMode::Todos { todos, selected, .. } => {
            assert_eq!(*selected, 0);
            assert!(!todos[0].done);
            assert!(!todos[1].done);
            assert!(todos[2].done); // done sorted last
        }
        other => panic!("expected Todos view, got {other:?}"),
    }
}

#[test]
fn q_restores_previous_view() {
    let mut app = make_app();
    app.update(Message::Todo(TodoMessage::Show(vec![make_todo(1, "x", false, 0)])));
    app.update(Message::Todo(TodoMessage::Close));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn todo_keys_inert_outside_todos_viewmode() {
    // The in-view 'space' toggle must NOT leak to the board: pressing space on
    // the board emits no TodoCommand. (A vacuous "view_mode unchanged" assertion
    // would pass even with zero todo code — this guards the routing instead.)
    use crossterm::event::{KeyCode, KeyEvent};
    let mut app = make_app();
    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char(' ')));
    assert!(
        !cmds.iter().any(|c| matches!(c, Command::Todo(_))),
        "space on the board must not produce a TodoCommand"
    );
}
