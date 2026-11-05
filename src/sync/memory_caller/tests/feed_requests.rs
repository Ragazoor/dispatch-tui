use super::feed::feed_item;
use super::*;
use crate::models::{FeedRequestId, FeedRequestState, PollScopeId};

// -- Feed requests (store-split.allium: FeedRequest) -----------------------------

/// Alice's caller owning `laptop` and `desktop`, and a feed epic.
async fn world() -> (MemoryReducerCaller, Arc<SharedRows>, EpicId) {
    let (caller, rows) = caller();
    world_on(caller, rows).await
}

async fn world_on(
    caller: MemoryReducerCaller,
    rows: Arc<SharedRows>,
) -> (MemoryReducerCaller, Arc<SharedRows>, EpicId) {
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

/// Put `external_id`'s task on `host`, holding a worktree and a window. The
/// window lives on the task's current agent row, as in production: the Task
/// row's own column is always blank.
fn hold(caller: &MemoryReducerCaller, external_id: &str, host: &str) {
    hold_with(caller, external_id, host, "/wt/a", "dispatch:a");
}

fn hold_with(
    caller: &MemoryReducerCaller,
    external_id: &str,
    host: &str,
    worktree: &str,
    window: &str,
) {
    let mut tables = caller.lock();
    let task = tables
        .tasks
        .values_mut()
        .find(|t| t.external_id == external_id)
        .unwrap();
    task.host = host.into();
    task.worktree = worktree.into();
    let id = task.id;
    if !window.is_empty() {
        let agent = module::Agent {
            tmux_window: window.into(),
            ..module::new_agent(id, &format!("agent-{id}"), host, "fresh", TEST_STAMP)
        };
        module::AgentStore::insert_agent(&mut caller.agent_store(&mut tables), agent);
    }
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
    assert_eq!(
        held[0].tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:a"),
        "the window is read from the agent row before the delete removes it"
    );
    assert_eq!(held[0].repo_path, "/repo");
    assert!(held[0].task_id.is_some());
    assert_eq!(held[0].requested_by, "alice");
}

#[tokio::test]
async fn a_stale_delete_names_the_window_of_a_task_that_holds_no_worktree() {
    let (caller, rows, epic) = world().await;
    caller
        .ingest_feed(
            epic,
            vec![feed_item("w")],
            false,
            "alice".into(),
            "laptop".into(),
        )
        .await
        .unwrap()
        .applied()
        .unwrap();
    hold_with(&caller, "w", "desktop", "", "dispatch:w");
    caller
        .ingest_feed(epic, vec![], false, "alice".into(), "laptop".into())
        .await
        .unwrap()
        .applied()
        .unwrap();

    let held = rows.feed_requests();
    assert_eq!(held.len(), 1, "a window alone owes a teardown");
    assert_eq!(held[0].worktree, None);
    assert_eq!(
        held[0].tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:w")
    );
}

#[tokio::test]
async fn a_refresh_asked_after_the_owner_claimed_the_first_is_a_new_row() {
    let (caller, rows, epic) = world().await;
    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "laptop".into())
        .await
        .unwrap();
    caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap();
    let first = rows.feed_requests()[0].id;
    caller
        .claim_feed_request(first, "laptop".into())
        .await
        .unwrap()
        .applied()
        .unwrap();

    caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap()
        .applied()
        .unwrap();

    assert_eq!(rows.feed_requests().len(), 2);
}

#[tokio::test]
async fn a_running_request_is_claimed_again_only_after_the_grace_period() {
    let clock = crate::clock::FixedClock::new(
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 1, 1, 0, 0, 0).unwrap(),
    );
    let rows = Arc::new(SharedRows::new());
    let (caller, rows, epic) = world_on(
        MemoryReducerCaller::new(rows.clone(), Arc::new(clock.clone())),
        rows,
    )
    .await;
    caller
        .claim_poll_owner(PollScopeId::Epic(epic), "laptop".into())
        .await
        .unwrap();
    caller
        .request_feed_refresh(epic, "desktop".into())
        .await
        .unwrap();
    let id = rows.feed_requests()[0].id;
    let claim = || caller.claim_feed_request(id, "laptop".into());
    claim().await.unwrap().applied().unwrap();

    clock.advance(chrono::Duration::seconds(
        module::RECLAIM_RUNNING_FEED_REQUEST_SECS - 1,
    ));
    assert!(matches!(claim().await.unwrap(), ReducerOutcome::Refused(_)));
    clock.advance(chrono::Duration::seconds(2));
    claim().await.unwrap().applied().unwrap();
}

/// Review Q2: a patch may not aim another person's host at a worktree. The
/// task's host and the host a patch names are judged by who owns them.
#[tokio::test]
async fn a_patch_may_not_move_a_task_onto_or_rewrite_another_persons_host() {
    let (caller, rows, epic) = world().await;
    let caller = caller.with_registered_host("host-bob", "bob");
    caller
        .ingest_feed(
            epic,
            vec![feed_item("a")],
            false,
            "alice".into(),
            "laptop".into(),
        )
        .await
        .unwrap()
        .applied()
        .unwrap();
    let task = rows
        .tasks()
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some("a"))
        .unwrap()
        .id;
    let patch = |host: Option<&str>, worktree: Option<&str>| bindings::TaskPatch {
        host: host.map(Into::into),
        worktree: worktree.map(Into::into),
        ..blank_task_patch()
    };

    let onto = caller
        .patch_task(task, patch(Some("host-bob"), None))
        .await
        .unwrap();
    assert!(matches!(onto, ReducerOutcome::Refused(_)), "{onto:?}");

    // Held by bob's host, nobody else rewrites its worktree.
    caller.lock().tasks.get_mut(&task.0).unwrap().host = "host-bob".into();
    let rewrite = caller
        .patch_task(task, patch(None, Some("/home/alice/.worktrees/999-other")))
        .await
        .unwrap();
    assert!(matches!(rewrite, ReducerOutcome::Refused(_)), "{rewrite:?}");

    // alice's own host is hers to move a task onto.
    caller.lock().tasks.get_mut(&task.0).unwrap().host = String::new();
    caller
        .patch_task(task, patch(Some("laptop"), Some("/wt/a")))
        .await
        .unwrap()
        .applied()
        .unwrap();
}
