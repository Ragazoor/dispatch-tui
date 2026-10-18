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
        .filter(|r| r.task_id == task_id)
        .map(|r| r.id)
        .max()
        .expect("the request");
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

fn patch(store: &Store, id: i64, patch: TaskPatch) {
    store
        .call("alice", |ctx| patch_task(ctx, id, patch))
        .expect("patch_task");
}

fn claim(store: &Store, row: i64, host: &str) -> Result<(), String> {
    store.call("alice", |ctx| claim_session_request(ctx, row, host.into()))
}

fn report_done(store: &Store, row: i64, host: &str) -> Result<(), String> {
    store.call("alice", |ctx| {
        report_session_request_done(ctx, row, host.into())
    })
}

fn report_failed(
    store: &Store,
    row: i64,
    host: &str,
    reason: &str,
    conflict: bool,
) -> Result<(), String> {
    store.call("alice", |ctx| {
        report_session_request_failed(ctx, row, host.into(), reason.into(), conflict)
    })
}

fn exit_session(
    store: &Store,
    task_id: i64,
    token: i64,
    action: &str,
    pr_url: &str,
) -> Result<(), String> {
    store.call("alice", |ctx| {
        request_exit_session(ctx, task_id, token, action.into(), pr_url.into())
    })
}

fn only_row(store: &Store, task_id: i64, kind: &str) -> SessionRequest {
    session_rows(store, task_id, kind).remove(0)
}

fn timers_for(store: &Store, request: i64) -> usize {
    store
        .db()
        .session_request_timeouts()
        .iter()
        .filter(|t| t.request_id == request)
        .count()
}

fn timer_count(store: &Store) -> usize {
    store.db().session_request_timeouts().iter().count()
}

fn current_window(store: &Store, task_id: i64) -> String {
    store
        .db()
        .agents()
        .iter()
        .find(|a| a.task_id == task_id && a.state == "current")
        .map(|a| a.tmux_window)
        .unwrap_or_default()
}

fn set_sub_status(store: &Store, id: i64, sub_status: &str) {
    patch(
        store,
        id,
        TaskPatch {
            sub_status: Some(sub_status.into()),
            ..TaskPatch::default()
        },
    );
}

/// A running task inside a fresh epic, on `HOST_A`.
fn ready_in_epic(auto_dispatch: bool) -> (Store, i64, i64) {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "E", auto_dispatch);
    let id = store.create("alice", epic_task("t", epic, "backlog"));
    dispatch_to_running(&store, id);
    (store, epic, id)
}

/// A backlog subtask of `epic`, in board position `order`.
fn backlog_subtask(store: &Store, epic: i64, order: i64, phoenix: bool) -> i64 {
    let id = store.create("alice", epic_task("next", epic, "backlog"));
    patch(
        store,
        id,
        TaskPatch {
            sort_order: Some(Some(order)),
            phoenix: Some(phoenix),
            ..TaskPatch::default()
        },
    );
    id
}

#[test]
fn a_rebase_wrap_up_row_names_its_host_and_requester_and_the_task_is_untouched() {
    let (store, id) = ready();
    let before = store.task(id).updated_at;
    wrap_up(&store, id, "rebase").expect("request_wrap_up");

    let row = only_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(row.host, HOST_A);
    assert_eq!(row.requested_by, identity_of("alice"));
    assert!(!row.requested_at.is_empty());
    assert!(!row.deadline.is_empty());
    assert!(row.claimed_at.is_empty());
    assert!(row.finished_at.is_empty());
    assert_eq!(timers_for(&store, row.id), 1);
    assert_eq!(store.task(id).updated_at, before);
}

#[test]
fn a_pr_wrap_up_is_written_finished_with_no_timer() {
    let (store, id) = ready();
    wrap_up(&store, id, "pr").expect("request_wrap_up");

    let row = only_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(row.state, SESSION_DONE);
    assert_eq!(row.action, "pr");
    assert!(!row.finished_at.is_empty());
    assert!(row.claimed_at.is_empty());
    assert_eq!(timers_for(&store, row.id), 0);
}

