//! The session-request reducers (`request_wrap_up`, `request_exit_session`,
//! the epic chain and the backstop), run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// Dispatch `task_id` end to end through the public reducers, leaving it
/// running on `HOST_A` with a worktree and a live window.
fn dispatch_to_running(store: &Store, task_id: i64) {
    store
        .call("alice", |ctx| {
            request_dispatch(ctx, task_id, HOST_A.into(), "user".into(), String::new())
        })
        .expect("request_dispatch");
    let req = store
        .db()
        .dispatch_requests()
        .iter()
        .find(|r| r.task_id == task_id)
        .expect("the request")
        .id;
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
                format!("/wt/{task_id}"),
                format!("task-{task_id}"),
                false,
            )
        })
        .expect("report dispatched");
    // A later dispatch voids an exit token issued at or before it, and the
    // clock does not move inside a test, so step past the report here.
    store.advance_secs(1);
}

fn wrap_up(store: &Store, task_id: i64, action: &str) -> Result<(), String> {
    store.call("alice", |ctx| {
        request_wrap_up(ctx, task_id, HOST_A.into(), action.into())
    })
}

fn session_rows(store: &Store, task_id: i64, kind: &str) -> Vec<SessionRequest> {
    store
        .db()
        .session_requests()
        .iter()
        .filter(|r| r.task_id == task_id && r.kind == kind)
        .collect()
}

/// A wrap_up(rebase) the host claimed and reported done: returns its id, the
/// exit token.
fn finished_rebase_wrap_up(store: &Store, task_id: i64) -> i64 {
    wrap_up(store, task_id, "rebase").expect("request_wrap_up");
    let row = session_rows(store, task_id, SESSION_KIND_WRAP_UP).remove(0);
    store
        .call("alice", |ctx| {
            claim_session_request(ctx, row.id, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_session_request_done(ctx, row.id, HOST_A.into())
        })
        .expect("report done");
    row.id
}

fn ready() -> (Store, i64) {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let id = store.create("alice", user_task("t", "alice"));
    dispatch_to_running(&store, id);
    (store, id)
}

#[test]
fn a_rebase_wrap_up_writes_a_requested_row_with_a_timer() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");

    let row = session_rows(&store, id, SESSION_KIND_WRAP_UP).remove(0);
    assert_eq!(row.state, SESSION_REQUESTED);
    assert_eq!(row.action, "rebase");
    assert_eq!(store.db().session_request_timeouts().iter().count(), 1);
    assert_eq!(
        store.task(id).status,
        "running",
        "wrap_up writes nothing to the task"
    );
}

#[test]
fn a_done_wrap_up_is_written_finished_with_no_timer() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("request_wrap_up");

    let row = session_rows(&store, id, SESSION_KIND_WRAP_UP).remove(0);
    assert_eq!(row.state, SESSION_DONE);
    assert_eq!(store.db().session_request_timeouts().iter().count(), 0);
}

#[test]
fn a_second_wrap_up_while_one_is_open_is_refused() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("first");
    assert!(wrap_up(&store, id, "rebase").is_err());
    assert_eq!(session_rows(&store, id, SESSION_KIND_WRAP_UP).len(), 1);
}

#[test]
fn a_wrap_up_for_a_task_that_is_not_running_is_refused() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let id = store.create("alice", user_task("t", "alice"));
    assert!(wrap_up(&store, id, "rebase").is_err());
}

#[test]
fn exit_session_with_the_token_closes_the_task_and_clears_the_window() {
    let (store, id) = ready();
    let token = finished_rebase_wrap_up(&store, id);

    store
        .call("alice", |ctx| {
            request_exit_session(ctx, id, token, "rebase".into(), String::new())
        })
        .expect("request_exit_session");

    let task = store.task(id);
    assert_eq!(task.status, "done");
    assert!(!task.completed_at.is_empty());
    let exit = session_rows(&store, id, SESSION_KIND_EXIT_SESSION).remove(0);
    assert_eq!(exit.wrap_up_request, token, "the row consumed the token");
}

#[test]
fn a_spent_token_cannot_close_twice() {
    let (store, id) = ready();
    let token = finished_rebase_wrap_up(&store, id);
    store
        .call("alice", |ctx| {
            request_exit_session(ctx, id, token, "rebase".into(), String::new())
        })
        .expect("first exit");

    let again = store.call("alice", |ctx| {
        request_exit_session(ctx, id, token, "rebase".into(), String::new())
    });
    assert!(again.is_err());
}

#[test]
fn exit_session_with_a_token_for_another_action_is_refused_and_writes_nothing() {
    let (store, id) = ready();
    let token = finished_rebase_wrap_up(&store, id);

    let refused = store.call("alice", |ctx| {
        request_exit_session(ctx, id, token, "done".into(), String::new())
    });
    assert!(refused.is_err());
    assert_eq!(store.task(id).status, "running");
    assert!(session_rows(&store, id, SESSION_KIND_EXIT_SESSION).is_empty());
}

#[test]
fn closing_a_task_in_an_auto_dispatch_epic_requests_the_next_subtask() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "Chain", true);
    let first = store.create("alice", epic_task("first", epic, "backlog"));
    let second = store.create("alice", epic_task("second", epic, "backlog"));
    dispatch_to_running(&store, first);
    let token = finished_rebase_wrap_up(&store, first);

    store
        .call("alice", |ctx| {
            request_exit_session(ctx, first, token, "rebase".into(), String::new())
        })
        .expect("request_exit_session");

    assert_eq!(store.task(first).status, "done");
    let chained: Vec<_> = store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == second)
        .collect();
    assert_eq!(chained.len(), 1, "the chain requested the next subtask");
    assert_eq!(chained[0].origin, DISPATCH_ORIGIN_EPIC_CHAIN);
    assert_eq!(store.task(second).status, "running");
}

#[test]
fn the_wrap_up_backstop_fails_a_rebase_nobody_claimed() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    let timer = store
        .db()
        .session_request_timeouts()
        .iter()
        .next()
        .expect("the backstop timer row");

    store.advance_secs(WRAP_UP_REBASE_TIMEOUT_SECS + SESSION_REQUEST_BACKSTOP_GRACE_SECS);
    store
        .call_as_scheduler(|ctx| expire_session_request(ctx, timer))
        .expect("the scheduler runs the backstop");

    let row = session_rows(&store, id, SESSION_KIND_WRAP_UP).remove(0);
    assert_eq!(row.state, SESSION_FAILED);
    assert_eq!(row.reason, SESSION_REASON_TIMED_OUT);
}

#[test]
fn a_client_cannot_run_the_session_backstop() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    let timer = store
        .db()
        .session_request_timeouts()
        .iter()
        .next()
        .expect("the backstop timer row");

    assert!(store
        .call("alice", |ctx| expire_session_request(ctx, timer))
        .is_err());
}

#[test]
fn the_session_backstop_leaves_a_request_that_already_finished() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    let timer = store
        .db()
        .session_request_timeouts()
        .iter()
        .next()
        .expect("the backstop timer row");
    let row = session_rows(&store, id, SESSION_KIND_WRAP_UP).remove(0);
    store
        .call("alice", |ctx| {
            claim_session_request(ctx, row.id, HOST_A.into())
        })
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_session_request_done(ctx, row.id, HOST_A.into())
        })
        .expect("done");

    store
        .call_as_scheduler(|ctx| expire_session_request(ctx, timer))
        .expect("a stale timer is harmless");
    let after = session_rows(&store, id, SESSION_KIND_WRAP_UP).remove(0);
    assert_eq!(after.state, SESSION_DONE);
}
