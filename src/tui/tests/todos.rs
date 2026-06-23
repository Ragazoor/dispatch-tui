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

fn show(app: &mut App, todos: Vec<Todo>) {
    app.update(Message::Todo(TodoMessage::Show(todos)));
}

#[test]
fn open_returns_load_command() {
    let mut app = make_app();
    let cmds = app.update(Message::Todo(TodoMessage::Open));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Todo(crate::tui::commands::TodoCommand::Load))));
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
        ViewMode::Todos {
            todos, selected, ..
        } => {
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
    app.update(Message::Todo(TodoMessage::Show(vec![make_todo(
        1, "x", false, 0,
    )])));
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

#[test]
fn space_toggles_done_on_selected_and_emits_update() {
    use crate::tui::commands::TodoCommand;
    let mut app = make_app();
    show(&mut app, vec![make_todo(1, "x", false, 0)]);
    let cmds = app.update(Message::Todo(TodoMessage::ToggleDone(TodoId(1))));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Todo(TodoCommand::Update { id, update })
            if *id == TodoId(1) && update.done == Some(true)
    )));
    if let ViewMode::Todos { todos, .. } = &app.board.view_mode {
        assert!(todos[0].done);
    } else {
        panic!("expected Todos view");
    }
}

#[test]
fn shift_jk_reorders_within_list_two_updates() {
    use crate::tui::commands::TodoCommand;
    let mut app = make_app();
    show(
        &mut app,
        vec![make_todo(1, "a", false, 0), make_todo(2, "b", false, 1)],
    );
    // selected = 0 (item a). Move down: swap with b.
    let cmds = app.update(Message::Todo(TodoMessage::Reorder(1)));
    let updates: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::Todo(TodoCommand::Update { .. })))
        .collect();
    assert_eq!(updates.len(), 2);
    if let ViewMode::Todos {
        todos, selected, ..
    } = &app.board.view_mode
    {
        assert_eq!(todos[0].id, TodoId(2));
        assert_eq!(todos[1].id, TodoId(1));
        assert_eq!(*selected, 1); // selection follows the moved item
    } else {
        panic!("expected Todos view");
    }
}

#[test]
fn clear_done_drops_done_and_emits_command() {
    use crate::tui::commands::TodoCommand;
    let mut app = make_app();
    show(
        &mut app,
        vec![
            make_todo(1, "keep", false, 0),
            make_todo(2, "gone", true, 1),
        ],
    );
    let cmds = app.update(Message::Todo(TodoMessage::ClearDone));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Todo(TodoCommand::ClearDone))));
    if let ViewMode::Todos { todos, .. } = &app.board.view_mode {
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, TodoId(1));
    } else {
        panic!("expected Todos view");
    }
}

#[test]
fn delete_drops_selected_and_emits_command() {
    use crate::tui::commands::TodoCommand;
    let mut app = make_app();
    show(&mut app, vec![make_todo(1, "x", false, 0)]);
    let cmds = app.update(Message::Todo(TodoMessage::Delete(TodoId(1))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Todo(TodoCommand::Delete(id)) if *id == TodoId(1))));
    if let ViewMode::Todos { todos, .. } = &app.board.view_mode {
        assert!(todos.is_empty());
    }
}

#[test]
fn edit_prefills_buffer_from_selected_item() {
    let mut app = make_app();
    show(&mut app, vec![make_todo(1, "edit me", false, 0)]);
    app.update(Message::Todo(TodoMessage::Edit(TodoId(1))));
    assert_eq!(app.input.buffer, "edit me");
    assert!(matches!(
        app.input.mode,
        crate::tui::types::InputMode::TodoTitle
    ));
}

#[test]
fn add_opens_input_mode_todo_title() {
    let mut app = make_app();
    show(&mut app, vec![make_todo(1, "existing", false, 0)]);
    app.update(Message::Todo(TodoMessage::Add));
    assert_eq!(app.input.buffer, "");
    assert!(matches!(
        app.input.mode,
        crate::tui::types::InputMode::TodoTitle
    ));
    assert!(app.pending_todo_edit.is_none());
}

#[test]
fn d_routes_through_confirm_delete() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = make_app();
    show(&mut app, vec![make_todo(1, "delete me", false, 0)]);
    // Press 'd' — should set ConfirmDeleteTodo mode and store pending id.
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(matches!(
        app.input.mode,
        crate::tui::types::InputMode::ConfirmDeleteTodo
    ));
    assert_eq!(app.pending_todo_delete, Some(TodoId(1)));
}