#[test]
fn each_wrap_up_refusal_names_why_and_writes_nothing() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    let backlog = store.create("alice", user_task("backlog", "alice"));
    let tagged: Vec<i64> = ["pr-review", "dependabot", "release-please"]
        .iter()
        .map(|tag| {
            let id = store.create("alice", user_task(tag, "alice"));
            dispatch_to_running(&store, id);
            patch(
                &store,
                id,
                TaskPatch {
                    tag: Some((*tag).into()),
                    ..TaskPatch::default()
                },
            );
            id
        })
        .collect();
    let running = store.create("alice", user_task("running", "alice"));
    dispatch_to_running(&store, running);
    let done = store.create("alice", user_task("done", "alice"));
    dispatch_to_running(&store, done);
    patch(&store, done, status_patch("done"));

    let mut cases = vec![
        (backlog, HOST_A, "cannot be wrapped up"),
        (running, HOST_B, HOST_A),
        (done, HOST_A, "cannot be wrapped up"),
        (99, HOST_A, "99"),
    ];
    cases.extend(tagged.iter().map(|id| (*id, HOST_A, "retag")));
    for (task, host, why) in cases {
        let refused = store
            .call("alice", |ctx| {
                request_wrap_up(ctx, task, host.into(), "rebase".into())
            })
            .expect_err("a refusal");
        assert!(
            refused.contains(why),
            "task {task} must say {why:?}: {refused}"
        );
    }
    let unknown = store.call("alice", |ctx| {
        request_wrap_up(ctx, running, HOST_A.into(), "merge".into())
    });
    assert!(unknown.is_err());
    assert_eq!(store.db().session_requests().iter().count(), 0);
    assert_eq!(timer_count(&store), 0);
}

#[test]
fn a_second_wrap_up_is_refused_until_the_first_finishes() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("first");
    let refused = wrap_up(&store, id, "done").expect_err("a second");
    assert!(refused.contains("open session request"), "{refused}");

    let row = only_row(&store, id, SESSION_KIND_WRAP_UP);
    claim(&store, row.id, HOST_A).expect("claim");
    report_done(&store, row.id, HOST_A).expect("done");
    wrap_up(&store, id, "done").expect("a new one after the first finished");
}

#[test]
fn the_claim_is_the_rows_hosts_alone_once_and_clears_a_conflict() {
    let (store, id) = ready();
    set_sub_status(&store, id, "conflict");
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    let row = only_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(
        store.task(id).sub_status,
        "conflict",
        "the request alone does not clear it"
    );

    store.register_host_for(HOST_B, "alice");
    assert!(claim(&store, row.id, HOST_B).is_err());
    assert_eq!(
        only_row(&store, id, SESSION_KIND_WRAP_UP).state,
        SESSION_REQUESTED
    );
    assert_eq!(store.task(id).sub_status, "conflict");

    claim(&store, row.id, HOST_A).expect("claim");
    let claimed = only_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(claimed.state, SESSION_RUNNING);
    assert!(!claimed.claimed_at.is_empty());
    assert_eq!(store.task(id).sub_status, "active");

    assert!(claim(&store, row.id, HOST_A).is_err(), "only once");
}

#[test]
fn a_done_report_finishes_the_row_cancels_the_timer_and_may_repeat() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    let row = only_row(&store, id, SESSION_KIND_WRAP_UP);

    assert!(report_done(&store, row.id, HOST_A).is_err(), "unclaimed");
    claim(&store, row.id, HOST_A).expect("claim");
    store.register_host_for(HOST_B, "alice");
    assert!(report_done(&store, row.id, HOST_B).is_err(), "another host");
    assert_eq!(
        only_row(&store, id, SESSION_KIND_WRAP_UP).state,
        SESSION_RUNNING
    );

    report_done(&store, row.id, HOST_A).expect("done");
    let done = only_row(&store, id, SESSION_KIND_WRAP_UP);
    assert_eq!(done.state, SESSION_DONE);
    assert!(!done.finished_at.is_empty());
    assert_eq!(timers_for(&store, row.id), 0);

    store.advance_secs(5);
    report_done(&store, row.id, HOST_A).expect("a repeat is accepted");
    assert_eq!(
        only_row(&store, id, SESSION_KIND_WRAP_UP).finished_at,
        done.finished_at
    );
}

