//! The seam between the connection state machine and a real shared store.
//!
//! Spec: `docs/specs/sync.allium`'s `StoreBoundary` surface — the three events
//! the far side owes the board, and the guarantees that come with them.
//!
//! One trait, for two reasons. The obvious one is that the state machine can
//! then be driven against a fake that refuses, hangs or drops on command, so
//! every branch of the retry loop is testable without a server. The less
//! obvious one is that `sync.allium` names no transport, and a trait is how
//! that stays true in the code: sockets, polling and a person carrying a disk
//! all satisfy this, and nothing above it can tell which it got.

use async_trait::async_trait;
use std::fmt;

/// What the store says when it accepts a connection.
///
/// The identity arrives WITH the acceptance rather than being fetched
/// afterwards. A store that accepted a connection without saying who it thinks
/// you are would leave a window in which the board is up and nobody, and every
/// rule downstream would need an arm for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// The store's answer to "who is this client?". Opaque here: nothing in
    /// dispatch generates, validates or can recover one.
    pub identity: String,
    /// The credential that will prove this identity on the next connection.
    ///
    /// Presenting it is what makes the identity stable across restarts. Lose
    /// it and the store issues a new identity, which is the conflict
    /// `host.allium: RefuseAChangedUserIdentity` refuses.
    pub token: String,
}

/// Why an attempt did not produce a connection.
///
/// Deliberately one type rather than a taxonomy. `sync.allium`'s
/// ConnectionAttemptFailed covers refusal, an unresolvable host, a rejected
/// credential and the attempt that simply never answered, and folds them into
/// one state because an operator's next action is the same for all of them:
/// read the reason. The reason is carried, so nothing is lost by not branching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectError {
    reason: String,
}

impl ConnectError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// The attempt exceeded [`super::CONNECT_TIMEOUT`].
    ///
    /// Named rather than left to each connector to phrase, because silence is
    /// the failure an operator is least likely to guess at from a generic
    /// message, and it is the one that most often means the store is up but
    /// unreachable.
    pub fn timed_out(server: &str) -> Self {
        Self::new(format!(
            "{server} did not answer within {}s",
            super::CONNECT_TIMEOUT.as_secs()
        ))
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for ConnectError {}

/// Everything this board is asking the store to send it.
///
/// Exactly two things, and the second may be empty: this person's own user
/// board, and the epics they follow. There is no third field and there must not
/// be one — see `sync.allium`'s
/// `SubscriptionsCoverOnlyTheOwnBoardAndItsEpics`. A colleague's user board is
/// not absent from this struct because it is refused somewhere; it is absent
/// because there is nowhere to put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionRequest {
    /// The identity whose own user board is wanted. Always this connection's
    /// own — the type carries one identity, so asking for somebody else's is
    /// not expressible.
    pub owner_board: String,
    /// The epics to follow. Ascending, and may be empty.
    pub epics: Vec<i64>,
}

impl SubscriptionRequest {
    pub fn new(owner_board: impl Into<String>, epics: Vec<i64>) -> Self {
        Self {
            owner_board: owner_board.into(),
            epics,
        }
    }
}

/// A shared store this board can connect to.
///
/// Implementations own the transport and nothing else: no retry, no backoff and
/// no opinion about identity. Those live above, in
/// [`super::connection`] and [`super::identity`], so that swapping the
/// transport cannot change when the board gives up or who it thinks it is.
#[async_trait]
pub trait StoreConnector: Send + Sync {
    /// Attempt one connection.
    ///
    /// `token` is the stored credential, absent on an install that has never
    /// connected. Presenting it is what returns the same identity; presenting
    /// none asks the store to mint one.
    ///
    /// **Must not retry.** One call is one attempt, because the caller counts
    /// attempts to compute the backoff and a connector that retried internally
    /// would make that count a lie.
    ///
    /// **Must not block past [`super::CONNECT_TIMEOUT`].** A store that accepts
    /// and never answers is the failure this bound exists for; returning
    /// [`ConnectError::timed_out`] is how it becomes an outage the board can
    /// see rather than a `Connecting` state it never leaves.
    async fn connect(&self, server: &str, token: Option<&str>) -> Result<Accepted, ConnectError>;

    /// Ask the store for exactly `request`, replacing any previous ask.
    ///
    /// Called on every connection, including the reconnects where nothing
    /// changed: subscriptions are per-connection and a dropped connection takes
    /// them with it. Re-asserting an unchanged set costs a round trip; assuming
    /// it survived costs a board that silently stops updating.
    async fn subscribe(&self, request: &SubscriptionRequest) -> Result<(), ConnectError>;

    /// Close the connection and release what it holds.
    ///
    /// Called when the board has decided it will not connect again — today only
    /// on an identity conflict, which is terminal. Without it that state leaves
    /// a live socket and a live thread running for the lifetime of the process,
    /// in the one state where the board has decided to do nothing.
    ///
    /// Idempotent, and a no-op where there is nothing to close: a caller should
    /// not have to know whether a connection was ever established.
    async fn disconnect(&self) {}
}
