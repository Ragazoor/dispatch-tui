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
use chrono::{DateTime, Utc};
use spacetimedb_sdk::{
    DbContext, Identity, SubscriptionHandle as _, Table as _, TableWithPrimaryKey as _,
};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

use super::encode;
use super::{
    Accepted, ConnectError, SharedRows, StoreConnector, SubscriptionRequest, CONNECT_TIMEOUT,
    MUTATION_TIMEOUT,
};
use crate::models::{
    EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId, RetrievalSource,
    SubStatus, TaskId,
};
use crate::spacetime::bindings;
use crate::spacetime::bindings::{
    apply_learning_verdicts as _, archive_stale_learnings as _, batch_delete as _,
    batch_patch_sub_status as _, claim_backlog_task as _, claim_poll_owner as _,
    clear_setting as _, create_epic as _, create_learning as _, create_managed_role_epic as _,
    create_repo_group_sub_epic as _, create_task as _, create_task_watcher as _, delete_epic as _,
    delete_learning as _, delete_repo_path as _, delete_stale_subtree_feed_tasks as _,
    delete_task as _, delete_task_watcher as _, delete_watches_by_watcher as _,
    delete_watches_of_target as _, drop_closed_retired_feed_items as _,
    mark_pr_learnings_gate_shown as _, override_poll_owner as _, patch_epic as _,
    patch_learning as _, patch_task as _, recalculate_epic_status as _, record_base_branch as _,
    record_learning_retrieval as _, record_notification as _, record_pre_tool_use as _,
    record_usage_event as _, record_user_prompt_submit as _, register_host as _,
    release_backlog_claim as _, rescope_epic_learnings as _, respawn_phoenix_successor as _,
    save_repo_path as _, save_setting as _, set_task_epic as _, set_verify_command as _,
    subagent_clear as _, subagent_clear_and_void_pending_stop as _, subagent_start as _,
    subagent_stop as _, subscribe_to_epic as _, try_record_stop as _, unsubscribe_from_epic as _,
    upsert_feed_tasks as _, upsert_feed_tasks_additive as _, DbConnection, EpicsTableAccess as _,
    HostsTableAccess as _, LearningRetrievalsTableAccess as _, LearningsTableAccess as _,
    PollOwnersTableAccess as _, RepoBaseBranchesTableAccess as _, RepoPathsTableAccess as _,
    RetiredFeedItemsTableAccess as _, SettingsTableAccess as _, SubscriptionHandle,
    SubscriptionsTableAccess as _, TaskWatchersTableAccess as _, TasksTableAccess as _,
    UsageEventsTableAccess as _,
};
use crate::sync::subtree::SubtreeCover;
use crate::sync::writes::{DrainReadBack, ReducerCaller, ReducerOutcome};

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
        self.wire_tables(db);
        self.wire_subtree_walk(db);
    }

    /// Every table's row callbacks. See [`Self::wire_rows`].
    fn wire_tables(&self, db: &bindings::RemoteTables) {
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

        wire!(
            poll_owners,
            upsert_poll_owner,
            remove_poll_owner,
            |row: &bindings::PollOwner| row.id
        );
        wire!(
            learnings,
            upsert_learning,
            remove_learning,
            |row: &bindings::Learning| crate::models::LearningId(row.id)
        );
        wire!(
            learning_retrievals,
            upsert_learning_retrieval,
            remove_learning_retrieval,
            |row: &bindings::LearningRetrieval| row.id
        );
        wire!(
            usage_events,
            upsert_usage_event,
            remove_usage_event,
            |row: &bindings::UsageEvent| row.id
        );
        wire!(
            retired_feed_items,
            upsert_retired_feed_item,
            remove_retired_feed_item,
            |row: &bindings::RetiredFeedItem| row.id
        );

        wire!(
            task_watchers,
            upsert_task_watcher,
            remove_task_watcher,
            |row: &bindings::TaskWatcher| row.id
        );
        wire!(
            subscriptions,
            upsert_subscription,
            remove_subscription,
            |row: &bindings::Subscription| row.id.clone()
        );
        wire!(
            settings,
            upsert_setting,
            remove_setting,
            |row: &bindings::Setting| { row.id.clone() }
        );
    }

    /// The sub-epic walk: about the ASK rather than the rows, so apart from
    /// the table wiring. A follow widens the ask, on the initial load and live
    /// alike — see `follow_epic`.
    fn wire_subtree_walk(&self, db: &bindings::RemoteTables) {
        let subtree = Arc::clone(&self.subtree);
        db.subscriptions()
            .on_insert(move |ctx, row| follow_epic(ctx, &subtree, row.epic_id));
        let subtree = Arc::clone(&self.subtree);
        db.epics()
            .on_insert(move |ctx, row| widen_subtree(ctx, &subtree, row));
        let subtree = Arc::clone(&self.subtree);
        db.epics().on_update(move |ctx, old, new| {
            if old.parent_epic_id != new.parent_epic_id {
                widen_subtree(ctx, &subtree, new);
            }
        });
    }

    /// Build the SDK connection and hand back the channel its identity will
    /// arrive on. Everything up to, and not including, advancing it.
    async fn open_connection(
        &self,
        server: &str,
        token: Option<&str>,
    ) -> Result<
        (
            DbConnection,
            oneshot::Receiver<Result<(Identity, String), String>>,
        ),
        ConnectError,
    > {
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

        Ok((connection, identified))
    }

    /// Replace any previous connection, disconnecting it first, and drop what
    /// the old one had delivered.
    ///
    /// Dropping the old handle without disconnecting leaks a socket and a
    /// thread, and on a board that reconnects several times a day that is not a
    /// slow leak.
    fn install(&self, connection: Arc<DbConnection>) {
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
    fn reset_subtree(&self, followed: &[i64]) {
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
    fn take_dropped(&self) -> Option<String> {
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn current(&self) -> Option<Arc<DbConnection>> {
        self.connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl StoreConnector for SpacetimeSdkConnector {
    async fn connect(&self, server: &str, token: Option<&str>) -> Result<Accepted, ConnectError> {
        let target = format!("{server}/{}", self.database);
        let (connection, identified) = self.open_connection(server, token).await?;

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
        // The walk was that connection's too; its widenings died with it.
        self.reset_subtree(&[]);
        // And the drop slot goes with them: closing deliberately is not an
        // outage to report, and `disconnect` is called on paths the session has
        // already recorded the failure for.
        self.take_dropped();
    }

    async fn take_drop(&self) -> Option<String> {
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

        self.reset_subtree(&request.epics);
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

/// One connection's sub-epic walk: what is covered, and the subscriptions
/// that widened the ask to reach it.
///
/// The handles are kept for the reason [`SpacetimeSdkConnector::subscription`]
/// keeps its own: dropping one does not unsubscribe, so the next walk must be
/// able to end this one's.
#[derive(Default)]
struct Subtree {
    cover: SubtreeCover,
    widenings: Vec<SubscriptionHandle>,
}

/// An epic row arrived or moved: if it now sits under a covered epic, ask
/// for its subtree — and for that of any descendant this board already holds.
///
/// Runs on the SDK's thread, inside a row callback, which is why the answer
/// is a fresh subscription rather than a reply to anybody. A widening that
/// fails is logged and not retried: the rows it would have brought stay
/// missing until the next connection re-walks the tree (the rule's
/// "NOT RETRIED" clause).
fn widen_subtree(ctx: &bindings::EventContext, subtree: &Mutex<Subtree>, row: &bindings::Epic) {
    let mut subtree = subtree.lock().unwrap_or_else(|e| e.into_inner());
    // Checked before the cache is gathered: nearly every arrival — the whole
    // initial load included — sits under an uncovered parent or none.
    if !subtree.cover.covers(row.parent_epic_id) {
        return;
    }
    let newly = subtree
        .cover
        .delivered(row.id, row.parent_epic_id, &known_epics(ctx));
    let queries: Vec<String> = newly.into_iter().flat_map(subtree_queries).collect();
    subscribe_widening(ctx, &mut subtree, queries, "a sub-epic");
}

/// Every `(id, parent)` pair the connection holds, for [`SubtreeCover`].
///
/// The SDK's own cache, not `SharedRows`: it already holds the arriving row
/// when a callback fires, and callback order between the two epic handlers is
/// not something to depend on.
fn known_epics(ctx: &bindings::EventContext) -> Vec<(i64, i64)> {
    ctx.db
        .epics()
        .iter()
        .map(|epic| (epic.id, epic.parent_epic_id))
        .collect()
}

/// Send one widening subscription, and keep its handle with the walk so the
/// next `subscribe` unsubscribes it. A widening the store refuses is logged,
/// not retried.
fn subscribe_widening(
    ctx: &bindings::EventContext,
    subtree: &mut Subtree,
    queries: Vec<String>,
    what: &'static str,
) {
    if queries.is_empty() {
        return;
    }
    let handle = ctx
        .subscription_builder()
        .on_error(move |_ctx, error| {
            tracing::warn!("widening the subscription to {what} failed: {error}");
        })
        .subscribe(queries);
    subtree.widenings.push(handle);
}

/// A `Subscription` row arrived: ask for the epic it follows, and its tree.
///
/// Spec: `sync.allium`'s `ASubscriptionRowWidensTheAsk`. This is how followed
/// epics reach the ask at all. The session builds its first subscription
/// before any row has arrived, so the followed-epic list it can read then is
/// empty; the subscription rows come in with that first subscription, and
/// each one widens it from here. The same callback is what makes an epic
/// followed a moment ago — on this machine or another of this person's —
/// arrive without a reconnect.
///
/// Only widens, like `widen_subtree`: an unfollow leaves the epic asked for
/// until the next connection starts a fresh walk (`SubtreeCover`'s "only
/// grows").
fn follow_epic(ctx: &bindings::EventContext, subtree: &Mutex<Subtree>, epic: i64) {
    let mut subtree = subtree.lock().unwrap_or_else(|e| e.into_inner());
    if subtree.cover.covers(epic) {
        return;
    }
    let newly = subtree.cover.follow(epic, &known_epics(ctx));
    if newly.is_empty() {
        return;
    }
    let mut queries = vec![format!("SELECT * FROM epics WHERE id = {epic}")];
    queries.extend(newly.into_iter().flat_map(subtree_queries));
    subscribe_widening(ctx, &mut subtree, queries, "a followed epic");
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
        let sender = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(tx) = sender {
            fire(tx, value);
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
/// Three things vary, as `sync.allium`'s `SubscribeOnceIdentityIsSettled`
/// requires: this person's own user board, everything they created themselves
/// (own_creations), and the epics they follow. Beside them sit the tables that
/// have no per-person or per-epic dimension at all — the host registry, this
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

    let host = &request.host;
    // A minted host id is a UUID (`uuid::Uuid::new_v4().to_string()`), not
    // hex-only like the store identity above — hyphens are part of the
    // alphabet here. Still a closed alphabet, so still validated rather than
    // escaped, for the same reason `owner` is.
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(anyhow!(
            "a host id must be non-empty and alphanumeric, got {host:?}"
        ));
    }

    let mut queries = vec![
        // The host registry. Unfiltered on purpose: a task carrying a `host`
        // needs that host to resolve to something, and which machines those
        // are is not knowable in advance.
        "SELECT * FROM hosts".to_string(),
        // Poll ownership claims. Unfiltered for the same reason `hosts` is:
        // every host's tick needs to know who owns EVERY scope, not only the
        // ones it happens to already hold, to tell "unclaimed" from "someone
        // else's" (`core.allium: PollOwner`).
        "SELECT * FROM poll_owners".to_string(),
        // This person's own subscription rows, so a change made on another of
        // their machines arrives here.
        format!("SELECT * FROM subscriptions WHERE subscriber = '{owner}'"),
        // The user board: epic-less tasks this person owns.
        format!("SELECT * FROM tasks WHERE owner = '{owner}'"),
        // own_creations (sync.allium: SubscribeOnceIdentityIsSettled):
        // unconditional, unlike the per-epic asks below — a task or epic this
        // person created is in the subscription cache the moment a create
        // answers, regardless of which epic it landed in or whether anyone
        // follows it yet. This is what makes the reducer-completion read-back
        // in `generated_id` (below) actually work for an epic and for a task
        // in an unfollowed epic, neither of which `owner`/`epic_id` cover.
        format!("SELECT * FROM tasks WHERE created_by = '{owner}'"),
        format!("SELECT * FROM epics WHERE created_by = '{owner}'"),
        // The repo lists, unfiltered and deliberately so. Neither table has an
        // owner to filter on and neither wants one: a path and a branch name
        // describe the work rather than the person, and a colleague adding a
        // repo is a colleague saying where the code lives. Nothing private
        // travels in them, so the containment claim above is untouched.
        "SELECT * FROM repo_paths".to_string(),
        "SELECT * FROM repo_base_branches".to_string(),
        // Who is watching which task (`task-watchers.allium`). Unfiltered,
        // like the repo lists: a row is two task ids and a timestamp, and a
        // watch on a task this board holds may have been placed by a watcher
        // task it does not, so there is nothing narrower to ask for that
        // would still deliver every watch the fan-out needs.
        "SELECT * FROM task_watchers".to_string(),
        // Settings (`docs/specs/settings.allium`). Scoped by HOST, not by
        // owner: a setting is this machine's own, and
        // host is known even before an identity settles, unlike everything
        // above that filters by `owner`.
        format!("SELECT * FROM settings WHERE host = '{host}'"),
        // The knowledge base (`docs/specs/learnings.allium`'s Storage Backend
        // section). Unfiltered, like `repo_paths` above and for the same
        // reason: a learning's visibility is governed entirely by its own
        // scope/scope_ref, not by who created it or which machine is asking.
        "SELECT * FROM learnings".to_string(),
        "SELECT * FROM learning_retrievals".to_string(),
        // Usage telemetry (Phase 11, task #4915). Unfiltered, like the
        // knowledge base above: append-only with no user-observable rule
        // beyond "recorded", and nothing scopes it by owner or host.
        "SELECT * FROM usage_events".to_string(),
        // Retired feed items (`core.allium: RetiredFeedItem`, task #4971).
        // Unfiltered: every host on the board must refuse the same ids, or a
        // second host's feed cycle would re-insert what the first one's human
        // deleted — see the module's own doc comment on `RetiredFeedItem`.
        "SELECT * FROM retired_feed_items".to_string(),
    ];

    // The epic ids are integers by type, so they need no validation beyond
    // being integers. Each followed epic brings its own row plus its subtree
    // asks; everything deeper arrives as the connector widens the
    // subscription (`SubtreeCover`).
    for epic in &request.epics {
        queries.push(format!("SELECT * FROM epics WHERE id = {epic}"));
        queries.extend(subtree_queries(*epic));
    }

    Ok(queries)
}

/// What a covered epic is asked for beyond its own row: its direct tasks and
/// its direct sub-epics.
///
/// Spec: `sync.allium`'s `ASubEpicOfAFollowedEpicIsAskedForToo`. The second
/// query is what walks the tree — the store's SQL cannot follow a parent
/// chain, so each level is asked for by the one above it, and each sub-epic
/// that arrives is asked for in turn by [`SubtreeCover`]. The sub-epic's own
/// row needs no query of its own: its parent's sub-epics ask already covers it.
pub(super) fn subtree_queries(epic: i64) -> Vec<String> {
    vec![
        format!("SELECT * FROM tasks WHERE epic_id = {epic}"),
        format!("SELECT * FROM epics WHERE parent_epic_id = {epic}"),
    ]
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
    /// Where the session publishes why the connection is down.
    ///
    /// Read only when there is no connection, to make the refusal name the
    /// outage rather than merely report one — `sync.allium`'s
    /// `AWriteWithNoConnectionIsRefused` carries `connection.last_error`, and
    /// the session that owns it is deliberately not reachable from the
    /// transport.
    status: Arc<super::SettledIdentity>,
}

impl SdkReducerCaller {
    pub fn new(connector: Arc<SpacetimeSdkConnector>, status: Arc<super::SettledIdentity>) -> Self {
        Self { connector, status }
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
            .ok_or_else(|| match self.status.last_error() {
                Some(why) => anyhow!(
                    "the shared store is unreachable ({why}), so the change was not made \
                     and nothing was queued"
                ),
                // Before the first connection has failed there is nothing to
                // quote, and inventing a cause would be worse than saying only
                // what is certain.
                None => anyhow!(
                    "the shared store is not connected, so the change was not made \
                     and nothing was queued"
                ),
            })
    }
}

/// Deliver a reducer's answer to the caller waiting on it.
///
/// A send fails only when the caller already stopped waiting — it timed out in
/// [`awaiting_answer`] — so the answer has nowhere to go. Logged rather than
/// dropped silently: a late answer is the one trace that a write did land
/// after the caller was told it "may or may not" have.
fn fire<T>(tx: oneshot::Sender<T>, answer: T) {
    if tx.send(answer).is_err() {
        tracing::debug!("a reducer answered after its caller stopped waiting");
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
///
/// Generic over the payload `T` rather than fixed to [`ReducerOutcome`]: most
/// callers still send that (via [`outcome_of`]/[`outcome_with_ids`]), but the
/// agent-session-state methods below send a smaller, precisely typed answer
/// instead of squeezing theirs into `ReducerOutcome`'s `Vec<i64>`.
async fn awaiting_answer<T, F>(what: &str, invoke: F) -> anyhow::Result<T>
where
    F: FnOnce(oneshot::Sender<T>) -> std::result::Result<(), spacetimedb_sdk::Error>,
{
    let (tx, rx) = oneshot::channel();
    invoke(tx).map_err(|why| anyhow!("could not send {what} to the shared store: {why}"))?;
    match tokio::time::timeout(MUTATION_TIMEOUT, rx).await {
        Ok(Ok(answer)) => Ok(answer),
        // The sender was dropped: the callback registry went with the
        // connection.
        Ok(Err(_)) => Err(anyhow!(
            "the connection to the shared store dropped before {what} was answered, \
             so it may or may not have been applied"
        )),
        Err(_) => Err(anyhow!(
            "the shared store did not answer {what} within {}s, so it may or may not \
             have been applied",
            MUTATION_TIMEOUT.as_secs()
        )),
    }
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
                    fire(tx, outcome_of(result));
                })
        })
        .await
    }};
}

/// A create-shaped reducer call: send it, then read the generated id back off
/// the transaction by matching the row this board just sent.
///
/// The sibling of [`answered_call!`] for the calls whose answer is an id. What
/// varies is the reducer, the table to search and the predicate that picks
/// out the caller's own row; everything else — the callback, the read-back
/// and the refusal text of [`generated_id`] — is written once here.
macro_rules! created_call {
    ($self:ident, $what:expr, $label:expr, $reducer:ident ( $($arg:expr),* $(,)? ), $table:ident, |$row:ident| $pred:expr) => {{
        let connection = $self.connection()?;
        let answer = awaiting_answer($what, move |tx| {
            connection
                .reducers
                .$reducer($($arg,)* move |ctx, result| {
                    fire(
                        tx,
                        outcome_with_ids(result, || {
                            ctx.db
                                .$table()
                                .iter()
                                .filter(|$row| $pred)
                                .map(|$row| $row.id)
                                .collect()
                        }),
                    );
                })
        })
        .await?;
        generated_id(answer, $label)
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
    /// owner, the epic, the creator and the creation instant, which is this
    /// board's clock to the millisecond. The highest matching id is taken.
    ///
    /// **The tie is real and it is benign.** Two identical creates from the
    /// same board inside one millisecond produce two indistinguishable rows,
    /// and this returns the later one's id. Both were genuinely created and
    /// both are the caller's; returning either returns a task the caller just
    /// made. What it cannot do is return somebody else's row, because the
    /// creator and the creation instant are ours.
    async fn create_task(&self, row: bindings::Task) -> anyhow::Result<TaskId> {
        let wanted = row.clone();
        created_call!(
            self,
            "the new task",
            "task",
            create_task_then(row),
            tasks,
            |t| matches_create(t, &wanted)
        )
        .map(TaskId)
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
        epic_id: Option<EpicId>,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the epic move",
            set_task_epic_then(id.0, encode::epic_ref(epic_id), owner)
        )
    }

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the claim", claim_backlog_task_then(id.0, host))
    }

    async fn release_backlog_claim(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the claim release", release_backlog_claim_then(id.0))
    }

    /// The epic twin of [`Self::create_task`], matched the same way.
    async fn create_epic(&self, row: bindings::Epic) -> anyhow::Result<EpicId> {
        let wanted = row.clone();
        created_call!(
            self,
            "the new epic",
            "epic",
            create_epic_then(row),
            epics,
            |e| matches_created_epic(e, &wanted)
        )
        .map(EpicId)
    }

    async fn patch_epic(
        &self,
        id: EpicId,
        patch: bindings::EpicPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the epic change", patch_epic_then(id.0, patch))
    }

    async fn delete_epic(&self, id: EpicId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the epic deletion", delete_epic_then(id.0))
    }

    async fn batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the batch delete",
            batch_delete_then(
                task_ids.into_iter().map(|id| id.0).collect(),
                epic_ids.into_iter().map(|id| id.0).collect()
            )
        )
    }

    async fn recalculate_epic_status(&self, id: EpicId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the epic recalculation",
            recalculate_epic_status_then(id.0)
        )
    }

    async fn save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the repo path",
            save_repo_path_then(path, encode::stamp(last_used))
        )
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
        last_used: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the base branch",
            record_base_branch_then(repo_path, branch, encode::stamp(last_used))
        )
    }

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the subscription",
            subscribe_to_epic_then(subscriber, epic_id.0)
        )
    }

    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the unsubscribe",
            unsubscribe_from_epic_then(subscriber, epic_id.0)
        )
    }

    async fn save_setting(
        &self,
        host: String,
        key: String,
        value: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the setting", save_setting_then(host, key, value))
    }

    async fn clear_setting(&self, host: String, key: String) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the setting clear", clear_setting_then(host, key))
    }

    // -- Learnings and retrievals (Phase 10, task #4914) ----------------------

    /// Create a learning and read its generated id back off the transaction —
    /// the same mechanism [`Self::create_task`] uses, matched on content
    /// instead of on identity because a learning is not owned by anyone.
    async fn create_learning(&self, row: bindings::Learning) -> anyhow::Result<LearningId> {
        let wanted = row.clone();
        created_call!(
            self,
            "the new learning",
            "learning",
            create_learning_then(row),
            learnings,
            |l| matches_created_learning(l, &wanted)
        )
        .map(LearningId)
    }

    async fn patch_learning(
        &self,
        id: LearningId,
        patch: bindings::LearningPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning change",
            patch_learning_then(id.0, patch)
        )
    }

    async fn delete_learning(&self, id: LearningId) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the learning deletion", delete_learning_then(id.0))
    }

    async fn rescope_epic_learnings(
        &self,
        from: EpicId,
        to: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning re-scope",
            rescope_epic_learnings_then(from.0, to.0)
        )
    }

    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning retrieval",
            record_learning_retrieval_then(task_id.0, learning_id.0, source.as_str().to_string())
        )
    }

    async fn apply_learning_verdicts(
        &self,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the learning verdicts",
            apply_learning_verdicts_then(encode::verdict_inputs(&verdicts))
        )
    }

    async fn archive_stale_learnings(
        &self,
        cutoff: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the stale-learning sweep",
            archive_stale_learnings_then(encode::stamp(cutoff))
        )
    }

    // -- Usage events (Phase 11, task #4915) ---------------------------------

    async fn record_usage_event(
        &self,
        row: bindings::UsageEvent,
        cap: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(self, "the usage event", record_usage_event_then(row, cap))
    }

    // -- Agent session state (Phase 6b) --------------------------------------
    //
    // Every method below acts on a row whose id is already known — never a
    // generated one — so its answer is a read of THAT row by primary key,
    // inside the `_then` callback, whose view is "after this transaction" the
    // same way `create_task`'s is. No subscription-widening question arises:
    // a task with a live hook firing against it is one this board is already
    // running, hence already subscribed.
    //
    // Six of these answer with a bespoke type (`i64`, `DrainReadBack`,
    // `Option<bool>`) rather than `ReducerOutcome`'s `Vec<i64>`, because each
    // has a genuine fact to read back and a positional slot is exactly the
    // kind of thing a transposed index compiles cleanly through. None of them
    // has an application-level refusal that `ReducerOutcome::Refused` needs
    // to carry EXCEPT `try_record_stop`, whose `None` plays that role
    // directly. The other five never refuse (they mirror SQL paths with no
    // `requires` guard at all), so an `InternalError` from the SDK is folded
    // into a genuine `anyhow::Error` here rather than into "nothing
    // happened" — unlike [`outcome_of`]'s fold, which is correct for every
    // reducer that DOES have an ordinary refusal to answer with.

    /// `live_subagents` after the write. Never refuses — matches
    /// `src/db/queries/subagents.rs::subagent_start`, which has no precondition.
    async fn subagent_start(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> anyhow::Result<i64> {
        let connection = self.connection()?;
        let (task_id, started_at) = (task_id.0, encode::subagent_started_at(started_at));
        awaiting_answer("the subagent start", move |tx| {
            connection.reducers.subagent_start_then(
                task_id,
                agent_id,
                session_id,
                started_at,
                move |ctx, result| {
                    fire(
                        tx,
                        value_or_bail(result, "the subagent start", || {
                            ctx.db
                                .tasks()
                                .id()
                                .find(&task_id)
                                .map_or(0, |t| t.live_subagents)
                        }),
                    );
                },
            )
        })
        .await?
    }

    /// The live subagent count and whether the row is now in `review`, after
    /// the write. Never refuses — matches
    /// `src/db/queries/subagents.rs::subagent_stop`, where an unrecognised
    /// `agent_id` is a no-op rather than an error.
    async fn subagent_stop(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
    ) -> anyhow::Result<DrainReadBack> {
        let connection = self.connection()?;
        let task_id = task_id.0;
        awaiting_answer("the subagent stop", move |tx| {
            connection.reducers.subagent_stop_then(
                task_id,
                agent_id,
                session_id,
                move |ctx, result| {
                    fire(
                        tx,
                        value_or_bail(result, "the subagent stop", || {
                            subagent_drain_read_back(ctx, task_id)
                        }),
                    );
                },
            )
        })
        .await?
    }

    /// Same shape as [`Self::subagent_stop`] — see
    /// `src/db/queries/subagents.rs::subagent_clear`.
    async fn subagent_clear(&self, task_id: TaskId) -> anyhow::Result<DrainReadBack> {
        let connection = self.connection()?;
        let task_id = task_id.0;
        awaiting_answer("the subagent clear", move |tx| {
            connection
                .reducers
                .subagent_clear_then(task_id, move |ctx, result| {
                    fire(
                        tx,
                        value_or_bail(result, "the subagent clear", || {
                            subagent_drain_read_back(ctx, task_id)
                        }),
                    );
                })
        })
        .await?
    }

    async fn subagent_clear_and_void_pending_stop(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the subagent clear",
            subagent_clear_and_void_pending_stop_then(task_id.0)
        )
    }

    /// `None` when the task was not `Running` (this task's refusal, and the
    /// ONE agent-session-state method here with an application-level "no").
    /// Otherwise `Some(is_review)` — `true` if the row is now in `review`,
    /// `false` if it deferred instead. See `try_record_stop`'s doc comment in
    /// the module for why refusing the precondition is what makes this
    /// unambiguous.
    async fn try_record_stop(
        &self,
        id: TaskId,
        stop_pending_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<bool>> {
        let connection = self.connection()?;
        let (id, stop_pending_at) = (id.0, encode::stamp(stop_pending_at));
        awaiting_answer("the stop", move |tx| {
            connection
                .reducers
                .try_record_stop_then(id, stop_pending_at, move |ctx, result| {
                    fire(
                        tx,
                        flag_or_refused(result, || {
                            ctx.db
                                .tasks()
                                .id()
                                .find(&id)
                                .is_some_and(|t| is_review(&t.status))
                        }),
                    );
                })
        })
        .await
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the activity stamp",
            record_pre_tool_use_then(id.0, sub_status.as_str().to_string(), encode::stamp(at))
        )
    }

    async fn record_notification(
        &self,
        id: TaskId,
        mode: NotificationWrite,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let Some(mode) = encode::notification_mode(mode) else {
            return Ok(ReducerOutcome::Applied(vec![]));
        };
        answered_call!(
            self,
            "the notification",
            record_notification_then(id.0, mode.to_string(), encode::stamp(at))
        )
    }

    /// Plain applied/refused — `Resumed` vs. `Refreshed` is not decodable
    /// here at all (both write the identical row); `ReducerWriter` classifies
    /// it from a pre-read instead. See its own doc comment.
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        activity_at: DateTime<Utc>,
        prompt_at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the prompt",
            record_user_prompt_submit_then(
                id.0,
                encode::stamp(activity_at),
                encode::stamp(prompt_at)
            )
        )
    }

    async fn mark_pr_learnings_gate_shown(
        &self,
        id: TaskId,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the PR learnings gate",
            mark_pr_learnings_gate_shown_then(id.0, encode::stamp(at))
        )
    }

    // -- Feed ingestion (Phase 6c) --------------------------------------------
    //
    // Plain applied-or-refused calls: which rows a stale-delete removed is
    // decoded in `ReducerWriter`, from its own pre-read, not here — see this
    // task's plan doc, decision 1.

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the feed upsert",
            upsert_feed_tasks_then(epic_id.0, items, created_by)
        )
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the additive feed upsert",
            upsert_feed_tasks_additive_then(epic_id.0, items, created_by)
        )
    }

    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the stale feed task cleanup",
            delete_stale_subtree_feed_tasks_then(parent_id.0, keep_external_ids)
        )
    }

    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the closed-retirement cleanup",
            drop_closed_retired_feed_items_then(feed_epic_id.0, keep_external_ids)
        )
    }

    /// Find-or-create: matched by the domain key `(parent_id, title)` rather
    /// than content/timestamp — exact, not a tie-break, because that pair is
    /// genuinely unique (the module's own `create_repo_group_sub_epic` doc
    /// comment). Covers both arms: a FOUND epic answers with its existing id
    /// the same way a freshly created one answers with its new one.
    async fn create_repo_group_sub_epic(
        &self,
        parent_id: EpicId,
        title: String,
        created_by: String,
    ) -> anyhow::Result<EpicId> {
        let parent_id = parent_id.0;
        let wanted_title = title.clone();
        created_call!(
            self,
            "the repo-group epic",
            "repo-group epic",
            create_repo_group_sub_epic_then(parent_id, title, created_by),
            epics,
            |e| e.parent_epic_id == parent_id
                && e.title == wanted_title
                && e.origin == "repo-group"
        )
        .map(EpicId)
    }

    /// The managed-role twin, matched on `(parent_epic_id, feed_role)` — the
    /// module's own uniqueness key for this find-or-create, same reasoning as
    /// [`Self::create_repo_group_sub_epic`].
    async fn create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: Option<EpicId>,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> anyhow::Result<EpicId> {
        let parent_epic_id = encode::epic_ref(parent_epic_id);
        let wanted_role = role.clone();
        created_call!(
            self,
            "the managed-role epic",
            "managed-role epic",
            create_managed_role_epic_then(
                title,
                parent_epic_id,
                role,
                feed_command,
                feed_interval_secs,
                created_by
            ),
            epics,
            |e| e.parent_epic_id == parent_epic_id && e.feed_role == wanted_role
        )
        .map(EpicId)
    }

    // -- Task watchers -----------------------------------------------------------

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the watch",
            create_task_watcher_then(watcher_task_id.0, target_task_id.0)
        )
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the watch removal",
            delete_task_watcher_then(watcher_task_id.0, target_task_id.0)
        )
    }

    async fn delete_watches_of_target(
        &self,
        target_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the target's watches",
            delete_watches_of_target_then(target_task_id.0)
        )
    }

    async fn delete_watches_by_watcher(
        &self,
        watcher_task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the watcher's watches",
            delete_watches_by_watcher_then(watcher_task_id.0)
        )
    }

    async fn claim_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        answered_call!(
            self,
            "the poll claim",
            claim_poll_owner_then(scope.to_string(), scope_id, host)
        )
    }

    async fn override_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        answered_call!(
            self,
            "the poll override",
            override_poll_owner_then(scope.to_string(), scope_id, host)
        )
    }

    // -- Stragglers ------------------------------------------------------------

    async fn batch_patch_sub_status(
        &self,
        updates: Vec<(TaskId, SubStatus)>,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the sub-status batch",
            batch_patch_sub_status_then(encode::sub_status_updates(&updates))
        )
    }

    /// The phoenix twin of [`Self::create_task`]: matched the same way, on
    /// the fields the caller chose, not on `predecessor` — the successor is
    /// as much "the caller's own" as any other create, and a tie between two
    /// identical successors is the same benign case `matches_create` already
    /// accepts.
    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        successor: bindings::Task,
    ) -> anyhow::Result<TaskId> {
        let predecessor = predecessor.0;
        let wanted = successor.clone();
        created_call!(
            self,
            "the phoenix successor",
            "phoenix successor",
            respawn_phoenix_successor_then(predecessor, successor),
            tasks,
            |t| matches_create(t, &wanted)
        )
        .map(TaskId)
    }

    // -- Host registry (Phase 6c) -----------------------------------------------

    async fn register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        answered_call!(
            self,
            "the host registration",
            register_host_then(id, label, owner)
        )
    }
}

