//! The one limit on starts in an orchestrated epic (`store-split.allium:
//! RequestDispatch`, its limit requires; `orchestrators.allium:
//! running_here`), run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn switch_on(store: &Store, epic: i64, max: Option<i64>) {
    store
        .call("alice", |ctx| switch_orchestrator_on(ctx, epic, None, max))
        .expect("switch on");
}

fn request_on(store: &Store, task: i64, host: &str, origin: &str) -> Result<(), String> {
    store.call("alice", |ctx| {
        request_dispatch(ctx, task, host.into(), origin.into(), String::new())
    })
}

fn request(store: &Store, task: i64) -> Result<(), String> {
    request_on(store, task, HOST_A, "user")
}

fn open_request_id(store: &Store, task: i64) -> i64 {
    store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == task)
        .map(|r| r.id)
        .max()
        .expect("the request")
}

/// Request, claim and report the task dispatched, with a window.
fn start_live(store: &Store, task: i64) {
    request(store, task).expect("request");
    let req = open_request_id(store, task);
    store
        .call("alice", |ctx| {
            claim_dispatch_request(ctx, req, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_dispatch_provisioned(
                ctx,
                req,
                HOST_A.into(),
                format!("/wt/{task}"),
                format!("task-{task}"),
                false,
            )
        })
        .expect("report dispatched");
}

fn patch(store: &Store, task: i64, patch: TaskPatch) {
    store
        .call("alice", |ctx| patch_task(ctx, task, patch))
        .expect("patch_task");
}

/// An epic of alice's with `count` backlog tasks, and a registered host.
fn epic_with(store: &Store, count: usize) -> (i64, Vec<i64>) {
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "E", false);
    let ids = (0..count)
        .map(|_| store.create("alice", epic_task("t", epic, "backlog")))
        .collect();
    (epic, ids)
}

fn at_limit(epic: i64, n: i64, limit: i64) -> String {
    format!("epic {epic} already runs {n} of {limit} tasks")
}

#[test]
fn a_burst_of_starts_stops_at_the_limit() {
    let store = Store::new();
    let (epic, ids) = epic_with(&store, 3);
    switch_on(&store, epic, Some(2));

    request(&store, ids[0]).expect("first");
    request(&store, ids[1]).expect("second");

    assert_eq!(request(&store, ids[2]).unwrap_err(), at_limit(epic, 2, 2));
    assert_eq!(store.task(ids[2]).status, "backlog");
    assert!(store
        .db()
        .dispatch_requests()
        .iter()
        .all(|r| r.task_id != ids[2]));
}

#[test]
fn live_sessions_hold_their_slots() {
    let store = Store::new();
    let (epic, ids) = epic_with(&store, 3);
    switch_on(&store, epic, Some(2));
    start_live(&store, ids[0]);
    start_live(&store, ids[1]);

    assert_eq!(request(&store, ids[2]).unwrap_err(), at_limit(epic, 2, 2));
}

#[test]
fn the_default_limit_is_three() {
    let store = Store::new();
    let (epic, ids) = epic_with(&store, 4);
    switch_on(&store, epic, None);
    for id in &ids[..3] {
        request(&store, *id).expect("within the default limit");
    }

    assert_eq!(request(&store, ids[3]).unwrap_err(), at_limit(epic, 3, 3));
}

