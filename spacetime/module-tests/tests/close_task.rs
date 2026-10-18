//! `close_task`, `patch_task`'s transition rules and the close row they write
//! (`store-split.allium`: RequestCloseTask X1, X2), run against the in-memory
//! store. The pure rules are pinned by the module's own unit tests; here it is
//! the transaction.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// Dispatch `task_id` through the public reducers, leaving it running on
/// `HOST_A` with a worktree and the window `task-<id>`.
fn dispatch_to_running(store: &Store, task_id: i64) {
    store
        .call("alice", |ctx| {
            request_dispatch(ctx, task_id, HOST_A.into(), "user".into(), String::new())
        })
        .expect("request_dispatch");
    let request = store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == task_id)
        .map(|r| r.id)
        .max()
        .expect("the request");
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
                format!("/wt/{task_id}-t"),
                format!("task-{task_id}"),
                false,
            )
        })
        .expect("report provisioned");
}

/// An epic with one running task per entry of `statuses` ("running" or
/// "review"), each with a worktree, a window and `HOST_A`. Both hosts are
/// alice's. Returns the epic and the task ids.
fn wrappable(auto_dispatch: bool, statuses: &[&str]) -> (Store, i64, Vec<i64>) {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    let epic = store.create_epic("alice", "E", auto_dispatch);
    let ids = statuses
        .iter()
        .map(|status| {
            let id = store.create("alice", epic_task("t", epic, "backlog"));
            dispatch_to_running(&store, id);
            if *status == "review" {
                patch(&store, id, status_patch("review")).expect("to review");
            }
            id
        })
        .collect();
    (store, epic, ids)
}

fn close(store: &Store, task: i64, host: &str) -> Result<(), String> {
    store.call("alice", |ctx| close_task(ctx, task, host.into()))
}

fn patch(store: &Store, task: i64, patch: TaskPatch) -> Result<(), String> {
    store.call("alice", |ctx| patch_task(ctx, task, patch))
}

fn current_agent(store: &Store, task: i64) -> Agent {
    store
        .db()
        .agents()
        .iter()
        .find(|a| a.task_id == task && a.state == "current")
        .expect("a current agent")
}

fn session_rows(store: &Store, task: i64, kind: &str) -> Vec<SessionRequest> {
    store
        .db()
        .session_requests()
        .iter()
        .filter(|r| r.task_id == task && r.kind == kind)
        .collect()
}

/// The one row of `kind` the task has.
fn session_row(store: &Store, task: i64, kind: &str) -> SessionRequest {
    let mut rows = session_rows(store, task, kind);
    assert_eq!(rows.len(), 1, "one {kind} row");
    rows.remove(0)
}

fn session_count(store: &Store) -> usize {
    store.db().session_requests().iter().count()
}

fn timers_for(store: &Store, request: i64) -> usize {
    store
        .db()
        .session_request_timeouts()
        .iter()
        .filter(|t| t.request_id == request)
        .count()
}

fn task_count(store: &Store) -> usize {
    store.db().tasks().iter().count()
}

/// A running task with a window closes whole: Done, window cleared, completed,
/// the epic rolled up, and ONE close row with its timer; no token is touched.
#[test]
fn close_task_applies_the_done_close_and_writes_a_close_row() {
    let (store, epic, ids) = wrappable(false, &["running"]);
    let id = ids[0];
    patch(
        &store,
        id,
        TaskPatch {
            stop_pending: Some(true),
            ..TaskPatch::default()
        },
    )
    .expect("stop_pending");

    close(&store, id, HOST_B).expect("close_task");

    let task = store.task(id);
    assert_eq!(task.status, "done");
    assert_eq!(task.sub_status, "none");
    assert_eq!(current_agent(&store, id).tmux_window, "");
    assert_eq!(task.completed_at.len(), 23);
    assert!(!current_agent(&store, id).stop_pending);
    assert_eq!(store.epic(epic).status, "done");

    assert_eq!(session_count(&store), 1);
    let row = session_row(&store, id, SESSION_KIND_CLOSE);
    assert_eq!(row.state, SESSION_REQUESTED);
    assert_eq!(row.action, "done");
    assert_eq!(row.host, HOST_A, "the task's own host, not the argument");
    assert_eq!(row.tmux_window, format!("task-{id}"));
    assert_eq!(row.wrap_up_request, 0);
    assert_eq!(row.chained_task, 0);
    assert_eq!(row.pr_url, "");
    assert_eq!(row.requested_by, identity_of("alice"));
    assert_eq!(row.deadline.len(), 23);
    assert_eq!(timers_for(&store, row.id), 1);
}

