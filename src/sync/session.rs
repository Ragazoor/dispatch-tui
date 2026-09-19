//! The driver: one connection, kept up, re-identified and re-subscribed.
//!
//! Spec: `docs/specs/sync.allium`. This is where the rules in that file are
//! sequenced — `OpenBoardConnection`, `ConnectionAccepted`,
//! `SubscribeOnceIdentityIsSettled`, `ConnectionAttemptFailed`,
//! `RetryAfterBackoff` and `StopOnAUserIdentityConflict` — around the state
//! machine in [`super::connection`], which owns the edges but does no I/O.
//!
//! **The loop is here because nothing below provides one.** The SpacetimeDB
//! Rust SDK does not reconnect on its own, so without this a board needs
//! restarting after every wifi handover. That is the entire reason this module
//! exists, and it is why [`SyncSession::step`] takes the instant rather than
//! reading a clock: a twenty-minute outage is then something a test states
//! rather than something it waits out.

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

use super::{
    settle_identity, BoardConnection, ConnectionEvent, ConnectionStatus, IdentityVerdict,
    StoreConnector, SubscriptionRequest,
};
use crate::db::{HostStore, IdentityCredentialStore, SubscriptionStore};

/// The store surface a sync session needs: who this install is, what proves it,
/// and what it follows.
///
/// Narrower than the whole database on purpose — a session has no business
/// reading tasks — and assembled from existing traits rather than declared
/// afresh, so there is one definition of "store the identity" and not two.
///
/// It spans BOTH halves of the store seam, and that is the honest shape rather
/// than an oversight: the identity and the subscriptions are shared domain,
/// while the credential that proves the identity is a local secret that must
/// never reach a store other people can read.
pub trait SyncStore: HostStore + SubscriptionStore + IdentityCredentialStore {}

impl<T: HostStore + SubscriptionStore + IdentityCredentialStore> SyncStore for T {}

/// What one [`SyncSession::step`] did, for a caller that wants to log or
/// render it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Nothing was due. The overwhelmingly common answer once connected.
    Idle,
    /// An attempt was made and accepted; the identity settled and the
    /// subscriptions were asserted.
    Connected,
    /// An attempt was made and failed. The board is disconnected and saying
    /// why.
    Failed,
    /// The backoff elapsed and a retry is now in flight.
    Retrying,
    /// The store identified this install as somebody else. Terminal.
    Conflicted,
    /// The transport reported that a connection which was up has gone down.
    Dropped,
}

/// One board's connection to one shared store, driven by repeated [`Self::step`]
/// calls.
pub struct SyncSession {
    connector: Arc<dyn StoreConnector>,
    connection: BoardConnection,
}

impl SyncSession {
    /// Open a connection to `server`, in flight and not yet answered.
    ///
    /// Nothing is attempted here. `OpenBoardConnection` creates the connection
    /// and returns; the board draws immediately and the first attempt happens
    /// on the first [`Self::step`]. A constructor that connected would make
    /// every cold start as slow as the slowest network the board has been on.
    pub fn open(server: impl Into<String>, connector: Arc<dyn StoreConnector>) -> Self {
        Self {
            connector,
            connection: BoardConnection::opening(server),
        }
    }

    pub fn connection(&self) -> &BoardConnection {
        &self.connection
    }

    pub fn status(&self) -> ConnectionStatus {
        self.connection.status()
    }

    /// Report that a connection which was up has gone down.
    ///
    /// Called from outside because a drop is not something a poll discovers: a
    /// store that stops answering without closing anything is the normal case
    /// on a sleeping laptop, and a session that only learned of drops by
    /// failing to act would report `Connected` at a screen nobody is updating.
    pub fn report_drop(&mut self, reason: impl Into<String>, now: Instant) -> bool {
        self.connection.apply(
            ConnectionEvent::Dropped {
                reason: reason.into(),
            },
            now,
        )
    }

