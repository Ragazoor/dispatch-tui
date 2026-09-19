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
use crate::models::TaskId;
use crate::spacetime::bindings;
use crate::spacetime::bindings::{
    claim_backlog_task as _, create_epic as _, create_task as _, create_todo as _,
    delete_done_todos as _, delete_epic as _, delete_repo_path as _, delete_task as _,
    delete_todo as _, patch_epic as _, patch_task as _, patch_todo as _,
    recalculate_epic_status as _, record_base_branch as _, release_backlog_claim as _,
    save_repo_path as _, set_task_epic as _, set_verify_command as _, subscribe_to_epic as _,
    unsubscribe_from_epic as _, DbConnection, EpicsTableAccess as _, HostsTableAccess as _,
    RepoBaseBranchesTableAccess as _, RepoPathsTableAccess as _, SubscriptionHandle,
    TasksTableAccess as _, TodosTableAccess as _,
};
use crate::sync::writes::{ReducerCaller, ReducerOutcome};

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

        // One table's three callbacks, so "every table gets all three" is
        // structural rather than something a reader verifies by counting.
        //
        // An update is an UPSERT, not a patch: the SDK hands over the old row
        // and the new one, and only the new one is kept — the rows are keyed by
        // id and a row's id cannot change.
        macro_rules! wire {
            ($table:ident, $upsert:ident, $remove:ident, $key:expr) => {{
                let rows = self.rows.clone();
                db.$table().on_insert(move |_, row| rows.$upsert(row));
                let rows = self.rows.clone();
                db.$table().on_update(move |_, _old, new| rows.$upsert(new));
                let rows = self.rows.clone();
                let key = $key;
                db.$table().on_delete(move |_, row| rows.$remove(key(row)));
            }};
        }

        wire!(tasks, upsert_task, remove_task, |row: &bindings::Task| {
            crate::models::TaskId(row.id)
        });
        wire!(epics, upsert_epic, remove_epic, |row: &bindings::Epic| {
            crate::models::EpicId(row.id)
        });
        wire!(todos, upsert_todo, remove_todo, |row: &bindings::Todo| {
            crate::models::TodoId(row.id)
        });
        wire!(
            repo_paths,
            upsert_repo_path,
            remove_repo_path,
            |row: &bindings::RepoPath| row.id
        );
        wire!(
            repo_base_branches,
            upsert_repo_base_branch,
            remove_repo_base_branch,
            |row: &bindings::RepoBaseBranch| row.id
        );
        // `hosts` is written out rather than passed through `wire!`. Its key is
        // a borrowed `&str` rather than an owned id, and a closure returning a
        // borrow of its own argument needs a higher-ranked bound the macro's
        // `$key:expr` cannot carry. Three lines of repetition beat a macro
        // contorted to fit one caller.
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
        self.take_dropped();
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

    /// Take whatever the SDK's `on_disconnect` last recorded, leaving nothing
    /// behind.
    ///
    /// One helper for the three places that need it — collecting a drop, and
    /// clearing a stale one on install and on disconnect — because each was
    /// otherwise four lines with its own `#[allow]`.
    fn take_dropped(&self) -> Option<String> {
        #[allow(clippy::unwrap_used)]
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
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
        self.take_dropped();
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

// ---------------------------------------------------------------------------
// The write side
// ---------------------------------------------------------------------------

/// [`ReducerCaller`] over this connector's live connection.
///
/// Spec: `docs/specs/sync.allium`'s `BoardWritesThroughTheStore`.
///
/// Holds the connector rather than a connection, because the connection is
/// replaced on every reconnect and a caller that captured one would go on
/// talking to a socket that is closed. Every call reads the current one, and
/// finding none is the refusal `AWriteWithNoConnectionIsRefused` demands.
pub struct SdkReducerCaller {
    connector: Arc<SpacetimeSdkConnector>,
}

impl SdkReducerCaller {
    pub fn new(connector: Arc<SpacetimeSdkConnector>) -> Self {
        Self { connector }
    }

    /// The live connection, or the refusal.
    ///
    /// ONE PLACE, so no call site can spell "the store is down" differently —
    /// the operator reads this string, and `EveryFailureNamesItself` says it
    /// has to be actionable.
    fn connection(&self) -> anyhow::Result<Arc<DbConnection>> {
        self.connector
            .connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                anyhow!(
                    "the shared store is not connected, so the change was not made \
                     and nothing was queued"
                )
            })
    }
}

