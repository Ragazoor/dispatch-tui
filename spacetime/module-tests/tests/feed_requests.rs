//! Feed requests (`store-split.allium: FeedRequest`), run against the in-memory
//! store: the refresh a non-owner host asks of the poll owner, and the teardown
//! a stale delete asks of the host that holds a removed task's worktree.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// alice's two hosts, her feed epic, and `bob`'s host.
fn world() -> (Store, i64) {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    store.register_host_for("host-bob", "bob");
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
    (store, epic)
}

fn item(external_id: &str) -> FeedIngestItem {
    FeedIngestItem {
        item: FeedTaskUpsertItem {
            external_id: external_id.into(),
            title: external_id.into(),
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
    }
}

fn ingest(store: &Store, epic: i64, items: Vec<FeedIngestItem>, host: &str) {
    store
        .call("alice", |ctx| {
            ingest_feed(ctx, epic, items, false, "creator".into(), host.into())
        })
        .expect("ingest_feed");
}

/// The task for `external_id`, held by `host` with a worktree and a window.
/// The window lives on the task's current agent row, as in production: the
/// task row's own `tmux_window` column is always blank (agents A4).
fn hold(store: &Store, external_id: &str, host: &str, worktree: &str, window: &str) -> i64 {
    let task = store
        .db()
        .tasks()
        .iter()
        .find(|t| t.external_id == external_id)
        .expect("the feed task");
    let id = task.id;
    store.db().tasks().id().update(Task {
        host: host.into(),
        worktree: worktree.into(),
        ..task
    });
    if !window.is_empty() {
        store.db().agents().insert(Agent {
            tmux_window: window.into(),
            ..new_agent(
                id,
                &format!("agent-{id}"),
                host,
                "fresh",
                "2026-01-01 00:00:00.000",
            )
        });
    }
    id
}

fn feed_rows(store: &Store) -> Vec<FeedRequest> {
    store.db().feed_requests().iter().collect()
}

fn claim_owner(store: &Store, epic: i64, host: &str) {
    store
        .call("alice", |ctx| {
            claim_poll_owner(ctx, "epic".into(), epic, host.into())
        })
        .expect("claim_poll_owner");
}

fn refresh(store: &Store, who: &str, epic: i64, host: &str) -> Result<(), String> {
    store.call(who, |ctx| request_feed_refresh(ctx, epic, host.into()))
}

// -- Teardown rows ------------------------------------------------------------

#[test]
fn a_stale_delete_asks_the_holding_host_to_tear_a_foreign_task_down() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    let task = hold(&store, "a", HOST_B, "/wt/a", "win-a");

    ingest(&store, epic, vec![], HOST_A);

    assert!(store.db().tasks().id().find(task).is_none());
    let rows = feed_rows(&store);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, FEED_REQUEST_TEARDOWN);
    assert_eq!(row.host, HOST_B);
    assert_eq!(row.task_id, task);
    assert_eq!(row.repo_path, "/repo");
    assert_eq!(row.worktree, "/wt/a");
    assert_eq!(row.tmux_window, "win-a");
    assert_eq!(row.state, FEED_REQUESTED);
    assert_eq!(row.requested_by, identity_of("alice"));
}

#[test]
fn a_stale_delete_names_the_window_of_a_task_that_holds_no_worktree() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    let task = hold(&store, "a", HOST_B, "", "win-a");

    ingest(&store, epic, vec![], HOST_A);

    let rows = feed_rows(&store);
    assert_eq!(rows.len(), 1, "a window alone owes a teardown");
    assert_eq!(rows[0].task_id, task);
    assert_eq!(rows[0].worktree, "");
    assert_eq!(rows[0].tmux_window, "win-a");
}

#[test]
fn a_stale_delete_writes_no_row_for_the_ingesting_hosts_own_task() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    hold(&store, "a", HOST_A, "/wt/a", "win-a");

    ingest(&store, epic, vec![], HOST_A);

    assert!(feed_rows(&store).is_empty());
}

