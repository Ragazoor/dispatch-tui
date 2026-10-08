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

use crate::models::EpicId;
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

use super::{
    settle_identity, BoardConnection, ConnectionEvent, ConnectionStatus, IdentityVerdict,
    StoreConnector, SubscriptionRequest,
};
use crate::store::{HostStore, IdentityCredentialStore, SubscriptionStore};

/// The store surface a sync session needs: who this install is, what proves it,
/// and what it follows.
///
/// Narrower than the whole database on purpose — a session has no business
/// reading tasks — and assembled from existing traits rather than declared
/// afresh, so there is one definition of "store the identity" and not two.
///
/// It mixes what is routed with what is not, and that is the honest shape
/// rather than an oversight: the subscriptions are shared rows, while the
/// identity and the credential that proves it are this install's own and must
/// never reach a store other people can read.
pub trait SyncStore: HostStore + SubscriptionStore + IdentityCredentialStore {}

impl<T: HostStore + SubscriptionStore + IdentityCredentialStore> SyncStore for T {}

/// What one [`SyncSession::step`] did, for a caller that wants to log or
/// render it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Nothing was due. The overwhelmingly common answer once connected. Also
    /// covers a quiet reassert reacting to a shrunk followed-epic set — see
    /// [`SyncSession::reassert_on_shrink`] — since the connection's own status
    /// did not change.
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
    /// This connection's settled identity. Empty until one has been, and
    /// never read before then — cached here, rather than re-read from the
    /// store on every tick, because it does not change for the life of a
    /// connection (a changed identity is `StopOnAUserIdentityConflict`,
    /// terminal, not a value this session would ever see updated in place).
    identity: String,
    /// The epics the last successful `subscribe` asked for. Compared against
    /// the store's current answer on every connected step, so a shrink —
    /// an unfollow — can be noticed and reasserted without a reconnect. A
    /// growth is not acted on here: that path is already live via the
    /// connector's own widen mechanism (`ASubscriptionRowWidensTheAsk`), and
    /// reasserting on every follow too would turn a flicker-free live-follow
    /// into a flickering one.
    subscribed_epics: Vec<EpicId>,
}

/// The shared shape of a subscribe request: this identity, asking for
/// `epics`, plus this host's own id. Both `SyncSession::attempt`'s initial
/// subscribe and `SyncSession::reassert_on_shrink`'s full reassert send
/// exactly this, so it is built once here rather than twice.
async fn build_subscription_request(
    store: &dyn SyncStore,
    identity: impl Into<String>,
    epics: Vec<EpicId>,
) -> Result<SubscriptionRequest> {
    let (host, _label) = store.ensure_host_identity().await?;
    let epics = epics.into_iter().map(|epic| epic.0).collect();
    Ok(SubscriptionRequest::new(identity, epics, host))
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
            identity: String::new(),
            subscribed_epics: Vec::new(),
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

    /// Make the first attempt, and answer whether the board may start.
    ///
    /// Spec: `sync.allium`'s `OpenBoardConnection`,
    /// `FirstConnectionLetsTheBoardDraw`, `FirstConnectionFailureAbortsStartup`
    /// and `IdentityConflictAtStartupAbortsIt`. The store is mandatory (task
    /// #4916), so a board that cannot reach it has nothing to draw; this is
    /// where startup finds out, before anything is on screen.
    ///
    /// `Ok` means connected, identity settled, and the initial subscription
    /// applied (the connector's `subscribe` returns only once it is). `Err`
    /// carries the reason, for `StartupAbort::StoreUnavailable`: the attempt's
    /// own on a failure, the composed conflict message on a conflict. There is
    /// no retry — the backoff is for a board that was up. Either way the
    /// session is left in the state the attempt put it in, so the caller can
    /// hand a connected one to [`Self::step`]'s loop.
    pub async fn connect_at_startup(
        &mut self,
        store: &dyn SyncStore,
        now: Instant,
    ) -> std::result::Result<(), String> {
        let outcome = self
            .attempt(store, now)
            .await
            .map_err(|e| format!("{e:#}"))?;
        match outcome {
            StepOutcome::Connected => Ok(()),
            _ => Err(self
                .connection
                .last_error()
                .unwrap_or("the store did not accept the connection")
                .to_string()),
        }
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
                None => {
                    self.reassert_on_shrink(store).await?;
                    Ok(StepOutcome::Idle)
                }
            },
            // `Failed`, and a `Disconnected` whose backoff has not elapsed.
            // `Failed` is terminal and deliberately never retried.
            _ => Ok(StepOutcome::Idle),
        }
    }

    /// If this identity's followed-epic set has shrunk since the last thing
    /// subscribed, reassert the whole subscription with the smaller list.
    ///
    /// Spec: `sync.allium`'s `AnUnfollowReassertsTheWholeAsk`. Only a shrink is
    /// acted on here — a growth already reaches the connector live through its
    /// own widen mechanism, and this reasserting on every follow too would
    /// turn that flicker-free path into a flickering one.
    ///
    /// A failed reassert is logged, not escalated: `self.subscribed_epics` is
    /// left unchanged, so the very next step notices the same shrink again and
    /// retries, rather than the whole connection being torn down over what is
    /// still, from the connection's point of view, healthy.
    async fn reassert_on_shrink(&mut self, store: &dyn SyncStore) -> Result<()> {
        let epics = store.subscribed_epics(&self.identity).await?;
        let shrank = self
            .subscribed_epics
            .iter()
            .any(|epic| !epics.contains(epic));
        if !shrank {
            self.subscribed_epics = epics;
            return Ok(());
        }
        let request =
            build_subscription_request(store, self.identity.clone(), epics.clone()).await?;
        match self.connector.subscribe(&request).await {
            Ok(()) => self.subscribed_epics = epics,
            Err(error) => {
                tracing::warn!("reasserting the subscription after an unfollow failed: {error}");
            }
        }
        Ok(())
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
                // ONE write: the owner and the credential that proves it
                // land together, so a crash never leaves an identity this
                // install cannot prove again (`host.allium: AdoptUserIdentity`).
                store
                    .adopt_user_identity_with_credential(&accepted.identity, &accepted.token)
                    .await?;
            }
            let epics = store.subscribed_epics(&accepted.identity).await?;
            let request =
                build_subscription_request(store, accepted.identity.clone(), epics.clone()).await?;
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
            self.identity = accepted.identity;
            self.subscribed_epics = epics;
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