/// `[live, task_is_now_in_review]` off the task the drain acted on, shared by
/// every reducer whose answer is that shape (`subagent_stop`/`subagent_clear`).
/// `live` is `live_subagents`; giving both the identical slot layout is what
/// lets one function serve them rather than two near-duplicates.
fn subagent_drain_read_back(ctx: &bindings::ReducerEventContext, task_id: i64) -> DrainReadBack {
    match ctx.db.tasks().id().find(&task_id) {
        Some(t) => DrainReadBack {
            live: t.live_subagents,
            is_review: is_review(&t.status),
        },
        None => DrainReadBack::default(),
    }
}

/// Whether a task row's `status` is the module's spelling of `review`. One
/// predicate rather than the string literal compared twice.
fn is_review(status: &str) -> bool {
    status == "review"
}

/// Fold a raw reducer answer that has NO application-level refusal into its
/// read-back value, or a genuine error. Unlike [`outcome_of`], an
/// `Err(InternalError)` here is a real anomaly rather than an ordinary "the
/// store said no" — none of the methods this serves has anything to fold it
/// into, since SQL never refuses them either.
fn value_or_bail<T>(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
    what: &str,
    value: impl FnOnce() -> T,
) -> anyhow::Result<T> {
    match result {
        Ok(Ok(())) => Ok(value()),
        Ok(Err(why)) => Err(anyhow!(
            "the shared store refused {what}, which should never happen: {why}"
        )),
        Err(why) => Err(anyhow!("the shared store could not answer {what}: {why}")),
    }
}

