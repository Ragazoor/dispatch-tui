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

/// The request row for `task_id`.
fn request_for(store: &Store, task_id: i64) -> DispatchRequest {
    store
        .db()
        .dispatch_requests()
        .iter()
        .find(|r| r.task_id == task_id)
        .expect("a request for the task")
}

fn request_as(
    store: &Store,
    who: &str,
    task_id: i64,
    host: &str,
    origin: &str,
    mode: &str,
) -> Result<(), String> {
    store.call(who, |ctx| {
        request_dispatch(ctx, task_id, host.into(), origin.into(), mode.into())
    })
}

fn claim(store: &Store, who: &str, req: i64, host: &str) -> Result<(), String> {
    store.call(who, |ctx| claim_dispatch_request(ctx, req, host.into()))
}

fn report_dispatched(
    store: &Store,
    who: &str,
    req: i64,
    host: &str,
    worktree: &str,
) -> Result<(), String> {
    store.call(who, |ctx| {
        report_dispatch_provisioned(ctx, req, host.into(), worktree.into(), "task-1".into(), false)
    })
}

fn report_failed(store: &Store, who: &str, req: i64, host: &str) -> Result<(), String> {
    store.call(who, |ctx| {
        report_dispatch_failed(ctx, req, host.into(), "boom".into())
    })
}

fn sweep(store: &Store, who: &str, host: &str) -> Result<(), String> {
    store.call(who, |ctx| {
        fail_interrupted_dispatch_requests(ctx, host.into())
    })
}

fn patch(store: &Store, id: i64, patch: TaskPatch) {
    store
        .call("alice", |ctx| patch_task(ctx, id, patch))
        .expect("patch_task");
}

fn timer_count(store: &Store, req: i64) -> usize {
    store
        .db()
        .dispatch_request_timeouts()
        .iter()
        .filter(|t| t.request_id == req)
        .count()
}

/// Alice's tasks inside a fresh epic, plus the epic's id.
fn epic_with_tasks(store: &Store, count: usize) -> (i64, Vec<i64>) {
    let epic = store.create_epic("alice", "E", false);
    let ids = (0..count)
        .map(|_| store.create("alice", epic_task("t", epic, "backlog")))
        .collect();
    (epic, ids)
}

/// Leave the epic `done` over its unfinished subtask, as a rollup that never
/// ran would; any rollup returns it to backlog.
fn make_epic_stale(store: &Store, epic: i64) {
    let mut row = store.epic(epic);
    row.status = "done".into();
    store
        .call("alice", |ctx| seed_epics(ctx, vec![row]))
        .expect("seed_epics");
    assert_eq!(store.epic(epic).status, "done");
}

/// A request claimed by `HOST_A`, left provisioning, under a stale epic.
fn provisioning_under_a_stale_epic(store: &Store) -> (i64, i64) {
    store.register_host_for(HOST_A, "alice");
    let (epic, ids) = epic_with_tasks(store, 1);
    request_as(store, "alice", ids[0], HOST_A, "user", "").expect("request");
    let req = request_for(store, ids[0]).id;
    claim(store, "alice", req, HOST_A).expect("claim");
    make_epic_stale(store, epic);
    (epic, req)
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

#[test]
fn a_request_stores_its_deadline_sixty_seconds_after_it_was_requested() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");

    let row = only_request(&store);
    assert_eq!(row.kind, DISPATCH_REQUEST_KIND);
    assert_eq!(row.host, HOST_A);
    assert_eq!(row.origin, "user");
    assert_eq!(row.mode, "dispatch");
    assert_eq!(row.requested_by, identity_of("alice"));
    assert_eq!(row.claimed_at, "");
    assert_eq!(row.finished_at, "");
    assert_eq!(row.requested_at.len(), 23, "the store's timestamp spelling");
    let format = "%Y-%m-%d %H:%M:%S%.3f";
    let parse = |s: &str| {
        chrono::NaiveDateTime::parse_from_str(s, format)
            .unwrap_or_else(|e| panic!("{s:?} is not a store timestamp: {e}"))
    };
    assert_eq!(
        parse(&row.deadline) - parse(&row.requested_at),
        chrono::Duration::seconds(DISPATCH_REQUEST_TIMEOUT_SECS)
    );
}