#[test]
fn a_failed_report_records_the_reason_and_a_conflict_sets_the_sub_status() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let ids: Vec<i64> = (0..2)
        .map(|_| {
            let id = store.create("alice", user_task("t", "alice"));
            dispatch_to_running(&store, id);
            wrap_up(&store, id, "rebase").expect("request_wrap_up");
            let row = only_row(&store, id, SESSION_KIND_WRAP_UP);
            claim(&store, row.id, HOST_A).expect("claim");
            id
        })
        .collect();
    let first = only_row(&store, ids[0], SESSION_KIND_WRAP_UP).id;
    let second = only_row(&store, ids[1], SESSION_KIND_WRAP_UP).id;

    report_failed(&store, first, HOST_A, "conflict in src/a.rs", true).expect("conflict");
    let row = only_row(&store, ids[0], SESSION_KIND_WRAP_UP);
    assert_eq!(row.state, SESSION_FAILED);
    assert_eq!(row.reason, "conflict in src/a.rs");
    assert_eq!(store.task(ids[0]).sub_status, "conflict");
    assert_eq!(timers_for(&store, first), 0);

    report_failed(&store, second, HOST_A, "dirty primary worktree", false).expect("plain");
    assert_eq!(store.task(ids[1]).sub_status, "active");

    assert!(report_done(&store, first, HOST_A).is_err(), "a late report");
    assert_eq!(
        only_row(&store, ids[0], SESSION_KIND_WRAP_UP).state,
        SESSION_FAILED
    );
}

#[test]
fn a_conflict_report_leaves_a_task_that_moved_on_alone() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    let row = only_row(&store, id, SESSION_KIND_WRAP_UP);
    claim(&store, row.id, HOST_A).expect("claim");
    patch(&store, id, status_patch("done"));
    set_sub_status(&store, id, "none");

    report_failed(&store, row.id, HOST_A, "conflict", true).expect("report");
    assert_eq!(store.task(id).sub_status, "none");
}

#[test]
fn an_exit_session_applies_the_close_and_writes_the_teardown_row() {
    let (store, epic, id) = ready_in_epic(false);
    let token = finished_rebase_wrap_up(&store, id);

    exit_session(&store, id, token, "rebase", "").expect("request_exit_session");

    let task = store.task(id);
    assert_eq!(task.status, "done");
    assert_eq!(task.sub_status, "none");
    assert_eq!(current_window(&store, id), "");
    assert_eq!(task.worktree, format!("/wt/{id}"));
    assert!(!task.completed_at.is_empty());
    assert_eq!(
        store.epic(epic).status,
        "done",
        "the epic rolls up in the same transaction"
    );

    let exit = only_row(&store, id, SESSION_KIND_EXIT_SESSION);
    assert_eq!(exit.state, SESSION_REQUESTED);
    assert_eq!(exit.host, HOST_A);
    assert_eq!(exit.action, "rebase");
    assert_eq!(exit.tmux_window, format!("task-{id}"));
    assert_eq!(exit.wrap_up_request, token);
    assert_eq!(exit.chained_task, 0);
    assert_eq!(exit.requested_by, identity_of("alice"));
    assert!(!exit.deadline.is_empty());
    assert_eq!(timers_for(&store, exit.id), 1);
}

#[test]
fn a_pr_exit_session_takes_the_task_to_review_with_the_url() {
    let (store, id) = ready();
    wrap_up(&store, id, "pr").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    let url = "https://github.com/acme/repo/pull/7";

    exit_session(&store, id, token, "pr", url).expect("request_exit_session");

    let task = store.task(id);
    assert_eq!(task.status, "review");
    assert_eq!(task.sub_status, "awaiting_review");
    assert_eq!(task.url, url);
    assert_eq!(task.url_type, "pr");
    assert_eq!(current_window(&store, id), "");
    assert!(task.completed_at.is_empty());
    assert_eq!(only_row(&store, id, SESSION_KIND_EXIT_SESSION).pr_url, url);
}

#[test]
fn an_exit_token_is_consumed_by_the_close() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    exit_session(&store, id, token, "done", "").expect("first");
    // Finish the teardown so the open-request check is not what refuses.
    let exit = only_row(&store, id, SESSION_KIND_EXIT_SESSION).id;
    claim(&store, exit, HOST_A).expect("claim");
    report_done(&store, exit, HOST_A).expect("done");

    let again = exit_session(&store, id, token, "done", "").expect_err("a spent token");
    assert!(
        again.contains("no exit token — call wrap_up first"),
        "{again}"
    );
    assert_eq!(store.db().session_requests().iter().count(), 2);
}

