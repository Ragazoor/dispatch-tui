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

// ---------------------------------------------------------------------------
// Dirty signal: todo popup navigation
// ---------------------------------------------------------------------------

#[test]
fn todo_selection_move_sets_dirty() {
    let mut app = make_app();
    // Open todos with two items so j can actually move.
    app.update(Message::Todo(crate::tui::messages::TodoMessage::Show(vec![
        make_todo(1, "first"),
        make_todo(2, "second"),
    ])));
    app.dirty = false;

    // j moves selection from 0 → 1 — a real state change.
    app.handle_key(make_key(KeyCode::Char('j')));

    assert!(
        app.dirty,
        "pressing j in the todo popup when cursor can move must set dirty; got dirty=false"
    );
}

#[test]
fn todo_selection_at_boundary_leaves_dirty_false() {
    let mut app = make_app();
    // Single item — j is a no-op (already at last row).
    app.update(Message::Todo(crate::tui::messages::TodoMessage::Show(vec![
        make_todo(1, "only"),
    ])));
    app.dirty = false;

    app.handle_key(make_key(KeyCode::Char('j')));

    assert!(
        !app.dirty,
        "pressing j at the last todo row must not set dirty; got dirty=true"
    );
}

// ---------------------------------------------------------------------------
// Dirty signal: nested epic navigation
// ---------------------------------------------------------------------------

#[test]
fn entering_nested_epic_sets_dirty() {
    use crate::models::EpicId;
    use crate::tui::messages::EpicMessage;
    use crate::tui::types::{BoardSelection, ViewMode};

    let mut app = make_app();
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child_epic];
    // Start inside epic 10
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.dirty = false;

    // Enter the nested sub-epic 20 — same ViewMode discriminant, different epic_id
    app.update(Message::Epic(EpicMessage::Enter(EpicId(20))));

    assert!(
        app.dirty,
        "entering a nested epic must set dirty; got dirty=false"
    );
}

#[test]
fn exiting_nested_epic_sets_dirty() {
    use crate::models::EpicId;
    use crate::tui::messages::EpicMessage;
    use crate::tui::types::{BoardSelection, ViewMode};

    let mut app = make_app();
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child_epic];
    // Start inside nested epic 20, parent is epic 10
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(20),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Epic {
            epic_id: EpicId(10),
            selection: BoardSelection::new_for_epic(),
            parent: Box::new(ViewMode::Board(BoardSelection::new())),
        }),
    };
    app.dirty = false;

    // Exit back to parent epic 10 — same ViewMode discriminant, different epic_id
    app.update(Message::Epic(EpicMessage::Exit));

    assert!(
        app.dirty,
        "exiting a nested epic must set dirty; got dirty=false"
    );
}

// ---------------------------------------------------------------------------
// Dirty signal: learnings overlay navigation (same ViewMode::view_selected path)
// ---------------------------------------------------------------------------

#[test]
fn learnings_selection_move_sets_dirty() {
    use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
    use crate::tui::messages::LearningMessage;
    let mut app = make_app();
    let now = chrono::Utc::now();
    let make_learning = |id: i64| Learning {
        id: LearningId(id),
        kind: LearningKind::Convention,
        summary: format!("learning {id}"),
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: vec![],
        status: LearningStatus::Approved,
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: now,
        updated_at: now,
    };
    app.update(Message::Learning(LearningMessage::Show(vec![
        make_learning(1),
        make_learning(2),
    ])));
    app.dirty = false;

    app.handle_key(make_key(KeyCode::Char('j')));

    assert!(
        app.dirty,
        "pressing j in the learnings overlay when cursor can move must set dirty; got dirty=false"
    );
}