#[test]
fn a_request_clears_the_tasks_subagent_state() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request_dispatch");
    let req = only_request(&store).id;
    claim(&store, "alice", req, HOST_A).expect("claim");
    report_dispatched(&store, "alice", req, HOST_A, "/wt/1-t").expect("report");
    store
        .call("alice", |ctx| {
            subagent_start(
                ctx,
                id,
                "agent-1".into(),
                "session-1".into(),
                "2026-10-10T10:00:00Z".into(),
            )
        })
        .expect("subagent_start");
    assert_eq!(store.db().task_subagents().iter().count(), 1);

    // Back to backlog by hand, then a fresh request: the claim clears them.
    patch(&store, id, status_patch("backlog"));
    request(&store, id).expect("second request");

    assert_eq!(store.db().task_subagents().iter().count(), 0);
    let agent = store
        .db()
        .agents()
        .iter()
        .find(|a| a.task_id == id && a.state != "ended")
        .expect("the new agent");
    assert_eq!(agent.live_subagents, 0);
    assert!(!agent.stop_pending);
}

#[test]
fn a_request_for_a_task_on_another_host_is_refused_and_writes_nothing() {
    let store = Store::new();
    let id = host_and_task(&store);
    store.register_host_for(HOST_B, "alice");
    patch(
        &store,
        id,
        TaskPatch {
            host: Some(HOST_A.into()),
            ..TaskPatch::default()
        },
    );

    assert!(request_as(&store, "alice", id, HOST_B, "user", "").is_err());
    assert_eq!(store.task(id).status, "backlog");
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.db().dispatch_request_timeouts().iter().count(), 0);

    request(&store, id).expect("its own host takes it");
}

#[test]
fn a_second_request_says_the_task_is_already_claimed() {
    let store = Store::new();
    let id = host_and_task(&store);
    store.register_host_for(HOST_B, "alice");
    request(&store, id).expect("first");
    let refused = request_as(&store, "alice", id, HOST_B, "mcp", "").unwrap_err();
    assert!(refused.contains("already claimed"), "{refused}");
}

#[test]
fn a_backlog_task_with_an_open_request_is_refused() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("first");
    patch(&store, id, status_patch("backlog"));

    let refused = request(&store, id).unwrap_err();
    assert!(refused.contains("open dispatch request"), "{refused}");
    assert_eq!(store.db().dispatch_requests().iter().count(), 1);
}

#[test]
fn the_request_mode_is_derived_when_omitted_and_kept_when_named() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let make = |tag: &str, plan: &str| {
        store.create(
            "alice",
            Task {
                tag: tag.into(),
                plan_path: plan.into(),
                ..user_task("t", "alice")
            },
        )
    };
    let plain = make("", "");
    let research = make("research", "");
    let planned = make("research", "/plans/p.md");
    let quick = make("", "");
    let named = make("", "");
    for (task, origin, mode) in [
        (plain, "user", ""),
        (research, "user", ""),
        (planned, "user", ""),
        (quick, "quick", "quick"),
        (named, "user", "research"),
    ] {
        request_as(&store, "alice", task, HOST_A, origin, mode).expect("request");
    }
    assert_eq!(request_for(&store, plain).mode, "dispatch");
    assert_eq!(request_for(&store, research).mode, "research");
    assert_eq!(request_for(&store, planned).mode, "dispatch");
    assert_eq!(request_for(&store, quick).mode, "quick");
    assert_eq!(request_for(&store, quick).origin, "quick");
    assert_eq!(request_for(&store, named).mode, "research");
}

#[test]
fn only_the_rows_host_claims_a_request_and_only_once() {
    let store = Store::new();
    let id = host_and_task(&store);
    store.register_host_for(HOST_B, "alice");
    request(&store, id).expect("request");
    let req = only_request(&store).id;

    assert!(claim(&store, "alice", req, HOST_B).is_err());
    assert_eq!(only_request(&store).state, DISPATCH_REQUESTED);

    claim(&store, "alice", req, HOST_A).expect("claim");
    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_PROVISIONING);
    assert_eq!(row.claimed_at.len(), 23);

    assert!(claim(&store, "alice", req, HOST_A).is_err(), "only once");
}

#[test]
fn a_dispatched_report_records_the_result_on_the_row_and_the_task() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;
    claim(&store, "alice", req, HOST_A).expect("claim");
    store
        .call("alice", |ctx| {
            report_dispatch_provisioned(
                ctx,
                req,
                HOST_A.into(),
                "/wt/1-t".into(),
                "task-1".into(),
                true,
            )
        })
        .expect("report");

    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_DISPATCHED);
    assert_eq!(row.finished_at.len(), 23);
    assert_eq!(row.worktree, "/wt/1-t");
    assert_eq!(row.tmux_window, "task-1");
    assert!(row.reused_worktree, "the host's answer is kept on the row");
    let task = store.task(id);
    assert_eq!(task.worktree, "/wt/1-t");
    assert_eq!(task.host, HOST_A);
    assert_eq!(task.status, "running");
    let agent = store
        .db()
        .agents()
        .iter()
        .find(|a| a.task_id == id)
        .expect("an agent");
    assert_eq!(agent.tmux_window, "task-1");
}

