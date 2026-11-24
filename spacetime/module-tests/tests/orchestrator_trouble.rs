//! A task agent in trouble (`orchestrators.allium: OrchestratorRecordsATroubleAction`,
//! `already_retried`, `TaskEnteringReviewIsLogged`), run against the
//! in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

const HOST: &str = "host-a";

/// An orchestrated epic with one backlog task and its orchestrator on HOST:
/// (epic, task, orchestrator).
fn setup(store: &Store) -> (i64, i64, Orchestrator) {
    store.register_host_for(HOST, "alice");
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    store
        .call("alice", |ctx| switch_orchestrator_on(ctx, epic, None, None))
        .expect("switch on");
    store
        .call("alice", |ctx| ensure_orchestrator(ctx, epic, HOST.to_string()))
        .expect("ensure");
    let o = store
        .db()
        .orchestrators()
        .epic_id()
        .filter(epic)
        .next()
        .expect("the orchestrator");
    (epic, task, o)
}

fn log(store: &Store, epic: i64) -> Vec<OrchestratorLogEntry> {
    let mut all: Vec<_> = store.db().orchestrator_log().epic_id().filter(epic).collect();
    all.sort_by_key(|e| e.id);
    all
}

fn record(store: &Store, o: &Orchestrator, task: i64, trouble: &str) -> Result<(), String> {
    store.advance_secs(1);
    store.call("alice", |ctx| {
        record_orchestrator_trouble(ctx, o.name.clone(), task, trouble.into(), "resumed the agent".into())
    })
}

fn raise_and_resolve(store: &Store, o: &Orchestrator, task: i64) {
    store.advance_secs(1);
    store
        .call("alice", |ctx| {
            raise_waiting_item(ctx, o.name.clone(), task, "crashed again".into(), "resumed once".into(), "look".into())
        })
        .expect("raise");
    let id = store
        .db()
        .waiting_items()
        .iter()
        .find(|i| i.task_id == task && i.resolved_at.is_empty())
        .expect("the item")
        .id;
    store.advance_secs(1);
    store.call("alice", |ctx| resolve_waiting_item(ctx, id)).expect("resolve");
}

#[test]
fn the_first_retry_is_recorded_with_its_trouble() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);

    record(&store, &o, task, TROUBLE_CRASHED).expect("first retry");

    let entries = log(&store, epic);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].kind, LOG_ACTED_ON_TROUBLE);
    assert_eq!(entries[0].trouble, TROUBLE_CRASHED);
    assert_eq!(entries[0].task_id, task);
    assert_eq!(entries[0].text, "resumed the agent");
    assert_eq!(entries[0].host, HOST);
}

#[test]
fn a_second_retry_in_the_same_bout_is_refused_and_says_to_raise_an_item() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);
    record(&store, &o, task, TROUBLE_CRASHED).expect("first retry");

    let why = record(&store, &o, task, TROUBLE_CRASHED).unwrap_err();

    assert!(why.contains("waiting-item_create"), "{why}");
    assert_eq!(log(&store, epic).len(), 1);
}

#[test]
fn each_trouble_has_its_own_retry() {
    let store = Store::new();
    let (_, task, o) = setup(&store);
    record(&store, &o, task, TROUBLE_CRASHED).expect("crashed");

    record(&store, &o, task, TROUBLE_CONFLICT).expect("conflict");
    record(&store, &o, task, TROUBLE_STALE).expect("stale");
}

#[test]
fn a_resolved_item_gives_the_retry_back() {
    let store = Store::new();
    let (_, task, o) = setup(&store);
    record(&store, &o, task, TROUBLE_CRASHED).expect("first retry");
    raise_and_resolve(&store, &o, task);

    record(&store, &o, task, TROUBLE_CRASHED).expect("a fresh bout");
}

#[test]
fn a_later_start_gives_the_retry_back_but_not_for_a_failed_start() {
    let store = Store::new();
    let (_, task, o) = setup(&store);
    record(&store, &o, task, TROUBLE_CRASHED).expect("crashed");
    record(&store, &o, task, TROUBLE_START_FAILED).expect("start failed");
    store.advance_secs(1);
    store
        .call("alice", |ctx| {
            orchestrator_request_dispatch(ctx, task, HOST.into(), o.name.clone(), "retry".into())
        })
        .expect("the retry's start");

    record(&store, &o, task, TROUBLE_CRASHED).expect("a new start is a new bout");
    let why = record(&store, &o, task, TROUBLE_START_FAILED).unwrap_err();
    assert!(why.contains("already had its one retry"), "{why}");
}

#[test]
fn a_trouble_record_needs_a_known_trouble_a_text_and_a_covered_task() {
    let store = Store::new();
    let (_, task, o) = setup(&store);
    let other_epic = store.create_epic("alice", "F", false);
    let outside = store.create("alice", epic_task("u", other_epic, "backlog"));

    assert!(record(&store, &o, task, "bored").is_err());
    assert!(record(&store, &o, outside, TROUBLE_STALE).is_err());
    let blank = store.call("alice", |ctx| {
        record_orchestrator_trouble(ctx, o.name.clone(), task, TROUBLE_STALE.into(), " ".into())
    });
    assert!(blank.is_err());
}

#[test]
fn a_covered_task_entering_review_is_logged_once_per_entry() {
    let store = Store::new();
    let (epic, task, _) = setup(&store);
    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("running")))
        .expect("running");

    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("review")))
        .expect("review");
    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("review")))
        .expect("review again");

    let reviews: Vec<_> = log(&store, epic)
        .into_iter()
        .filter(|e| e.kind == LOG_REVIEW)
        .collect();
    assert_eq!(reviews.len(), 1, "{reviews:?}");
    assert_eq!(reviews[0].task_id, task);
    assert_eq!(reviews[0].text, REVIEW_ENTERED);
}

#[test]
fn a_task_no_orchestrator_covers_logs_nothing_on_review() {
    let store = Store::new();
    store.register_host_for(HOST, "alice");
    let epic = store.create_epic("alice", "E", false);
    let task = store.create("alice", epic_task("t", epic, "running"));

    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("review")))
        .expect("review");

    assert!(log(&store, epic).is_empty());
}

/// `started_by_orchestrator(request)`: the start's entry carries the
/// request's own stamp, which is how the board tells the orchestrator's
/// failed starts from anyone else's.
#[test]
fn an_orchestrators_start_entry_carries_its_requests_stamp() {
    let store = Store::new();
    let (epic, task, o) = setup(&store);

    store
        .call("alice", |ctx| {
            orchestrator_request_dispatch(ctx, task, HOST.into(), o.name.clone(), "go".into())
        })
        .expect("start");

    let request = store
        .db()
        .dispatch_requests()
        .task_id()
        .filter(task)
        .next()
        .expect("the request");
    let entry = log(&store, epic)
        .into_iter()
        .find(|e| e.kind == LOG_STARTED_TASK)
        .expect("the entry");
    assert_eq!(entry.at, request.requested_at);
}
