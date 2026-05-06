use crossterm::event::KeyCode;

use super::*;

fn make_app() -> App {
    App::new(vec![], crate::models::ProjectId(1), TEST_TIMEOUT)
}

// ── keybinding ──

#[test]
fn colon_without_dir_enters_main_session_dir_mode() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char(':')));
    assert!(cmds.is_empty());
    assert_eq!(app.mode(), &InputMode::MainSessionDir);
}

#[test]
fn colon_with_dir_configured_emits_open_main_session() {
    let mut app = make_app();
    app.set_main_session_dir(Some("/home/user".to_string()));
    let cmds = app.handle_key(make_key(KeyCode::Char(':')));
    assert!(cmds.iter().any(|c| matches!(c, Command::OpenMainSession)));
}

#[test]
fn colon_with_dir_and_active_session_emits_open_main_session() {
    let mut app = make_app();
    app.set_main_session_dir(Some("/home/user".to_string()));
    app.set_main_session(Some("dispatch-main".to_string()));
    let cmds = app.handle_key(make_key(KeyCode::Char(':')));
    assert!(cmds.iter().any(|c| matches!(c, Command::OpenMainSession)));
}

// ── text input in MainSessionDir mode ──

#[test]
fn typing_in_main_session_dir_mode_accumulates_in_buffer() {
    let mut app = make_app();
    app.input.mode = InputMode::MainSessionDir;
    app.handle_key(make_key(KeyCode::Char('/')));
    app.handle_key(make_key(KeyCode::Char('h')));
    app.handle_key(make_key(KeyCode::Char('o')));
    assert_eq!(app.input_buffer(), "/ho");
}

#[test]
fn enter_in_main_session_dir_mode_emits_submit_message() {
    let mut app = make_app();
    app.input.mode = InputMode::MainSessionDir;
    app.input.buffer = "/home/user".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds.iter().any(
        |c| matches!(c, Command::PersistStringSetting { key, .. } if key == "main_session.dir")
    ));
    assert!(cmds.iter().any(|c| matches!(c, Command::OpenMainSession)));
}

#[test]
fn enter_with_empty_buffer_cancels_main_session_dir() {
    let mut app = make_app();
    app.input.mode = InputMode::MainSessionDir;
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds.is_empty());
    assert_eq!(app.mode(), &InputMode::Normal);
}

#[test]
fn esc_in_main_session_dir_mode_returns_to_normal() {
    let mut app = make_app();
    app.input.mode = InputMode::MainSessionDir;
    app.input.buffer = "/some/path".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.mode(), &InputMode::Normal);
    assert_eq!(app.input_buffer(), "");
}

// ── message handlers ──

#[test]
fn submit_main_session_dir_sets_dir_on_app() {
    let mut app = make_app();
    app.update(Message::SubmitMainSessionDir("/home/user".to_string()));
    assert_eq!(app.main_session_dir(), Some("/home/user"));
}

#[test]
fn submit_main_session_dir_expands_tilde() {
    let mut app = make_app();
    app.update(Message::SubmitMainSessionDir("~/code".to_string()));
    let dir = app.main_session_dir().unwrap();
    assert!(
        !dir.starts_with('~'),
        "tilde should be expanded, got: {dir}"
    );
}

#[test]
fn submit_main_session_dir_returns_persist_and_open_commands() {
    let mut app = make_app();
    let cmds = app.update(Message::SubmitMainSessionDir("/home/user".to_string()));
    assert!(cmds.iter().any(
        |c| matches!(c, Command::PersistStringSetting { key, .. } if key == "main_session.dir")
    ));
    assert!(cmds.iter().any(|c| matches!(c, Command::OpenMainSession)));
}

#[test]
fn submit_main_session_dir_resets_input_mode() {
    let mut app = make_app();
    app.input.mode = InputMode::MainSessionDir;
    app.update(Message::SubmitMainSessionDir("/home/user".to_string()));
    assert_eq!(app.mode(), &InputMode::Normal);
}

#[test]
fn main_session_created_sets_window_on_app() {
    let mut app = make_app();
    app.update(Message::MainSessionCreated("dispatch-main".to_string()));
    assert_eq!(app.main_session(), Some("dispatch-main"));
}

#[test]
fn main_session_created_returns_persist_command() {
    let mut app = make_app();
    let cmds = app.update(Message::MainSessionCreated("dispatch-main".to_string()));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistStringSetting { key, value }
        if key == "main_session.window" && value == "dispatch-main")));
}
