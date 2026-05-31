#![allow(clippy::unwrap_used)]
use super::*;
use crate::models::{UsageActor, UsageCategory};
use crossterm::event::KeyCode;

fn has_usage_event(cmds: &[Command], action: &str, detail: &str) -> bool {
    cmds.iter().any(|c| {
        matches!(c, Command::RecordUsageEvent(e)
            if e.category == UsageCategory::Keybinding
            && e.action == action
            && e.detail.as_deref() == Some(detail)
            && e.actor == UsageActor::Human)
    })
}

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

// ── Tips overlay key usage events ──────────────────────────────────────────

#[test]
fn tips_l_key_emits_browse_tips_next_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Char('l')));
    assert!(
        has_usage_event(&cmds, "browse_tips_next", "l"),
        "expected RecordUsageEvent(browse_tips_next, l) for 'l'"
    );
}

#[test]
fn tips_right_arrow_emits_browse_tips_next_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Right));
    assert!(
        has_usage_event(&cmds, "browse_tips_next", "Right"),
        "expected RecordUsageEvent(browse_tips_next, Right) for Right arrow"
    );
}

#[test]
fn tips_h_key_emits_browse_tips_prev_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Char('h')));
    assert!(
        has_usage_event(&cmds, "browse_tips_prev", "h"),
        "expected RecordUsageEvent(browse_tips_prev, h) for 'h'"
    );
}

#[test]
fn tips_left_arrow_emits_browse_tips_prev_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Left));
    assert!(
        has_usage_event(&cmds, "browse_tips_prev", "Left"),
        "expected RecordUsageEvent(browse_tips_prev, Left) for Left arrow"
    );
}

#[test]
fn tips_n_key_emits_set_tips_mode_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(
        has_usage_event(&cmds, "set_tips_mode", "n"),
        "expected RecordUsageEvent(set_tips_mode, n) for 'n'"
    );
}

#[test]
fn tips_x_key_emits_disable_tips_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(
        has_usage_event(&cmds, "disable_tips", "x"),
        "expected RecordUsageEvent(disable_tips, x) for 'x'"
    );
}

#[test]
fn tips_q_key_emits_close_tips_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Char('q')));
    assert!(
        has_usage_event(&cmds, "close_tips", "q"),
        "expected RecordUsageEvent(close_tips, q) for 'q'"
    );
}

#[test]
fn tips_esc_key_emits_close_tips_event() {
    let mut app = app_with_tips();
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(
        has_usage_event(&cmds, "close_tips", "Esc"),
        "expected RecordUsageEvent(close_tips, Esc) for Esc"
    );
}
