//! A [`StoreConnector`] over the SpacetimeDB Rust SDK.
//!
//! Spec: `docs/specs/sync.allium`'s `StoreBoundary` surface. This is one
//! implementation of it; nothing above it can tell which it got, which is what
//! keeps the retry loop and the identity rules testable without a server.
//!
//! # What the SDK does and does not give us
//!
//! It gives a WebSocket, a subscription protocol, and an identity handshake —
//! all of which would otherwise be hand-written, and the reason the plan links
//! it here rather than shelling out to the CLI as `SpacetimeCliStore` does for
//! dump and restore.
//!
//! **It does not reconnect.** There is no retry, no backoff and no
//! reconnection anywhere in the SDK. [`super::SyncSession`] is that, and this
//! type is deliberately one attempt per call: a connector that retried
//! internally would make the caller's attempt counter — and therefore the whole
//! backoff — a lie.
//!
//! # Blocking
//!
//! `DbConnection::builder().build()` performs a synchronous handshake, so it
//! runs on a blocking thread rather than on the async runtime. The timeout
//! around it is what turns a store that accepts and never answers into a
//! visible outage rather than a `Connecting` state the board never leaves.

mod answer;
mod callers;
mod connect;
mod outcome;
mod queries;
mod wiring;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod wiring_tests;

pub use callers::SdkReducerCaller;
pub(in crate::sync) use queries::{subscription_queries, subtree_queries};
use wiring::Subtree;

use spacetimedb_sdk::{DbContext as _, SubscriptionHandle as _};
use std::sync::{Arc, Mutex};

use crate::spacetime::bindings::{DbConnection, SubscriptionHandle};
use crate::sync::subtree::SubtreeCover;
use crate::sync::SharedRows;

/// Talks to one SpacetimeDB database over a WebSocket.
pub struct SpacetimeSdkConnector {
    /// The database's name or identity on the server, e.g. `dispatch`.
    ///
    /// Separate from the `server` passed to [`StoreConnector::connect`]: a
    /// server hosts many databases, and the one this board wants is a property
    /// of the connector rather than of the address.
    database: String,
    /// The live connection, once there is one.
    ///
    /// Behind a mutex because [`StoreConnector`] takes `&self` — a connector is
    /// shared, and a connection is the mutable thing it holds.
    connection: Mutex<Option<Arc<DbConnection>>>,
    /// The subscription this board currently holds, if any.
    ///
    /// Kept so the next one can REPLACE it. `subscribe` does not replace on its
    /// own — each call adds a set, and dropping the handle does not
    /// unsubscribe, because unsubscribing consumes it. Without this, a board
    /// that re-subscribed after following a new epic would hold two overlapping
    /// sets and receive every shared row twice.
    subscription: Mutex<Option<SubscriptionHandle>>,
    /// The sub-epic tree this connection's subscription reaches, and the
    /// extra subscriptions that widened it there.
    ///
    /// Spec: `sync.allium`'s `ASubEpicOfAFollowedEpicIsAskedForToo`. Shared
    /// with the epic row callbacks, which widen it on the SDK's thread as
    /// sub-epics arrive; reset by every `subscribe`, and emptied with the
    /// connection.
    subtree: Arc<Mutex<Subtree>>,
    /// Where arriving rows land.
    ///
    /// Held rather than passed per call because the row callbacks are
    /// registered once per connection and outlive the call that made them: they
    /// fire on the SDK's own thread, for as long as the connection is up.
    rows: Arc<SharedRows>,
    /// A drop the SDK reported and [`StoreConnector::take_drop`] has not yet
    /// handed over.
    ///
    /// The SDK's `on_disconnect` fires on its own thread and there is nothing
    /// to call: the session is not reachable from here and must not be, or the
    /// transport would be deciding the retry policy. So the reason lands here
    /// and the next step collects it.
    dropped: Arc<Mutex<Option<String>>>,
}

impl SpacetimeSdkConnector {
    pub fn new(database: impl Into<String>, rows: Arc<SharedRows>) -> Self {
        Self {
            database: database.into(),
            connection: Mutex::new(None),
            subscription: Mutex::new(None),
            subtree: Arc::new(Mutex::new(Subtree::default())),
            rows,
            dropped: Arc::new(Mutex::new(None)),
        }
    }

    /// Replace any previous connection, disconnecting it first, and drop what
    /// the old one had delivered.
    ///
    /// Dropping the old handle without disconnecting leaks a socket and a
    /// thread, and on a board that reconnects several times a day that is not a
    /// slow leak.
    pub(super) fn install(&self, connection: Arc<DbConnection>) {
        let mut slot = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = slot.take() {
            let _ = previous.disconnect();
            // Same reasoning as `disconnect` below: the rows were that
            // connection's, and the new subscription re-delivers from scratch.
            self.rows.clear();
        }
        // Cleared on EVERY install, not only when one is replaced. Closing the
        // old connection fires its own `on_disconnect` on the SDK's thread, and
        // that report must not surface as an outage of the connection that just
        // succeeded — which would take the board down one step after it came up.
        self.take_dropped();
        *slot = Some(connection);
    }

    /// Install a subscription, unsubscribing whatever this board held before.
    ///
    /// Unsubscribing consumes the handle, which is why the previous one has to
    /// be kept rather than dropped — see the field's own comment.
    pub(super) fn replace_subscription(&self, handle: SubscriptionHandle) {
        let previous = self
            .subscription
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(handle);
        if let Some(previous) = previous {
            let _ = previous.unsubscribe();
        }
    }

    /// Start a fresh subtree walk from `followed`, unsubscribing whatever the
    /// previous one had widened to.
    ///
    /// Called BEFORE the new subscription is sent: its initial rows fire the
    /// epic callbacks, and they must widen against the new followed set, not
    /// the old one.
    pub(super) fn reset_subtree(&self, followed: &[i64]) {
        let previous = std::mem::replace(
            &mut *self.subtree.lock().unwrap_or_else(|e| e.into_inner()),
            Subtree {
                cover: SubtreeCover::new(followed.iter().copied()),
                widenings: Vec::new(),
            },
        );
        for handle in previous.widenings {
            let _ = handle.unsubscribe();
        }
    }

    /// Take whatever the SDK's `on_disconnect` last recorded, leaving nothing
    /// behind.
    ///
    /// One helper for the three places that need it — collecting a drop, and
    /// clearing a stale one on install and on disconnect — because each was
    /// otherwise four lines with its own `#[allow]`.
    pub(super) fn take_dropped(&self) -> Option<String> {
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub(super) fn current(&self) -> Option<Arc<DbConnection>> {
        self.connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
