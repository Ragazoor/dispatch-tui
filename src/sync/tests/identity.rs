//! The three-way verdict on an identity the store offers.
//!
//! `docs/specs/host.allium`: AdoptUserIdentity, ConfirmUnchangedUserIdentity,
//! RefuseAChangedUserIdentity.

use crate::db::{Database, HostStore};
use crate::sync::{identity_conflict_message, settle_identity, IdentityVerdict};

#[test]
fn an_install_with_no_stored_identity_adopts_the_one_it_is_offered() {
    let verdict = settle_identity(None, "user-a");

    assert_eq!(verdict, IdentityVerdict::Adopt("user-a".into()));
    assert!(verdict.is_adoption());
    assert_eq!(verdict.settled(), Some("user-a"));
}

/// The common case by a very large margin: every reconnect after the first.
#[test]
fn an_unchanged_identity_settles_without_a_write() {
    let verdict = settle_identity(Some("user-a"), "user-a");

    assert_eq!(verdict, IdentityVerdict::Unchanged("user-a".into()));
    assert!(
        !verdict.is_adoption(),
        "nothing changed, so nothing should be written"
    );
}

/// Both settled arms must answer "may I proceed?" the same way.
///
/// The failure this guards is specific and nasty: a caller that treated only
/// `Adopt` as permission would sync on its very first connection and never
/// again, which looks like a network problem and is not one.
#[test]
fn both_settled_verdicts_permit_the_board_to_proceed() {
    assert_eq!(settle_identity(None, "user-a").settled(), Some("user-a"));
    assert_eq!(
        settle_identity(Some("user-a"), "user-a").settled(),
        Some("user-a")
    );
}

#[test]
fn a_different_identity_is_a_conflict_and_never_settles() {
    let verdict = settle_identity(Some("user-a"), "user-b");

    assert_eq!(
        verdict,
        IdentityVerdict::Conflict {
            stored: "user-a".into(),
            offered: "user-b".into(),
        }
    );
    assert_eq!(
        verdict.settled(),
        None,
        "a board must not act as either identity while they disagree"
    );
    assert!(!verdict.is_adoption(), "the conflict must not be adopted");
}

/// The message has to carry both, because the identity the board is about to
/// stop holding is the one the operator cannot look up afterwards.
#[test]
fn the_conflict_message_names_both_identities() {
    let message = identity_conflict_message("user-a", "user-b");

    assert!(message.contains("user-a"), "{message}");
    assert!(message.contains("user-b"), "{message}");
}

// -- storage: the identity persists across restarts ------------------------

#[tokio::test]
async fn a_fresh_install_has_no_user_identity() {
    let db = Database::open_in_memory().await.unwrap();

    assert_eq!(db.user_identity().await.unwrap(), None);
    assert_eq!(db.user_identity_token().await.unwrap(), None);
}

/// Test 1 of the phase plan: minted on first connect, and still there after a
/// restart.
///
/// "Restart" is a second `Database` handle over the same file, which is what a
/// restart actually is from this code's point of view. An in-memory database
/// would prove nothing here — it is the persistence that is under test.
#[tokio::test]
async fn a_user_identity_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.db");

    {
        let db = Database::open(&path).await.unwrap();
        db.adopt_user_identity("user-a", "token-a").await.unwrap();
        assert_eq!(db.user_identity().await.unwrap().as_deref(), Some("user-a"));
    }

    let restarted = Database::open(&path).await.unwrap();
    assert_eq!(
        restarted.user_identity().await.unwrap().as_deref(),
        Some("user-a")
    );
    assert_eq!(
        restarted.user_identity_token().await.unwrap().as_deref(),
        Some("token-a"),
        "the credential must survive too — without it the next connection is issued a new identity"
    );
}

/// `core.allium: LocalHostOwnerIsWrittenOnce`, at the level below the one that
/// refuses a conflict.
///
/// The store layer must not quietly do what `settle_identity` refuses. If it
/// did, a caller that forgot to consult the verdict would silently adopt.
#[tokio::test]
async fn adopting_a_second_identity_does_not_overwrite_the_first() {
    let db = Database::open_in_memory().await.unwrap();
    db.adopt_user_identity("user-a", "token-a").await.unwrap();

    db.adopt_user_identity("user-b", "token-b").await.unwrap();

    assert_eq!(
        db.user_identity().await.unwrap().as_deref(),
        Some("user-a"),
        "the stored identity is written once and never again"
    );
}

/// Refreshing the proof of the SAME identity is ordinary, and must work.
#[tokio::test]
async fn the_credential_is_refreshed_for_the_same_identity() {
    let db = Database::open_in_memory().await.unwrap();
    db.adopt_user_identity("user-a", "token-a").await.unwrap();

    db.adopt_user_identity("user-a", "token-a2").await.unwrap();

    assert_eq!(db.user_identity().await.unwrap().as_deref(), Some("user-a"));
    assert_eq!(
        db.user_identity_token().await.unwrap().as_deref(),
        Some("token-a2")
    );
}

/// An identity with no credential survives until the next connection and then
/// presents as a conflict — worse than not storing it at all.
#[tokio::test]
async fn an_identity_without_a_credential_is_refused() {
    let db = Database::open_in_memory().await.unwrap();

    assert!(db.adopt_user_identity("user-a", "").await.is_err());
    assert!(db.adopt_user_identity("", "token-a").await.is_err());
    assert_eq!(db.user_identity().await.unwrap(), None);
}

/// Test 2 of the phase plan: one person, two machines.
///
/// The two installs mint different host ids and settle on the SAME user
/// identity, because the identity comes from the store and the host id does
/// not. This is the whole of "one user with two host ids owns both" — see
/// `src/sync/tests/subscriptions.rs` for the other half, that worktree gating
/// still keys on the host.
#[tokio::test]
async fn two_installs_of_one_person_hold_one_identity_and_two_host_ids() {
    let laptop = Database::open_in_memory().await.unwrap();
    let desktop = Database::open_in_memory().await.unwrap();

    let (laptop_host, _) = laptop.ensure_host_identity().await.unwrap();
    let (desktop_host, _) = desktop.ensure_host_identity().await.unwrap();

    // The store recognises the same person on both, because both present a
    // credential for the same identity.
    for db in [&laptop, &desktop] {
        let stored = db.user_identity().await.unwrap();
        let verdict = settle_identity(stored.as_deref(), "user-a");
        assert!(verdict.is_adoption());
        db.adopt_user_identity("user-a", "token-a").await.unwrap();
    }

    assert_ne!(
        laptop_host, desktop_host,
        "two machines must not share a host id — a derived one would collide \
         exactly where a shared board needs them separated"
    );
    assert_eq!(
        laptop.user_identity().await.unwrap(),
        desktop.user_identity().await.unwrap(),
        "one person is one identity, whichever machine they are at"
    );
}