#[test]
fn a_repeated_dispatched_report_is_accepted_and_changes_nothing() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;
    claim(&store, "alice", req, HOST_A).expect("claim");
    report_dispatched(&store, "alice", req, HOST_A, "/wt/1-t").expect("first");
    let finished_at = only_request(&store).finished_at;
    let updated_at = store.task(id).updated_at;
    store.advance_secs(5);

    report_dispatched(&store, "alice", req, HOST_A, "/wt/1-t").expect("repeat");
    assert_eq!(only_request(&store).finished_at, finished_at);
    assert_eq!(store.task(id).updated_at, updated_at);

    assert!(report_dispatched(&store, "alice", req, HOST_A, "/wt/other").is_err());
    assert_eq!(only_request(&store).worktree, "/wt/1-t");
}

#[test]
fn the_epic_chain_may_not_request_a_phoenix_task() {
    let store = Store::new();
    let id = host_and_task(&store);
    patch(
        &store,
        id,
        TaskPatch {
            phoenix: Some(true),
            ..TaskPatch::default()
        },
    );

    let refused = request_as(&store, "alice", id, HOST_A, "epic_chain", "").unwrap_err();
    assert!(refused.contains("phoenix"), "{refused}");
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.task(id).status, "backlog");

    request(&store, id).expect("a request from the TUI takes it");
}

#[test]
fn a_report_from_another_host_or_on_an_unclaimed_row_is_refused() {
    let store = Store::new();
    let id = host_and_task(&store);
    store.register_host_for(HOST_B, "alice");
    request(&store, id).expect("request");
    let req = only_request(&store).id;

    assert!(report_dispatched(&store, "alice", req, HOST_A, "/wt/1-t").is_err());
    assert!(report_failed(&store, "alice", req, HOST_A).is_err());

    claim(&store, "alice", req, HOST_A).expect("claim");
    assert!(report_dispatched(&store, "alice", req, HOST_B, "/wt/1-t").is_err());
    assert!(report_failed(&store, "alice", req, HOST_B).is_err());

    assert_eq!(only_request(&store).state, DISPATCH_PROVISIONING);
    assert_eq!(store.task(id).worktree, "");
}

#[test]
fn a_failed_report_fails_the_row_and_releases_the_claim_completely() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;
    claim(&store, "alice", req, HOST_A).expect("claim");
    report_failed(&store, "alice", req, HOST_A).expect("report failed");

    assert_eq!(only_request(&store).finished_at.len(), 23);
    let task = store.task(id);
    assert_eq!(task.status, "backlog");
    assert_eq!(task.sub_status, "none");
    let agent = store
        .db()
        .agents()
        .iter()
        .find(|a| a.task_id == id)
        .expect("the agent the claim started");
    assert_eq!(agent.state, "ended");
    assert_eq!(timer_count(&store, req), 0);

    request(&store, id).expect("released, so it can be requested afresh");
}

#[test]
fn a_failed_report_leaves_a_task_that_got_a_worktree_alone() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;
    claim(&store, "alice", req, HOST_A).expect("claim");
    patch(
        &store,
        id,
        TaskPatch {
            worktree: Some("/wt/1-t".into()),
            ..TaskPatch::default()
        },
    );

    report_failed(&store, "alice", req, HOST_A).expect("report failed");
    assert_eq!(only_request(&store).state, DISPATCH_FAILED);
    assert_eq!(store.task(id).status, "running");
}

#[test]
fn a_dispatched_report_after_the_row_failed_is_refused() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;
    claim(&store, "alice", req, HOST_A).expect("claim");
    sweep(&store, "alice", HOST_A).expect("sweep");

    assert!(report_dispatched(&store, "alice", req, HOST_A, "/wt/1-t").is_err());
    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_FAILED);
    assert_eq!(row.worktree, "");
    let task = store.task(id);
    assert_eq!(task.worktree, "");
    assert_eq!(task.status, "backlog");
}

