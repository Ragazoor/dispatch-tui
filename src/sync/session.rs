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
use crate::db::{HostStore, SubscriptionStore};

/// The store surface a sync session needs: who this install is, and what it
/// follows.
///
/// Narrower than the whole database on purpose — a session has no business
/// reading tasks — and assembled from the two existing shared-half traits
/// rather than declared afresh, so there is one definition of "store the
/// identity" and not two.
pub trait SyncStore: HostStore + SubscriptionStore {}

impl<T: HostStore + SubscriptionStore> SyncStore for T {}

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
            // Connected, Failed, and a Disconnected whose backoff has not
            // elapsed. `Failed` is terminal and deliberately never retried.
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

        let IdentityVerdict::Conflict { .. } = &verdict else {
            self.connection.apply(ConnectionEvent::Accepted, now);
            if verdict.is_adoption() {
                store
                    .adopt_user_identity(&accepted.identity, &accepted.token)
                    .await?;
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
                return Ok(StepOutcome::Failed);
            }
            return Ok(StepOutcome::Connected);
        };

        let IdentityVerdict::Conflict { stored, offered } = verdict else {
            unreachable!("the `let ... else` above matched Conflict");
        };
        // `Accepted` first, because `StopOnAUserIdentityConflict` fires on a
        // connection that is up — the conflict is discovered downstream of
        // acceptance, and the graph has no `connecting -> failed` edge that
        // skips it.
        self.connection.apply(ConnectionEvent::Accepted, now);
        self.connection
            .apply(ConnectionEvent::IdentityConflict { stored, offered }, now);
        Ok(StepOutcome::Conflicted)
    }
}