    /// Do whatever is due at `now`.
    ///
    /// Safe to call as often as the caller likes: every state but `Connecting`
    /// and a due `Disconnected` answers [`StepOutcome::Idle`] without touching
    /// the store.
    pub async fn step(&mut self, store: &dyn SyncStore, now: Instant) -> Result<StepOutcome> {
        match self.connection.status() {
            ConnectionStatus::Connecting => self.attempt(store, now).await,
            ConnectionStatus::Disconnected if self.connection.is_retry_due(now) => {
                self.connection.apply(ConnectionEvent::RetryDue, now);
                Ok(StepOutcome::Retrying)
            }
            // Collected only while connected. The transport may notice a socket
            // die after the board has already given up on it, and applying that
            // late report would re-enter `disconnected` and restart the
            // backoff — a long outage turned into a hot retry loop.
            ConnectionStatus::Connected => match self.connector.take_drop().await {
                Some(reason) => {
                    self.report_drop(reason, now);
                    // CLOSED, not merely marked down, and this is the same call
                    // the failed-subscribe arm below already makes. Marking the
                    // state alone leaves the transport holding a dead handle
                    // for the whole backoff window, which has two consequences
                    // the spec forbids: the board goes on drawing the dropped
                    // connection's rows (`ADisconnectedBoardDrawsNoSharedRows`
                    // discards them), and a write is ATTEMPTED against the dead
                    // handle instead of being refused up front
                    // (`AWriteWithNoConnectionIsRefused`). Disconnecting is
                    // what clears both.
                    self.connector.disconnect().await;
                    Ok(StepOutcome::Dropped)
                }
                None => Ok(StepOutcome::Idle),
            },
            // `Failed`, and a `Disconnected` whose backoff has not elapsed.
            // `Failed` is terminal and deliberately never retried.
            _ => Ok(StepOutcome::Idle),
        }
    }

    async fn attempt(&mut self, store: &dyn SyncStore, now: Instant) -> Result<StepOutcome> {
        let token = store.user_identity_token().await?;
        let accepted = match self
            .connector
            .connect(self.connection.server(), token.as_deref())
            .await
        {
            Ok(accepted) => accepted,
            Err(error) => {
                self.connection.apply(
                    ConnectionEvent::AttemptFailed {
                        reason: error.reason().to_string(),
                    },
                    now,
                );
                return Ok(StepOutcome::Failed);
            }
        };

        // The identity is settled BEFORE anything is subscribed. A session that
        // subscribed on connect and identified afterwards would, in the
        // conflict case, already have asked for a stranger's rows before
        // discovering it was a stranger.
        let stored = store.user_identity().await?;
        let verdict = settle_identity(stored.as_deref(), &accepted.identity);

        // `Accepted` before the verdict is acted on, in BOTH arms.
        // `StopOnAUserIdentityConflict` fires on a connection that is up — the
        // conflict is discovered downstream of acceptance, and the transition
        // graph has no `connecting -> failed` edge that skips it.
        self.connection.apply(ConnectionEvent::Accepted, now);

        // The two settled verdicts share an arm on purpose. They answer "may I
        // proceed?" the same way, and a match that treated only the adopting
        // one as permission would sync on a board's first connection and never
        // again — a bug that reads as a network problem and is not one.
        let IdentityVerdict::Conflict { stored, offered } = verdict else {
            if verdict.is_adoption() {
                // Two writes on two halves of the store seam, and deliberately
                // not atomic. The credential goes first: an identity with no
                // credential cannot be proved again and presents as a conflict
                // on the next connection, whereas a credential with no identity
                // is simply unused and is overwritten by the next adoption.
                store.set_user_identity_token(&accepted.token).await?;
                store.adopt_user_identity(&accepted.identity).await?;
            }
            let epics = store.subscribed_epics(&accepted.identity).await?;
            let request = SubscriptionRequest::new(accepted.identity, epics);
            if let Err(error) = self.connector.subscribe(&request).await {
                // Subscribing is part of coming up. A connection that is
                // accepted and then cannot be subscribed is not a working
                // connection, so it becomes an ordinary outage and is retried,
                // rather than sitting in `Connected` syncing nothing.
                self.connection.apply(
                    ConnectionEvent::Dropped {
                        reason: error.reason().to_string(),
                    },
                    now,
                );
                // Closed, not merely abandoned. The socket underneath is live
                // and its row callbacks are still firing, so a connection left
                // installed goes on writing rows into a board whose status now
                // says `disconnected` — and the next attempt would install a
                // second connection beside it.
                self.connector.disconnect().await;
                return Ok(StepOutcome::Failed);
            }
            return Ok(StepOutcome::Connected);
        };

        self.connection
            .apply(ConnectionEvent::IdentityConflict { stored, offered }, now);
        // Terminal, so nothing will ever reuse this connection. Closing it here
        // is the only chance: `failed` is never retried, so no later `connect`
        // replaces it, and a live socket in the state the board calls fatal
        // would outlive everything else in this session.
        self.connector.disconnect().await;
        Ok(StepOutcome::Conflicted)
    }
}
