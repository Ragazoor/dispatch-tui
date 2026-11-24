//! Batch sub-status, phoenix respawn and host registration, run against the
//! in-memory store. Twins of `tests/spacetime_module/epics_and_hosts.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// One call updates every task in the batch, and leaves a missing id as a
/// silent no-op rather than failing the whole call.
#[test]
fn batch_patch_sub_status_applies_the_whole_batch_in_one_call() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let a = store.create("alice", epic_task("a", epic, "running"));
    let b = store.create("alice", epic_task("b", epic, "running"));

    let updates = vec![
        SubStatusUpdate {
            task_id: a,
            sub_status: "active".into(),
        },
        SubStatusUpdate {
            task_id: b,
            sub_status: "stale".into(),
        },
        SubStatusUpdate {
            task_id: 999,
            sub_status: "active".into(),
        },
    ];
    store
        .call("alice", |ctx| batch_patch_sub_status(ctx, updates))
        .expect("a missing id must not fail the batch");

    assert_eq!(store.task(a).sub_status, "active");
    assert_eq!(store.task(b).sub_status, "stale");
}

/// Plain upsert by id: a second call with a changed label overwrites rather
/// than duplicating.
#[test]
fn register_host_upserts_by_id() {
    let store = Store::new();
    let owner = identity_of("alice");
    for label in ["first-label", "second-label"] {
        store
            .call("alice", |ctx| {
                register_host(ctx, "host-1".into(), label.into(), owner.clone())
            })
            .expect("register_host");
    }
    assert_eq!(store.db().hosts().iter().count(), 1);
    assert_eq!(
        store
            .db()
            .hosts()
            .id()
            .find("host-1".to_string())
            .expect("host")
            .label,
        "second-label"
    );
}