/// The teardown of a close row is served like an exit_session's: claim, done,
/// and the timer goes.
#[test]
fn a_close_row_is_claimed_and_reported_like_a_teardown() {
    let (store, _, ids) = wrappable(false, &["running"]);
    let id = ids[0];
    close(&store, id, HOST_A).expect("close_task");
    let row = session_row(&store, id, SESSION_KIND_CLOSE);

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

    assert_eq!(
        session_row(&store, id, SESSION_KIND_CLOSE).state,
        SESSION_DONE
    );
    assert_eq!(timers_for(&store, row.id), 0);
    assert_eq!(store.task(id).status, "done");
}

/// No window and no pr url: nothing for a host to do, so no row.
#[test]
fn close_task_without_host_work_writes_no_row() {
    let (store, _, ids) = wrappable(false, &["review"]);
    let id = ids[0];
    patch(
        &store,
        id,
        TaskPatch {
            tmux_window: Some(String::new()),
            ..TaskPatch::default()
        },
    )
    .expect("clear the window");

    close(&store, id, HOST_A).expect("close_task");
    assert_eq!(store.task(id).status, "done");
    assert_eq!(session_count(&store), 0);
}

/// A pr-typed url alone is host work, on the caller's host when the task has none.
#[test]
fn close_task_with_a_pr_url_and_no_host_writes_a_row_on_the_argument_host() {
    let (store, _, ids) = wrappable(false, &["review"]);
    let id = ids[0];
    patch(
        &store,
        id,
        TaskPatch {
            tmux_window: Some(String::new()),
            host: Some(String::new()),
            url: Some("https://github.com/a/b/pull/1".into()),
            url_type: Some("pr".into()),
            ..TaskPatch::default()
        },
    )
    .expect("clear the host and set the pr");

    close(&store, id, HOST_B).expect("close_task");
    let row = session_row(&store, id, SESSION_KIND_CLOSE);
    assert_eq!(row.host, HOST_B);
    assert_eq!(row.tmux_window, "");
}

/// D11: a teammate's host is not the caller's to tear down. The task still closes.
#[test]
fn close_task_on_a_teammates_host_closes_without_a_row() {
    let (store, _, ids) = wrappable(false, &["running"]);
    let id = ids[0];
    store.register_host_for("their-host", "bob");
    patch(
        &store,
        id,
        TaskPatch {
            host: Some("their-host".into()),
            ..TaskPatch::default()
        },
    )
    .expect("hand the task to their host");

    close(&store, id, HOST_A).expect("close_task");
    assert_eq!(store.task(id).status, "done");
    assert_eq!(current_agent(&store, id).tmux_window, "");
    assert_eq!(session_count(&store), 0);
}

/// A stuck wrap_up does not block closing: it is failed "task closed", its
/// timer cancelled, and the close row written after.
#[test]
fn close_task_fails_the_open_session_request() {
    let (store, _, ids) = wrappable(false, &["running"]);
    let id = ids[0];
    store
        .call("alice", |ctx| {
            request_wrap_up(ctx, id, HOST_A.into(), "rebase".into())
        })
        .expect("request_wrap_up");
    let wrap = session_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(timers_for(&store, wrap.id), 1);

    close(&store, id, HOST_A).expect("close_task");

    let wrap_after = session_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(wrap_after.state, SESSION_FAILED);
    assert_eq!(wrap_after.reason, "task closed");
    assert_eq!(timers_for(&store, wrap.id), 0);
    assert_eq!(
        session_row(&store, id, SESSION_KIND_CLOSE).state,
        SESSION_REQUESTED
    );
    // A late report from the failed rebase is refused.
    let late = store.call("alice", |ctx| {
        report_session_request_done(ctx, wrap.id, HOST_A.into())
    });
    assert!(late.is_err());
}

