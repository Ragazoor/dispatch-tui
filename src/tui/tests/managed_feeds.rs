//! Tests for the managed-feed config popup (the `C` key).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::tui::messages::{ManagedFeedConfigMessage as Msg, ManagedFeedField};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

#[test]
fn c_key_opens_config_populated_from_settings() {
    let mut app = make_app();
    app.set_managed_feed_settings(crate::tui::ManagedFeedSettings {
        reviews_command: Some("fetch-reviews.sh".to_string()),
        reviews_interval_secs: Some(300),
        cve_command: None,
        cve_interval_secs: None,
    });

    app.handle_key(key('C'));

    assert_eq!(*app.mode(), InputMode::ManagedFeedConfig);
    let state = app.managed_feed_config().expect("popup open");
    assert_eq!(state.reviews_command, "fetch-reviews.sh");
    assert_eq!(state.reviews_interval, "300");
    assert_eq!(state.cve_command, "");
    assert_eq!(state.cve_interval, "");
    assert_eq!(state.field, ManagedFeedField::ReviewsCommand);
}

#[test]
fn input_appends_to_focused_command_field() {
    let mut app = make_app();
    app.update(Message::ManagedFeedConfig(Msg::Open));
    for c in "abc".chars() {
        app.update(Message::ManagedFeedConfig(Msg::Input(c)));
    }
    assert_eq!(app.managed_feed_config().unwrap().reviews_command, "abc");
}

#[test]
fn interval_field_accepts_digits_only() {
    let mut app = make_app();
    app.update(Message::ManagedFeedConfig(Msg::Open));
    // Move to the reviews interval field.
    app.update(Message::ManagedFeedConfig(Msg::MoveField(1)));
    assert_eq!(
        app.managed_feed_config().unwrap().field,
        ManagedFeedField::ReviewsInterval
    );
    for c in "3a0x0".chars() {
        app.update(Message::ManagedFeedConfig(Msg::Input(c)));
    }
    assert_eq!(app.managed_feed_config().unwrap().reviews_interval, "300");
}

#[test]
fn move_field_wraps_through_all_fields() {
    let mut app = make_app();
    app.update(Message::ManagedFeedConfig(Msg::Open));
    let seq = [
        ManagedFeedField::ReviewsInterval,
        ManagedFeedField::CveCommand,
        ManagedFeedField::CveInterval,
        ManagedFeedField::ReviewsCommand, // wraps
    ];
    for expected in seq {
        app.update(Message::ManagedFeedConfig(Msg::MoveField(1)));
        assert_eq!(app.managed_feed_config().unwrap().field, expected);
    }
    // Backwards wraps too.
    app.update(Message::ManagedFeedConfig(Msg::MoveField(-1)));
    assert_eq!(
        app.managed_feed_config().unwrap().field,
        ManagedFeedField::CveInterval
    );
}

#[test]
fn esc_discards_without_persisting() {
    let mut app = make_app();
    app.update(Message::ManagedFeedConfig(Msg::Open));
    app.update(Message::ManagedFeedConfig(Msg::Input('x')));
    let cmds = app.update(Message::ManagedFeedConfig(Msg::Close { save: false }));

    assert_eq!(*app.mode(), InputMode::Normal);
    assert!(app.managed_feed_config().is_none());
    assert!(cmds.is_empty(), "cancel must emit no commands");
}

#[test]
fn save_emits_persist_and_provision() {
    let mut app = make_app();
    app.update(Message::ManagedFeedConfig(Msg::Open));
    for c in "rev.sh".chars() {
        app.update(Message::ManagedFeedConfig(Msg::Input(c)));
    }
    app.update(Message::ManagedFeedConfig(Msg::MoveField(1)));
    for c in "300".chars() {
        app.update(Message::ManagedFeedConfig(Msg::Input(c)));
    }

    let cmds = app.update(Message::ManagedFeedConfig(Msg::Close { save: true }));

    assert_eq!(*app.mode(), InputMode::Normal);
    assert!(app.managed_feed_config().is_none());

    use crate::tui::commands::ManagedFeedCommand;
    let persist = cmds.iter().find_map(|c| match c {
        Command::ManagedFeed(ManagedFeedCommand::PersistConfig {
            reviews_command,
            reviews_interval_secs,
            cve_command,
            cve_interval_secs,
        }) => Some((
            reviews_command.clone(),
            *reviews_interval_secs,
            cve_command.clone(),
            *cve_interval_secs,
        )),
        _ => None,
    });
    assert_eq!(
        persist,
        Some((Some("rev.sh".to_string()), Some(300), None, None))
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::ManagedFeed(ManagedFeedCommand::ProvisionAndRefresh)
        )),
        "save must re-fire provisioning"
    );
    // In-memory snapshot updated so a re-open shows saved values.
    assert_eq!(
        app.managed_feed_settings.reviews_command.as_deref(),
        Some("rev.sh")
    );
}

#[test]
fn save_rejects_nonpositive_interval() {
    let mut app = make_app();
    app.update(Message::ManagedFeedConfig(Msg::Open));
    app.update(Message::ManagedFeedConfig(Msg::MoveField(1))); // reviews interval
    app.update(Message::ManagedFeedConfig(Msg::Input('0')));

    let cmds = app.update(Message::ManagedFeedConfig(Msg::Close { save: true }));

    assert!(cmds.is_empty(), "invalid interval must emit no commands");
    assert_eq!(
        *app.mode(),
        InputMode::ManagedFeedConfig,
        "popup stays open on invalid input"
    );
}

#[test]
fn save_clears_empty_command_to_none() {
    let mut app = make_app();
    app.set_managed_feed_settings(crate::tui::ManagedFeedSettings {
        reviews_command: Some("old.sh".to_string()),
        reviews_interval_secs: Some(120),
        cve_command: None,
        cve_interval_secs: None,
    });
    app.update(Message::ManagedFeedConfig(Msg::Open));
    // Clear the reviews command field entirely.
    for _ in 0.."old.sh".len() {
        app.update(Message::ManagedFeedConfig(Msg::Backspace));
    }
    let cmds = app.update(Message::ManagedFeedConfig(Msg::Close { save: true }));

    use crate::tui::commands::ManagedFeedCommand;
    let reviews_command = cmds.iter().find_map(|c| match c {
        Command::ManagedFeed(ManagedFeedCommand::PersistConfig {
            reviews_command, ..
        }) => Some(reviews_command.clone()),
        _ => None,
    });
    assert_eq!(
        reviews_command,
        Some(None),
        "empty command persists as None"
    );
}
