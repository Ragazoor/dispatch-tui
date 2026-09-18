//! The board's one connection to the shared store, as a state machine.
//!
//! Spec: `docs/specs/sync.allium` — the `BoardConnection` entity, its
//! transition graph, and the five rules that drive it.
//!
//! **Disconnection is ordinary here, not exceptional.** A laptop lid, a VPN
//! handover and a tunnel reconnect all produce a few seconds of outage several
//! times a day, so three of the four states below are healthy ones. Exactly one
//! is fatal, and it is not about the network at all — see
//! [`ConnectionEvent::IdentityConflict`].
//!
//! **Nothing here does any I/O.** Every transition is driven by an event the
//! caller supplies and an instant the caller reads, which is what lets the
//! retry schedule be asserted without a test ever waiting for one (see
//! `docs/testing.md`'s no-wall-clock-sleep rule).

use std::time::{Duration, Instant};

/// The wait before the first retry, doubled per consecutive failure.
///
/// `sync.allium: config.reconnect_backoff_base`. Short because the common
/// outage is seconds long, and a board that takes half a minute to notice the
/// network came back feels broken for half a minute after it was fixed.
pub const RECONNECT_BACKOFF_BASE: Duration = Duration::from_secs(1);

/// The longest the board will ever wait between attempts.
///
/// `sync.allium: config.reconnect_backoff_max`. The ceiling is for the hour-long
/// outage: unbounded doubling would have the board checking once a day by the
/// time somebody fixed it.
pub const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// How long one attempt may hang before it counts as failed.
///
/// `sync.allium: config.connect_timeout`. The interesting failure is silence,
/// not refusal: without this a store that accepts and never answers leaves the
/// board in [`ConnectionStatus::Connecting`] forever, which reads as "starting
/// up" rather than as the outage it is.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a connection is in its lifecycle.
///
/// The variants and the edges between them are `sync.allium`'s transition graph
/// verbatim; [`BoardConnection::apply`] is what enforces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// An attempt is in flight. Also the state a freshly opened connection
    /// starts in, and the one a board draws in.
    Connecting,
    /// Up, identified and syncing.
    Connected,
    /// Down and retrying. Ordinary.
    Disconnected,
    /// Terminal. Reached only by an identity conflict, never by a network
    /// failure, and deliberately not retried.
    Failed,
}

impl ConnectionStatus {
    /// Whether the board should be telling the operator something is wrong.
    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Connecting | Self::Connected)
    }
}

/// Something the outside world did to the connection.
///
/// One variant per rule in `sync.allium`, so a reader can line the two up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// The store accepted the attempt (`ConnectionAccepted`).
    Accepted,
    /// The attempt was refused, timed out or could not be made
    /// (`ConnectionAttemptFailed`).
    AttemptFailed { reason: String },
    /// A connection that was up is no longer (`ConnectionDropped`).
    Dropped { reason: String },
    /// The backoff has elapsed (`RetryAfterBackoff`).
    RetryDue,
    /// The store identified this install as somebody else
    /// (`StopOnAUserIdentityConflict`). The only fatal event.
    IdentityConflict { stored: String, offered: String },
}

/// This board's connection to one shared store.
#[derive(Debug, Clone)]
pub struct BoardConnection {
    server: String,
    status: ConnectionStatus,
    attempts: u32,
    last_error: Option<String>,
    last_failure_at: Option<Instant>,
}

impl BoardConnection {
    /// A connection in flight to `server`, as `OpenBoardConnection` creates it.
    pub fn opening(server: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            status: ConnectionStatus::Connecting,
            attempts: 0,
            last_error: None,
            last_failure_at: None,
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn status(&self) -> ConnectionStatus {
        self.status
    }

    /// Consecutive failed attempts since the last success.
    ///
    /// Counts the CURRENT outage: reset on acceptance rather than decayed, so a
    /// new outage backs off from the start rather than being punished for one
    /// that is over.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Why the connection is down, in terms an operator can act on.
    ///
    /// Present exactly while the status is unhealthy, which is what makes the
    /// pair of unhealthy states self-describing.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// When [`ConnectionEvent::RetryDue`] becomes true, or `None` when no retry
    /// is pending.
    ///
    /// `None` in every state but `Disconnected` — including `Failed`, where a
    /// retry would repeat an identity conflict forever, since nothing about a
    /// retry changes which person the store believes this is.
    pub fn next_attempt_at(&self) -> Option<Instant> {
        match self.status {
            ConnectionStatus::Disconnected => Some(self.last_failure_at? + backoff(self.attempts)),
            _ => None,
        }
    }

    /// Whether a retry is due at `now`.
    pub fn is_retry_due(&self, now: Instant) -> bool {
        self.next_attempt_at().is_some_and(|due| now >= due)
    }

    /// Apply `event`, returning whether it actually fired.
    ///
    /// `false` means the event's `requires` clause did not hold — a drop
    /// reported against a connection that was already down, say. The spec's
    /// answer for an unmet precondition on this kind of trigger is that the
    /// rule does not fire, and silently: there is no error, because an event
    /// that describes a state the connection is already in has nothing to
    /// report.
    pub fn apply(&mut self, event: ConnectionEvent, now: Instant) -> bool {
        use ConnectionEvent as E;
        use ConnectionStatus as S;

        match (self.status, event) {
            (S::Connecting, E::Accepted) => {
                self.status = S::Connected;
                self.attempts = 0;
                self.last_error = None;
                self.last_failure_at = None;
                true
            }
            (S::Connecting, E::AttemptFailed { reason })
            | (S::Connected, E::Dropped { reason }) => {
                self.status = S::Disconnected;
                self.attempts = self.attempts.saturating_add(1);
                self.last_error = Some(reason);
                self.last_failure_at = Some(now);
                true
            }
            (S::Disconnected, E::RetryDue) => {
                self.status = S::Connecting;
                self.last_error = None;
                self.last_failure_at = None;
                true
            }
            (S::Connected, E::IdentityConflict { stored, offered }) => {
                self.status = S::Failed;
                self.last_error = Some(super::identity_conflict_message(&stored, &offered));
                self.last_failure_at = None;
                true
            }
            _ => false,
        }
    }
}

/// How long to wait before the retry that follows `attempts` consecutive
/// failures.
///
/// Exponential from [`RECONNECT_BACKOFF_BASE`], capped at
/// [`RECONNECT_BACKOFF_MAX`]. `attempts` is 1 for the first failure, so the
/// first wait is the base itself.
pub fn backoff(attempts: u32) -> Duration {
    // `checked_mul` rather than shifting: a long outage runs the exponent past
    // what a Duration can hold, and the saturating answer is the ceiling
    // anyway. Overflowing to a short wait would turn the longest outage into
    // the busiest retry loop, which is the opposite of what a backoff is for.
    RECONNECT_BACKOFF_BASE
        .checked_mul(
            1u32.checked_shl(attempts.saturating_sub(1))
                .unwrap_or(u32::MAX),
        )
        .unwrap_or(RECONNECT_BACKOFF_MAX)
        .min(RECONNECT_BACKOFF_MAX)
}
