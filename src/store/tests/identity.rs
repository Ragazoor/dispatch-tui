//! The identity a shared store issued, kept by this install's [`Store`]: it
//! persists across restarts, is written once, and its credential refreshes.
//!
//! `docs/specs/host.allium`: AdoptUserIdentity. The verdict on an offered
//! identity is tested beside `settle_identity`, in `sync`'s own tests.

use crate::store::{HostStore, IdentityCredentialStore, Store};
use crate::sync::settle_identity;

#[tokio::test]
async fn a_fresh_install_has_no_user_identity() {
    let db = Store::open_in_memory().unwrap();

    assert_eq!(db.user_identity().await.unwrap(), None);
    assert_eq!(db.user_identity_token().await.unwrap(), None);
}

/// Test 1 of the phase plan: minted on first connect, and still there after a
/// restart.
///
/// "Restart" is a second `Store` handle over the same host file, which is
/// what a restart actually is from this code's point of view. A handle with a
/// private directory would prove nothing here — it is the persistence that is
/// under test.
#[tokio::test]
async fn a_user_identity_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    crate::host_file::resolve_for_launch(dir.path()).unwrap();

    {
        let db = Store::in_memory_with_host_file(dir.path());
        db.set_user_identity_token("token-a").await.unwrap();
        db.adopt_user_identity("user-a").await.unwrap();
        assert_eq!(db.user_identity().await.unwrap().as_deref(), Some("user-a"));
    }

    let restarted = Store::in_memory_with_host_file(dir.path());
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
    let db = Store::open_in_memory().unwrap();
    db.set_user_identity_token("token-a").await.unwrap();
    db.adopt_user_identity("user-a").await.unwrap();

    db.set_user_identity_token("token-b").await.unwrap();

    db.adopt_user_identity("user-b").await.unwrap();

    assert_eq!(
        db.user_identity().await.unwrap().as_deref(),
        Some("user-a"),
        "the stored identity is written once and never again"
    );
}

/// Refreshing the proof of the SAME identity is ordinary, and must work.
#[tokio::test]
async fn the_credential_is_refreshed_for_the_same_identity() {
    let db = Store::open_in_memory().unwrap();
    db.set_user_identity_token("token-a").await.unwrap();
    db.adopt_user_identity("user-a").await.unwrap();

    db.set_user_identity_token("token-a2").await.unwrap();

    db.adopt_user_identity("user-a").await.unwrap();

    assert_eq!(db.user_identity().await.unwrap().as_deref(), Some("user-a"));
    assert_eq!(
        db.user_identity_token().await.unwrap().as_deref(),
        Some("token-a2")
    );
}

/// Neither half of the identity may be blank.
///
/// An empty credential is refused because an identity this install cannot prove
/// again survives until the next connection and then presents as a conflict —
/// worse than not storing it at all. An empty identity is refused because it is
/// not an identity.
#[tokio::test]
async fn a_blank_identity_or_credential_is_refused() {
    let db = Store::open_in_memory().unwrap();

    assert!(db.set_user_identity_token("").await.is_err());
    assert!(db.set_user_identity_token("   ").await.is_err());
    assert!(db.adopt_user_identity("").await.is_err());
    assert!(db.adopt_user_identity("   ").await.is_err());
    assert_eq!(db.user_identity().await.unwrap(), None);
    assert_eq!(db.user_identity_token().await.unwrap(), None);
}

/// Test 2 of the phase plan: one person, two machines.
///
/// The two installs mint different host ids and settle on the SAME user
/// identity, because the identity comes from the store and the host id does
/// not. This is the whole of "one user with two host ids owns both" — see
/// `src/store/tests/over_sync/subscriptions.rs` for the other half, that worktree gating
/// still keys on the host.
#[tokio::test]
async fn two_installs_of_one_person_hold_one_identity_and_two_host_ids() {
    // Each machine mints its own host file, as a first launch does.
    let laptop_dir = tempfile::tempdir().unwrap();
    let desktop_dir = tempfile::tempdir().unwrap();
    crate::host_file::resolve_for_launch(laptop_dir.path()).unwrap();
    crate::host_file::resolve_for_launch(desktop_dir.path()).unwrap();
    let laptop = Store::in_memory_with_host_file(laptop_dir.path());
    let desktop = Store::in_memory_with_host_file(desktop_dir.path());

    let (laptop_host, _) = laptop.ensure_host_identity().await.unwrap();
    let (desktop_host, _) = desktop.ensure_host_identity().await.unwrap();

    // The store recognises the same person on both, because both present a
    // credential for the same identity.
    for db in [&laptop, &desktop] {
        let stored = db.user_identity().await.unwrap();
        let verdict = settle_identity(stored.as_deref(), "user-a");
        assert!(verdict.is_adoption());
        db.set_user_identity_token("token-a").await.unwrap();
        db.adopt_user_identity("user-a").await.unwrap();
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
