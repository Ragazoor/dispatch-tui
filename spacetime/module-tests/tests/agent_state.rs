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
    try_stop_with(store, task, at, BackgroundWork::default())
}

fn try_stop_with(
    store: &Store,
    task: i64,
    at: &str,
    background: BackgroundWork,
) -> Result<(), String> {
    store.call("alice", |ctx| {
        try_record_stop(ctx, task, at.into(), background)
    })
}

fn shells(n: i64) -> BackgroundWork {
    BackgroundWork {
        shells: n,
        ..BackgroundWork::default()
    }
}

fn current(store: &Store, task: i64) -> Agent {
    store
        .db()
        .agents()
        .iter()
        .find(|a| a.task_id == task && a.state == "current")
        .expect("a current agent")
}

fn expire(store: &Store, task: i64, now: &str) {
    try_expire(store, task, now).expect("expire_background_wait");
}

fn try_expire(store: &Store, task: i64, now: &str) -> Result<(), String> {
    store.call("alice", |ctx| expire_background_wait(ctx, task, now.into()))
}

/// `HookStop`: a Stop whose list holds background work defers like one with
/// live subagents, and records the counts it saw.
#[test]
fn try_record_stop_defers_while_background_work_is_listed() {
    let (store, id) = running_task();
    let background = BackgroundWork {
        shells: 2,
        workflows: 1,
        monitors: 1,
        mcp_tasks: 1,
    };

    try_stop_with(&store, id, "2026-10-10 13:00:00.000", background).expect("stop");

    assert_eq!(store.task(id).status, "running");
    let agent = current(&store, id);
    assert!(agent.stop_pending);
    assert_eq!(agent.stop_pending_at, "2026-10-10 13:00:00.000");
    assert_eq!(agent.background_work(), background);
}

/// `HookStop`: the wake-up turn's Stop lists nothing, so it flips the task,
/// even though the earlier Stop left the bit set.
#[test]
fn try_record_stop_with_an_empty_list_ends_the_wait() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("first stop");

    try_stop(&store, id, "2026-10-10 13:05:00.000").expect("second stop");

    assert_eq!(store.task(id).status, "review");
    assert!(!current(&store, id).stop_pending);
}

/// Every defer overwrites all four counts: a Stop deferred for subagents only
/// after a background wait leaves no shell behind.
#[test]
fn a_subagent_only_defer_overwrites_the_background_counts() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("first stop");
    store
        .call("alice", |ctx| {
            subagent_start(ctx, id, "a1".into(), "s1".into(), "2026-10-10 13:01:00".into())
        })
        .expect("subagent_start");

    try_stop(&store, id, "2026-10-10 13:02:00.000").expect("second stop");

    let agent = current(&store, id);
    assert!(agent.stop_pending);
    assert_eq!(agent.background_work(), BackgroundWork::default());
}

/// `HookNotification`: an idle_prompt while the agent waits on background work
/// is noise, not a block.
#[test]
fn idle_prompt_is_ignored_while_waiting_on_background_work() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");

    store
        .call("alice", |ctx| {
            record_notification(
                ctx,
                id,
                "raise_if_no_own_work_live".into(),
                "2026-10-10 13:01:00".into(),
            )
        })
        .expect("record_notification");

    assert_ne!(store.task(id).sub_status, "needs_input");
    assert_eq!(current(&store, id).last_notification_at, "");
}

/// `ExpireBackgroundWait`: past the cap on both clocks, the deferred Stop
/// applies.
#[test]
fn expire_background_wait_flips_a_wait_past_the_cap() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");

    expire(&store, id, "2026-10-10 13:30:00.000");

    assert_eq!(store.task(id).status, "review");
    assert!(!current(&store, id).stop_pending);
}

/// The cap counts from the Stop: a Stop younger than the module's cap holds.
#[test]
fn expire_background_wait_leaves_a_recent_stop_alone() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");

    expire(&store, id, "2026-10-10 13:29:59.000");

    assert_eq!(store.task(id).status, "running");
}

