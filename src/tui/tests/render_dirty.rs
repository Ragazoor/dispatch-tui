//! Tests for dirty-flag correctness: handle_key must set dirty=true only when
//! visible state actually changes.  No-op navigation (cursor at boundary) must
//! leave dirty=false so the render loop skips redundant frames.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crossterm::event::KeyCode;

// ---------------------------------------------------------------------------
// Dirty signal: no-op navigation
// ---------------------------------------------------------------------------

#[test]
fn noop_nav_at_row_boundary_leaves_dirty_false() {
    let mut app = make_app(); // 2 tasks in Backlog (col 1)
    // Move to last row (row index 1 = second task).
    app.update(Message::NavigateRow(1));
    app.dirty = false;

    // j at the last row is a no-op — cursor doesn't move.
    app.handle_key(make_key(KeyCode::Char('j')));

    assert!(
        !app.dirty,
        "pressing j at the last row must not set dirty; got dirty=true"
    );
}

#[test]
fn noop_nav_at_col_boundary_leaves_dirty_false() {
    let mut app = make_app(); // starts in Backlog (leftmost task column)
    app.dirty = false;

    // h at the leftmost column stays in the same column.
    app.handle_key(make_key(KeyCode::Char('h')));

    assert!(
        !app.dirty,
        "pressing h at the leftmost column must not set dirty; got dirty=true"
    );
}

// ---------------------------------------------------------------------------
// Dirty signal: navigation that actually moves the cursor
// ---------------------------------------------------------------------------

#[test]
fn nav_that_moves_row_sets_dirty() {
    let mut app = make_app(); // 2 tasks in Backlog, cursor at row 0
    app.dirty = false;

    // j moves from row 0 to row 1 — a real state change.
    app.handle_key(make_key(KeyCode::Char('j')));

    assert!(
        app.dirty,
        "pressing j when cursor can move must set dirty; got dirty=false"
    );
}

#[test]
fn nav_that_moves_column_sets_dirty() {
    let mut app = make_app(); // starts in Backlog (col 1)
    app.dirty = false;

    // l moves to Running (col 2) — a real state change.
    app.handle_key(make_key(KeyCode::Char('l')));

    assert!(
        app.dirty,
        "pressing l when cursor can move right must set dirty; got dirty=false"
    );
}

// ---------------------------------------------------------------------------
// Dirty signal: non-navigation keys always set dirty
// ---------------------------------------------------------------------------

#[test]
fn non_nav_key_sets_dirty() {
    let mut app = make_app();
    app.dirty = false;

    // 'n' opens the new-task input — always a state change.
    app.handle_key(make_key(KeyCode::Char('n')));

    assert!(
        app.dirty,
        "pressing 'n' (open new task) must set dirty; got dirty=false"
    );
}

#[test]
fn noop_nav_via_down_arrow_leaves_dirty_false() {
    let mut app = make_app();
    app.update(Message::NavigateRow(1)); // move to last row
    app.dirty = false;

    app.handle_key(make_key(KeyCode::Down));

    assert!(
        !app.dirty,
        "Down arrow at last row must not set dirty; got dirty=true"
    );
}