#[test]
fn a_refused_exit_session_leaves_the_task_and_the_token_untouched() {
    let (store, id) = ready();
    wrap_up(&store, id, "pr").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    let rows_before = store.db().session_requests().iter().count();

    for (action, token_arg, url, why) in [
        ("pr", 999, "https://x/pull/1", "invalid exit token"),
        (
            "rebase",
            token,
            "",
            "exit token was issued for wrap_up(action=\"pr\"), but exit_session was called with action=\"rebase\"",
        ),
        ("pr", token, "", "pr_url is required for action 'pr'"),
    ] {
        let refused = exit_session(&store, id, token_arg, action, url).expect_err(why);
        assert!(refused.contains(why), "{why}: {refused}");
    }
    assert_eq!(store.db().session_requests().iter().count(), rows_before);
    assert_eq!(store.task(id).status, "running");
    assert_eq!(current_window(&store, id), format!("task-{id}"));

    exit_session(&store, id, token, "pr", "https://x/pull/1").expect("the same token closes");
    assert_eq!(store.task(id).status, "review");
}

#[test]
fn an_exit_session_without_a_token_or_a_window_is_refused() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let one = store.create("alice", user_task("one", "alice"));
    let two = store.create("alice", user_task("two", "alice"));
    dispatch_to_running(&store, one);
    dispatch_to_running(&store, two);

    let none = exit_session(&store, one, 5, "done", "").expect_err("no token");
    assert!(
        none.contains("no exit token — call wrap_up first"),
        "{none}"
    );

    wrap_up(&store, two, "done").expect("request_wrap_up");
    let token = only_row(&store, two, SESSION_KIND_WRAP_UP).id;
    // The window went away by some other path (the TUI, crash detection).
    patch(
        &store,
        two,
        TaskPatch {
            tmux_window: Some(String::new()),
            ..TaskPatch::default()
        },
    );
    let closed = exit_session(&store, two, token, "done", "").expect_err("no window");
    assert!(
        closed.contains(&format!("task #{two} has no active session")),
        "{closed}"
    );
    assert_eq!(store.task(two).status, "running");
}

#[test]
fn a_newer_wrap_up_supersedes_the_older_token() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("first");
    let old = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    store.advance_secs(1);
    wrap_up(&store, id, "pr").expect("newer");
    let new = session_rows(&store, id, SESSION_KIND_WRAP_UP)
        .into_iter()
        .find(|r| r.action == "pr")
        .expect("the pr row")
        .id;

    let refused = exit_session(&store, id, old, "done", "").expect_err("the old token");
    assert!(refused.contains("invalid exit token"), "{refused}");
    exit_session(&store, id, new, "pr", "https://x/pull/1").expect("the new token closes");
}

#[test]
fn a_later_dispatch_voids_the_exit_token() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    patch(&store, id, status_patch("backlog"));
    set_sub_status(&store, id, "none");
    dispatch_to_running(&store, id);

    let refused = exit_session(&store, id, token, "done", "").expect_err("a voided token");
    assert!(refused.contains("invalid exit token"), "{refused}");
    assert_eq!(store.task(id).status, "running");
}

#[test]
fn an_open_session_request_refuses_the_close() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    // A second wrap_up (rebase) that is still open is not a done row, so it
    // does not supersede the token; it holds the task instead.
    wrap_up(&store, id, "rebase").expect("a rebase");

    assert!(exit_session(&store, id, token, "done", "").is_err());
    assert_eq!(store.task(id).status, "running");
}

