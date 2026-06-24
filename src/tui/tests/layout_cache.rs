//! Tests for the layout cache on App.
//!
//! Two caches live and die together (all cleared by `invalidate_layout_cache`):
//!   - `epic_stats_cache`    — O(1) Arc clone instead of full HashMap copy
//!   - `column_anchor_cache` — sorted selectable items per status (O(1) nav)
//!
//! Core invariants:
//!   1. Both caches are empty after invalidation.
//!   2. `cached_epic_stats()` populates both caches atomically.
//!   3. Navigation never invalidates a populated cache.
//!   4. Board mutations always invalidate and repopulate the caches.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use super::*;
use crate::models::{EpicId, TaskId, TaskStatus};
use crate::tui::messages::{EpicMessage, TaskMessage};
use crate::tui::types::{ColumnAnchor, Message};

// ---------------------------------------------------------------------------
// Startup state
// ---------------------------------------------------------------------------

#[test]
fn epic_stats_cache_is_populated_after_new() {
    // App::new() calls cached_epic_stats() to seed the anchor, so the cache
    // is warm (not None) from the very first render onward.
    let app = make_app();
    assert!(app.epic_stats_cache.is_some());
}

// ---------------------------------------------------------------------------
// cached_epic_stats
// ---------------------------------------------------------------------------

#[test]
fn cached_epic_stats_populates_cache_after_invalidation() {
    let mut app = make_app();
    app.invalidate_layout_cache();
    assert!(app.epic_stats_cache.is_none());
    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());
}

#[test]
fn cached_epic_stats_returns_consistent_value_on_repeated_calls() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(10)];
    let first = app.cached_epic_stats();
    let second = app.cached_epic_stats();
    assert_eq!(first.len(), second.len());
    assert_eq!(
        first.contains_key(&EpicId(10)),
        second.contains_key(&EpicId(10))
    );
}

// ---------------------------------------------------------------------------
// invalidate_layout_cache
// ---------------------------------------------------------------------------

#[test]
fn invalidate_layout_cache_clears_populated_cache() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(10)];
    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());

    app.invalidate_layout_cache();
    assert!(app.epic_stats_cache.is_none());
}

#[test]
fn invalidate_layout_cache_is_idempotent() {
    let mut app = make_app();
    // First invalidate on a warm cache.
    app.invalidate_layout_cache();
    assert!(app.epic_stats_cache.is_none());
    // Second invalidate on an already-empty cache should not panic.
    app.invalidate_layout_cache();
    assert!(app.epic_stats_cache.is_none());
}

// ---------------------------------------------------------------------------
// Navigation must NOT invalidate the cache
// ---------------------------------------------------------------------------

#[test]
fn navigate_row_does_not_invalidate_populated_cache() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());

    app.update(Message::NavigateRow(1));
    assert!(
        app.epic_stats_cache.is_some(),
        "navigate_row must not clear the cache"
    );
}

#[test]
fn navigate_column_does_not_invalidate_populated_cache() {
    let mut app = make_app();
    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());

    app.update(Message::NavigateColumn(1));
    assert!(
        app.epic_stats_cache.is_some(),
        "navigate_column must not clear the cache"
    );
}

// ---------------------------------------------------------------------------
// Mutations must repopulate the cache with fresh data
// ---------------------------------------------------------------------------

#[test]
fn refresh_tasks_repopulates_cache_with_new_task_stats() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    // Direct field mutation bypasses the message system, so invalidate manually.
    app.board.epics = vec![make_epic(10)];
    app.invalidate_layout_cache();

    let before = app.cached_epic_stats();
    assert_eq!(before[&EpicId(10)].backlog, 0, "epic has no subtasks yet");

    // Refresh board with a new subtask belonging to epic 10
    let mut subtask = make_task(2, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.update(Message::Task(TaskMessage::Refresh(vec![
        make_task(1, TaskStatus::Backlog),
        subtask,
    ])));

    let after = app.cached_epic_stats();
    assert_eq!(
        after[&EpicId(10)].backlog,
        1,
        "cache must reflect the new subtask"
    );
}

#[test]
fn refresh_epics_invalidates_and_repopulates_cache() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(10)];

    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());

    // Replace epics: remove epic 10, add epic 20
    app.update(Message::Epic(EpicMessage::Refresh(vec![make_epic(20)])));

    let after = app.cached_epic_stats();
    assert!(after.contains_key(&EpicId(20)), "new epic must be in cache");
    assert!(
        !after.contains_key(&EpicId(10)),
        "removed epic must not be in cache"
    );
}