/// The cap also counts from the latest tool call: a turn that started without
/// voiding the Stop is not waiting. The Stop is 40 minutes old here, the tool
/// call 20.
#[test]
fn expire_background_wait_leaves_a_recently_active_agent_alone() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");
    store
        .call("alice", |ctx| {
            record_pre_tool_use(ctx, id, "active".into(), "2026-10-10 13:20:00".into())
        })
        .expect("record_pre_tool_use");

    expire(&store, id, "2026-10-10 13:40:00.000");

    assert_eq!(store.task(id).status, "running");
}

/// A Stop deferred for subagents only is not a background wait, so the cap
/// never applies it.
#[test]
fn expire_background_wait_ignores_a_subagent_only_defer() {
    let (store, id) = running_task();
    store
        .call("alice", |ctx| {
            subagent_start(ctx, id, "a1".into(), "s1".into(), "2026-10-10 12:00:00".into())
        })
        .expect("subagent_start");
    try_stop(&store, id, "2026-10-10 13:00:00.000").expect("stop");

    expire(&store, id, "2026-10-10 14:00:00.000");

    assert_eq!(store.task(id).status, "running");
}

/// Live subagents hold the task past the cap, as they always have.
#[test]
fn expire_background_wait_waits_for_live_subagents() {
    let (store, id) = running_task();
    store
        .call("alice", |ctx| {
            subagent_start(ctx, id, "a1".into(), "s1".into(), "2026-10-10 12:00:00".into())
        })
        .expect("subagent_start");
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");

    expire(&store, id, "2026-10-10 14:00:00.000");

    assert_eq!(store.task(id).status, "running");
}

/// `DetachTmux` kills the window and the background work with it, so its
/// drain applies a Stop deferred for background work.
#[test]
fn detach_applies_a_stop_deferred_for_background_work() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");

    store
        .call("alice", |ctx| subagent_clear(ctx, id))
        .expect("subagent_clear");

    assert_eq!(store.task(id).status, "review");
}

/// `HookStop`: nothing live, so the Stop flips the task straight to `review`.
#[test]
fn try_record_stop_flips_a_running_task_with_nothing_live() {
    let (store, id) = running_task();

    try_stop(&store, id, "2026-10-10 13:00:00.000").expect("try_record_stop");

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
        try_stop(&store, id, "2026-10-10 13:00:00.000").is_err(),
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
            mark_pr_learnings_gate_shown(ctx, id, "2026-10-10 13:00:00.000".into())
        })
        .expect("the first call wins");

    let second = store.call("alice", |ctx| {
        mark_pr_learnings_gate_shown(ctx, id, "2026-10-10 13:00:01.000".into())
    });
    assert!(second.is_err(), "a second call must be refused");
}

/// The module owns the cap: the board sends only its own `now`, so no value it
/// can send shortens the wait. A `now` that is not a store timestamp is
/// refused rather than read as "long ago".
#[test]
fn expire_background_wait_refuses_a_now_that_is_not_a_store_timestamp() {
    let (store, id) = running_task();
    try_stop_with(&store, id, "2026-10-10 13:00:00.000", shells(1)).expect("stop");

    assert!(try_expire(&store, id, "").is_err());
    assert!(try_expire(&store, id, "yesterday").is_err());

    assert_eq!(store.task(id).status, "running");
}

#[test]
fn background_wait_cutoff_is_the_cap_before_now() {
    assert_eq!(BACKGROUND_WAIT_CAP, chrono::Duration::minutes(30));
    assert_eq!(
        background_wait_cutoff("2026-10-10 13:30:00.000").unwrap(),
        "2026-10-10 13:00:00.000"
    );
    assert_eq!(
        background_wait_cutoff("2026-10-11 00:10:00.500").unwrap(),
        "2026-10-10 23:40:00.500"
    );
}
