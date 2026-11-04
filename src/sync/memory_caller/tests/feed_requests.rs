use super::feed::feed_item;
use super::*;
use crate::models::{FeedRequestId, FeedRequestState, PollScopeId};

// -- Feed requests (store-split.allium: FeedRequest) -----------------------------

/// Alice's caller owning `laptop` and `desktop`, and a feed epic.
async fn world() -> (MemoryReducerCaller, Arc<SharedRows>, EpicId) {
    let (caller, rows) = caller();
    let caller = caller.with_sender("alice");
    for host in ["laptop", "desktop"] {
        caller
            .register_host(host.into(), host.into(), "alice".into())
            .await
            .unwrap();
    }
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    {
        let mut tables = caller.lock();
        let row = tables.epics.get_mut(&epic.0).unwrap();
        row.feed_command = "echo []".into();
        row.feed_command_set_by = "alice".into();
    }
    (caller, rows, epic)
}

#[tokio::test]
async fn a_refresh_request_goes_to_the_poll_owner_and_is_coalesced() {
    let (caller, rows, epic) = world().await;
    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "laptop".into())
        .await
        .unwrap();

    let first = caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap();
    assert!(matches!(first, ReducerOutcome::Applied(_)), "{first:?}");
    caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap()
        .applied()
        .unwrap();

    let held = rows.feed_requests();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].host, "laptop");
    assert_eq!(held[0].epic_id, Some(epic));
    assert_eq!(held[0].state, FeedRequestState::Requested);
}

#[tokio::test]
async fn a_refresh_request_is_refused_without_an_owner_or_from_the_owner() {
    let (caller, rows, epic) = world().await;
    let no_owner = caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap();
    assert!(matches!(no_owner, ReducerOutcome::Refused(_)));

    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "laptop".into())
        .await
        .unwrap();
    let from_owner = caller
        .request_feed_refresh(epic, "laptop".into())
        .await
        .unwrap();
    assert!(matches!(from_owner, ReducerOutcome::Refused(_)));
    assert!(rows.feed_requests().is_empty());
}

#[tokio::test]
async fn the_owning_host_claims_and_reports_a_request() {
    let (caller, rows, epic) = world().await;
    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "laptop".into())
        .await
        .unwrap();
    caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap();
    let id = rows.feed_requests()[0].id;

    let wrong = caller
        .claim_feed_request(id, "desktop".into())
        .await
        .unwrap();
    assert!(matches!(wrong, ReducerOutcome::Refused(_)));
    caller
        .claim_feed_request(id, "laptop".into())
        .await
        .unwrap()
        .applied()
        .unwrap();
    assert_eq!(
        rows.feed_request(id).unwrap().state,
        FeedRequestState::Running
    );
    caller
        .report_feed_request_done(id, "laptop".into())
        .await
        .unwrap()
        .applied()
        .unwrap();
    assert_eq!(rows.feed_request(id).unwrap().state, FeedRequestState::Done);
    let missing = caller
        .claim_feed_request(FeedRequestId(99), "laptop".into())
        .await
        .unwrap();
    assert!(matches!(missing, ReducerOutcome::Refused(_)));
}

/// Put `external_id`'s task on `host`, holding a worktree and a window.
fn hold(caller: &MemoryReducerCaller, external_id: &str, host: &str) {
    let mut tables = caller.lock();
    let task = tables
        .tasks
        .values_mut()
        .find(|t| t.external_id == external_id)
        .unwrap();
    task.host = host.into();
    task.worktree = "/wt/a".into();
    task.tmux_window = "dispatch:a".into();
}

#[tokio::test]
async fn a_stale_delete_asks_the_holding_host_to_tear_down_and_not_the_ingesting_host() {
    let (caller, rows, epic) = world().await;
    let ingest = |items: Vec<bindings::FeedIngestItem>| {
        let caller = &caller;
        async move {
            caller
                .ingest_feed(epic, items, false, "alice".into(), "laptop".into())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
    };

    // A task held by the ingesting host itself: no row.
    ingest(vec![feed_item("own")]).await;
    hold(&caller, "own", "laptop");
    ingest(vec![]).await;
    assert!(rows.feed_requests().is_empty());

    // A task held by another host of the same person: a teardown row for it.
    ingest(vec![feed_item("a")]).await;
    hold(&caller, "a", "desktop");
    ingest(vec![]).await;

    let held = rows.feed_requests();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].host, "desktop");
    assert_eq!(held[0].worktree.as_deref(), Some("/wt/a"));
    assert_eq!(held[0].repo_path, "/repo");
    assert!(held[0].task_id.is_some());
    assert_eq!(held[0].requested_by, "alice");
}
