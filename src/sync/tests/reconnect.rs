//! Test 4 of the phase plan: a dropped connection reconnects, and the board
//! shows disconnected state while it is down.
//!
//! The SpacetimeDB Rust SDK has no auto-reconnect. Everything asserted here is
//! behaviour this repo writes by hand, which is why it is asserted at all.

use super::{accepted, refused, ScriptedConnector};
use crate::db::{Database, HostStore, SubscriptionStore};
use crate::sync::{ConnectionStatus, StepOutcome, SyncSession, RECONNECT_BACKOFF_BASE};
use std::time::{Duration, Instant};

async fn store() -> Database {
    Database::open_in_memory().await.unwrap()
}

/// The happy path, stated so the outage tests below have something to differ
/// from.
#[tokio::test]
async fn a_first_step_connects_and_settles_the_identity() {
    let db = store().await;
    let connector = ScriptedConnector::new(vec![accepted("user-a", "token-a")]);
    let mut session = SyncSession::open("store.example", connector.clone());
    let now = Instant::now();

    assert_eq!(
        session.step(&db, now).await.unwrap(),
        StepOutcome::Connected
    );

    assert_eq!(session.status(), ConnectionStatus::Connected);
    assert_eq!(
        connector.presented_tokens(),
        vec![None],
        "a first connection has no credential to present, so the store mints one"
    );
    assert_eq!(
        db.user_identity().await.unwrap().as_deref(),
        Some("user-a"),
        "the identity the store issued is now stored"
    );
}

/// A drop, then a reconnect. The whole of the loop.
#[tokio::test]
async fn a_dropped_connection_comes_back_by_itself() {
    let db = store().await;
    let connector = ScriptedConnector::new(vec![
        accepted("user-a", "token-a"),
        accepted("user-a", "token-a"),
    ]);
    let mut session = SyncSession::open("store.example", connector.clone());
    let start = Instant::now();
    session.step(&db, start).await.unwrap();

    assert!(session.report_drop("socket closed", start));
    assert_eq!(session.status(), ConnectionStatus::Disconnected);

    // Nothing happens before the backoff elapses. This is the assertion that a
    // hot retry loop would fail.
    assert_eq!(
        session.step(&db, start).await.unwrap(),
        StepOutcome::Idle,
        "a retry before the backoff would hammer a store that is already down"
    );
    assert_eq!(connector.attempts(), 1);

    let due = start + RECONNECT_BACKOFF_BASE;
    assert_eq!(session.step(&db, due).await.unwrap(), StepOutcome::Retrying);
    assert_eq!(session.status(), ConnectionStatus::Connecting);

    assert_eq!(
        session.step(&db, due).await.unwrap(),
        StepOutcome::Connected
    );
    assert_eq!(session.status(), ConnectionStatus::Connected);
    assert_eq!(connector.attempts(), 2);
}

/// While it is down, the board says so and says why — continuously, not once.
#[tokio::test]
async fn the_board_shows_disconnected_for_as_long_as_the_store_is_down() {
    let db = store().await;
    let connector = ScriptedConnector::new(vec![
        accepted("user-a", "token-a"),
        refused("connection refused"),
        refused("connection refused"),
        accepted("user-a", "token-a"),
    ]);
    let mut session = SyncSession::open("store.example", connector.clone());
    let start = Instant::now();
    session.step(&db, start).await.unwrap();
    session.report_drop("host unreachable", start);

    let mut now = start;
    let mut seen_unhealthy = 0;
    // Walk the outage minute by minute rather than waiting it out.
    for _ in 0..3 {
        now += Duration::from_secs(60);
        let outcome = session.step(&db, now).await.unwrap();
        if outcome == StepOutcome::Retrying {
            session.step(&db, now).await.unwrap();
        }
        if !session.status().is_healthy() {
            seen_unhealthy += 1;
            assert!(
                session.connection().last_error().is_some(),
                "an unhealthy connection must carry a reason an operator can act on"
            );
        }
    }

    assert!(
        seen_unhealthy >= 2,
        "the disconnected state must persist across the outage, not flash once"
    );
    assert_eq!(session.status(), ConnectionStatus::Connected, "and recover");
}

