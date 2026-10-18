//! Agent session state: the subagent count, the deferred Stop and the PR
//! learnings gate (`docs/specs/agent-health.allium`), run against the
//! in-memory store.
//!
//! Not here: the cases that delete an existing `task_subagents` entry (the
//! dedupe of a repeated start, a drain that applies a deferred Stop). The table
//! has no primary key, so the module deletes by value, which is a raw host call
//! the in-memory store does not serve. Those stay in
//! `tests/spacetime_module/agent_state.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// A running task on `HOST_A` with a live agent, dispatched through the public
/// reducers.
fn running_task() -> (Store, i64) {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let id = store.create("alice", user_task("t", "alice"));
    store
        .call("alice", |ctx| {
            request_dispatch(ctx, id, HOST_A.into(), "user".into(), String::new())
        })
        .expect("request_dispatch");
    let request = store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == id)
        .map(|r| r.id)
        .max()
        .expect("a dispatch request");
    store
        .call("alice", |ctx| {
            claim_dispatch_request(ctx, request, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_dispatch_provisioned(
                ctx,
                request,
                HOST_A.into(),
                format!("/wt/{id}-t"),
                format!("task-{id}"),
                false,
            )
        })
        .expect("report provisioned");
    assert_eq!(store.task(id).status, "running");
    (store, id)
}

fn try_stop(store: &Store, task: i64, at: &str) -> Result<(), String> {
    store.call("alice", |ctx| try_record_stop(ctx, task, at.into()))
}

/// `HookStop`: nothing live, so the Stop flips the task straight to `review`.
#[test]
fn try_record_stop_flips_a_running_task_with_nothing_live() {
    let (store, id) = running_task();

    try_stop(&store, id, "2026-09-19T10:00:00.000").expect("try_record_stop");

    assert_eq!(store.task(id).status, "review");
}

/// A Stop against a task that is not `Running` is refused outright, which lets
/// the `Store` read `NoOp` back unambiguously, rather than a silent no-op the
/// caller could not tell apart from a real flip.
#[test]
fn try_record_stop_refuses_a_task_that_is_not_running() {
    let store = Store::new();
    let id = store.create("alice", user_task("t", "alice"));

    assert!(
        try_stop(&store, id, "2026-09-19T10:00:00.000").is_err(),
        "a Stop against a non-Running task must be refused"
    );
}

/// The PR learnings gate fires EXACTLY once: the first call sets it and wins,
/// a second is refused. The `Store` reads that refusal as `false` via
/// `ReducerOutcome::won()`, the same shape as the dispatch claim.
#[test]
fn mark_pr_learnings_gate_shown_wins_exactly_once() {
    let (store, id) = running_task();

    store
        .call("alice", |ctx| {
            mark_pr_learnings_gate_shown(ctx, id, "2026-09-19T10:00:00.000".into())
        })
        .expect("the first call wins");

    let second = store.call("alice", |ctx| {
        mark_pr_learnings_gate_shown(ctx, id, "2026-09-19T10:00:01.000".into())
    });
    assert!(second.is_err(), "a second call must be refused");
}