#[test]
fn task_created_repopulates_cache() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];

    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());

    let mut new_subtask = make_task(99, TaskStatus::Backlog);
    new_subtask.epic_id = Some(EpicId(10));
    app.update(Message::Task(TaskMessage::Created { task: new_subtask }));

    let after = app.cached_epic_stats();
    assert_eq!(
        after[&EpicId(10)].backlog,
        1,
        "created subtask must be reflected in cache"
    );
}

#[test]
fn epic_created_invalidates_cache() {
    let mut app = make_app();

    let _ = app.cached_epic_stats();
    assert!(app.epic_stats_cache.is_some());

    app.update(Message::Epic(EpicMessage::Created(make_epic(42))));

    // Cache should be None (invalidated) or Some with new epic — either is acceptable;
    // the key property is that it reflects the new state.
    let after = app.cached_epic_stats();
    assert!(
        after.contains_key(&EpicId(42)),
        "created epic must appear in cache"
    );
}

// ---------------------------------------------------------------------------
// Arc — cached_epic_stats returns the same Arc on repeated calls
// ---------------------------------------------------------------------------

#[test]
fn cached_epic_stats_returns_same_arc_on_repeated_calls() {
    let mut app = make_app();
    let first = app.cached_epic_stats();
    let second = app.cached_epic_stats();
    assert!(
        Arc::ptr_eq(&first, &second),
        "second call should return a clone of the same Arc, not a new allocation"
    );
}

#[test]
fn invalidate_then_reprime_produces_new_arc() {
    let mut app = make_app();
    let before = app.cached_epic_stats();
    app.invalidate_layout_cache();
    let after = app.cached_epic_stats();
    assert!(
        !Arc::ptr_eq(&before, &after),
        "after invalidation a new Arc must be allocated"
    );
}

// ---------------------------------------------------------------------------
// column_anchor_cache — built and read correctly
// ---------------------------------------------------------------------------

#[test]
fn column_anchor_cache_starts_empty_after_invalidation() {
    let mut app = make_app();
    app.invalidate_layout_cache();
    assert!(
        app.column_anchor_cache.is_none(),
        "invalidate_layout_cache must clear column_anchor_cache"
    );
}

#[test]
fn cached_epic_stats_populates_column_anchor_cache() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    app.invalidate_layout_cache();
    assert!(app.column_anchor_cache.is_none());
    let _ = app.cached_epic_stats();
    assert!(
        app.column_anchor_cache.is_some(),
        "cached_epic_stats must populate column_anchor_cache"
    );
}

#[test]
fn column_anchor_cache_lists_tasks_in_correct_order() {
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.sort_order = Some(10);
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.sort_order = Some(5); // t2 sorts before t1

    let mut app = App::new(vec![t1, t2]);
    let _ = app.cached_epic_stats();

    let anchors = app
        .column_anchor_cache
        .as_ref()
        .unwrap()
        .get(&TaskStatus::Backlog)
        .unwrap();
    assert_eq!(anchors.len(), 2);
    assert_eq!(anchors[0], ColumnAnchor::Task(TaskId(2)), "lower sort_order first");
    assert_eq!(anchors[1], ColumnAnchor::Task(TaskId(1)));
}

#[test]
fn navigate_row_does_not_clear_column_anchor_cache() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    let _ = app.cached_epic_stats();
    assert!(app.column_anchor_cache.is_some());

    app.update(Message::NavigateRow(1));
    assert!(
        app.column_anchor_cache.is_some(),
        "navigation must not clear column_anchor_cache"
    );
}

#[test]
fn update_anchor_from_current_sets_anchor_using_cache() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    // prime cache and set cursor to row 1 in Backlog column
    let _ = app.cached_epic_stats();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);

    app.update_anchor_from_current();

    let anchor = app.selection().anchor;
    // The anchor should be set to the task at selectable row 1 in Backlog
    assert!(anchor.is_some(), "anchor should be set after update_anchor_from_current");
}

#[test]
fn column_anchor_cache_invalidated_on_task_mutation() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let _ = app.cached_epic_stats();
    assert!(app.column_anchor_cache.is_some());

    app.update(Message::Task(TaskMessage::Refresh(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(99, TaskStatus::Backlog),
    ])));

    // After cache is repopulated, new task should appear
    let _ = app.cached_epic_stats();
    let anchors = app
        .column_anchor_cache
        .as_ref()
        .unwrap()
        .get(&TaskStatus::Backlog)
        .unwrap();
    let ids: Vec<_> = anchors
        .iter()
        .filter_map(|a| match a {
            ColumnAnchor::Task(id) => Some(id),
            ColumnAnchor::Epic(_) => None,
        })
        .collect();
    assert!(ids.iter().any(|id| **id == TaskId(99)), "new task must appear in anchor cache");
}