#[test]
fn a_teammate_may_not_act_for_my_hosts_session_requests() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let one = store.create("alice", user_task("one", "alice"));
    let two = store.create("alice", user_task("two", "alice"));
    dispatch_to_running(&store, one);
    dispatch_to_running(&store, two);
    store.register_host_for("their-host", "bob");
    wrap_up(&store, one, "rebase").expect("request_wrap_up");
    let row = only_row(&store, one, SESSION_KIND_WRAP_UP).id;
    wrap_up(&store, two, "done").expect("request_wrap_up");
    let token = only_row(&store, two, SESSION_KIND_WRAP_UP).id;

    let teammate =
        |f: &dyn Fn(&spacetimedb::ReducerContext) -> Result<(), String>| store.call("bob", f);
    let refused = [
        teammate(&|ctx| request_wrap_up(ctx, two, HOST_A.into(), "done".into())),
        teammate(&|ctx| request_wrap_up(ctx, two, "their-host".into(), "done".into())),
        teammate(&|ctx| claim_session_request(ctx, row, HOST_A.into())),
        teammate(&|ctx| report_session_request_done(ctx, row, HOST_A.into())),
        teammate(&|ctx| report_session_request_failed(ctx, row, HOST_A.into(), "x".into(), false)),
        teammate(&|ctx| request_exit_session(ctx, two, token, "done".into(), String::new())),
        teammate(&|ctx| fail_interrupted_session_requests(ctx, HOST_A.into())),
    ];
    for out in refused {
        assert!(out.is_err());
    }
    assert_eq!(
        only_row(&store, one, SESSION_KIND_WRAP_UP).state,
        SESSION_REQUESTED
    );
    assert_eq!(store.task(two).status, "running");

    let ownerless = store
        .call("alice", |ctx| {
            request_wrap_up(ctx, two, "nowhere".into(), "done".into())
        })
        .expect_err("an unregistered host");
    assert!(ownerless.contains("not registered"), "{ownerless}");
}

#[test]
fn a_failed_teardown_does_not_revert_the_close() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    exit_session(&store, id, token, "done", "").expect("close");
    let exit = only_row(&store, id, SESSION_KIND_EXIT_SESSION).id;

    claim(&store, exit, HOST_A).expect("claim");
    assert_eq!(store.task(id).sub_status, "none");
    report_failed(&store, exit, HOST_A, "kill failed", false).expect("failed");

    let row = only_row(&store, id, SESSION_KIND_EXIT_SESSION);
    assert_eq!(row.state, SESSION_FAILED);
    assert_eq!(row.reason, "kill failed");
    assert_eq!(store.task(id).status, "done");
    assert_eq!(timers_for(&store, exit), 0);
    assert!(report_failed(&store, exit, HOST_A, "again", true).is_err());
}

#[test]
fn a_teardown_reported_done_finishes_the_row_and_cancels_the_timer() {
    let (store, id) = ready();
    wrap_up(&store, id, "done").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;
    exit_session(&store, id, token, "done", "").expect("close");
    let exit = only_row(&store, id, SESSION_KIND_EXIT_SESSION).id;
    claim(&store, exit, HOST_A).expect("claim");

    report_done(&store, exit, HOST_A).expect("done");
    assert_eq!(
        only_row(&store, id, SESSION_KIND_EXIT_SESSION).state,
        SESSION_DONE
    );
    assert_eq!(timers_for(&store, exit), 0);
}

#[test]
fn an_exit_session_chains_the_next_subtask_passing_over_a_phoenix_one() {
    let (store, epic, id) = ready_in_epic(true);
    let phoenix = backlog_subtask(&store, epic, 1, true);
    let next = backlog_subtask(&store, epic, 2, false);
    let later = backlog_subtask(&store, epic, 3, false);
    let token = finished_rebase_wrap_up(&store, id);

    exit_session(&store, id, token, "rebase", "").expect("request_exit_session");

    assert_eq!(
        only_row(&store, id, SESSION_KIND_EXIT_SESSION).chained_task,
        next
    );
    let chained: Vec<_> = store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == next)
        .collect();
    assert_eq!(chained.len(), 1);
    assert_eq!(chained[0].origin, DISPATCH_ORIGIN_EPIC_CHAIN);
    assert_eq!(chained[0].host, HOST_A);
    assert_eq!(chained[0].state, DISPATCH_REQUESTED);
    assert_eq!(chained[0].requested_by, identity_of("alice"));
    assert_eq!(store.task(next).status, "running");
    assert_eq!(store.task(phoenix).status, "backlog");
    assert_eq!(store.task(later).status, "backlog");
    // The first dispatch's request is finished, so only the chained one is open.
    let open: Vec<_> = store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id != id)
        .collect();
    assert_eq!(open.len(), 1);
    assert_eq!(
        store
            .db()
            .dispatch_request_timeouts()
            .iter()
            .filter(|t| t.request_id == chained[0].id)
            .count(),
        1
    );
}

