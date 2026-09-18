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

use anyhow::anyhow;
use async_trait::async_trait;
use spacetimedb_sdk::{DbContext, Identity};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

use super::{Accepted, ConnectError, StoreConnector, SubscriptionRequest, CONNECT_TIMEOUT};
use crate::spacetime::bindings::{DbConnection, SubscriptionHandle};

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
}

impl SpacetimeSdkConnector {
    pub fn new(database: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            connection: Mutex::new(None),
        }
    }

    /// Replace any previous connection, disconnecting it first.
    ///
    /// Dropping the old handle without disconnecting leaks a socket and a
    /// thread, and on a board that reconnects several times a day that is not a
    /// slow leak.
    fn install(&self, connection: Arc<DbConnection>) {
        #[allow(clippy::unwrap_used)]
        let mut slot = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = slot.take() {
            let _ = previous.disconnect();
        }
        *slot = Some(connection);
    }

    fn current(&self) -> Option<Arc<DbConnection>> {
        #[allow(clippy::unwrap_used)]
        self.connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl StoreConnector for SpacetimeSdkConnector {
    async fn connect(&self, server: &str, token: Option<&str>) -> Result<Accepted, ConnectError> {
        let server = server.to_string();
        let target = format!("{server}/{}", self.database);
        let database = self.database.clone();
        let token = token.map(str::to_owned);

        // `on_connect` fires on the SDK's own thread; the oneshot is how the
        // identity crosses back. `on_connect_error` feeds the same channel, so
        // exactly one of the two arms always answers and the timeout below is
        // the only other way out.
        let (tx, rx) = oneshot::channel::<Result<(Identity, String), String>>();
        let sender = Arc::new(Mutex::new(Some(tx)));
        let on_connect_sender = Arc::clone(&sender);
        let on_error_sender = Arc::clone(&sender);

        let built = tokio::task::spawn_blocking(move || {
            DbConnection::builder()
                .with_uri(server)
                .with_database_name(database)
                .with_token(token)
                .on_connect(move |_conn, identity, token| {
                    if let Some(tx) = take(&on_connect_sender) {
                        let _ = tx.send(Ok((identity, token.to_string())));
                    }
                })
                .on_connect_error(move |_ctx, error| {
                    if let Some(tx) = take(&on_error_sender) {
                        let _ = tx.send(Err(error.to_string()));
                    }
                })
                .build()
        });

        let connection = match tokio::time::timeout(CONNECT_TIMEOUT, built).await {
            Ok(Ok(Ok(connection))) => connection,
            Ok(Ok(Err(error))) => return Err(ConnectError::new(error.to_string())),
            Ok(Err(join)) => return Err(ConnectError::new(format!("connect task failed: {join}"))),
            Err(_elapsed) => return Err(ConnectError::timed_out(&target)),
        };

        // Nothing arrives, and no callback fires, until the connection is being
        // advanced. This is the SDK's explicit requirement rather than an
        // optimisation: a built connection that is never ticked never
        // progresses.
        connection.run_threaded();

        let (identity, token) = match tokio::time::timeout(CONNECT_TIMEOUT, rx).await {
            Ok(Ok(Ok(answer))) => answer,
            Ok(Ok(Err(error))) => return Err(ConnectError::new(error)),
            // The sender was dropped without answering — the SDK neither
            // connected nor reported an error. Named as its own reason because
            // "no answer at all" is the failure an operator is least likely to
            // guess at from a generic message.
            Ok(Err(_recv)) => {
                return Err(ConnectError::new(
                    "the store closed the connection without identifying it",
                ))
            }
            Err(_elapsed) => return Err(ConnectError::timed_out(&target)),
        };

        self.install(Arc::new(connection));
        Ok(Accepted {
            identity: identity.to_hex().to_string(),
            token,
        })
    }

    async fn subscribe(&self, request: &SubscriptionRequest) -> Result<(), ConnectError> {
        let connection = self
            .current()
            .ok_or_else(|| ConnectError::new("cannot subscribe before connecting"))?;
        let queries = subscription_queries(request)
            .map_err(|e| ConnectError::new(format!("refusing to subscribe: {e}")))?;

        let (tx, rx) = oneshot::channel::<Result<(), String>>();
        let sender = Arc::new(Mutex::new(Some(tx)));
        let applied_sender = Arc::clone(&sender);
        let error_sender = Arc::clone(&sender);

        let _handle: SubscriptionHandle = connection
            .subscription_builder()
            .on_applied(move |_ctx| {
                if let Some(tx) = take(&applied_sender) {
                    let _ = tx.send(Ok(()));
                }
            })
            .on_error(move |_ctx, error| {
                if let Some(tx) = take(&error_sender) {
                    let _ = tx.send(Err(error.to_string()));
                }
            })
            .subscribe(queries);

        match tokio::time::timeout(CONNECT_TIMEOUT, rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => Err(ConnectError::new(error)),
            Ok(Err(_recv)) => Err(ConnectError::new(
                "the store dropped the subscription without applying it",
            )),
            Err(_elapsed) => Err(ConnectError::new(format!(
                "the subscription was not applied within {}s",
                CONNECT_TIMEOUT.as_secs()
            ))),
        }
    }
}

fn take<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    #[allow(clippy::unwrap_used)]
    slot.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The SQL this board asks the store for.
///
/// Exactly two things, as `sync.allium`'s
/// `SubscriptionsCoverOnlyTheOwnBoardAndItsEpics` requires: this person's own
/// user board, and the epics they follow. Everything else a board needs —
/// hosts, and this person's own subscription rows — follows from those.
///
/// **The identity is validated, not escaped.** It comes from the store as hex
/// and nothing else is a valid identity, so anything outside that alphabet is
/// refused rather than quoted. A quoting rule is a thing to get subtly wrong;
/// a closed alphabet is not.
pub(super) fn subscription_queries(request: &SubscriptionRequest) -> anyhow::Result<Vec<String>> {
    let owner = &request.owner_board;
    if owner.is_empty() || !owner.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "a user identity must be non-empty hexadecimal, got {owner:?}"
        ));
    }

    let mut queries = vec![
        // The host registry. Unfiltered on purpose: a task carrying a `host`
        // needs that host to resolve to something, and which machines those
        // are is not knowable in advance.
        "SELECT * FROM hosts".to_string(),
        // This person's own subscription rows, so a change made on another of
        // their machines arrives here.
        format!("SELECT * FROM subscriptions WHERE subscriber = '{owner}'"),
        // The user board: epic-less tasks this person owns.
        format!("SELECT * FROM tasks WHERE owner = '{owner}'"),
    ];

    // The epic ids are integers by type, so they need no validation beyond
    // being integers.
    for epic in &request.epics {
        queries.push(format!("SELECT * FROM epics WHERE id = {epic}"));
        queries.push(format!("SELECT * FROM tasks WHERE epic_id = {epic}"));
    }

    Ok(queries)
}
