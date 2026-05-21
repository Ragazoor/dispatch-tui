#![allow(clippy::unwrap_used)]
use super::*;
use crate::models::{UsageActor, UsageCategory};
use crossterm::event::KeyCode;

#[test]
fn pressing_n_emits_record_usage_event() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));

    let found = cmds.iter().any(|c| {
        matches!(c, Command::RecordUsageEvent(e)
            if e.category == UsageCategory::Keybinding
            && e.action == "create_task"
            && e.detail.as_deref() == Some("n")
            && e.actor == UsageActor::Human)
    });
    assert!(found, "expected RecordUsageEvent(create_task) for 'n'");
}

#[test]
fn navigation_keys_do_not_emit_record_usage_event() {
    let mut app = make_app();
    for code in [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Down,
        KeyCode::Up,
    ] {
        let cmds = app.handle_key(make_key(code));
        let has_usage = cmds
            .iter()
            .any(|c| matches!(c, Command::RecordUsageEvent(_)));
        assert!(!has_usage, "{code:?} should not emit RecordUsageEvent");
    }
}
