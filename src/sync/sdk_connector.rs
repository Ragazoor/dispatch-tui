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
use spacetimedb_sdk::{
    DbContext, Identity, SubscriptionHandle as _, Table as _, TableWithPrimaryKey as _,
};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

use super::{
    Accepted, ConnectError, SharedRows, StoreConnector, SubscriptionRequest, CONNECT_TIMEOUT,
};
use crate::spacetime::bindings::{
    DbConnection, EpicsTableAccess as _, HostsTableAccess as _, RepoBaseBranchesTableAccess as _,
    RepoPathsTableAccess as _, SubscriptionHandle, TasksTableAccess as _, TodosTableAccess as _,
};

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
            rows,
            dropped: Arc::new(Mutex::new(None)),
        }
    }

    /// Point every subscribed table at [`Self::rows`].
    ///
    /// Registered once per connection, right after it is installed and BEFORE
    /// anything is subscribed. The order matters: a subscription applied first
    /// delivers its initial rows through these same callbacks, and callbacks
    /// registered afterwards would miss every row that was already there —
    /// producing a board that is empty until somebody else edits something.
    ///
    /// **An update is an upsert, not a patch.** The SDK hands over the old row
    /// and the new one; only the new one is kept, because
    /// [`SharedRows`] is keyed by id and a row's id cannot change.
    fn wire_rows(&self, connection: &DbConnection) {
        let db = connection.db();

        let rows = self.rows.clone();
        db.tasks().on_insert(move |_, row| rows.upsert_task(row));
        let rows = self.rows.clone();
        db.tasks()
            .on_update(move |_, _old, new| rows.upsert_task(new));
        let rows = self.rows.clone();
        db.tasks()
            .on_delete(move |_, row| rows.remove_task(crate::models::TaskId(row.id)));

        let rows = self.rows.clone();
        db.epics().on_insert(move |_, row| rows.upsert_epic(row));
        let rows = self.rows.clone();
        db.epics()
            .on_update(move |_, _old, new| rows.upsert_epic(new));
        let rows = self.rows.clone();
        db.epics()
            .on_delete(move |_, row| rows.remove_epic(crate::models::EpicId(row.id)));

        let rows = self.rows.clone();
        db.todos().on_insert(move |_, row| rows.upsert_todo(row));
        let rows = self.rows.clone();
        db.todos()
            .on_update(move |_, _old, new| rows.upsert_todo(new));
        let rows = self.rows.clone();
        db.todos()
            .on_delete(move |_, row| rows.remove_todo(crate::models::TodoId(row.id)));

        let rows = self.rows.clone();
        db.repo_paths()
            .on_insert(move |_, row| rows.upsert_repo_path(row));
        let rows = self.rows.clone();
        db.repo_paths()
            .on_update(move |_, _old, new| rows.upsert_repo_path(new));
        let rows = self.rows.clone();
        db.repo_paths()
            .on_delete(move |_, row| rows.remove_repo_path(row.id));

        let rows = self.rows.clone();
        db.repo_base_branches()
            .on_insert(move |_, row| rows.upsert_repo_base_branch(row));
        let rows = self.rows.clone();
        db.repo_base_branches()
            .on_update(move |_, _old, new| rows.upsert_repo_base_branch(new));
        let rows = self.rows.clone();
        db.repo_base_branches()
            .on_delete(move |_, row| rows.remove_repo_base_branch(row.id));

        let rows = self.rows.clone();
        db.hosts().on_insert(move |_, row| rows.upsert_host(row));
        let rows = self.rows.clone();
        db.hosts()
            .on_update(move |_, _old, new| rows.upsert_host(new));
        let rows = self.rows.clone();
        db.hosts()
            .on_delete(move |_, row| rows.remove_host(&row.id));
    }

    /// Replace any previous connection, disconnecting it first, and drop what
    /// the old one had delivered.
    ///
    /// Dropping the old handle without disconnecting leaks a socket and a
    /// thread, and on a board that reconnects several times a day that is not a
    /// slow leak.
    fn install(&self, connection: Arc<DbConnection>) {
        #[allow(clippy::unwrap_used)]
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
        #[allow(clippy::unwrap_used)]
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        *slot = Some(connection);
    }

    /// Install a subscription, unsubscribing whatever this board held before.
    ///
    /// Unsubscribing consumes the handle, which is why the previous one has to
    /// be kept rather than dropped — see the field's own comment.
    fn replace_subscription(&self, handle: SubscriptionHandle) {
        #[allow(clippy::unwrap_used)]
        let previous = self
            .subscription
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(handle);
        if let Some(previous) = previous {
            let _ = previous.unsubscribe();
        }
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
        let (answer, identified) = answer_once::<Result<(Identity, String), String>>();
        let on_error = answer.clone();
        let on_drop = Arc::clone(&self.dropped);

        let built = tokio::task::spawn_blocking(move || {
            DbConnection::builder()
                .with_uri(server)
                .with_database_name(database)
                .with_token(token)
                .on_connect(move |_conn, identity, token| answer(Ok((identity, token.to_string()))))
                .on_connect_error(move |_ctx, error| on_error(Err(error.to_string())))
                // THE ONLY PLACE A LOST CONNECTION IS EVER NOTICED. Without
                // it the board sits in `connected` through a closed lid, a
                // tunnel or a restarted server: never retrying, never saying
                // anything, and still drawing rows nothing refreshes.
                .on_disconnect(move |_ctx, error| {
                    let reason = error.map_or_else(
                        || "the store closed the connection".to_string(),
                        |e| e.to_string(),
                    );
                    #[allow(clippy::unwrap_used)]
                    let mut slot = on_drop.lock().unwrap_or_else(|e| e.into_inner());
                    // First writer wins. A reconnect clears the slot, so a
                    // value already here is this same outage — and the first
                    // reason is the one that explains it.
                    slot.get_or_insert(reason);
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

        // FROM HERE ON A FAILURE MUST DISCONNECT. Past `run_threaded` there is
        // a live socket and a live thread, and dropping the handle closes
        // neither — so a bare `return Err(..)` below would leak both. That
        // matters because the failure this arm exists for, a store that accepts
        // and never identifies, is retried forever on the backoff schedule: at
        // one attempt a minute it is sixty leaked threads an hour on an
        // otherwise idle board.
        let answer = tokio::time::timeout(CONNECT_TIMEOUT, identified).await;
        let (identity, token) = match answer {
            Ok(Ok(Ok(answer))) => answer,
            Ok(Ok(Err(error))) => return Err(abandon(connection, ConnectError::new(error))),
            // The sender was dropped without answering — the SDK neither
            // connected nor reported an error. Named as its own reason because
            // "no answer at all" is the failure an operator is least likely to
            // guess at from a generic message.
            Ok(Err(_recv)) => {
                return Err(abandon(
                    connection,
                    ConnectError::new("the store closed the connection without identifying it"),
                ))
            }
            Err(_elapsed) => return Err(abandon(connection, ConnectError::timed_out(&target))),
        };

        self.wire_rows(&connection);
        self.install(Arc::new(connection));
        Ok(Accepted {
            identity: identity.to_hex().to_string(),
            token,
        })
    }

    async fn disconnect(&self) {
        #[allow(clippy::unwrap_used)]
        let previous = self
            .connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(connection) = previous {
            let _ = connection.disconnect();
        }
        // The rows belonged to that connection. Left behind they would be a
        // board's contents on screen with nothing live behind them, which is
        // the read-through `SharedRows` exists not to have.
        self.rows.clear();
        // And the drop slot goes with them: closing deliberately is not an
        // outage to report, and `disconnect` is called on paths the session has
        // already recorded the failure for.
        #[allow(clippy::unwrap_used)]
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }

    async fn take_drop(&self) -> Option<String> {
        #[allow(clippy::unwrap_used)]
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    async fn subscribe(&self, request: &SubscriptionRequest) -> Result<(), ConnectError> {
        let connection = self
            .current()
            .ok_or_else(|| ConnectError::new("cannot subscribe before connecting"))?;
        let queries = subscription_queries(request)
            .map_err(|e| ConnectError::new(format!("refusing to subscribe: {e}")))?;

        let (answer, applied) = answer_once::<Result<(), String>>();
        let on_error = answer.clone();

        let handle: SubscriptionHandle = connection
            .subscription_builder()
            .on_applied(move |_ctx| answer(Ok(())))
            .on_error(move |_ctx, error| on_error(Err(error.to_string())))
            .subscribe(queries);
        self.replace_subscription(handle);

        match tokio::time::timeout(CONNECT_TIMEOUT, applied).await {
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

/// A one-shot answer several callbacks can share: the first to fire wins.
///
/// The SDK hands out callbacks in pairs — connected/failed, applied/errored —
/// and exactly one of each pair will fire, but the type system does not say
/// which. One channel behind a shared slot is how both feed a single await,
/// and taking the sender is what makes the second caller a no-op rather than a
/// panic on a consumed channel.
///
/// Returned as a cloneable `Fn` rather than as the slot itself, so the "first
/// writer wins" protocol is written once here instead of at every callback.
fn answer_once<T: Send + 'static>() -> (
    impl Fn(T) + Clone + Send + Sync + 'static,
    oneshot::Receiver<T>,
) {
    let (tx, rx) = oneshot::channel();
    let slot = Arc::new(Mutex::new(Some(tx)));
    let answer = move |value: T| {
        #[allow(clippy::unwrap_used)]
        let sender = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(tx) = sender {
            let _ = tx.send(value);
        }
    };
    (answer, rx)
}

/// Close a connection that was established but cannot be used, and return the
/// error explaining why.
///
/// Exists so the failure paths past `run_threaded` cannot forget: dropping a
/// `DbConnection` closes neither its socket nor its thread, and these paths are
/// retried for as long as the store stays broken.
fn abandon(connection: DbConnection, error: ConnectError) -> ConnectError {
    let _ = connection.disconnect();
    error
}

/// The SQL this board asks the store for.
///
/// Two things vary, as `sync.allium`'s
/// `SubscriptionsCoverOnlyTheOwnBoardAndItsEpics` requires: this person's own
/// user board, and the epics they follow. Beside them sit the tables that have
/// no per-person or per-epic dimension at all — the host registry, this
/// person's own subscription rows, their checklist, and the shared repo lists.
///
/// **A table absent from this list renders empty.** The board keeps no second
/// copy, so an unasked-for table does not degrade to stale data; it degrades to
/// no data, which on screen is indistinguishable from having none. That is why
/// the set is enumerated here rather than grown as each view is noticed.
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
        // The checklist. Filtered by owner for the same reason the user board
        // is: `todo.allium` calls the overlay personal, and an unfiltered ask
        // is every colleague's checklist on this screen.
        format!("SELECT * FROM todos WHERE owner = '{owner}'"),
        // The repo lists, unfiltered and deliberately so. Neither table has an
        // owner to filter on and neither wants one: a path and a branch name
        // describe the work rather than the person, and a colleague adding a
        // repo is a colleague saying where the code lives. Nothing private
        // travels in them, so the containment claim above is untouched.
        "SELECT * FROM repo_paths".to_string(),
        "SELECT * FROM repo_base_branches".to_string(),
    ];

    // The epic ids are integers by type, so they need no validation beyond
    // being integers.
    for epic in &request.epics {
        queries.push(format!("SELECT * FROM epics WHERE id = {epic}"));
        queries.push(format!("SELECT * FROM tasks WHERE epic_id = {epic}"));
    }

    Ok(queries)
}