/// Send a reducer call and wait for the store's answer.
///
/// The `*_then` form rather than the fire-and-forget one, and that is the whole
/// point: `sync.allium: EveryMutationIsAtomicAndAnswered` says a mutation ends
/// in acceptance or rejection, and a caller that did not wait could not tell
/// the operator which.
///
/// A REFUSAL IS NOT AN ERROR HERE. It comes back as
/// [`ReducerOutcome::Refused`], because at this layer the store was reached and
/// answered — which is a different thing from the store being unreachable, and
/// one caller (the claim) treats them differently. Turning a refusal into an
/// error is [`ReducerOutcome::into_result`], one level up.
///
/// The two things that ARE errors:
///
///   * the request could not be SENT — the socket went while we held it;
///   * the connection dropped before an answer came, which is the one outcome
///     where the caller genuinely cannot know whether the write landed.
async fn awaiting_answer<F>(what: &str, invoke: F) -> anyhow::Result<ReducerOutcome>
where
    F: FnOnce(oneshot::Sender<ReducerOutcome>) -> std::result::Result<(), spacetimedb_sdk::Error>,
{
    let (tx, rx) = oneshot::channel();
    invoke(tx).map_err(|why| anyhow!("could not send {what} to the shared store: {why}"))?;
    rx.await.map_err(|_| {
        anyhow!(
            "the connection to the shared store dropped before {what} was answered, \
             so it may or may not have been applied"
        )
    })
}

/// One reducer call whose only answer is "did it work?".
///
/// Written as a macro because the body is identical fifteen times over and the
/// only things that vary are the reducer's name and its arguments. Spelled out
/// fifteen times it would be fifteen chances to forget the callback.
macro_rules! answered_call {
    ($self:ident, $what:expr, $reducer:ident ( $($arg:expr),* $(,)? )) => {{
        let connection = $self.connection()?;
        awaiting_answer($what, move |tx| {
            connection
                .reducers
                .$reducer($($arg,)* move |_, result| {
                    let _ = tx.send(outcome_of(result));
                })
        })
        .await
    }};
}

#[async_trait]
impl ReducerCaller for SdkReducerCaller {
    /// Create a task and read its generated id back off the transaction.
    ///
    /// # How a reducer answers, given that it cannot
    ///
    /// The callback runs with a view of the database AFTER this transaction, so
    /// the row is there — the problem is saying which one it is. The row is
    /// matched on the fields this board just sent: the title, the repo, the
    /// owner, the epic and the creation instant, which is this board's clock to
    /// the millisecond. The highest matching id is taken.
    ///
    /// **The tie is real and it is benign.** Two identical creates from the
    /// same board inside one millisecond produce two indistinguishable rows,
    /// and this returns the later one's id. Both were genuinely created and
    /// both are the caller's; returning either returns a task the caller just
    /// made. What it cannot do is return somebody else's row, because the owner
    /// and the creation instant are ours.
    async fn create_task(&self, row: bindings::Task) -> anyhow::Result<TaskId> {
        let connection = self.connection()?;
        let wanted = row.clone();
        let answer = awaiting_answer("the new task", move |tx| {
            connection
                .reducers
                .create_task_then(row, move |ctx, result| {
                    let _ = tx.send(match result {
                        Ok(Ok(())) => ReducerOutcome::Applied(
                            ctx.db
                                .tasks()
                                .iter()
                                .filter(|t| matches_create(t, &wanted))
                                .map(|t| t.id)
                                .collect(),
                        ),
                        Ok(Err(why)) => ReducerOutcome::Refused(why),
                        Err(why) => ReducerOutcome::Refused(why.to_string()),
                    });
                })
        })
        .await?;

        generated_id(answer, "task").map(TaskId)
    }

    async fn patch_task(
        &self,
        id: TaskId,
        patch: bindings::TaskPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the task change", patch_task_then(id.0, patch))
    }

