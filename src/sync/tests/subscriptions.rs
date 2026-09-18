//! Test 3 of the phase plan: a user subscribes to their own user board and any
//! number of epics, and cannot subscribe to another user's board.
//!
//! Also the other half of test 2: one user with two host ids owns both, and
//! worktree gating still keys on host.

use super::{accepted, ScriptedConnector};
use crate::db::{
    CreateTaskRequest, Database, EpicCrud, HostStore, IdentityCredentialStore, SubscriptionStore,
    TaskCrud,
};
use crate::models::TaskStatus;
use crate::sync::{StepOutcome, SubscriptionRequest, SyncSession};
use std::time::Instant;

async fn store() -> Database {
    Database::open_in_memory().await.unwrap()
}

#[tokio::test]
async fn a_fresh_install_follows_no_epics() {
    let db = store().await;

    assert!(db.subscribed_epics("user-a").await.unwrap().is_empty());
}

#[tokio::test]
async fn a_user_can_follow_any_number_of_epics() {
    let db = store().await;

    for epic in [9, 7, 42] {
        db.subscribe_to_epic("user-a", epic).await.unwrap();
    }

    assert_eq!(db.subscribed_epics("user-a").await.unwrap(), vec![7, 9, 42]);
}

/// `core.allium: SubscriptionIsUniquePerSubscriberAndEpic`. Re-subscribing is
/// subscribing, which is what lets a client re-assert its interests on every
/// reconnect without checking first.
#[tokio::test]
async fn subscribing_twice_is_subscribing_once() {
    let db = store().await;

    db.subscribe_to_epic("user-a", 7).await.unwrap();
    db.subscribe_to_epic("user-a", 7).await.unwrap();

    assert_eq!(db.subscribed_epics("user-a").await.unwrap(), vec![7]);
}

/// The asymmetry is deliberate: subscribing twice expresses what the caller
/// wanted, and unsubscribing from nothing means they were wrong about the state
/// they were in.
#[tokio::test]
async fn unsubscribing_reports_whether_anything_was_following() {
    let db = store().await;
    db.subscribe_to_epic("user-a", 7).await.unwrap();

    assert!(db.unsubscribe_from_epic("user-a", 7).await.unwrap());
    assert!(!db.unsubscribe_from_epic("user-a", 7).await.unwrap());
    assert!(db.subscribed_epics("user-a").await.unwrap().is_empty());
}

/// Two people's subscriptions do not mix, and unsubscribing cannot reach
/// another person's row: the id is derived from the subscriber, so there is no
/// argument through which one could be named.
#[tokio::test]
async fn one_persons_subscriptions_are_invisible_to_another() {
    let db = store().await;
    db.subscribe_to_epic("user-a", 7).await.unwrap();
    db.subscribe_to_epic("user-b", 9).await.unwrap();

    assert_eq!(db.subscribed_epics("user-a").await.unwrap(), vec![7]);
    assert_eq!(db.subscribed_epics("user-b").await.unwrap(), vec![9]);

    assert!(
        !db.unsubscribe_from_epic("user-a", 9).await.unwrap(),
        "user-a naming user-b's epic must not remove user-b's row"
    );
    assert_eq!(db.subscribed_epics("user-b").await.unwrap(), vec![9]);
}

/// An install that has never connected has no identity, so it has nothing to
/// subscribe as. A row with an empty subscriber belongs to nobody — and would
/// then be sent to everybody by a store that matches on it.
#[tokio::test]
async fn subscribing_without_an_identity_is_refused() {
    let db = store().await;

    assert!(db.subscribe_to_epic("", 7).await.is_err());
    assert!(db.subscribe_to_epic("   ", 7).await.is_err());
}

/// **There is no way to ask for somebody else's user board.**
///
/// This is not a permission check that could be got wrong — it is an operation
/// that does not exist. The assertion below is therefore about the TYPE: a
/// request carries one identity and a list of epics, so a second person's board
/// has nowhere to go.
///
/// Stated as a test because the property is easy to lose. Adding a
/// `boards: Vec<String>` field to the request would compile, would look like a
/// generalisation, and would be the whole of the leak.
#[tokio::test]
async fn a_subscription_request_can_only_ever_name_its_own_board() {
    let db = store().await;
    db.subscribe_to_epic("user-a", 7).await.unwrap();
    let connector = ScriptedConnector::new(vec![accepted("user-a", "token-a")]);
    let mut session = SyncSession::open("store.example", connector.clone());

    assert_eq!(
        session.step(&db, Instant::now()).await.unwrap(),
        StepOutcome::Connected
    );

    let requests = connector.subscriptions();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0],
        SubscriptionRequest::new("user-a", vec![7]),
        "exactly two things are asked for: this person's own board, and the epics they follow"
    );
}

