#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task #16755: adopting the identity the store hands out is ONE write of the
//! host file (`host.allium: AdoptUserIdentity`, `HostFileIsWrittenWhole`).

use super::{accepted, ScriptedConnector};
use crate::host_file::{host_file_path, read_for_cli, resolve_for_launch};
use crate::store::{Database, HostStore, IdentityCredentialStore};
use crate::sync::SyncSession;
use std::time::Instant;

async fn host_file_database(dir: &std::path::Path) -> Database {
    resolve_for_launch(dir).unwrap();
    Database::open_in_memory()
        .await
        .unwrap()
        .with_host_file(dir)
}

/// The combined port method writes owner and credential into host.json
/// together, and keeps a stored owner.
#[tokio::test]
async fn adopt_with_credential_writes_both_and_keeps_an_existing_owner() {
    let dir = tempfile::tempdir().unwrap();
    let db = host_file_database(dir.path()).await;

    db.adopt_user_identity_with_credential("c0ffee", "token-1")
        .await
        .unwrap();
    let file = read_for_cli(dir.path()).unwrap();
    assert_eq!(file.user_identity.as_deref(), Some("c0ffee"));
    assert_eq!(file.credential.as_deref(), Some("token-1"));

    db.adopt_user_identity_with_credential("other", "token-2")
        .await
        .unwrap();
    let file = read_for_cli(dir.path()).unwrap();
    assert_eq!(file.user_identity.as_deref(), Some("c0ffee"), "write-once");
    assert_eq!(file.credential.as_deref(), Some("token-2"));
}

/// The session's adoption never leaves a credential without its owner: with
/// the identity-only and credential-only writes made impossible by a
/// directory standing where host.json must be replaced mid-way, both fields
/// still arrive together or not at all. Here the plain path: a session
/// adoption lands both.
#[tokio::test]
async fn a_session_adoption_lands_owner_and_credential_in_host_json() {
    let dir = tempfile::tempdir().unwrap();
    let db = host_file_database(dir.path()).await;
    let connector = ScriptedConnector::new(vec![accepted("c0ffee", "token-1")]);
    let mut session = SyncSession::open("store.example", connector);

    session
        .connect_at_startup(&db, Instant::now())
        .await
        .unwrap();

    let file = read_for_cli(dir.path()).unwrap();
    assert_eq!(file.user_identity.as_deref(), Some("c0ffee"));
    assert_eq!(file.credential.as_deref(), Some("token-1"));
    assert!(host_file_path(dir.path()).exists());
    assert_eq!(
        db.user_identity_token().await.unwrap().as_deref(),
        Some("token-1")
    );
}