    async fn delete_task(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the task deletion", delete_task_then(id.0))
    }

    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: i64,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the epic move",
            set_task_epic_then(id.0, epic_id, owner)
        )
    }

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the claim", claim_backlog_task_then(id.0, host))
    }

    async fn release_backlog_claim(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the claim release", release_backlog_claim_then(id.0))
    }

    /// The epic twin of [`Self::create_task`], matched the same way.
    async fn create_epic(&self, row: bindings::Epic) -> anyhow::Result<i64> {
        let connection = self.connection()?;
        let wanted = row.clone();
        let answer = awaiting_answer("the new epic", move |tx| {
            connection
                .reducers
                .create_epic_then(row, move |ctx, result| {
                    let _ = tx.send(match result {
                        Ok(Ok(())) => ReducerOutcome::Applied(
                            ctx.db
                                .epics()
                                .iter()
                                .filter(|e| {
                                    e.title == wanted.title
                                        && e.parent_epic_id == wanted.parent_epic_id
                                        && e.created_at == wanted.created_at
                                })
                                .map(|e| e.id)
                                .collect(),
                        ),
                        Ok(Err(why)) => ReducerOutcome::Refused(why),
                        Err(why) => ReducerOutcome::Refused(why.to_string()),
                    });
                })
        })
        .await?;

        generated_id(answer, "epic")
    }

    async fn patch_epic(
        &self,
        id: i64,
        patch: bindings::EpicPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the epic change", patch_epic_then(id, patch))
    }

    async fn delete_epic(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the epic deletion", delete_epic_then(id))
    }

    async fn recalculate_epic_status(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the epic recalculation",
            recalculate_epic_status_then(id)
        )
    }

    /// The todo twin of [`Self::create_task`].
    ///
    /// Matched on the title, the owner and the creation instant. Weaker than
    /// the task's — a todo has no repo — and weaker than it looks: two todos
    /// with the same title on one checklist in one millisecond tie, and the
    /// later id wins. Both are the caller's, as above.
    async fn create_todo(&self, row: bindings::Todo) -> anyhow::Result<i64> {
        let connection = self.connection()?;
        let wanted = row.clone();
        let answer = awaiting_answer("the new todo", move |tx| {
            connection
                .reducers
                .create_todo_then(row, move |ctx, result| {
                    let _ = tx.send(match result {
                        Ok(Ok(())) => ReducerOutcome::Applied(
                            ctx.db
                                .todos()
                                .iter()
                                .filter(|t| {
                                    t.title == wanted.title
                                        && t.owner == wanted.owner
                                        && t.created_at == wanted.created_at
                                })
                                .map(|t| t.id)
                                .collect(),
                        ),
                        Ok(Err(why)) => ReducerOutcome::Refused(why),
                        Err(why) => ReducerOutcome::Refused(why.to_string()),
                    });
                })
        })
        .await?;

        generated_id(answer, "todo")
    }

    async fn patch_todo(
        &self,
        id: i64,
        patch: bindings::TodoPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the todo change", patch_todo_then(id, patch))
    }

    async fn delete_todo(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the todo deletion", delete_todo_then(id))
    }

    async fn delete_done_todos(&self, owner: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the checklist clear", delete_done_todos_then(owner))
    }

    async fn save_repo_path(
        &self,
        path: String,
        last_used: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the repo path", save_repo_path_then(path, last_used))
    }

    async fn delete_repo_path(&self, path: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the repo path removal", delete_repo_path_then(path))
    }

    async fn set_verify_command(
        &self,
        path: String,
        command: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the verify command",
            set_verify_command_then(path, command)
        )
    }

    async fn record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the base branch",
            record_base_branch_then(repo_path, branch, last_used)
        )
    }

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the subscription",
            subscribe_to_epic_then(subscriber, epic_id)
        )
    }

    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the unsubscribe",
            unsubscribe_from_epic_then(subscriber, epic_id)
        )
    }
}

/// The answer of a call whose only answer is "did it work?".
fn outcome_of(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
) -> ReducerOutcome {
    match result {
        Ok(Ok(())) => ReducerOutcome::Applied(Vec::new()),
        Ok(Err(why)) => ReducerOutcome::Refused(why),
        Err(why) => ReducerOutcome::Refused(why.to_string()),
    }
}

/// Pull the generated id out of a create's answer.
///
/// An applied create with NO matching row is the one confusing case, and the
/// message says what it really means: the store made the row, and this board's
/// subscriptions do not cover where it landed. That is a configuration problem
/// rather than a failed write, and telling the operator the create failed would
/// send them looking for the wrong thing.
fn generated_id(answer: ReducerOutcome, what: &str) -> anyhow::Result<i64> {
    match answer {
        ReducerOutcome::Applied(ids) => ids.into_iter().max().ok_or_else(|| {
            anyhow!(
                "the shared store created the {what} but it is outside this board's \
                 subscriptions, so its id could not be read back"
            )
        }),
        ReducerOutcome::Refused(why) => Err(anyhow!("the shared store refused: {why}")),
    }
}

/// Whether `candidate` is a row this board's create could have produced.
///
/// Every field here is one the CLIENT chose, so a match cannot be a coincidence
/// with somebody else's work: `owner` and `created_at` between them narrow it to
/// this person, on this machine, in this millisecond.
fn matches_create(candidate: &bindings::Task, sent: &bindings::Task) -> bool {
    candidate.title == sent.title
        && candidate.repo_path == sent.repo_path
        && candidate.owner == sent.owner
        && candidate.epic_id == sent.epic_id
        && candidate.created_at == sent.created_at
}
