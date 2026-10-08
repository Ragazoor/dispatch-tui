//! The first connection is part of starting — Phase 12a (task #4916).
//!
//! Spec: `docs/specs/sync.allium`'s `OpenBoardConnection`,
//! `FirstConnectionLetsTheBoardDraw`, `FirstConnectionFailureAbortsStartup` and
//! `IdentityConflictAtStartupAbortsIt`; `docs/specs/startup.allium`'s
//! `AbortWhenTheStoreCannotBeReached`.

use super::{accepted, refused, ScriptedConnector};
use crate::store::{HostStore, IdentityCredentialStore, Store};
use crate::sync::{ConnectionStatus, SyncSession};
use std::time::Instant;

/// **Test 3 of the phase plan.** A fresh install — nothing on disk at all —
/// starts, connects, and ends up holding everything it needs: a host id it
/// minted itself, and the user identity and credential the store issued.
#[tokio::test]
async fn a_fresh_install_connects_and_mints_its_identities() {
    let db = Store::open_in_memory().unwrap();
    let connector = ScriptedConnector::new(vec![accepted("c0ffee", "token-1")]);
    let mut session = SyncSession::open("store.example", connector.clone());

    session
        .connect_at_startup(&db, Instant::now())
        .await
        .expect("a reachable store lets the board start");

    assert_eq!(session.status(), ConnectionStatus::Connected);
    assert_eq!(
        connector.presented_tokens(),
        vec![None],
        "a fresh install has no credential to present"
    );
    let (host, _label) = db.ensure_host_identity().await.unwrap();
    assert!(!host.is_empty(), "the host id is minted locally");
    assert_eq!(db.user_identity().await.unwrap().as_deref(), Some("c0ffee"));
    assert_eq!(
        db.user_identity_token().await.unwrap().as_deref(),
        Some("token-1")
    );
    let subscribed = connector.subscriptions();
    assert_eq!(
        subscribed.len(),
        1,
        "subscribed exactly once, and before returning"
    );
    assert_eq!(subscribed[0].owner_board, "c0ffee");
    assert_eq!(subscribed[0].host, host);
}

/// A first attempt that fails aborts the start, with the attempt's own reason.
/// There is no retry: the backoff is for a board that was up.
#[tokio::test]
async fn an_unreachable_store_fails_the_start_with_its_reason() {
    let db = Store::open_in_memory().unwrap();
    let connector = ScriptedConnector::new(vec![refused("connection refused")]);
    let mut session = SyncSession::open("store.example", connector.clone());

    let reason = session
        .connect_at_startup(&db, Instant::now())
        .await
        .expect_err("an unreachable store must not let the board start");

    assert!(reason.contains("connection refused"), "{reason}");
    assert_eq!(connector.attempts(), 1, "one attempt, no retry");
    assert_eq!(db.user_identity().await.unwrap(), None);
}

/// An identity conflict on the first connection aborts the start too, with
/// the conflict's own composed message rather than a generic one.
#[tokio::test]
async fn an_identity_conflict_fails_the_start() {
    let db = Store::open_in_memory().unwrap();
    db.set_user_identity_token("token-a").await.unwrap();
    db.adopt_user_identity("aaaa").await.unwrap();
    let connector = ScriptedConnector::new(vec![accepted("bbbb", "token-b")]);
    let mut session = SyncSession::open("store.example", connector.clone());

    let reason = session
        .connect_at_startup(&db, Instant::now())
        .await
        .expect_err("a conflict must not let the board start");

    assert!(
        reason.contains("aaaa") && reason.contains("bbbb"),
        "{reason}"
    );
    assert!(
        connector.subscriptions().is_empty(),
        "nothing is subscribed as a stranger"
    );
}