/// Test 5 of the phase plan. The board is up; the connection is not; the reason
/// is on screen.
#[tokio::test]
async fn an_unreachable_store_at_startup_is_reported_not_fatal() {
    let db = store().await;
    let connector = ScriptedConnector::new(vec![refused("no route to host")]);
    let mut session = SyncSession::open("store.example", connector);
    let now = Instant::now();

    assert_eq!(session.step(&db, now).await.unwrap(), StepOutcome::Failed);

    assert_eq!(session.status(), ConnectionStatus::Disconnected);
    assert_eq!(session.connection().last_error(), Some("no route to host"));
    assert_ne!(
        session.status(),
        ConnectionStatus::Failed,
        "an unreachable store is an outage, not a fatal condition"
    );
    assert!(
        session.connection().next_attempt_at().is_some(),
        "and it must be retried"
    );
}

/// Each reconnect presents the stored credential, which is what returns the
/// SAME identity rather than a new one.
#[tokio::test]
async fn every_reconnect_presents_the_stored_credential() {
    let db = store().await;
    let connector = ScriptedConnector::new(vec![
        accepted("user-a", "token-a"),
        accepted("user-a", "token-a"),
    ]);
    let mut session = SyncSession::open("store.example", connector.clone());
    let start = Instant::now();
    session.step(&db, start).await.unwrap();
    session.report_drop("dropped", start);

    let due = start + RECONNECT_BACKOFF_BASE;
    session.step(&db, due).await.unwrap();
    session.step(&db, due).await.unwrap();

    assert_eq!(
        connector.presented_tokens(),
        vec![None, Some("token-a".to_string())],
        "the first attempt had nothing stored; the second must present what it learned"
    );
}

/// The conflict, end to end. Terminal, never retried, and the message names
/// both identities.
#[tokio::test]
async fn a_changed_identity_stops_the_connection_for_good() {
    let db = store().await;
    db.adopt_user_identity("user-a", "token-a").await.unwrap();
    let connector = ScriptedConnector::new(vec![accepted("user-b", "token-b")]);
    let mut session = SyncSession::open("store.example", connector.clone());
    let now = Instant::now();

    assert_eq!(
        session.step(&db, now).await.unwrap(),
        StepOutcome::Conflicted
    );

    assert_eq!(session.status(), ConnectionStatus::Failed);
    let message = session.connection().last_error().unwrap();
    assert!(message.contains("user-a"), "{message}");
    assert!(message.contains("user-b"), "{message}");
    assert_eq!(
        db.user_identity().await.unwrap().as_deref(),
        Some("user-a"),
        "the stored identity must not be quietly replaced by the one that conflicted"
    );

    // Terminal: no retry, ever, and no further calls on the connector.
    assert_eq!(
        session
            .step(&db, now + Duration::from_secs(86_400))
            .await
            .unwrap(),
        StepOutcome::Idle
    );
    assert_eq!(connector.attempts(), 1);
}

/// Nothing is subscribed before the identity settles. In the conflict case that
/// means a stranger's rows are never asked for.
#[tokio::test]
async fn a_conflicted_connection_never_subscribes() {
    let db = store().await;
    db.adopt_user_identity("user-a", "token-a").await.unwrap();
    db.subscribe_to_epic("user-a", 7).await.unwrap();
    let connector = ScriptedConnector::new(vec![accepted("user-b", "token-b")]);
    let mut session = SyncSession::open("store.example", connector.clone());

    session.step(&db, Instant::now()).await.unwrap();

    assert!(
        connector.subscriptions().is_empty(),
        "a board that subscribed before identifying would ask for rows as a stranger"
    );
}

/// A connection accepted but not subscribable is not a working connection. It
/// becomes an ordinary outage rather than sitting in `Connected` syncing
/// nothing.
#[tokio::test]
async fn a_connection_that_cannot_subscribe_is_treated_as_an_outage() {
    use crate::sync::{Accepted, ConnectError, StoreConnector, SubscriptionRequest};
    use async_trait::async_trait;

    struct RefusesSubscriptions;

    #[async_trait]
    impl StoreConnector for RefusesSubscriptions {
        async fn connect(
            &self,
            _server: &str,
            _token: Option<&str>,
        ) -> Result<Accepted, ConnectError> {
            Ok(Accepted {
                identity: "user-a".into(),
                token: "token-a".into(),
            })
        }
        async fn subscribe(&self, _request: &SubscriptionRequest) -> Result<(), ConnectError> {
            Err(ConnectError::new("subscription rejected"))
        }
    }

    let db = store().await;
    let mut session = SyncSession::open("store.example", std::sync::Arc::new(RefusesSubscriptions));
    let now = Instant::now();

    assert_eq!(session.step(&db, now).await.unwrap(), StepOutcome::Failed);
    assert_eq!(session.status(), ConnectionStatus::Disconnected);
    assert_eq!(
        session.connection().last_error(),
        Some("subscription rejected")
    );
    assert!(session.connection().next_attempt_at().is_some());
}
