//! Task and epic reducers, run against the in-memory store. Twins of the
//! live-store cases in `tests/spacetime_module/mutations.rs` that need no
//! publish: create, patch, delete and the epic status they drive.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

#[test]
fn create_task_ignores_a_supplied_id_and_stamps_nothing_else_it_was_not_given() {
    let store = Store::new();
    let row = Task {
        id: 99,
        ..user_task("first", "alice")
    };
    let id = store.create("alice", row);
    assert_ne!(id, 99, "the store reserves ids, the caller does not");
    assert_eq!(store.task(id).title, "first");
}

#[test]
fn the_store_refuses_a_user_board_task_with_no_owner() {
    let store = Store::new();
    let row = Task {
        owner: String::new(),
        ..user_task("orphan", "alice")
    };
    let refused = store.call("alice", |ctx| create_task(ctx, row));
    assert!(refused.is_err(), "an epic-less task needs an owner");
    assert_eq!(store.db().tasks().iter().count(), 0, "nothing was written");
}

#[test]
fn an_epics_status_is_derived_from_every_hosts_children() {
    let store = Store::new();
    let epic = store.create_epic("alice", "Shared", false);
    let a = store.create("alice", epic_task("finished on host-a", epic, "done"));
    let b = store.create("bob", epic_task("still going on host-b", epic, "running"));
    assert_ne!(a, b);
    assert_eq!(
        store.epic(epic).status,
        "backlog",
        "one child is still running elsewhere, so the epic is not done"
    );

    store
        .call("bob", |ctx| patch_task(ctx, b, status_patch("done")))
        .expect("patch_task");
    assert_eq!(store.epic(epic).status, "done");
}

#[test]
fn patching_a_task_to_done_stamps_completion_from_the_stores_clock() {
    let store = Store::new();
    let id = store.create("alice", user_task("t", "alice"));
    assert_eq!(store.task(id).completed_at, "");

    store.advance_secs(90);
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    assert_eq!(store.task(id).completed_at, "2026-10-10 12:01:30.000");
}

#[test]
fn patching_a_missing_task_is_a_silent_no_op() {
    let store = Store::new();
    store
        .call("alice", |ctx| patch_task(ctx, 404, status_patch("done")))
        .expect("a missing id is not an error");
}

#[test]
fn deleting_an_undone_task_is_refused_at_the_reducer() {
    let store = Store::new();
    let id = store.create("alice", user_task("still running", "alice"));
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("running")))
        .expect("patch_task");

    let refused = store.call("alice", |ctx| delete_task(ctx, id));
    let why = refused.expect_err("a task that is not done must be refused");
    assert!(why.contains("not done"), "the refusal says why: {why}");
    assert!(store.db().tasks().id().find(id).is_some());

    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("a done task deletes");
    assert!(store.db().tasks().id().find(id).is_none());
}

#[test]
fn deleting_the_last_unfinished_child_lets_the_epic_finish() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let done = store.create("alice", epic_task("done", epic, "done"));
    store.create("alice", epic_task("open", epic, "backlog"));
    assert_eq!(store.epic(epic).status, "backlog");

    // Finish and delete the open one: only the done child remains.
    let open = store
        .db()
        .tasks()
        .iter()
        .find(|t| t.status == "backlog")
        .expect("the open child")
        .id;
    store
        .call("alice", |ctx| patch_task(ctx, open, status_patch("done")))
        .expect("patch_task");
    store
        .call("alice", |ctx| delete_task(ctx, open))
        .expect("delete_task");
    assert_eq!(store.task(done).status, "done");
    assert_eq!(store.epic(epic).status, "done");
}