#[test]
fn a_stale_delete_writes_no_row_for_a_task_holding_nothing_or_held_by_no_host() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a"), item("b")], HOST_A);
    hold(&store, "a", HOST_B, "", "");
    hold(&store, "b", "", "/wt/b", "win-b");

    ingest(&store, epic, vec![], HOST_A);

    assert!(feed_rows(&store).is_empty());
}

#[test]
fn a_stale_delete_writes_no_row_for_a_host_of_another_person() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    hold(&store, "a", "host-bob", "/wt/a", "win-a");

    ingest(&store, epic, vec![], HOST_A);

    assert!(feed_rows(&store).is_empty());
}

#[test]
fn an_ingest_from_a_host_the_caller_does_not_own_is_refused() {
    let (store, epic) = world();
    let refused = store.call("alice", |ctx| {
        ingest_feed(
            ctx,
            epic,
            vec![],
            false,
            "creator".into(),
            "host-bob".into(),
        )
    });
    assert!(refused.is_err());
}

// -- Refresh rows -------------------------------------------------------------

#[test]
fn a_non_owner_host_asks_the_poll_owner_to_refresh() {
    let (store, epic) = world();
    claim_owner(&store, epic, HOST_A);

    refresh(&store, "alice", epic, HOST_B).expect("request");

    let rows = feed_rows(&store);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, FEED_REQUEST_REFRESH);
    assert_eq!(rows[0].host, HOST_A);
    assert_eq!(rows[0].epic_id, epic);
    assert_eq!(rows[0].state, FEED_REQUESTED);
}

#[test]
fn a_second_refresh_request_is_coalesced_into_the_open_one() {
    let (store, epic) = world();
    claim_owner(&store, epic, HOST_A);

    refresh(&store, "alice", epic, HOST_B).expect("first");
    refresh(&store, "alice", epic, HOST_B).expect("second");

    assert_eq!(feed_rows(&store).len(), 1);
}

#[test]
fn the_poll_owner_may_not_ask_itself() {
    let (store, epic) = world();
    claim_owner(&store, epic, HOST_A);
    assert!(refresh(&store, "alice", epic, HOST_A).is_err());
    assert!(feed_rows(&store).is_empty());
}

#[test]
fn a_refresh_is_refused_while_no_host_owns_the_epic() {
    let (store, epic) = world();
    let why = refresh(&store, "alice", epic, HOST_B).expect_err("no owner");
    assert!(why.contains("no poll owner"), "{why}");
}

#[test]
fn a_refresh_may_not_be_asked_of_another_persons_host() {
    let (store, epic) = world();
    claim_owner(&store, epic, HOST_A);
    // bob owns a host and may name it, but the poll owner is alice's.
    let who = refresh(&store, "bob", epic, "host-bob");
    assert!(who.is_err());
    assert!(feed_rows(&store).is_empty());
}

// -- Claim and report ---------------------------------------------------------

fn requested_refresh(store: &Store, epic: i64) -> i64 {
    claim_owner(store, epic, HOST_A);
    refresh(store, "alice", epic, HOST_B).expect("request");
    feed_rows(store)[0].id
}

#[test]
fn only_the_rows_own_host_claims_and_reports_it() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);

    assert!(store
        .call("alice", |ctx| claim_feed_request(ctx, id, HOST_B.into()))
        .is_err());
    store
        .call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()))
        .expect("claim");
    assert_eq!(feed_rows(&store)[0].state, FEED_RUNNING);
    assert!(store
        .call("alice", |ctx| report_feed_request_done(
            ctx,
            id,
            HOST_B.into()
        ))
        .is_err());
    store
        .call("alice", |ctx| {
            report_feed_request_done(ctx, id, HOST_A.into())
        })
        .expect("done");
    assert_eq!(feed_rows(&store)[0].state, FEED_DONE);
    assert!(!feed_rows(&store)[0].finished_at.is_empty());
}

#[test]
fn a_report_needs_a_running_row_and_a_failure_keeps_its_reason() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    assert!(store
        .call("alice", |ctx| report_feed_request_done(
            ctx,
            id,
            HOST_A.into()
        ))
        .is_err());
    store
        .call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()))
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_feed_request_failed(ctx, id, HOST_A.into(), "boom".into())
        })
        .expect("failed");
    let row = &feed_rows(&store)[0];
    assert_eq!(row.state, FEED_FAILED);
    assert_eq!(row.reason, "boom");
}