#[test]
fn a_restart_fails_only_this_hosts_provisioning_requests() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    let ids: Vec<i64> = (0..3)
        .map(|_| store.create("alice", user_task("t", "alice")))
        .collect();
    request_as(&store, "alice", ids[0], HOST_A, "user", "").unwrap();
    request_as(&store, "alice", ids[1], HOST_A, "user", "").unwrap();
    request_as(&store, "alice", ids[2], HOST_B, "user", "").unwrap();
    let first = request_for(&store, ids[0]).id;
    let third = request_for(&store, ids[2]).id;
    claim(&store, "alice", first, HOST_A).unwrap();
    claim(&store, "alice", third, HOST_B).unwrap();

    sweep(&store, "alice", HOST_A).expect("sweep");

    let row = request_for(&store, ids[0]);
    assert_eq!(row.state, DISPATCH_FAILED);
    assert_eq!(row.reason, DISPATCH_REASON_HOST_RESTARTED);
    assert_eq!(store.task(ids[0]).status, "backlog");
    assert_eq!(timer_count(&store, first), 0);
    assert_eq!(request_for(&store, ids[1]).state, DISPATCH_REQUESTED);
    assert_eq!(request_for(&store, ids[2]).state, DISPATCH_PROVISIONING);
}

#[test]
fn deleting_a_task_fails_its_open_request_and_cancels_its_timer() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;
    // delete_task requires done; a manual move leaves the request open.
    patch(&store, id, status_patch("done"));

    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");

    assert!(store.db().tasks().id().find(id).is_none());
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(timer_count(&store, req), 0);
}

#[test]
fn close_task_fails_the_tasks_open_dispatch_request() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;

    store
        .call("alice", |ctx| close_task(ctx, id, HOST_A.into()))
        .expect("close_task");

    let row = only_request(&store);
    assert_eq!(row.state, DISPATCH_FAILED);
    assert_eq!(row.reason, DISPATCH_REASON_TASK_CLOSED);
    assert_eq!(store.task(id).status, "done");
    assert_eq!(timer_count(&store, req), 0);
}

#[test]
fn batch_delete_fails_the_open_requests_of_the_tasks_it_deletes() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let a = store.create("alice", user_task("a", "alice"));
    let b = store.create("alice", user_task("b", "alice"));
    request_as(&store, "alice", a, HOST_A, "user", "").unwrap();
    request_as(&store, "alice", b, HOST_A, "user", "").unwrap();
    let second = request_for(&store, b).id;
    claim(&store, "alice", second, HOST_A).unwrap();
    patch(&store, a, status_patch("done"));
    patch(&store, b, status_patch("done"));

    store
        .call("alice", |ctx| batch_delete(ctx, vec![a, b], vec![]))
        .expect("batch_delete");

    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.db().dispatch_request_timeouts().iter().count(), 0);
}

#[test]
fn deleting_an_epic_fails_the_open_requests_of_its_subtree_tasks() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let (epic, ids) = epic_with_tasks(&store, 1);
    request_as(&store, "alice", ids[0], HOST_A, "user", "").unwrap();
    patch(&store, ids[0], status_patch("done"));

    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![epic]))
        .expect("batch_delete");

    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.db().dispatch_request_timeouts().iter().count(), 0);
}

#[test]
fn a_feed_ingest_that_deletes_a_stale_task_fails_its_open_request() {
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
    request_as(&store, "alice", task, HOST_A, "user", "").expect("request");

    ingest(vec![]).expect("second ingest");

    assert!(store.db().tasks().id().find(task).is_none());
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.db().dispatch_request_timeouts().iter().count(), 0);
}

#[test]
fn a_request_rolls_up_the_claimed_subtasks_epic() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let (epic, ids) = epic_with_tasks(&store, 1);
    make_epic_stale(&store, epic);

    request_as(&store, "alice", ids[0], HOST_A, "user", "").expect("request");

    assert_eq!(store.epic(epic).status, "backlog");
}

#[test]
fn a_failed_report_rolls_up_the_released_subtasks_epic() {
    let store = Store::new();
    let (epic, req) = provisioning_under_a_stale_epic(&store);
    report_failed(&store, "alice", req, HOST_A).expect("report failed");
    assert_eq!(store.epic(epic).status, "backlog");
}

#[test]
fn a_restart_sweep_rolls_up_the_released_subtasks_epic() {
    let store = Store::new();
    let (epic, _) = provisioning_under_a_stale_epic(&store);
    sweep(&store, "alice", HOST_A).expect("sweep");
    assert_eq!(store.epic(epic).status, "backlog");
}

