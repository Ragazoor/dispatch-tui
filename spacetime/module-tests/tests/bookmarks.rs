//! Task bookmarks (`core.allium: Bookmark`; `tasks.allium: SetBookmark`,
//! `BookmarkEndsWithTheTaskFinishing`), run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn bookmarks_for(store: &Store, who: &str) -> Vec<i64> {
    let subscriber = identity_of(who);
    let mut ids: Vec<i64> = store
        .db()
        .bookmarks()
        .iter()
        .filter(|b| b.subscriber == subscriber)
        .map(|b| b.task_id)
        .collect();
    ids.sort_unstable();
    ids
}

fn set(store: &Store, who: &str, task: i64, on: bool) -> Result<(), String> {
    store.call(who, |ctx| set_bookmark(ctx, identity_of(who), task, on))
}

/// Setting writes one row for the caller only; repeating it changes nothing;
/// clearing removes it, and clearing again is not an error.
#[test]
fn set_and_clear_are_per_person_and_idempotent() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.enrol("bob");
    let task = store.create("alice", epic_task("t", epic, "backlog"));

    set(&store, "bob", task, true).expect("set");
    set(&store, "bob", task, true).expect("set again");
    assert_eq!(bookmarks_for(&store, "bob"), vec![task]);
    assert!(bookmarks_for(&store, "alice").is_empty());

    set(&store, "alice", task, true).expect("alice sets her own");
    set(&store, "bob", task, false).expect("clear");
    set(&store, "bob", task, false).expect("clear again");
    assert!(bookmarks_for(&store, "bob").is_empty());
    assert_eq!(bookmarks_for(&store, "alice"), vec![task]);
}

/// Nobody writes another person's bookmark.
#[test]
fn a_bookmark_is_written_only_by_its_own_person() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.enrol("bob");
    let task = store.create("alice", epic_task("t", epic, "backlog"));

    assert!(store
        .call("alice", |ctx| set_bookmark(ctx, identity_of("bob"), task, true))
        .is_err());
    assert!(bookmarks_for(&store, "bob").is_empty());
}

/// A Done task cannot be bookmarked; removing one from a Done task is a no-op.
#[test]
fn a_done_task_cannot_be_bookmarked() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "done"));

    let err = set(&store, "alice", task, true).expect_err("done is refused");
    assert!(err.contains("done"), "{err}");
    set(&store, "alice", task, false).expect("clearing is never refused");
    assert!(bookmarks_for(&store, "alice").is_empty());
}

/// A task the caller cannot see is refused as a missing one.
#[test]
fn an_unseen_or_missing_task_is_refused() {
    let store = Store::new();
    let epic = store.create_epic("alice", "mine", false);
    store
        .call("alice", |ctx| {
            set_epic_visibility(ctx, epic, VISIBILITY_PRIVATE.into())
        })
        .expect("set_epic_visibility");
    store.enrol("bob");
    let task = store.create("alice", epic_task("t", epic, "backlog"));

    assert!(set(&store, "bob", task, true).is_err());
    assert!(set(&store, "bob", 9_999, true).is_err());
    assert!(bookmarks_for(&store, "bob").is_empty());
}

/// Reaching Done ends every person's bookmark on the task.
#[test]
fn finishing_a_task_ends_its_bookmarks() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.enrol("bob");
    let task = store.create("alice", epic_task("t", epic, "running"));
    set(&store, "alice", task, true).expect("set");
    set(&store, "bob", task, true).expect("set");

    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("done")))
        .expect("patch_task");

    assert!(store.db().bookmarks().iter().next().is_none());
}

/// Unlike the new-task signal, a move to another epic keeps the bookmark.
#[test]
fn moving_a_task_to_another_epic_keeps_its_bookmark() {
    let store = Store::new();
    let from = store.create_epic("alice", "from", false);
    let to = store.create_epic("alice", "to", false);
    let task = store.create("alice", epic_task("t", from, "backlog"));
    set(&store, "alice", task, true).expect("set");

    store
        .call("alice", |ctx| set_task_epic(ctx, task, to, String::new()))
        .expect("set_task_epic");

    assert_eq!(bookmarks_for(&store, "alice"), vec![task]);
}

/// A bookmark cannot outlive its task.
#[test]
fn deleting_a_task_removes_its_bookmarks() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "running"));
    // Done ends the bookmark first, so insert one by hand to prove the delete
    // path removes rows on its own.
    store.db().bookmarks().insert(Bookmark {
        id: bookmark_id(&identity_of("alice"), task),
        task_id: task,
        subscriber: identity_of("alice"),
    });
    store.db().tasks().id().update(Task {
        status: "done".into(),
        ..store.task(task)
    });

    store
        .call("alice", |ctx| delete_task(ctx, task))
        .expect("delete_task");

    assert!(bookmarks_for(&store, "alice").is_empty());
}
