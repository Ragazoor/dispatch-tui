//! The three-way verdict on an identity the store offers.
//!
//! `docs/specs/host.allium`: AdoptUserIdentity, ConfirmUnchangedUserIdentity,
//! RefuseAChangedUserIdentity.

use crate::sync::{identity_conflict_message, settle_identity, IdentityVerdict};

#[test]
fn an_install_with_no_stored_identity_adopts_the_one_it_is_offered() {
    let verdict = settle_identity(None, "user-a");

    assert_eq!(verdict, IdentityVerdict::Adopt("user-a".into()));
    assert!(verdict.is_adoption());
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

/// Only a conflict withholds permission to proceed.
///
/// The failure this guards is specific and nasty: a caller that treated only
/// `Adopt` as permission would sync on its very first connection and never
/// again, which looks like a network problem and is not one. Stated as "the
/// settled arms are exactly the non-conflict ones", because that is the
/// distinction every caller matches on.
#[test]
fn both_settled_verdicts_permit_the_board_to_proceed() {
    for verdict in [
        settle_identity(None, "user-a"),
        settle_identity(Some("user-a"), "user-a"),
    ] {
        assert!(
            !matches!(verdict, IdentityVerdict::Conflict { .. }),
            "{verdict:?} should permit the board to proceed"
        );
    }
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
