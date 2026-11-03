//! The board's keep-awake toggle (docs/specs/keep-awake.allium).
//!
//! These drive the App's state machine directly: the operating system's
//! answers arrive as `SystemMessage::KeepAwake` messages, so no process is
//! started here. The process side is covered in `src/keep_awake.rs`.

use crossterm::event::KeyCode;

use crate::keep_awake::HoldEvent;
use crate::tui::commands::SystemCommand;
use crate::tui::messages::SystemMessage;
use crate::tui::tests::helpers::{make_app, make_key, render_to_buffer};
use crate::tui::types::{Command, Message};
use crate::tui::update::keep_awake::KeepAwakeStatus;
use crate::tui::App;

fn toggle(app: &mut App) -> Vec<Command> {
    app.update(Message::System(SystemMessage::ToggleKeepAwake))
}

fn answer(app: &mut App, generation: u64, event: HoldEvent) -> Vec<Command> {
    app.update(Message::System(SystemMessage::KeepAwake {
        generation,
        event,
    }))
}

fn requested_generation(cmds: &[Command]) -> Option<u64> {
    cmds.iter().find_map(|c| match c {
        Command::System(SystemCommand::RequestKeepAwake { generation }) => Some(*generation),
        _ => None,
    })
}

fn releases(cmds: &[Command]) -> bool {
    cmds.iter()
        .any(|c| matches!(c, Command::System(SystemCommand::ReleaseKeepAwake)))
}

fn supported_app() -> App {
    let mut app = make_app();
    app.keep_awake.supported = true;
    app
}

/// An app whose keep-awake hold has been requested and granted.
fn app_on() -> (App, u64) {
    let mut app = supported_app();
    let generation = requested_generation(&toggle(&mut app)).expect("a request");
    answer(&mut app, generation, HoldEvent::Granted);
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::On);
    (app, generation)
}

fn top_row(app: &mut App) -> String {
    let buf = render_to_buffer(app, 160, 40);
    crate::tui::tests::helpers::buffer_line(&buf, 0)
}

// KeepAwakeStartsOff
#[test]
fn starts_off_with_no_badge() {
    let mut app = make_app();
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
    assert!(!top_row(&mut app).contains("awake"));
}

// RequestKeepAwake
#[test]
fn toggle_from_off_requests_a_hold() {
    let mut app = supported_app();
    let cmds = toggle(&mut app);
    assert!(requested_generation(&cmds).is_some(), "got {cmds:?}");
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Requesting);
}

#[test]
fn requesting_shows_no_badge() {
    let mut app = supported_app();
    toggle(&mut app);
    assert!(!top_row(&mut app).contains("awake"));
}

// KeepAwakeToggle: KeyIsInTheTable
#[test]
fn capital_w_on_the_board_toggles_keep_awake() {
    let mut app = supported_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('W')));
    assert!(requested_generation(&cmds).is_some(), "got {cmds:?}");
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Requesting);
}

// RefuseKeepAwakeOffLinux
#[test]
fn unsupported_platform_shows_an_error_and_stays_off() {
    let mut app = make_app();
    app.keep_awake.supported = false;
    let cmds = toggle(&mut app);
    assert!(requested_generation(&cmds).is_none(), "got {cmds:?}");
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
    let err = app.status.error_popup.clone().unwrap_or_default();
    assert!(err.contains("only supported on Linux"), "got {err:?}");
}

// IgnoreToggleWhileRequesting
#[test]
fn toggle_while_requesting_does_nothing() {
    let mut app = supported_app();
    toggle(&mut app);
    let cmds = toggle(&mut app);
    assert!(
        cmds.iter().all(|c| !matches!(c, Command::System(_))),
        "got {cmds:?}"
    );
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Requesting);
}

// KeepAwakeGranted + BadgeMeansHeld
#[test]
fn granted_turns_it_on_and_shows_the_badge() {
    let (mut app, _) = app_on();
    assert!(top_row(&mut app).contains("awake [W]"));
}

// KeepAwakeRefused
#[test]
fn refused_turns_it_off_with_an_error() {
    let mut app = supported_app();
    let generation = requested_generation(&toggle(&mut app)).expect("a request");
    answer(
        &mut app,
        generation,
        HoldEvent::Refused("no session bus".into()),
    );
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
    let err = app.status.error_popup.clone().unwrap_or_default();
    assert!(err.contains("Keep-awake failed"), "got {err:?}");
    assert!(err.contains("no session bus"), "got {err:?}");
    assert!(!top_row(&mut app).contains("awake"));
}

#[test]
fn refused_does_not_retry() {
    let mut app = supported_app();
    let generation = requested_generation(&toggle(&mut app)).expect("a request");
    let cmds = answer(&mut app, generation, HoldEvent::Refused("x".into()));
    assert!(requested_generation(&cmds).is_none(), "got {cmds:?}");
}

// KeepAwakeLost
#[test]
fn hold_ending_while_on_turns_it_off_with_an_error() {
    let (mut app, generation) = app_on();
    let cmds = answer(
        &mut app,
        generation,
        HoldEvent::Ended("helper exited".into()),
    );
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
    assert!(requested_generation(&cmds).is_none(), "no retry: {cmds:?}");
    let err = app.status.error_popup.clone().unwrap_or_default();
    assert!(err.contains("Keep-awake stopped"), "got {err:?}");
    assert!(err.contains("helper exited"), "got {err:?}");
    assert!(!top_row(&mut app).contains("awake"));
}

// ReleaseKeepAwake
#[test]
fn toggle_while_on_releases_the_hold() {
    let (mut app, _) = app_on();
    let cmds = toggle(&mut app);
    assert!(releases(&cmds), "got {cmds:?}");
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
    assert!(app.status.error_popup.is_none());
    assert!(!top_row(&mut app).contains("awake"));
}

// AnswersBelongToOneRequest
#[test]
fn the_end_of_a_released_hold_shows_no_error() {
    let (mut app, generation) = app_on();
    toggle(&mut app);
    answer(&mut app, generation, HoldEvent::Ended("released".into()));
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
    assert!(app.status.error_popup.is_none());
}

#[test]
fn an_answer_from_an_earlier_request_changes_nothing() {
    let (mut app, old) = app_on();
    toggle(&mut app); // release
    let new = requested_generation(&toggle(&mut app)).expect("a second request");
    assert_ne!(old, new, "each request gets its own generation");

    answer(&mut app, old, HoldEvent::Ended("released".into()));
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Requesting);
    answer(&mut app, old, HoldEvent::Granted);
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Requesting);
    assert!(app.status.error_popup.is_none());

    answer(&mut app, new, HoldEvent::Granted);
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::On);
}

#[test]
fn an_answer_while_off_changes_nothing() {
    let mut app = supported_app();
    answer(&mut app, 0, HoldEvent::Granted);
    assert_eq!(app.keep_awake.status, KeepAwakeStatus::Off);
}
