use super::*;

/// The retired startup tips popup used to sit in front of the board and swallow
/// exactly these six keys (`docs/plans/archive/2026-07-31-3809-keybinding-pruning-implementation.md`
/// §2). Nothing may intercept them on a freshly started app again: each one has
/// to reach its board handler on the very first keypress.
#[test]
fn a_freshly_started_app_routes_the_formerly_intercepted_keys_to_the_board() {
    // `q` reaches the board quit path, which asks for confirmation.
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmQuit,
        "'q' must reach the board quit path from a fresh board"
    );

    // `h` / `l` move the column cursor.
    let mut app = make_app();
    let start = app.selected_column();
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_ne!(
        app.selected_column(),
        start,
        "'l' must move the column cursor from a fresh board"
    );
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(
        app.selected_column(),
        start,
        "'h' must move it back from a fresh board"
    );

    // `n` opens the new-task form.
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(
        app.input.mode,
        InputMode::InputTitle,
        "'n' must open the new-task form from a fresh board"
    );

    // `x` reaches the delete-key routing, which confirms before moving anything.
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_ne!(
        app.input.mode,
        InputMode::Normal,
        "'x' must reach the board's delete-key routing from a fresh board"
    );

    // `Esc` clears the selection and leaves the board in Normal mode — it must
    // do that rather than being eaten by an overlay that never opened.
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(
        app.input.mode,
        InputMode::Normal,
        "Esc must leave a fresh board in Normal mode"
    );
}

#[test]
fn refresh_status_never_fetched() {
    let (text, color) = ui::refresh_status(None, false, Duration::from_secs(30));
    assert_eq!(text, "Never fetched  [r] refresh");
    assert_eq!(color, MUTED);
}

#[test]
fn refresh_status_loading() {
    let last = Instant::now() - Duration::from_secs(5);
    let (text, color) = ui::refresh_status(Some(last), true, Duration::from_secs(30));
    assert_eq!(text, "Refreshing...  [r] refresh");
    assert_eq!(color, MUTED);
}

#[test]
fn refresh_status_loading_overrides_never_fetched() {
    let (text, color) = ui::refresh_status(None, true, Duration::from_secs(30));
    assert_eq!(text, "Refreshing...  [r] refresh");
    assert_eq!(color, MUTED);
}

#[test]
fn refresh_status_fresh_seconds() {
    let last = Instant::now() - Duration::from_secs(1);
    let (text, color) = ui::refresh_status(Some(last), false, Duration::from_secs(30));
    assert!(
        text.starts_with("Updated ") && text.contains("s ago") && text.ends_with("  [r] refresh"),
        "expected 'Updated Xs ago  [r] refresh', got: {text}"
    );
    assert_eq!(color, FG);
}

#[test]
fn refresh_status_fresh_just_below_minutes_threshold() {
    // 59s elapsed, 30s interval: 59 < 2*30=60 → still White, seconds format
    let last = Instant::now() - Duration::from_secs(59);
    let (text, color) = ui::refresh_status(Some(last), false, Duration::from_secs(30));
    assert!(text.contains("59s ago"), "expected '59s ago' in: {text}");
    assert_eq!(color, FG);
}

#[test]
fn refresh_status_minutes_format() {
    // 60s elapsed → "1m 0s ago"
    let last = Instant::now() - Duration::from_secs(60);
    let (text, color) = ui::refresh_status(Some(last), false, Duration::from_secs(300));
    assert!(
        text.contains("1m") && text.contains("s ago"),
        "expected minutes format in: {text}"
    );
    assert_eq!(color, FG);
}

#[test]
fn refresh_status_yellow_at_2x_interval() {
    let interval = Duration::from_secs(30);
    // exactly 2× interval → Yellow
    let last = Instant::now() - interval * 2;
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, YELLOW);
}

#[test]
fn refresh_status_white_just_below_2x_interval() {
    let interval = Duration::from_secs(30);
    // 1ms under 2× interval → still White
    let last = Instant::now() - (interval * 2 - Duration::from_millis(100));
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, FG);
}

#[test]
fn refresh_status_yellow_just_below_4x_interval() {
    let interval = Duration::from_secs(30);
    // 1ms under 4× interval → still Yellow
    let last = Instant::now() - (interval * 4 - Duration::from_millis(100));
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, YELLOW);
}

#[test]
fn refresh_status_red_at_4x_interval() {
    let interval = Duration::from_secs(30);
    // exactly 4× interval → Red
    let last = Instant::now() - interval * 4;
    let (_, color) = ui::refresh_status(Some(last), false, interval);
    assert_eq!(color, RED);
}