#[test]
fn a_running_row_is_claimed_again_only_after_the_grace_period() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    let claim =
        |store: &Store| store.call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()));
    claim(&store).expect("first claim");

    // A second board on the same host id, a moment later, is refused: the first
    // board is most likely still serving the row.
    store.advance_secs(RECLAIM_RUNNING_FEED_REQUEST_SECS - 1);
    assert!(claim(&store).is_err());

    // A board that died mid-serve left it running: past the grace, it is served again.
    store.advance_secs(2);
    claim(&store).expect("reclaim after the grace period");
    assert_eq!(feed_rows(&store)[0].state, FEED_RUNNING);
}

#[test]
fn a_reclaim_restarts_the_grace_period() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    let claim =
        |store: &Store| store.call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()));
    claim(&store).expect("first claim");
    store.advance_secs(RECLAIM_RUNNING_FEED_REQUEST_SECS + 1);
    claim(&store).expect("reclaim");
    store.advance_secs(RECLAIM_RUNNING_FEED_REQUEST_SECS - 1);
    assert!(claim(&store).is_err(), "claimed_at moved with the reclaim");
}

// -- Q8: a refresh asked after the owner started is not swallowed ---------------

#[test]
fn a_refresh_asked_while_the_owner_runs_the_first_is_written_as_a_new_row() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    store
        .call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()))
        .expect("claim");

    // The owner has already read the feed for the first request, so this one
    // needs a cycle of its own.
    refresh(&store, "alice", epic, HOST_B).expect("second");

    let rows = feed_rows(&store);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|r| r.state == FEED_REQUESTED).count(), 1);
}

#[test]
fn a_refresh_asked_after_the_first_finished_is_written_as_a_new_row() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    store
        .call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()))
        .expect("claim");
    store
        .call("alice", |ctx| {
            report_feed_request_done(ctx, id, HOST_A.into())
        })
        .expect("done");

    refresh(&store, "alice", epic, HOST_B).expect("second");

    assert_eq!(feed_rows(&store).len(), 2);
}

// -- Q9: an ingest from a host that lost the claim ------------------------------

#[test]
fn an_ingest_from_a_host_that_does_not_hold_the_epics_claim_is_refused() {
    let (store, epic) = world();
    claim_owner(&store, epic, HOST_B);
    let refused = store.call("alice", |ctx| {
        ingest_feed(
            ctx,
            epic,
            vec![item("a")],
            false,
            "creator".into(),
            HOST_A.into(),
        )
    });
    assert!(refused.is_err(), "HOST_A's claim was overridden by HOST_B");
    assert!(store.db().tasks().iter().all(|t| t.external_id != "a"));
}

#[test]
fn an_ingest_from_the_claim_holder_or_before_any_claim_is_accepted() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    claim_owner(&store, epic, HOST_B);
    ingest(&store, epic, vec![item("b")], HOST_B);
}

// -- Retention (pure) ---------------------------------------------------------

fn row(id: i64, kind: &str, state: &str, requested_at: &str, finished_at: &str) -> FeedRequest {
    FeedRequest {
        id,
        kind: kind.into(),
        state: state.into(),
        requested_at: requested_at.into(),
        finished_at: finished_at.into(),
        ..blank_feed_request()
    }
}

