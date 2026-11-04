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
        tmux_window: window.into(),
        ..task
    });
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
        ingest_feed(ctx, epic, vec![], false, "creator".into(), "host-bob".into())
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
        .call("alice", |ctx| report_feed_request_done(ctx, id, HOST_B.into()))
        .is_err());
    store
        .call("alice", |ctx| report_feed_request_done(ctx, id, HOST_A.into()))
        .expect("done");
    assert_eq!(feed_rows(&store)[0].state, FEED_DONE);
    assert!(!feed_rows(&store)[0].finished_at.is_empty());
}

#[test]
fn a_report_needs_a_running_row_and_a_failure_keeps_its_reason() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    assert!(store
        .call("alice", |ctx| report_feed_request_done(ctx, id, HOST_A.into()))
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
fn a_row_left_running_by_a_dead_board_is_claimed_again() {
    let (store, epic) = world();
    let id = requested_refresh(&store, epic);
    for _ in 0..2 {
        store
            .call("alice", |ctx| claim_feed_request(ctx, id, HOST_A.into()))
            .expect("claim");
    }
    assert_eq!(feed_rows(&store)[0].state, FEED_RUNNING);
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
        row(1, FEED_REQUEST_TEARDOWN, FEED_REQUESTED, "2026-01-01 00:00:00.000", ""),
        row(2, FEED_REQUEST_TEARDOWN, FEED_RUNNING, "2026-01-01 00:00:00.000", ""),
        row(3, FEED_REQUEST_TEARDOWN, FEED_DONE, "2026-01-01 00:00:00.000", "2026-01-02 00:00:00.000"),
        row(4, FEED_REQUEST_REFRESH, FEED_FAILED, "2026-01-12 00:00:00.000", "2026-01-12 00:00:01.000"),
        row(5, FEED_REQUEST_REFRESH, FEED_REQUESTED, "2026-01-19 00:00:00.000", ""),
        row(6, FEED_REQUEST_REFRESH, FEED_REQUESTED, "2026-01-21 00:00:00.000", ""),
        row(7, FEED_REQUEST_REFRESH, FEED_RUNNING, "2026-01-19 00:00:00.000", ""),
    ];
    assert_eq!(prunable_feed_requests(cutoff, stale, &rows), vec![3, 5]);
}