#[test]
fn the_chain_stops_quietly_without_auto_dispatch_or_candidates() {
    let (off, epic, id) = ready_in_epic(false);
    let waiting = backlog_subtask(&off, epic, 2, false);
    wrap_up(&off, id, "done").expect("request_wrap_up");
    let token = only_row(&off, id, SESSION_KIND_WRAP_UP).id;
    exit_session(&off, id, token, "done", "").expect("close");
    assert_eq!(off.task(waiting).status, "backlog");
    assert!(off.db().dispatch_requests().iter().all(|r| r.task_id == id));
    assert_eq!(
        only_row(&off, id, SESSION_KIND_EXIT_SESSION).chained_task,
        0
    );

    let (only_phoenix, epic, id) = ready_in_epic(true);
    let phoenix = backlog_subtask(&only_phoenix, epic, 1, true);
    wrap_up(&only_phoenix, id, "done").expect("request_wrap_up");
    let token = only_row(&only_phoenix, id, SESSION_KIND_WRAP_UP).id;
    exit_session(&only_phoenix, id, token, "done", "").expect("close");
    assert_eq!(only_phoenix.task(phoenix).status, "backlog");
    assert!(only_phoenix
        .db()
        .dispatch_requests()
        .iter()
        .all(|r| r.task_id == id));
    assert_eq!(only_phoenix.task(id).status, "done");
}

#[test]
fn the_chain_passes_over_a_subtask_on_another_host() {
    let (store, epic, id) = ready_in_epic(true);
    store.register_host_for(HOST_B, "alice");
    let elsewhere = backlog_subtask(&store, epic, 1, false);
    patch(
        &store,
        elsewhere,
        TaskPatch {
            host: Some(HOST_B.into()),
            ..TaskPatch::default()
        },
    );
    let next = backlog_subtask(&store, epic, 2, false);
    wrap_up(&store, id, "done").expect("request_wrap_up");
    let token = only_row(&store, id, SESSION_KIND_WRAP_UP).id;

    exit_session(&store, id, token, "done", "").expect("close");
    assert_eq!(
        only_row(&store, id, SESSION_KIND_EXIT_SESSION).chained_task,
        next
    );
    assert_eq!(store.task(elsewhere).status, "backlog");
}

#[test]
fn a_phoenix_task_respawns_with_a_done_close_but_not_a_pr_close() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "E", false);
    let ids: Vec<i64> = (0..2)
        .map(|_| {
            let id = store.create("alice", epic_task("weekly", epic, "backlog"));
            dispatch_to_running(&store, id);
            patch(
                &store,
                id,
                TaskPatch {
                    phoenix: Some(true),
                    tag: Some("chore".into()),
                    ..TaskPatch::default()
                },
            );
            id
        })
        .collect();
    let tasks = || store.db().tasks().iter().count();

    wrap_up(&store, ids[1], "pr").expect("request_wrap_up");
    let token = only_row(&store, ids[1], SESSION_KIND_WRAP_UP).id;
    exit_session(&store, ids[1], token, "pr", "https://x/pull/1").expect("pr close");
    assert_eq!(
        tasks(),
        2,
        "a pr close lands in review and does not respawn"
    );
    assert!(store.task(ids[1]).phoenix);

    wrap_up(&store, ids[0], "done").expect("request_wrap_up");
    let token = only_row(&store, ids[0], SESSION_KIND_WRAP_UP).id;
    exit_session(&store, ids[0], token, "done", "").expect("done close");
    assert_eq!(tasks(), 3);
    assert!(!store.task(ids[0]).phoenix, "the flag is the receipt");
    let successor = store
        .db()
        .tasks()
        .iter()
        .find(|t| !ids.contains(&t.id))
        .expect("the successor");
    assert_eq!(successor.status, "backlog");
    assert!(successor.phoenix);
}