#[test]
fn t_on_board_enters_quick_add_mode() {
    use crossterm::event::{KeyCode, KeyEvent};
    let mut app = make_app();
    let _ = app.handle_key(KeyEvent::from(KeyCode::Char('t')));
    assert!(matches!(
        app.input.mode,
        crate::tui::types::InputMode::TodoQuickAdd
    ));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_))); // stays on board
}

#[test]
fn count_updated_sets_board_count() {
    let mut app = make_app();
    app.update(Message::Todo(TodoMessage::CountUpdated(3)));
    assert_eq!(app.board.todo_open_count, 3);
}

#[test]
fn todo_add_mode_shows_buffer_in_overlay() {
    // When the user presses 'a' inside the todos overlay and types text,
    // the typed text must be visible somewhere in the rendered output.
    let mut app = App::new(vec![]);
    app.update(Message::Todo(TodoMessage::Show(vec![make_todo(
        1, "existing", false, 0,
    )])));
    app.update(Message::Todo(TodoMessage::Add));
    app.input.buffer = "hello world".to_string();
    let buf = super::render_to_buffer(&mut app, 120, 40);
    assert!(
        super::buffer_contains(&buf, "hello world"),
        "typed text should be visible in the todos overlay"
    );
}

#[test]
fn todo_title_mode_status_bar_shows_buffer() {
    // The status bar must include the buffer contents so the user can
    // see what they are typing even before the overlay input row renders.
    let mut app = App::new(vec![]);
    app.update(Message::Todo(TodoMessage::Show(vec![])));
    app.update(Message::Todo(TodoMessage::Add));
    app.input.buffer = "typing here".to_string();
    let buf = super::render_to_buffer(&mut app, 120, 40);
    assert!(
        super::buffer_contains(&buf, "typing here"),
        "buffer content should appear in the status bar"
    );
}

#[test]
fn todo_quick_add_mode_status_bar_shows_buffer() {
    // TodoQuickAdd (board-level, no overlay) must show the buffer inline
    // in the status bar so the user sees what they are typing.
    let mut app = App::new(vec![]);
    app.input.mode = crate::tui::types::InputMode::TodoQuickAdd;
    app.input.buffer = "new item".to_string();
    let buf = super::render_to_buffer(&mut app, 120, 40);
    assert!(
        super::buffer_contains(&buf, "new item"),
        "buffer content should appear in the status bar for quick-add mode"
    );
}

#[test]
#[test]
fn submit_title_while_todos_open_preserves_board_as_previous() {
    // Regression: when Enter is pressed in the add-todo input (TodoTitle mode)
    // while the Todos overlay is open, exec_load_todos calls handle_show_todos
    // again. handle_show_todos must preserve the pre-Todos `previous` view so
    // effective_view_mode() keeps returning Board, not Todos — otherwise the
    // unreachable!() guards in tasks_for_current_view et al. panic.
    let mut app = make_app();
    show(
        &mut app,
        vec![make_todo(1, "existing", false, 0)],
    );
    assert!(matches!(app.board.view_mode, ViewMode::Todos { .. }));

    // Simulate Enter submitting the add form: SubmitTitle calls handle_show_todos
    // a second time (as exec_create_todo with reopen=true would do).
    app.update(Message::Todo(TodoMessage::Show(vec![
        make_todo(1, "existing", false, 0),
        make_todo(2, "new item", false, 1),
    ])));

    // The view must still be Todos, but previous must be Board (not Todos).
    match &app.board.view_mode {
        ViewMode::Todos { previous, .. } => {
            assert!(
                matches!(previous.as_ref(), ViewMode::Board(_)),
                "previous should be Board after re-Show, got {previous:?}"
            );
        }
        other => panic!("expected Todos view, got {other:?}"),
    }

    // effective_view_mode must return Board so callers don't hit unreachable!()
    assert!(
        matches!(app.effective_view_mode(), ViewMode::Board(_)),
        "effective_view_mode should return Board"
    );
}

#[test]
fn status_bar_shows_count_suffix_only_when_nonzero() {
    // When todo_open_count == 0, no "(N)" count suffix appears in the status bar.
    // When todo_open_count == 2, "(2)" appears in the status bar.
    let mut app = make_app();
    app.board.todo_open_count = 0;
    let buf = super::render_to_buffer(&mut app, 160, 20);
    assert!(
        !super::buffer_contains(&buf, "("),
        "status bar should not show a count suffix when count is 0"
    );

    app.board.todo_open_count = 2;
    let buf = super::render_to_buffer(&mut app, 160, 20);
    assert!(
        super::buffer_contains(&buf, "(2)"),
        "status bar should show '(2)' when todo_open_count is 2"
    );
}