/// Fold a raw reducer answer into `Some`/`None` — `None` for the ordinary
/// refusal `try_record_stop` uses as its `NoOp`, and ALSO for a transport
/// `InternalError`, mirroring [`outcome_of`]'s identical fold for every other
/// reducer that has a real refusal to answer with.
fn flag_or_refused<T>(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
    value: impl FnOnce() -> T,
) -> Option<T> {
    match result {
        Ok(Ok(())) => Some(value()),
        Ok(Err(_)) | Err(_) => None,
    }
}

/// A create's answer: the ids the callback found, or the store's refusal.
///
/// The two refusal arms are [`outcome_of`]'s rather than a third copy. They are
/// the arms no CI test reaches — they need a live store — so a fourth create
/// written by copy-paste with a dropped `Ok(Err(why))` arm would look correct
/// and silently turn a refusal into "created, but outside this board's
/// subscriptions".
///
/// `ids` is a closure so the scan only happens on the arm that uses it.
fn outcome_with_ids(
    result: std::result::Result<
        std::result::Result<(), String>,
        spacetimedb_sdk::__codegen::InternalError,
    >,
    ids: impl FnOnce() -> Vec<i64>,
) -> ReducerOutcome {
    match outcome_of(result) {
        ReducerOutcome::Applied(_) => ReducerOutcome::Applied(ids()),
        refused => refused,
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
/// # Why this can work at all
///
/// The view the callback reads is the client's subscription cache, so a
/// created row is findable only where a subscription already covers it
/// (`subscription_queries` above is the list). Task #4911 is the reason
/// `own_creations` is on that list unconditionally, for exactly this: an
/// epic-less task the operator owns arrives on `WHERE owner = …`, a task in a
/// followed epic on `WHERE epic_id = …`, and EVERYTHING ELSE — a brand-new
/// epic nobody follows yet, a task landing in an epic this board does not
/// follow — arrives on `WHERE created_by = …` instead, because
/// `SubscribeOnceIdentityIsSettled` asserts it the moment identity settles,
/// before any create this board makes could exist.
///
/// An applied create with NO matching row is now a real anomaly rather than
/// the ordinary case it used to be — own_creations covers every create this
/// identity can make — and the message still says what it would mean: the
/// store made the row, and this board's subscriptions do not cover where it
/// landed.
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

/// Whether `candidate` is a row this board's epic create could have produced.
///
/// `created_by` narrows to this identity's own epics — the field
/// `own_creations` subscribes by — which a coincidence with a colleague's epic
/// of the same title, parent and millisecond cannot satisfy.
fn matches_created_epic(candidate: &bindings::Epic, sent: &bindings::Epic) -> bool {
    candidate.title == sent.title
        && candidate.parent_epic_id == sent.parent_epic_id
        && candidate.created_at == sent.created_at
        && candidate.created_by == sent.created_by
}

/// Whether `candidate` is a row this board's create could have produced.
///
/// Every field here is one the CLIENT chose, so a match cannot be a coincidence
/// with somebody else's work. `owner` is blank for every task in an epic, so it
/// narrows nothing there; `created_by` is what actually pins a match to THIS
/// identity's own task when the candidate set includes an epic's other tasks
/// (from the `epic_id`-followed subscription) or nothing at all epic-scoped
/// (from `own_creations`).
fn matches_create(candidate: &bindings::Task, sent: &bindings::Task) -> bool {
    candidate.title == sent.title
        && candidate.repo_path == sent.repo_path
        && candidate.owner == sent.owner
        && candidate.epic_id == sent.epic_id
        && candidate.created_at == sent.created_at
        && candidate.created_by == sent.created_by
}

/// Whether `candidate` is a row this board's learning create could have
/// produced.
///
/// Every field here is one the CLIENT chose. Unlike `matches_create`, there
/// is no identity field to pin the match to THIS board's own call —
/// `docs/specs/learnings.allium` allows genuine duplicates (same kind,
/// summary, scope and scope_ref recorded twice), so two boards creating an
/// identical learning in the same millisecond tie benignly: both rows are
/// real, both are somebody's, and returning either returns a learning that call made.
fn matches_created_learning(candidate: &bindings::Learning, sent: &bindings::Learning) -> bool {
    candidate.kind == sent.kind
        && candidate.summary == sent.summary
        && candidate.scope == sent.scope
        && candidate.scope_ref == sent.scope_ref
        && candidate.source_task_id == sent.source_task_id
        && candidate.created_at == sent.created_at
}
