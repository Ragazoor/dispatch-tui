//! Whose install is this? — the three-way verdict on an offered identity.
//!
//! Spec: `docs/specs/host.allium` (AdoptUserIdentity, ConfirmUnchangedUserIdentity,
//! RefuseAChangedUserIdentity).
//!
//! The shared store answers every accepted connection with the identity it
//! believes the client to be. This module is the whole of what dispatch does
//! with that answer, and it is deliberately a pure function over two strings:
//! the decision is worth being able to read in one screen, because one of its
//! three arms is the only fatal condition in the sync subsystem.

/// What an offered identity means for an install that may already hold one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityVerdict {
    /// First connection ever. Store it; from here on this install can prove it
    /// is the same person.
    Adopt(String),
    /// Already ours. Nothing to write, and — importantly — the same permission
    /// to proceed as `Adopt` carries.
    Unchanged(String),
    /// The store says this install is somebody else. Never benign: see
    /// `RefuseAChangedUserIdentity` for why this is not repaired by adopting.
    Conflict { stored: String, offered: String },
}

impl IdentityVerdict {
    /// Whether this verdict obliges a write to the stored identity.
    ///
    /// The two settled arms deliberately have no accessor collapsing them:
    /// callers match on `Conflict` and treat everything else as permission to
    /// proceed, which is the distinction that matters. A helper answering "may
    /// I proceed?" existed here once and had no caller — the match says it
    /// better, because it cannot be forgotten.
    pub fn is_adoption(&self) -> bool {
        matches!(self, Self::Adopt(_))
    }
}

/// Decide what an offered identity means, given what this install already
/// stores.
///
/// `stored` is `core/Host.owner` for the local row — the one and only place an
/// install remembers who it is (see `core.allium`'s `LocalHostOwnerIsWrittenOnce`).
pub fn settle_identity(stored: Option<&str>, offered: &str) -> IdentityVerdict {
    match stored {
        None => IdentityVerdict::Adopt(offered.to_string()),
        Some(stored) if stored == offered => IdentityVerdict::Unchanged(offered.to_string()),
        Some(stored) => IdentityVerdict::Conflict {
            stored: stored.to_string(),
            offered: offered.to_string(),
        },
    }
}

/// The message a conflict puts in front of an operator.
///
/// Carries BOTH identities because the one the board is about to stop holding
/// is the one they cannot look up afterwards — see
/// `host.allium: RefuseAChangedUserIdentity`.
pub fn identity_conflict_message(stored: &str, offered: &str) -> String {
    format!(
        "the store identified this install as {offered}, but it is stored as {stored}. \
         Your tasks are under {stored}. Restore the credential for {stored}, or clear \
         the stored identity to start again as {offered}."
    )
}