#[test]
fn a_restart_fails_only_this_hosts_running_session_requests() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let ids: Vec<i64> = (0..2)
        .map(|_| {
            let id = store.create("alice", user_task("t", "alice"));
            dispatch_to_running(&store, id);
            wrap_up(&store, id, "rebase").expect("request_wrap_up");
            id
        })
        .collect();
    let first = only_row(&store, ids[0], SESSION_KIND_WRAP_UP).id;
    claim(&store, first, HOST_A).expect("claim");

    store
        .call("alice", |ctx| {
            fail_interrupted_session_requests(ctx, HOST_A.into())
        })
        .expect("sweep");

    let swept = only_row(&store, ids[0], SESSION_KIND_WRAP_UP);
    assert_eq!(swept.state, SESSION_FAILED);
    assert_eq!(swept.reason, "host restarted");
    assert_eq!(timers_for(&store, first), 0);
    assert_eq!(
        only_row(&store, ids[1], SESSION_KIND_WRAP_UP).state,
        SESSION_REQUESTED
    );

    store.register_host_for(HOST_B, "alice");
    store
        .call("alice", |ctx| {
            fail_interrupted_session_requests(ctx, HOST_B.into())
        })
        .expect("sweep of another host");
    assert_eq!(
        only_row(&store, ids[1], SESSION_KIND_WRAP_UP).state,
        SESSION_REQUESTED
    );
}

#[test]
fn deleting_a_task_fails_its_open_session_request_and_cancels_its_timer() {
    let (store, id) = ready();
    wrap_up(&store, id, "rebase").expect("request_wrap_up");
    patch(&store, id, status_patch("done"));

    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");

    assert!(store.db().tasks().id().find(id).is_none());
    // The row failed in the same reducer, then went with its task.
    assert!(session_rows(&store, id, SESSION_KIND_WRAP_UP).is_empty());
    assert_eq!(timer_count(&store), 0);
}

#[test]
fn batch_delete_fails_the_open_session_requests_of_the_tasks_it_deletes() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "E", false);
    let ids: Vec<i64> = (0..2)
        .map(|_| {
            let id = store.create("alice", epic_task("t", epic, "backlog"));
            dispatch_to_running(&store, id);
            wrap_up(&store, id, "rebase").expect("request_wrap_up");
            patch(&store, id, status_patch("done"));
            id
        })
        .collect();

    store
        .call("alice", |ctx| batch_delete(ctx, vec![ids[0]], vec![]))
        .expect("the task arm");
    assert!(session_rows(&store, ids[0], SESSION_KIND_WRAP_UP).is_empty());
    assert_eq!(
        only_row(&store, ids[1], SESSION_KIND_WRAP_UP).state,
        SESSION_REQUESTED
    );

    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![epic]))
        .expect("the epic arm");
    assert_eq!(store.db().session_requests().iter().count(), 0);
    assert_eq!(timer_count(&store), 0);
}

#[test]
fn a_feed_ingest_that_deletes_a_stale_task_fails_its_open_session_request() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let epic = store.create_epic("alice", "Feed", false);
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                epic,
                EpicPatch {
                    feed_command: Some("echo []".into()),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("patch_epic");
    let item = FeedIngestItem {
        item: FeedTaskUpsertItem {
            external_id: "ext-1".into(),
            title: "one".into(),
            description: String::new(),
            repo_path: "/repo".into(),
            status: "backlog".into(),
            sub_status: String::new(),
            base_branch: String::new(),
            tag: String::new(),
            labels: String::new(),
            sort_order: None,
            url: String::new(),
            url_type: String::new(),
            wrap_up_mode: String::new(),
        },
        signals: vec![],
    };
    let ingest = |items: Vec<FeedIngestItem>| {
        store.call("alice", |ctx| {
            ingest_feed(ctx, epic, items, false, "test-creator".into())
        })
    };
    ingest(vec![item]).expect("first ingest");
    let task = store
        .db()
        .tasks()
        .iter()
        .find(|t| t.external_id == "ext-1")
        .expect("the feed task")
        .id;
    dispatch_to_running(&store, task);
    wrap_up(&store, task, "rebase").expect("request_wrap_up");

    ingest(vec![]).expect("second ingest");

    assert!(store.db().tasks().id().find(task).is_none());
    assert!(session_rows(&store, task, SESSION_KIND_WRAP_UP).is_empty());
    assert_eq!(timer_count(&store), 0);
}