#[test]
fn retention_keeps_open_teardowns_and_prunes_old_terminal_and_stale_refresh_rows() {
    let cutoff = "2026-01-10 00:00:00.000";
    let stale = "2026-01-20 00:00:00.000";
    let rows = vec![
        row(
            1,
            FEED_REQUEST_TEARDOWN,
            FEED_REQUESTED,
            "2026-01-01 00:00:00.000",
            "",
        ),
        row(
            2,
            FEED_REQUEST_TEARDOWN,
            FEED_RUNNING,
            "2026-01-01 00:00:00.000",
            "",
        ),
        row(
            3,
            FEED_REQUEST_TEARDOWN,
            FEED_DONE,
            "2026-01-01 00:00:00.000",
            "2026-01-02 00:00:00.000",
        ),
        row(
            4,
            FEED_REQUEST_REFRESH,
            FEED_FAILED,
            "2026-01-12 00:00:00.000",
            "2026-01-12 00:00:01.000",
        ),
        row(
            5,
            FEED_REQUEST_REFRESH,
            FEED_REQUESTED,
            "2026-01-19 00:00:00.000",
            "",
        ),
        row(
            6,
            FEED_REQUEST_REFRESH,
            FEED_REQUESTED,
            "2026-01-21 00:00:00.000",
            "",
        ),
        row(
            7,
            FEED_REQUEST_REFRESH,
            FEED_RUNNING,
            "2026-01-19 00:00:00.000",
            "",
        ),
    ];
    assert_eq!(prunable_feed_requests(cutoff, stale, &rows), vec![3, 5]);
}

// -- Q2: nobody aims another host's teardown with a task patch ------------------

fn patch_as(store: &Store, who: &str, task: i64, patch: TaskPatch) -> Result<(), String> {
    store.call(who, |ctx| patch_task(ctx, task, patch))
}

fn a_feed_task_held_by(host: &str) -> (Store, i64) {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    let task = hold(&store, "a", host, "/wt/a", "");
    (store, task)
}

#[test]
fn a_patch_may_not_move_a_task_onto_another_persons_host() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    let task = hold(&store, "a", "", "", "");

    let why = patch_as(
        &store,
        "alice",
        task,
        TaskPatch {
            host: Some("host-bob".into()),
            ..TaskPatch::default()
        },
    )
    .expect_err("host-bob is bob's");
    assert!(why.contains("host-bob"), "{why}");
    assert_eq!(store.task(task).host, "");
}

#[test]
fn a_patch_may_not_rewrite_the_worktree_of_a_task_another_persons_host_holds() {
    let (store, task) = a_feed_task_held_by("host-bob");

    let refused = patch_as(
        &store,
        "alice",
        task,
        TaskPatch {
            worktree: Some("/home/bob/proj/.worktrees/999-other".into()),
            ..TaskPatch::default()
        },
    );

    assert!(refused.is_err());
    assert_eq!(store.task(task).worktree, "/wt/a");
}

#[test]
fn a_patch_may_not_rewrite_the_repo_of_a_task_another_persons_host_holds() {
    let (store, task) = a_feed_task_held_by("host-bob");

    let refused = patch_as(
        &store,
        "alice",
        task,
        TaskPatch {
            repo_path: Some("/home/bob/other".into()),
            ..TaskPatch::default()
        },
    );

    assert!(refused.is_err());
    assert_eq!(store.task(task).repo_path, "/repo");
}

#[test]
fn the_owner_of_the_holding_host_may_clear_the_worktree_and_a_repeated_value_is_no_change() {
    let (store, task) = a_feed_task_held_by(HOST_B);

    // The editor sends the repo on every save: a repeat changes nothing.
    patch_as(
        &store,
        "alice",
        task,
        TaskPatch {
            repo_path: Some("/repo".into()),
            ..TaskPatch::default()
        },
    )
    .expect("a repeated repo");
    patch_as(
        &store,
        "alice",
        task,
        TaskPatch {
            worktree: Some(String::new()),
            ..TaskPatch::default()
        },
    )
    .expect("alice owns HOST_B");
    assert_eq!(store.task(task).worktree, "");
}

#[test]
fn the_repo_of_a_task_no_host_holds_stays_editable() {
    let (store, epic) = world();
    ingest(&store, epic, vec![item("a")], HOST_A);
    let task = hold(&store, "a", "", "", "");

    patch_as(
        &store,
        "alice",
        task,
        TaskPatch {
            repo_path: Some("/elsewhere".into()),
            ..TaskPatch::default()
        },
    )
    .expect("no host holds it");
    assert_eq!(store.task(task).repo_path, "/elsewhere");
}
