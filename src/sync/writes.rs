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
    DispatchRequestId, EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId,
    RetrievalSource, SessionRequestId, SubStatus, TaskId, WorkPackageId, WorkPackageItem,
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

/// The store declined a mutation, and why.
///
/// A type of its own, inside the `anyhow` chain, so a layer that has to answer
/// a caller (the services, for MCP) can tell "the data rule said no" from "the
/// store is broken" and pass the reason on in the right class. The reason is
/// the store's own wording: with the data rules in the reducers
/// (`store-split.allium`, M2) it is the text an agent reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRefusal(pub String);

impl std::fmt::Display for StoreRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the shared store refused: {}", self.0)
    }
}

impl std::error::Error for StoreRefusal {}

impl ReducerOutcome {
    /// Turn a refusal into an error. The right reading for every mutation whose
    /// refusal means something went wrong, which is all of them except a claim.
    pub fn into_result(self) -> Result<Vec<i64>> {
        match self {
            Self::Applied(ids) => Ok(ids),
            Self::Refused(why) => Err(anyhow::Error::new(StoreRefusal(why))),
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
    /// [`Self::create_task`], and mark the task unseen for every follower of
    /// its epic and ancestors in the same transaction
    /// (`mcp-task-tools.allium: SignalMarksTheTaskForEveryFollower`).
    async fn create_task_signalled(&self, row: bindings::Task) -> Result<TaskId>;
    /// `tasks.allium: SeeTasksByEnteringTheirEpic`.
    async fn mark_epic_seen(&self, subscriber: String, epic_id: EpicId) -> Result<ReducerOutcome>;
    async fn patch_task(&self, id: TaskId, patch: bindings::TaskPatch) -> Result<ReducerOutcome>;
    async fn delete_task(&self, id: TaskId) -> Result<ReducerOutcome>;
    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: Option<EpicId>,
        owner: String,
    ) -> Result<ReducerOutcome>;

    /// Insert an epic and answer with the id the store generated.
    async fn create_epic(&self, row: bindings::Epic) -> Result<EpicId>;
    async fn patch_epic(&self, id: EpicId, patch: bindings::EpicPatch) -> Result<ReducerOutcome>;
    async fn delete_epic(&self, id: EpicId) -> Result<ReducerOutcome>;
    /// `epics.allium: RegroupEpic`, in one transaction: every direct task of
    /// `root` moves into its per-repo sub-epic. Idempotent.
    async fn regroup_epic(&self, root: EpicId) -> Result<ReducerOutcome>;
    /// `epics.allium: FlattenEpic`, in one transaction: every `repo-group`
    /// sub-epic's tasks move back onto `root` and the emptied sub-epics go.
    async fn flatten_epic(&self, root: EpicId) -> Result<ReducerOutcome>;

    /// `tasks.allium: BatchDelete`'s atomic call — see
    /// `spacetime/module/src/tasks_epics.rs::batch_delete`'s doc comment for why this
    /// is one reducer invocation over the whole selection rather than
    /// `delete_task`/`delete_epic` called once per item.
    async fn batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> Result<ReducerOutcome>;

    /// Record `path`, stamped with `host` — the install saving it, which alone
    /// may later judge the path gone (`core.allium: SavedRepoPath.host`).
    async fn save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
        host: String,
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
    // read-back `create_task` uses: unlike a find-or-create, a
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
        task_id: TaskId,
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

    /// Apply one parsed feed emission to a feed epic (store-split M3,
    /// `feeds.allium: IngestFeedEmission`). The module picks the strategy,
    /// routes, groups, upserts, deletes the stale and recalculates; which rows
    /// it removed is decoded in the `Store`, as for the calls above.
    async fn ingest_feed(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedIngestItem>,
        additive: bool,
        created_by: String,
    ) -> Result<ReducerOutcome>;

    /// `ProvisionManagedEpics` (M7, X4): the whole managed feed-epic tree in
    /// one transaction. `""` is a disabled feed (nothing created, nothing
    /// removed) and `0` no interval.
    async fn ensure_managed_epics(
        &self,
        reviews_command: String,
        reviews_interval_secs: i64,
        cve_command: String,
        cve_interval_secs: i64,
    ) -> Result<ReducerOutcome>;

    // -- Work packages (`work-packages.allium`) ----------------------------------
    /// Insert a work package and answer with the id the store generated.
    async fn create_work_package(&self, row: bindings::WorkPackage) -> Result<WorkPackageId>;
    /// Insert a work package and link `links` to it in one transaction
    /// (`create_work_package_with_links`): a missing item refuses the call and
    /// leaves no package. Answers the new package's id.
    async fn create_work_package_with_links(
        &self,
        row: bindings::WorkPackage,
        links: Vec<WorkPackageItem>,
    ) -> Result<WorkPackageId>;
    async fn patch_work_package(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
    ) -> Result<ReducerOutcome>;
    /// The patch, then every unlink, then every link, in one transaction
    /// (`update_work_package_with_links`).
    async fn update_work_package_with_links(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
        unlink: Vec<WorkPackageItem>,
        link: Vec<WorkPackageItem>,
    ) -> Result<ReducerOutcome>;
    async fn delete_work_package(&self, id: WorkPackageId) -> Result<ReducerOutcome>;
    async fn link_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome>;
    async fn unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome>;

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

    // -- Dispatch requests (store-split M4) ------------------------------------
    //
    // `store-split.allium`: `RequestDispatch` and the rules after it. Every
    // refusal is an ordinary answer (`Refused`), not an error: a lost race,
    // a late report and a redelivered claim all look like this.

    /// `RequestDispatch`: claim `task` and write a `requested` row naming
    /// `requester`. `mode` `""` lets the module derive it (D3). An applied
    /// call reads the new request's id back as the outcome's one id.
    async fn request_dispatch(
        &self,
        task: TaskId,
        requester: String,
        origin: String,
        mode: String,
    ) -> Result<ReducerOutcome>;
    /// `HostClaimsDispatch`: requested -> provisioning, by the row's host only.
    async fn claim_dispatch_request(
        &self,
        id: DispatchRequestId,
        host: String,
    ) -> Result<ReducerOutcome>;
    /// `HostReportsDispatched`. Refused on a row that is no longer
    /// provisioning — a late report after the backstop failed it — and past
    /// the row's deadline. A repeat of the report the row already records is
    /// accepted and changes nothing (`HostRepeatsDispatchedReport`).
    async fn report_dispatch_provisioned(
        &self,
        id: DispatchRequestId,
        reporter: String,
        worktree: String,
        tmux_window: String,
        reused_worktree: bool,
    ) -> Result<ReducerOutcome>;
    /// `HostReportsDispatchFailed`: the module fails the row and releases the
    /// claim (`ReleaseClaimOfFailedRequest`).
    async fn report_dispatch_failed(
        &self,
        id: DispatchRequestId,
        reporter: String,
        reason: String,
    ) -> Result<ReducerOutcome>;
    /// `HostRestartFailsInterrupted`: fail `host`'s provisioning rows.
    async fn fail_interrupted_dispatch_requests(&self, host: String) -> Result<ReducerOutcome>;

    // -- Session requests (store-split M5) ----------------------------------------
    //
    // `store-split.allium`: `RequestWrapUp` and the rules after it. As for the
    // dispatch requests, every refusal is an ordinary answer (`Refused`): a
    // lost race, a late report, a spent token and a redelivered claim all look
    // like this.

    /// `RequestWrapUp`: write a wrap_up row for `task` on `host`. `action` is
    /// `rebase`, `done` or `pr`; the last two are written already done. An
    /// applied call reads the new row's id back as the outcome's one id.
    async fn request_wrap_up(
        &self,
        task: TaskId,
        host: String,
        action: String,
    ) -> Result<ReducerOutcome>;
    /// `HostClaimsSessionRequest`: requested -> running, by the row's host
    /// only, clearing a Conflict sub_status for a wrap_up.
    async fn claim_session_request(
        &self,
        id: SessionRequestId,
        host: String,
    ) -> Result<ReducerOutcome>;
    /// `HostReportsSessionRequestDone`. Refused on a row that is no longer
    /// running, and, for a rebase, past the row's deadline. A repeat of a
    /// report the row already records is accepted and changes nothing
    /// (`HostRepeatsSessionRequestDone`).
    async fn report_session_request_done(
        &self,
        id: SessionRequestId,
        reporter: String,
    ) -> Result<ReducerOutcome>;
    /// `HostReportsSessionRequestFailed`. `conflict` is true only for a genuine
    /// rebase conflict; the module then sets the Conflict sub_status.
    async fn report_session_request_failed(
        &self,
        id: SessionRequestId,
        reporter: String,
        reason: String,
        conflict: bool,
    ) -> Result<ReducerOutcome>;
    /// `ReportPrStatus`: post what `gh pr view` printed, raw. The module parses
    /// it and writes the task; a state or review decision it does not know is
    /// refused. `host` is the board's own host.
    async fn report_pr_status(
        &self,
        task: TaskId,
        host: String,
        state: String,
        review_decision: String,
    ) -> Result<ReducerOutcome>;
    /// `ReportPrUnreadable`: the host gave up reading the task's PR.
    async fn report_pr_unreadable(
        &self,
        task: TaskId,
        host: String,
        reason: String,
    ) -> Result<ReducerOutcome>;
    /// `RequestExitSession`: check `token` (a done wrap_up row's id), apply the
    /// close, run the epic chain and write the exit_session row, all in one
    /// transaction. `pr_url` is `""` unless the action is `pr`. An applied call
    /// reads the new exit_session row's id back as the outcome's one id.
    async fn request_exit_session(
        &self,
        task: TaskId,
        token: SessionRequestId,
        action: String,
        pr_url: String,
    ) -> Result<ReducerOutcome>;
    /// `RequestCloseTask` (`update_tasks`' done path): fail the task's open
    /// session request, apply the Done close, respawn a phoenix task, roll the
    /// epic up and, when there is host work and `host` (the caller's own) is
    /// where it is, write a `close` row; all in one transaction. No token, no
    /// epic chain.
    async fn close_task(&self, task: TaskId, host: String) -> Result<ReducerOutcome>;
    /// `HostRestartFailsInterruptedSessionRequests`: fail `host`'s running rows.
    async fn fail_interrupted_session_requests(&self, host: String) -> Result<ReducerOutcome>;
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
    // `sync.allium: RegisterHostOnConnect` fires on a settled identity and
    // registers as it, never ownerless: the store refuses a NEW ownerless host
    // (`store-split.allium`, D11), and waiting for the next settle keeps the
    // id free for its owner rather than open to whoever registers it first.
    if owner.is_empty() {
        tracing::warn!(
            host_id,
            "not registering this host before its user identity has settled"
        );
        return;
    }
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
