//! The transport a shared mutation goes through.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardWritesThroughTheStore`,
//! `AWriteWithNoConnectionIsRefused` and `StoreRejectsAnInvalidMutation`.
//!
//! [`crate::store::Store`] encodes each row and hands it to a
//! [`ReducerCaller`]; this module holds that trait, the store's verdict on a
//! call ([`ReducerOutcome`]), and the identity a write is stamped with
//! ([`WriterIdentity`], [`SettledIdentity`]) — the things a mutation needs that
//! the store cannot supply.
//!
//! # Why the transport is a second trait
//!
//! Everything interesting here is the encoding and the refusal path, and
//! neither needs a server. Splitting the transport out means the tests for both
//! run in CI, where no SpacetimeDB exists — the same reason
//! [`super::StoreConnector`] is a trait. [`SdkReducerCaller`](crate::sync::sdk_connector::SdkReducerCaller) is the one
//! implementation that talks to a real store.
//!
//! # Nothing here retries
//!
//! A refusal is returned, once, to the caller that asked. There is no buffer to
//! put it in and no loop to put it through: `sync.allium: NoWriteIsEverQueued`,
//! and the reasoning is in its guidance — a replayed write carries an intent
//! formed against a board state that no longer holds, and the operator was told
//! it had already worked.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::models::{
    EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId, RetrievalSource,
    SubStatus, TaskId,
};
use crate::spacetime::bindings;

/// One reducer call, and the store's verdict on it.
///
/// A method per reducer rather than one `call(name, args)`, so the argument
/// types are the generated ones and a signature that drifts from the module is
/// a compile error rather than a runtime decode failure.
/// What one reducer call did.
///
/// `Applied` and `Refused` are both successful ROUND TRIPS: the store was
/// reached and answered. A transport failure is the `Err` of the enclosing
/// `Result` instead, and keeping the two apart is what lets a caller treat
/// "somebody else won the claim" as an ordinary answer while still failing
/// loudly when the store is down (`sync.allium: AWriteWithNoConnectionIsRefused`
/// against `StoreRejectsAnInvalidMutation`).
#[derive(Debug, PartialEq, Eq)]
pub enum ReducerOutcome {
    /// The store applied it. Carries the ids of any rows the caller asked to
    /// have read back, which is the only way a reducer answers. A method with
    /// a different fact to read back (a live count, whether a row is now in
    /// `review`) is typed for it instead — see [`DrainReadBack`] — rather than
    /// squeezing it into this `Vec<i64>` by position.
    Applied(Vec<i64>),
    /// The store reached the call and declined it, with a reason.
    Refused(String),
}

impl ReducerOutcome {
    /// Turn a refusal into an error. The right reading for every mutation whose
    /// refusal means something went wrong, which is all of them except a claim.
    pub fn into_result(self) -> Result<Vec<i64>> {
        match self {
            Self::Applied(ids) => Ok(ids),
            Self::Refused(why) => Err(anyhow::anyhow!("the shared store refused: {why}")),
        }
    }

    /// [`Self::into_result`] for a caller with no ids to collect, which is
    /// every mutation but the three creates.
    pub fn applied(self) -> Result<()> {
        self.into_result().map(|_| ())
    }

    /// Whether it applied. The right reading for a claim, where a refusal is
    /// "somebody else got there first" rather than a fault.
    pub fn won(&self) -> bool {
        matches!(self, Self::Applied(_))
    }
}

/// The post-transaction read-back for a drain: the live count of the counter
/// this call touched, and whether the row is now in `review`. Two named
/// facts rather than positional `Vec<i64>` slots — shared by `subagent_stop`
/// and `subagent_clear`, which both answer off the same predicate
/// (`apply_pending_stop_if_drained` in the module).
#[derive(Debug, Clone, Copy, Default)]
pub struct DrainReadBack {
    pub live: i64,
    pub is_review: bool,
}

