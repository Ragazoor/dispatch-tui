//! The dispatch-request reducers, run against the in-memory store, including
//! the backstop timer a unit test could not reach before `TestClock`.
//!
//! The in-memory host does not fire scheduled reducers itself, so a test that
//! wants the timer to fire calls `expire_dispatch_request` with the timer row,
//! as the live host would.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// A registered host and one backlog task, ready to dispatch.
fn host_and_task(store: &Store) -> i64 {
    store.register_host_for(HOST_A, "alice");
    store.create("alice", user_task("t", "alice"))
}

fn request(store: &Store, task_id: i64) -> Result<(), String> {
    store.call("alice", |ctx| {
        request_dispatch(ctx, task_id, HOST_A.into(), "user".into(), String::new())
    })
}

fn only_request(store: &Store) -> DispatchRequest {
    let mut rows: Vec<_> = store.db().dispatch_requests().iter().collect();
    assert_eq!(rows.len(), 1, "exactly one dispatch request");
    rows.remove(0)
}

#[test]
fn a_request_claims_the_task_and_schedules_its_backstop() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");

    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_REQUESTED);
    assert_eq!(row.task_id, id);
    assert_eq!(
        store.task(id).status,
        "running",
        "the claim is the task move"
    );
    assert_eq!(store.db().dispatch_request_timeouts().iter().count(), 1);
}

#[test]
fn a_second_request_for_a_claimed_task_is_refused() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("first request");
    assert!(request(&store, id).is_err());
    assert_eq!(store.db().dispatch_requests().iter().count(), 1);
}

#[test]
fn only_the_hosts_owner_may_request_on_it() {
    let store = Store::new();
    let id = host_and_task(&store);
    let refused = store.call("mallory", |ctx| {
        request_dispatch(ctx, id, HOST_A.into(), "user".into(), String::new())
    });
    assert!(refused.is_err());
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.task(id).status, "backlog");
}

#[test]
fn claim_then_report_dispatched_records_the_worktree_and_cancels_the_timer() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");
    let req = only_request(&store).id;

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
                "/wt/1-t".into(),
                "task-1".into(),
                false,
            )
        })
        .expect("report");

    assert_eq!(only_request(&store).state, DISPATCH_DISPATCHED);
    let task = store.task(id);
    assert_eq!(task.worktree, "/wt/1-t");
    assert_eq!(store.db().dispatch_request_timeouts().iter().count(), 0);
}

#[test]
fn a_failed_report_releases_the_bare_claim() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");
    let req = only_request(&store).id;
    store
        .call("alice", |ctx| {
            claim_dispatch_request(ctx, req, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_dispatch_failed(ctx, req, HOST_A.into(), "no tmux".into())
        })
        .expect("report failed");

    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_FAILED);
    assert_eq!(row.reason, "no tmux");
    assert_eq!(store.task(id).status, "backlog", "the claim is released");
}

#[test]
fn the_backstop_fails_a_request_nobody_finished_and_releases_the_claim() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");
    let timer = store
        .db()
        .dispatch_request_timeouts()
        .iter()
        .next()
        .expect("the backstop timer row");

    store.advance_secs(DISPATCH_REQUEST_TIMEOUT_SECS + DISPATCH_REQUEST_BACKSTOP_GRACE_SECS);
    store
        .call_as_scheduler(|ctx| expire_dispatch_request(ctx, timer))
        .expect("the scheduler runs the backstop");

    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_FAILED);
    assert_eq!(row.reason, DISPATCH_REASON_TIMED_OUT);
    assert_eq!(store.task(id).status, "backlog");
}

#[test]
fn a_client_cannot_run_the_backstop_early() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");
    let timer = store
        .db()
        .dispatch_request_timeouts()
        .iter()
        .next()
        .expect("the backstop timer row");

    let refused = store.call("alice", |ctx| expire_dispatch_request(ctx, timer));
    assert!(refused.is_err());
    assert_eq!(only_request(&store).state, DISPATCH_REQUESTED);
}

#[test]
fn the_backstop_leaves_a_request_that_already_finished() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");
    let req = only_request(&store).id;
    let timer = store
        .db()
        .dispatch_request_timeouts()
        .iter()
        .next()
        .expect("the backstop timer row");
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

    store
        .call_as_scheduler(|ctx| expire_dispatch_request(ctx, timer))
        .expect("a stale timer is harmless");
    assert_eq!(only_request(&store).reason, "boom", "its own report won");
}