#[test]
fn a_failed_request_gives_its_slot_back() {
    let store = Store::new();
    let (epic, ids) = epic_with(&store, 2);
    switch_on(&store, epic, Some(1));
    request(&store, ids[0]).expect("first");
    let req = open_request_id(&store, ids[0]);
    store
        .call("alice", |ctx| {
            claim_dispatch_request(ctx, req, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_dispatch_failed(ctx, req, HOST_A.into(), "boom".into())
        })
        .expect("report failed");

    request(&store, ids[1]).expect("the slot is free again");
}

#[test]
fn a_task_in_review_and_a_detached_task_take_no_slot() {
    let store = Store::new();
    let (epic, ids) = epic_with(&store, 4);
    switch_on(&store, epic, Some(2));
    start_live(&store, ids[0]);
    start_live(&store, ids[1]);
    patch(
        &store,
        ids[0],
        TaskPatch {
            status: Some("review".into()),
            ..TaskPatch::default()
        },
    );
    patch(
        &store,
        ids[1],
        TaskPatch {
            tmux_window: Some(String::new()),
            ..TaskPatch::default()
        },
    );
    assert_eq!(
        store.task(ids[1]).status,
        "running",
        "detached, not stopped"
    );

    request(&store, ids[2]).expect("first free slot");
    request(&store, ids[3]).expect("second free slot");
}

#[test]
fn a_task_no_orchestrator_covers_has_no_limit() {
    let store = Store::new();
    let (_, ids) = epic_with(&store, 5);
    for id in ids {
        request(&store, id).expect("no limit");
    }
}

#[test]
fn a_sub_epic_task_counts_toward_the_switch_above() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let root = store.create_epic("alice", "Root", false);
    let sub = store.create_sub_epic("alice", root);
    let in_sub = store.create("alice", epic_task("t", sub, "backlog"));
    let in_root = store.create("alice", epic_task("t", root, "backlog"));
    switch_on(&store, root, Some(1));

    request(&store, in_sub).expect("first");

    assert_eq!(request(&store, in_root).unwrap_err(), at_limit(root, 1, 1));
}

#[test]
fn another_machines_tasks_do_not_count() {
    let store = Store::new();
    let (epic, ids) = epic_with(&store, 2);
    store.register_host_for(HOST_B, "alice");
    switch_on(&store, epic, Some(1));

    request_on(&store, ids[0], HOST_B, "user").expect("on host b");

    request(&store, ids[1]).expect("host a has a free slot");
}

#[test]
fn every_origin_is_held_to_the_limit() {
    for origin in ["user", "mcp", "epic_chain", "quick"] {
        let store = Store::new();
        let (epic, ids) = epic_with(&store, 2);
        switch_on(&store, epic, Some(1));
        request(&store, ids[0]).expect("first");

        assert_eq!(
            request_on(&store, ids[1], HOST_A, origin).unwrap_err(),
            at_limit(epic, 1, 1),
            "origin {origin}"
        );
    }
}

/// The epic chain's own start, inside `request_exit_session`, is held too.
#[test]
fn the_epic_chain_does_not_pass_the_limit() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "E", true);
    let closing = store.create("alice", epic_task("t", epic, "backlog"));
    let others: Vec<i64> = (0..2)
        .map(|_| store.create("alice", epic_task("t", epic, "backlog")))
        .collect();
    switch_on(&store, epic, Some(3));
    start_live(&store, closing);
    for id in &others {
        start_live(&store, *id);
    }
    store
        .call("alice", |ctx| {
            change_orchestrator_settings(ctx, epic, None, Some(2))
        })
        .expect("lower the limit");
    let next = store.create("alice", epic_task("next", epic, "backlog"));
    store.advance_secs(1);

    store
        .call("alice", |ctx| {
            request_wrap_up(ctx, closing, HOST_A.into(), "rebase".into())
        })
        .expect("wrap up");
    let token = store
        .db()
        .session_requests()
        .iter()
        .find(|r| r.task_id == closing && r.kind == SESSION_KIND_WRAP_UP)
        .expect("the wrap-up row")
        .id;
    store
        .call("alice", |ctx| {
            claim_session_request(ctx, token, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_session_request_done(ctx, token, HOST_A.into())
        })
        .expect("done");
    store
        .call("alice", |ctx| {
            request_exit_session(ctx, closing, token, "rebase".into(), String::new())
        })
        .expect("exit");

    assert_eq!(store.task(closing).status, "done");
    assert_eq!(store.task(next).status, "backlog", "two still run, of two");
}

/// Two machines asking for one task: the first claim wins, the second is
/// refused, and one request exists.
#[test]
fn two_machines_cannot_both_start_one_task() {
    let store = Store::new();
    let (_, ids) = epic_with(&store, 1);
    store.register_host_for(HOST_B, "alice");

    request_on(&store, ids[0], HOST_A, "mcp").expect("host a wins");
    assert!(request_on(&store, ids[0], HOST_B, "mcp").is_err());

    assert_eq!(store.db().dispatch_requests().iter().count(), 1);
    assert_eq!(open_request_id(&store, ids[0]), {
        let only = store
            .db()
            .dispatch_requests()
            .iter()
            .next()
            .expect("one row");
        assert_eq!(only.host, HOST_A);
        only.id
    });
}