/// One reducer call, and the store's verdict on it.
///
/// A method per reducer rather than one `call(name, args)`, so the argument
/// types are the generated ones and a signature that drifts from the module is
/// a compile error rather than a runtime decode failure.
#[async_trait]
pub trait ReducerCaller: Send + Sync {
    /// Insert a task and answer with the id the store generated.
    ///
    /// The one call here that has to read a row back. A reducer cannot answer,
    /// so the id comes off the transaction the callback runs in — see
    /// [`super::SdkReducerCaller::create_task`] for how, and for what that
    /// costs.
    async fn create_task(&self, row: bindings::Task) -> Result<TaskId>;
    async fn patch_task(&self, id: TaskId, patch: bindings::TaskPatch) -> Result<ReducerOutcome>;
    async fn delete_task(&self, id: TaskId) -> Result<ReducerOutcome>;
    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: Option<EpicId>,
        owner: String,
    ) -> Result<ReducerOutcome>;

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> Result<ReducerOutcome>;
    async fn release_backlog_claim(&self, id: TaskId) -> Result<ReducerOutcome>;

    /// Insert an epic and answer with the id the store generated.
    async fn create_epic(&self, row: bindings::Epic) -> Result<EpicId>;
    async fn patch_epic(&self, id: EpicId, patch: bindings::EpicPatch) -> Result<ReducerOutcome>;
    async fn delete_epic(&self, id: EpicId) -> Result<ReducerOutcome>;
    async fn recalculate_epic_status(&self, id: EpicId) -> Result<ReducerOutcome>;

    /// `tasks.allium: BatchDelete`'s atomic call — see
    /// `spacetime/module/src/tasks_epics.rs::batch_delete`'s doc comment for why this
    /// is one reducer invocation over the whole selection rather than
    /// `delete_task`/`delete_epic` called once per item.
    async fn batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> Result<ReducerOutcome>;

    async fn save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
    ) -> Result<ReducerOutcome>;
    async fn delete_repo_path(&self, path: String) -> Result<ReducerOutcome>;
    async fn set_verify_command(&self, path: String, command: String) -> Result<ReducerOutcome>;
    async fn record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: DateTime<Utc>,
    ) -> Result<ReducerOutcome>;

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome>;
    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome>;

    // -- Settings (Phase 9) ---------------------------------------------------
    //
    // `host` is this connection's own host id, supplied by the `Store`
    // rather than by its caller — see `Store.host`'s doc comment. No id read-back needed for any of these: the primary key
    // is derived from `(host, key)`, which the caller already
    // knows before the call, the same trick `subscribe_to_epic` above uses.
    async fn save_setting(
        &self,
        host: String,
        key: String,
        value: String,
    ) -> Result<ReducerOutcome>;
    async fn clear_setting(&self, host: String, key: String) -> Result<ReducerOutcome>;

    // -- Learnings and retrievals (Phase 10, task #4914) ----------------------
    //
    // Nothing here is scoped by host — a learning's visibility is governed
    // entirely by its own scope/scope_ref (`docs/specs/learnings.allium`). `create_learning` needs the same content-matched id
    // read-back `create_task` uses: unlike `create_repo_group_sub_epic`, a
    // learning has no natural unique key — duplicates are a soft constraint
    // per `docs/specs/learnings.allium`, not something this call can match on
    // instead.
    async fn create_learning(&self, row: bindings::Learning) -> Result<LearningId>;
    async fn patch_learning(
        &self,
        id: LearningId,
        patch: bindings::LearningPatch,
    ) -> Result<ReducerOutcome>;
    async fn delete_learning(&self, id: LearningId) -> Result<ReducerOutcome>;
    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<ReducerOutcome>;
    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<ReducerOutcome>;
    async fn apply_learning_verdicts(
        &self,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<ReducerOutcome>;
    async fn archive_stale_learnings(&self, cutoff: DateTime<Utc>) -> Result<ReducerOutcome>;

    // -- Usage events (Phase 11, task #4915) ----------------------------------
    //
    // No id readback: recording an event returns nothing to the caller, so
    // there is no content-matched create like `create_task`/`create_learning`
    // need.
    async fn record_usage_event(
        &self,
        row: bindings::UsageEvent,
        cap: i64,
    ) -> Result<ReducerOutcome>;

    // Agent session state (Phase 6b). Every one of these acts on a row whose
    // id the caller already has, so what needs reading back is a FACT off
    // that row rather than an id to match by content — but unlike the three
    // creates, that fact isn't a bare id, so it gets its own type
    // ([`DrainReadBack`], a plain `i64`, `Option<bool>`) instead of being
    // squeezed into `ReducerOutcome`'s `Vec<i64>`: a positional slot the
    // caller has to remember the layout of is exactly the kind of thing a
    // typo compiles cleanly through. The four with no read-back ambiguity at
    // all (they're a plain applied-or-refused, nothing to decode) stay on
    // `ReducerOutcome` below, same as every earlier reducer.
    async fn subagent_start(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> Result<i64>;
    async fn subagent_stop(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
    ) -> Result<DrainReadBack>;
    async fn subagent_clear(&self, task_id: TaskId) -> Result<DrainReadBack>;
    async fn subagent_clear_and_void_pending_stop(&self, task_id: TaskId)
        -> Result<ReducerOutcome>;
    /// `None` for a refusal (the task was not `Running`) — this task's
    /// `StopOutcome::NoOp`. `Some(true)`/`Some(false)` is `Flipped`/`Deferred`,
    /// unambiguous once accepted (see `try_record_stop`'s doc comment in the
    /// module).
    async fn try_record_stop(
        &self,
        id: TaskId,
        stop_pending_at: DateTime<Utc>,
    ) -> Result<Option<bool>>;
    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome>;
    async fn record_notification(
        &self,
        id: TaskId,
        mode: NotificationWrite,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome>;
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        activity_at: DateTime<Utc>,
        prompt_at: DateTime<Utc>,
    ) -> Result<ReducerOutcome>;
    async fn mark_pr_learnings_gate_shown(
        &self,
        id: TaskId,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome>;

    // -- Feed ingestion (Phase 6c) --------------------------------------------
    //
    // No id read-back and no reported removals here — that decoding lives in
    // the `Store`, which predicts candidates from its own already-
    // subscribed view before the call and confirms them absent afterward
    // (see this task's plan doc, decision 1). These three are plain
    // applied-or-refused calls.
    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> Result<ReducerOutcome>;
    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> Result<ReducerOutcome>;
    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome>;

    // -- Retired feed items (task #4971) --------------------------------------
    //
    // A plain applied-or-refused call, like the three feed-ingestion ones
    // above: nothing here needs an id read back. There is no
    // `create_retired_feed_item` counterpart — every real retirement writes
    // the row inline from `delete_task`/`delete_epic` or the migration,
    // never through a standalone call.
    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome>;

    /// Find-or-create; answers with the epic's id either way. Matched by the
    /// domain key `(parent_id, title)` — exact, not a content/timestamp tie-
    /// break, because that pair is genuinely unique in the domain (decision 2
    /// of this task's plan doc).
    async fn create_repo_group_sub_epic(
        &self,
        parent_id: EpicId,
        title: String,
        created_by: String,
    ) -> Result<EpicId>;
    /// Find-or-create keyed on `(parent_epic_id, role)`, on the same terms as
    /// [`Self::create_repo_group_sub_epic`].
    async fn create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: Option<EpicId>,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> Result<EpicId>;

    // -- Task watchers ---------------------------------------------------------
    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome>;
    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome>;
    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<ReducerOutcome>;
    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<ReducerOutcome>;

    // -- Poll ownership (Phase 7) -----------------------------------------------
    async fn claim_poll_owner(&self, target: PollScopeId, host: String) -> Result<ReducerOutcome>;
    async fn override_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> Result<ReducerOutcome>;

    // -- Stragglers ------------------------------------------------------------
    async fn batch_patch_sub_status(
        &self,
        updates: Vec<(TaskId, SubStatus)>,
    ) -> Result<ReducerOutcome>;
    /// Insert the successor and read its generated id back — the same
    /// content-matched read-back [`Self::create_task`] uses.
    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        successor: bindings::Task,
    ) -> Result<TaskId>;

    // -- Host registry (Phase 6c) -----------------------------------------------
    //
    // NOT a store method — see [`push_host_registration`] and
    // `sync.allium: RegisterHostOnConnect`/`RegisterHostOnRename`. Still an
    // ordinary reducer call, which is why it lives on this transport trait
    // rather than being bolted on separately; [`push_host_registration`]
    // below is the best-effort wrapper both call sites share.
    async fn register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> Result<ReducerOutcome>;
}

/// Push this machine's current host row to the store, best-effort.
///
/// The shared wrapper `sync.allium: RegisterHostOnConnect` and
/// `RegisterHostOnRename` both call: a failed or delayed push must not block
/// the connection or the rename it followed, so this logs and returns rather
/// than propagating.
///
/// # The host registry is a decision, not an omission
///
/// `ensure_host_identity`, `adopt_user_identity` and `rename_host` are NOT
/// store writes and never will be: the identity handshake writes this
/// install's identity to `host.json`, before any connection exists, and that
/// write must keep happening unconditionally — it is the durable local
/// credential, not a shared row with one copy (the single-storage design
/// doc's declared permanent local exception). What reaches the store is this
/// separate best-effort mirror, pushed on every connect/reconnect and on a
/// live rename. Decided on task #4907 rather than assumed.
pub async fn push_host_registration(
    caller: &dyn ReducerCaller,
    id: String,
    label: String,
    owner: String,
) {
    let host_id = id.clone();
    match caller.register_host(id, label, owner).await {
        Ok(ReducerOutcome::Applied(_)) => {}
        Ok(ReducerOutcome::Refused(why)) => {
            tracing::warn!(
                host_id,
                "the shared store refused to register this host: {why}"
            );
        }
        Err(e) => {
            tracing::warn!(
                host_id,
                "failed to register this host with the shared store: {e:#}"
            );
        }
    }
}

/// Who the board is writing as.
///
/// # Why this is read per write rather than resolved at bootstrap
///
/// The obvious design is to resolve it once and hold the string. It does not
/// work: the writer is built at bootstrap, the identity is settled by the
/// CONNECTION, and the connection has not happened yet. A string captured then
/// would be whatever the last run stored, or nothing at all on a first run.
///
/// So this is a reader, consulted by the one mutation that needs an answer.
/// Creates are rare enough that a settings lookup on that path costs nothing
/// worth designing around, and everything else — every patch, every delete —
/// never asks.
#[async_trait]
pub trait WriterIdentity: Send + Sync {
    /// The user this board writes as, or `None` before it has ever connected.
    async fn user(&self) -> Result<Option<String>>;
}

/// The identity this board's connection settled on, once it has.
///
/// # Why a cell rather than a read of the local store
///
/// The local store does hold it — the handshake writes it there
/// (`host.allium: AdoptUserIdentity`) — but reading it from there at bootstrap
/// is not possible: the writer is built before the `Store` it would read,
/// and the two would refer to each other. Filling a cell afterwards breaks that
/// and is the truer statement besides. What a write may stamp is the identity
/// THIS CONNECTION settled, not whatever a previous run left on disk: a board
/// that has not connected must not create user-board tasks under a name it
/// cannot currently prove.
///
/// Empty until the first successful connection, and never emptied again. A
/// dropped connection does not clear it, because the person did not change —
/// and a write during the outage is refused by the transport anyway.
#[derive(Default)]
pub struct SettledIdentity {
    user: std::sync::Mutex<Option<String>>,
    /// Why the connection is currently down, as the session recorded it.
    ///
    /// Published here for one reader: the refusal a write produces while the
    /// store is unreachable. `sync.allium: AWriteWithNoConnectionIsRefused`
    /// says that refusal carries `connection.last_error` — "the store is
    /// unreachable: connection refused" is actionable and "could not save" is
    /// not — and the transport cannot reach the session that owns it. This is
    /// the one-way channel that makes the spec true.
    ///
    /// Cleared on a successful connection, so a refusal never quotes an outage
    /// that is over.
    last_error: std::sync::Mutex<Option<String>>,
}

impl SettledIdentity {
    /// Record who the store said we are. Called once per successful connection.
    pub fn settle(&self, user: impl Into<String>) {
        *self.user.lock().unwrap_or_else(|e| e.into_inner()) = Some(user.into());
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Record why the connection is down, or clear it with `None`.
    pub fn set_last_error(&self, reason: Option<String>) {
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = reason;
    }

    /// Why the connection is down, if it is and the session said so.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl WriterIdentity for SettledIdentity {
    async fn user(&self) -> Result<Option<String>> {
        Ok(self.user.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}