#[test]
fn a_dispatched_report_rolls_up_the_subtasks_epic() {
    let store = Store::new();
    let (epic, req) = provisioning_under_a_stale_epic(&store);
    report_dispatched(&store, "alice", req, HOST_A, "/wt/1-t").expect("report");
    assert_eq!(store.epic(epic).status, "backlog");
}

#[test]
fn a_request_naming_another_persons_host_is_refused() {
    let store = Store::new();
    let id = host_and_task(&store);
    store.register_host_for("their-host", "bob");

    let refused = request_as(&store, "alice", id, "their-host", "user", "").unwrap_err();
    assert!(refused.contains("their-host"), "{refused}");
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
    assert_eq!(store.task(id).status, "backlog");
}

#[test]
fn a_request_naming_an_ownerless_or_unregistered_host_is_refused() {
    let store = Store::new();
    let id = host_and_task(&store);
    let ownerless = store
        .call("alice", |ctx| {
            register_host(ctx, "fresh-host".into(), String::new(), String::new())
        })
        .unwrap_err();
    assert!(ownerless.contains("owner"), "{ownerless}");
    assert!(store.db().hosts().id().find("fresh-host".to_string()).is_none());

    for host in ["fresh-host", "nowhere"] {
        let refused = request_as(&store, "alice", id, host, "user", "").unwrap_err();
        assert!(refused.contains("not registered"), "{refused}");
    }
    assert_eq!(store.db().dispatch_requests().iter().count(), 0);
}

#[test]
fn claims_reports_and_the_sweep_are_refused_for_another_persons_host() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let a = store.create("alice", user_task("a", "alice"));
    let b = store.create("alice", user_task("b", "alice"));
    request_as(&store, "alice", a, HOST_A, "user", "").unwrap();
    request_as(&store, "alice", b, HOST_A, "user", "").unwrap();
    let requested = request_for(&store, a).id;
    let provisioning = request_for(&store, b).id;
    claim(&store, "alice", provisioning, HOST_A).unwrap();

    assert!(claim(&store, "bob", requested, HOST_A).is_err());
    assert!(report_dispatched(&store, "bob", provisioning, HOST_A, "/wt/2-t").is_err());
    assert!(report_failed(&store, "bob", provisioning, HOST_A).is_err());
    assert!(sweep(&store, "bob", HOST_A).is_err());

    assert_eq!(request_for(&store, a).state, DISPATCH_REQUESTED);
    assert_eq!(request_for(&store, b).state, DISPATCH_PROVISIONING);
}

#[test]
fn the_sweep_and_a_claim_are_refused_for_an_ownerless_host() {
    let store = Store::new();
    let id = host_and_task(&store);
    request(&store, id).expect("request");
    let req = only_request(&store).id;

    let swept = sweep(&store, "alice", "fresh-host").unwrap_err();
    assert!(swept.contains("not registered"), "{swept}");
    assert!(claim(&store, "alice", req, "fresh-host").is_err());
    assert_eq!(only_request(&store).state, DISPATCH_REQUESTED);
}

#[test]
fn a_teammate_cannot_overwrite_another_persons_host() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let me = identity_of("alice");
    let them = identity_of("bob");

    let register = |who: &str, label: &str, owner: &str| {
        store.call(who, |ctx| {
            register_host(ctx, HOST_A.into(), label.into(), owner.into())
        })
    };
    assert!(register("bob", "", &them).is_err(), "takeover");
    assert!(register("bob", "mine now", &me).is_err(), "relabel");
    assert_eq!(store.db().hosts().id().find(HOST_A.to_string()).unwrap().owner, me);
}

#[test]
fn a_host_cannot_be_registered_to_someone_elses_identity() {
    let store = Store::new();
    let them = identity_of("bob");
    let gifted = store.call("alice", |ctx| {
        register_host(ctx, "new-host".into(), String::new(), them)
    });
    assert!(gifted.is_err());
    assert!(store.db().hosts().id().find("new-host".to_string()).is_none());
}

#[test]
fn the_owner_may_relabel_but_not_disown_its_host() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    let me = identity_of("alice");
    let register = |label: &str, owner: &str| {
        store.call("alice", |ctx| {
            register_host(ctx, HOST_A.into(), label.into(), owner.into())
        })
    };
    register("laptop", &me).expect("rename");
    assert!(register("laptop", "").is_err(), "disown");

    let row = store.db().hosts().id().find(HOST_A.to_string()).unwrap();
    assert_eq!(row.label, "laptop");
    assert_eq!(row.owner, me);
}