/// The subscription set is re-asserted on every connection, including the
/// reconnects where nothing changed. Subscriptions are per-connection and a
/// dropped connection takes them with it.
#[tokio::test]
async fn every_connection_re_asserts_the_subscription_set() {
    let db = store().await;
    db.subscribe_to_epic("user-a", 7).await.unwrap();
    let connector = ScriptedConnector::new(vec![
        accepted("user-a", "token-a"),
        accepted("user-a", "token-a"),
    ]);
    let mut session = SyncSession::open("store.example", connector.clone());
    let start = Instant::now();
    session.step(&db, start).await.unwrap();
    session.report_drop("dropped", start);

    let due = start + crate::sync::RECONNECT_BACKOFF_BASE;
    session.step(&db, due).await.unwrap();
    session.step(&db, due).await.unwrap();

    assert_eq!(
        connector.subscriptions().len(),
        2,
        "assuming a subscription survived a drop is a board that silently stops updating"
    );
}

/// A subscription belongs to the PERSON, not the machine: following an epic on
/// the desktop is already true on the laptop, because the row is keyed by the
/// identity the two share.
#[tokio::test]
async fn a_subscription_follows_the_person_across_their_machines() {
    let laptop = store().await;
    let desktop = store().await;
    for db in [&laptop, &desktop] {
        db.set_user_identity_token("token-a").await.unwrap();
        db.adopt_user_identity("user-a").await.unwrap();
    }

    laptop.subscribe_to_epic("user-a", 7).await.unwrap();

    // Standing in for the shared store the two would both read: the SAME
    // subscriber key resolves the same row set, whichever machine asks.
    let identity = desktop.user_identity().await.unwrap().unwrap();
    assert_eq!(identity, "user-a");
    assert_eq!(laptop.subscribed_epics(&identity).await.unwrap(), vec![7]);
}

/// Test 2's second half. One person owning two machines must not make a
/// worktree on the other reachable from here.
///
/// Asserted against the REAL gate rather than a synthetic comparison:
/// `try_claim_backlog_task` folds `is_locally_owned` into its own WHERE clause
/// (`docs/specs/dispatch.allium`: DispatchTask), so this is the check a
/// dispatch actually passes through. It compares MACHINE against machine, and
/// adding an owner to the machine changes nothing about it — no amount of
/// shared ownership moves a directory.
#[tokio::test]
async fn shared_ownership_does_not_make_another_machines_worktree_dispatchable() {
    use crate::db::TaskPatch;

    let db = store().await;
    let (this_host, _) = db.ensure_host_identity().await.unwrap();
    // Both machines are this one person's, and both say so.
    db.set_user_identity_token("token-a").await.unwrap();
    db.adopt_user_identity("user-a").await.unwrap();

    let epic = db.create_epic("E", "", None).await.unwrap();
    let mine = new_backlog_task(&db, epic.id, "mine").await;
    let theirs = new_backlog_task(&db, epic.id, "theirs").await;
    db.patch_task(mine, &TaskPatch::new().host(Some(&this_host)))
        .await
        .unwrap();
    db.patch_task(theirs, &TaskPatch::new().host(Some("the-other-machine")))
        .await
        .unwrap();

    assert!(
        db.try_claim_backlog_task(mine, chrono::Utc::now())
            .await
            .unwrap(),
        "a task whose worktree is on THIS disk stays dispatchable"
    );
    assert!(
        !db.try_claim_backlog_task(theirs, chrono::Utc::now())
            .await
            .unwrap(),
        "the other machine's task must stay the other machine's, however the two are owned"
    );
}

async fn new_backlog_task(
    db: &Database,
    epic: crate::models::EpicId,
    title: &str,
) -> crate::models::TaskId {
    db.create_task(CreateTaskRequest {
        title,
        description: "",
        repo_path: "/repo",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: Some(epic),
        sort_order: Some(1),
        tag: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    })
    .await
    .unwrap()
}