/// An earlier close still open is failed by a re-close, so at most one is open.
#[test]
fn a_reclose_fails_the_earlier_open_close_row_and_keeps_completed_at() {
    let (store, _, ids) = wrappable(false, &["running"]);
    let id = ids[0];
    close(&store, id, HOST_A).expect("close_task");
    let first = session_row(&store, id, SESSION_KIND_CLOSE).id;
    let completed = store.task(id).completed_at;
    // Give the task a window again, as a hand-edit would, so the re-close has work.
    patch(
        &store,
        id,
        TaskPatch {
            tmux_window: Some("task-x".into()),
            ..TaskPatch::default()
        },
    )
    .expect("a window again");

    close(&store, id, HOST_A).expect("re-close");

    assert_eq!(store.task(id).completed_at, completed);
    let first_row = store
        .db()
        .session_requests()
        .id()
        .find(first)
        .expect("the first close row");
    assert_eq!(first_row.state, SESSION_FAILED);
    assert_eq!(timers_for(&store, first), 0);
    let open = store
        .db()
        .session_requests()
        .iter()
        .filter(|r| r.state == SESSION_REQUESTED)
        .count();
    assert_eq!(open, 1);
}

/// No token and no chain: an auto-dispatch epic's backlog subtask is not requested.
#[test]
fn close_task_runs_no_epic_chain() {
    let (store, epic, ids) = wrappable(true, &["running"]);
    let next = store.create("alice", epic_task("next", epic, "backlog"));

    close(&store, ids[0], HOST_A).expect("close_task");

    let chained = store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == next)
        .count();
    assert_eq!(chained, 0);
    assert_eq!(store.task(next).status, "backlog");
}

#[test]
fn close_task_of_a_missing_task_is_refused() {
    let (store, _, _) = wrappable(false, &["running"]);
    assert!(close(&store, 99, HOST_A).is_err());
    assert_eq!(session_count(&store), 0);
}

/// PhoenixRespawn on a close that enters Done; the flag is the receipt, and a
/// done -> done re-close does not respawn again.
#[test]
fn close_task_respawns_a_phoenix_task_once() {
    let (store, _, ids) = wrappable(false, &["running"]);
    let id = ids[0];
    patch(
        &store,
        id,
        TaskPatch {
            phoenix: Some(true),
            ..TaskPatch::default()
        },
    )
    .expect("phoenix");
    assert_eq!(task_count(&store), 1);

    close(&store, id, HOST_A).expect("close_task");
    assert_eq!(task_count(&store), 2);
    assert!(!store.task(id).phoenix);

    close(&store, id, HOST_A).expect("re-close");
    assert_eq!(task_count(&store), 2);
}

/// X2: a patch out of running clears stop_pending, unless it names the field.
#[test]
fn patch_task_leaving_running_clears_stop_pending() {
    let (store, _, ids) = wrappable(false, &["running", "running"]);
    for id in &ids {
        patch(
            &store,
            *id,
            TaskPatch {
                stop_pending: Some(true),
                ..TaskPatch::default()
            },
        )
        .expect("stop_pending");
    }

    patch(&store, ids[0], status_patch("review")).expect("to review");
    assert!(!current_agent(&store, ids[0]).stop_pending);

    patch(
        &store,
        ids[1],
        TaskPatch {
            status: Some("review".into()),
            stop_pending: Some(true),
            ..TaskPatch::default()
        },
    )
    .expect("to review, naming the field");
    assert!(current_agent(&store, ids[1]).stop_pending);
}

/// X2: a patch into Done from another status respawns a phoenix task, in the
/// same transaction; a feed-owned task and a patch that stays done do not.
#[test]
fn patch_task_into_done_respawns_a_phoenix_task() {
    let (store, _, ids) = wrappable(false, &["running", "running"]);
    let (first, feed_owned) = (ids[0], ids[1]);
    for id in &ids {
        patch(
            &store,
            *id,
            TaskPatch {
                phoenix: Some(true),
                ..TaskPatch::default()
            },
        )
        .expect("phoenix");
    }
    patch(
        &store,
        feed_owned,
        TaskPatch {
            external_id: Some("ext-2".into()),
            ..TaskPatch::default()
        },
    )
    .expect("external id");
    assert_eq!(task_count(&store), 2);

    patch(&store, first, status_patch("done")).expect("to done");
    assert_eq!(task_count(&store), 3);
    assert!(!store.task(first).phoenix);
    let respawn = store
        .db()
        .tasks()
        .iter()
        .find(|t| t.id != first && t.id != feed_owned)
        .expect("the respawned task");
    assert_eq!(respawn.status, "backlog");

    patch(
        &store,
        first,
        TaskPatch {
            title: Some("renamed".into()),
            ..TaskPatch::default()
        },
    )
    .expect("rename");
    assert_eq!(task_count(&store), 3, "no transition, no respawn");

    patch(&store, feed_owned, status_patch("done")).expect("to done");
    assert_eq!(task_count(&store), 3, "a feed-owned task is exempt");
}
