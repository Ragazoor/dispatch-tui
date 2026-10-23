mod board_reads;
#[cfg(any(test, feature = "test-support"))]
mod fixtures;
mod queries;

/// The `settings` keys naming an install's machine identity. Production keeps
/// that identity in `host.json` (`crate::host_file`); these are the names the
/// snapshot code (`src/spacetime/snapshot.rs`) gives the host registry's
/// columns. Spelled inline there instead, a rename would silently match
/// nothing rather than fail to compile.
#[cfg(test)]
pub(crate) use queries::{HOST_ID_KEY, HOST_LABEL_KEY, USER_IDENTITY_KEY};

pub use crate::models::{
    CreateLearningRow, CreateTaskRequest, EpicPatch, LearningFilter, LearningPatch, TaskPatch,
    UsageQuery,
};
pub use crate::sync::{HostStore, IdentityCredentialStore, SubscriptionStore};
pub use board_reads::BoardReads;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
use anyhow::Context;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

use crate::models::{
    Agent, DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, Epic, EpicId,
    FeedItem, Learning, LearningId, LearningRetrieval, LearningVerdict, NotificationWrite,
    PollScopeId, RetrievalSource, SessionRequest, SessionRequestId, StopOutcome, SubStatus,
    SubagentDrain, Task, TaskId, TmuxWindow, UserPromptOutcome, WorkPackage, WorkPackageId,
    WorkPackageItem, WorkPackageLink, WrapUpMode,
};

// ---------------------------------------------------------------------------
// RemovedFeedTask — a row a feed stale-delete removed
// ---------------------------------------------------------------------------

/// A task row that a feed stale-delete removed and which still owns on-disk or
/// in-tmux state, so the caller has something to tear down. Returned by
/// [`TaskCrud::ingest_feed`].
///
/// Rows with neither a worktree nor a tmux window are deleted just the same but
/// are not reported — there is nothing to clean up for a plain card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedFeedTask {
    pub id: TaskId,
    pub repo_path: String,
    pub worktree: Option<String>,
    pub tmux_window: Option<crate::models::TmuxWindow>,
}

/// What [`TaskCrud::ingest_feed`] did, as this board sees it afterwards.
///
/// `affected_epics` is the feed epic first, then every epic below it: the
/// module decides which of them it wrote to, and does not say, so the board
/// notifies them all. `removed` is the rows the module deleted that were in
/// this board's view of the subtree, for teardown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedIngestOutcome {
    pub affected_epics: Vec<EpicId>,
    pub removed: Vec<RemovedFeedTask>,
}

// ---------------------------------------------------------------------------
// Sub-traits — focused slices of the database API
// ---------------------------------------------------------------------------

/// Read-only task queries. Infallible by signature: the read traits answer
/// from the subscription's in-memory rows, so there is no read error to handle
/// or inject. Held (via [`TaskReadStore`]) by non-service consumers so
/// a direct task *mutation* from a handler is a compile error — see the
/// "Service layer is the mutation boundary" section of `docs/conventions.md`.
#[async_trait::async_trait]
pub trait TaskRead: Send + Sync {
    async fn get_task(&self, id: TaskId) -> Option<Task>;
    /// Whether a task row exists, without materialising it. For callers whose
    /// only question is existence — typically to honour a `NotFound` contract
    /// before a mutation — where [`get_task`](Self::get_task) would decode every
    /// column and discard the result.
    async fn task_exists(&self, id: TaskId) -> bool;
    async fn list_all(&self) -> Vec<Task>;
    /// The tasks [`Task::is_live_agent`] accepts, ordered by id — the
    /// agent-tree pane's agents section, read once a second per pane, so it
    /// filters in SQL rather than decoding every task ever created.
    async fn list_live_agent_tasks(&self) -> Vec<Task>;
    async fn find_task_by_plan(&self, plan: &str) -> Option<Task>;
}

/// Reading the agent rows the board's subscription holds, current and ended
/// (`agents.allium`). On [`TaskReadStore`]: the board never writes an agent,
/// the module does, inside the claim and the close.
#[async_trait::async_trait]
pub trait AgentRead: Send + Sync {
    /// Every agent, oldest first. What name resolution
    /// (`AgentNameResolvesToTask`) searches, ended agents included.
    async fn list_agents(&self) -> Vec<Agent>;
    /// `current_agent(task)`: the task's one current agent, if any.
    async fn current_agent(&self, task: TaskId) -> Option<Agent>;
}

/// Task mutations, layered on top of the read surface. Reachable only by the
/// service layer (which owns invariants like epic-status recalculation) and by
/// the sanctioned feed subsystem — non-service consumers hold [`TaskReadStore`].
#[async_trait::async_trait]
pub trait TaskCrud: TaskRead {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId>;
    /// [`Self::create_task`], marking the task unseen for every follower of
    /// its epic and ancestors in the same transaction
    /// (`mcp-task-tools.allium: SignalMarksTheTaskForEveryFollower`).
    async fn create_task_signalled(&self, req: CreateTaskRequest<'_>) -> Result<TaskId>;
    /// `PhoenixRespawn`, made atomic: inserts the successor (with `labels` set
    /// directly, not a follow-up patch) and clears `predecessor`'s `phoenix`
    /// flag in one transaction. Either both happen or neither does — a failure
    /// rolls back the insert too, so `TheFlagIsTheReceipt` holds literally and
    /// a retry after a failure cannot create a second successor.
    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        req: CreateTaskRequest<'_>,
        labels: &[String],
    ) -> Result<TaskId>;
    async fn delete_task(&self, id: TaskId) -> Result<()>;
    /// `tasks.allium: BatchDelete`'s atomic counterpart to looping
    /// [`Self::delete_task`]/[`EpicCrud::delete_epic`] once per selected item.
    /// Validates every task and epic against the store's TRUE state and
    /// deletes all of them, or none, together — see
    /// `spacetime/module/src/tasks_epics.rs::batch_delete`. The joint guard
    /// runs in the store because a stale subscription view is what "one
    /// operation, or nothing at all" has to hold up against.
    async fn batch_delete(&self, task_ids: &[TaskId], epic_ids: &[EpicId]) -> Result<()>;
    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()>;
    /// Atomically set `pr_learnings_gate_shown_at` to now if it is currently null.
    /// Returns `true` if this call set it (first `gh pr create` for the task →
    /// caller should block), `false` if it was already set or the task does not
    /// exist (caller should allow the PR).
    async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool>;
    /// Record a live subagent starting for `id`. Rows belonging to any session
    /// other than `session_id` are evicted first (a new `claude` process means
    /// a new session, so stale rows from a dead session are provably dead),
    /// then the `(task_id, agent_id)` row is upserted. Returns the resulting
    /// live count. Replaying the same `(agent_id, session_id)` is idempotent —
    /// it does not double-count.
    async fn subagent_start(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64>;
    /// Record a live subagent stopping for `id`. Rows belonging to any session
    /// other than `session_id` are evicted first, then the `(task_id,
    /// agent_id)` row is deleted if present. An `agent_id` that was never
    /// started is a no-op, not an underflow.
    ///
    /// If this is the write that drains the last subagent of a task carrying a
    /// deferred `Stop`, the flip to `Review` is applied **in the same
    /// transaction** and reported in the result. See `HookSubagentStop` in
    /// `docs/specs/agent-health.allium`.
    async fn subagent_stop(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SubagentDrain>;
    /// Remove every live-subagent row for `id`, zero `live_subagents`, and — if
    /// that drained a task carrying a deferred `Stop` — apply the flip, all in
    /// one transaction.
    ///
    /// For the draining clear point (`DetachTmux`), which owns that bit itself.
    /// Note the flip's `WHERE` requires a Running task, which is why this is not
    /// an unconditional `stop_pending = 0`: `docs/specs/split-pane.allium`
    /// clears the bit only for a Running task.
    async fn subagent_clear(&self, id: TaskId) -> Result<SubagentDrain>;
    /// [`subagent_clear`](Self::subagent_clear) minus the drain, plus
    /// `stop_pending = 0`, in one transaction. For the three non-draining clear
    /// points — `SessionStart`, crash and dispatch-claim — which void a
    /// deferred Stop rather than apply it, so the bit cannot survive into a
    /// later session and fire a spurious flip. See
    /// `ClearSubagentsOnSessionStart` in `docs/specs/agent-health.allium`.
    async fn subagent_clear_and_void_pending_stop(&self, id: TaskId) -> Result<()>;
    /// Apply the `Stop` hook to `id`, deciding against the row's committed
    /// state rather than a prior read.
    ///
    /// One transaction holding two conditional statements: flip to `Review`
    /// when nothing is live, else defer via `stop_pending`. Because the
    /// predicate is part of each statement, a concurrent hook process cannot
    /// interleave a stale decision — whichever commits second sees the other's
    /// value. See `HookStop` in `docs/specs/agent-health.allium`.
    ///
    /// `now` is when the hook *fired*, not when this write lands. The defer
    /// branch stores it as `stop_pending_at` so
    /// [`record_user_prompt_submit`](Self::record_user_prompt_submit) can order
    /// the two events independently of which write commits first.
    async fn try_record_stop(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<StopOutcome>;
    /// Apply a `PreToolUse`/`PostToolUse` hook to `id`: stamp
    /// `last_pre_tool_use_at` and set `sub_status`, only while the task is
    /// still running.
    ///
    /// The `status = running` guard rides in the statement rather than being
    /// checked against a prior read. A concurrent `Stop` flipping the row to
    /// `Review` between the two would otherwise leave this write producing
    /// `(review, active)`, which the `tasks` CHECK constraint rejects — a loud
    /// error out of a hook process that has no caller to report it to. A row
    /// the guard rejects is a silent no-op. See `HookPreToolUse` in
    /// `docs/specs/agent-health.allium`.
    ///
    /// `sub_status` is still classified from a snapshot by the caller, which
    /// is sound here where it is not for `record_notification`: the value is
    /// re-derived from fresh state by `ClassifyAgentActivity` on the next
    /// tick, so a stale read self-corrects within a tick.
    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    /// Apply a `Notification` hook to `id`, deciding against the row's
    /// committed state rather than a prior read.
    ///
    /// One conditional statement per [`NotificationWrite`] variant, each
    /// carrying its own predicate — `status = running` for all of them, plus
    /// `live_subagents = 0` for
    /// [`RaiseIfNoOwnWorkLive`](NotificationWrite::RaiseIfNoOwnWorkLive). The
    /// count is never read into the process first: every Claude Code hook is
    /// its own OS process, so a count read beforehand can already be stale by
    /// the time the write lands, and this is the same argument
    /// [`try_record_stop`](Self::try_record_stop) makes for the identical two
    /// counters.
    ///
    /// A row the predicate rejects — a task no longer running, or one whose
    /// own background work is still live — is a silent no-op, not an error:
    /// the hook observed a state that has since moved on. See
    /// `HookNotification` in `docs/specs/agent-health.allium`.
    async fn record_notification(
        &self,
        id: TaskId,
        write: NotificationWrite,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    /// Apply the `UserPromptSubmit` hook to `id`: resume a `Review` task to
    /// `Running`, or refresh an already-`Running` one, and void the deferred
    /// `Stop` the human's prompt supersedes.
    ///
    /// One transaction, and the `stop_pending` clear is **conditional**: only a
    /// Stop that fired before `now` is voided, because one that fired after
    /// belongs to the turn this prompt started and no drain could re-create it.
    /// `HookUserPromptSubmit` in `docs/specs/agent-health.allium` carries the
    /// full argument.
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserPromptOutcome>;
    /// Post one parsed feed emission to the store (store-split M3,
    /// `feeds.allium: IngestFeedEmission`). The three slices are co-indexed.
    /// The module alone decides what the items do: the strategy from the epic's
    /// own `feed_role` and `group_by_repo`, routing, sub-epics, grouping,
    /// upsert, the stale delete and the status recalculation.
    ///
    /// `additive` says the emission may not remove anything (degraded, or an
    /// append-only epic). The returned outcome is read from this board's own
    /// view before and after the call — see [`FeedIngestOutcome`].
    async fn ingest_feed(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
        additive: bool,
    ) -> Result<FeedIngestOutcome>;
    /// Atomically update `sub_status` for multiple tasks in a single transaction.
    /// Used by the tick to batch all per-task reclassifications into one DB round-trip.
    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()>;

    // Retired feed items (`docs/specs/core.allium`: `RetiredFeedItem`).
    //
    // The closed-retirement drop is part of the module's `ingest_feed`; the
    // read (`retired_without_task`) is a join a subscription's `WHERE` clause cannot
    // express, so it runs in Rust over the store's rows, the same as
    // `query_usage` and the learning reads. There is no
    // `create_retired_feed_item` here — every real retirement writes the row
    // inline, in the same reducer transaction as the delete (`delete_task`,
    // `delete_epic`), rather than through a separate call.
    /// Of `external_ids`, the subset that are retired under `feed_epic_id`
    /// AND have no existing task anywhere in `feed_epic_id`'s subtree. Used by
    /// `GroupedFeedUpsert`/`RoleRoutedFeedSync` (`docs/specs/feeds.allium`) to
    /// drop such items before a sub-epic is found-or-created for them.
    async fn retired_without_task(
        &self,
        feed_epic_id: EpicId,
        external_ids: &[String],
    ) -> Result<Vec<String>>;
}

// ---------------------------------------------------------------------------
// Dispatch requests (store-split M4)
// ---------------------------------------------------------------------------

/// What [`DispatchRequestStore::request_dispatch`] answered. A refusal is an
/// ORDINARY answer, not an error (`store-split.allium: RequestDispatch`): it
/// is how a caller learns another entry point claimed the task first, that
/// its worktree is on another machine, or that an open request already
/// exists, and it wrote nothing. A store that could not be reached is the
/// `Err` of the enclosing `Result` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestDispatchAnswer {
    /// The task is claimed and a `requested` row naming this host exists.
    Requested(DispatchRequestId),
    /// The module's reason, verbatim.
    Refused(String),
}

/// Reading this host's dispatch request rows — the ones its subscription
/// holds (`sync.allium: SubscribeToThisHostsDispatchRequests`), in every
/// state. On [`TaskReadStore`] so a read-only handle can follow a request's
/// outcome; the writes are on [`DispatchRequestStore`], behind the mutation
/// boundary.
#[async_trait::async_trait]
pub trait DispatchRequestRead: Send + Sync {
    async fn dispatch_request(&self, id: DispatchRequestId) -> Option<DispatchRequest>;
    /// Every row this host's subscription holds, oldest first.
    async fn dispatch_requests(&self) -> Vec<DispatchRequest>;
    /// `task`'s most recent request this subscription holds, if any — the
    /// row `get_task`'s `[Dispatch request]` line shows.
    async fn latest_dispatch_request_for_task(&self, task: TaskId) -> Option<DispatchRequest>;
    /// Fires whenever the subscribed rows move — a request row among them.
    /// What a caller WAITING on a request's outcome waits on (`dispatch_task`,
    /// `mcp-task-tools.allium: DispatchTaskViaMcp`), so the wait is a signal,
    /// never a poll. Subscribe before reading the row, then re-read on each
    /// wake.
    fn dispatch_request_changes(&self) -> tokio::sync::watch::Receiver<u64>;
    /// The identity this connection writes as — what the store stamps as
    /// `requested_by` on this board's own requests, and so the only
    /// requester whose rows this host serves (`store-split.allium`, D11).
    /// `None` before an identity has settled.
    async fn own_identity(&self) -> Option<String>;
}

/// The dispatch request reducers (`store-split.allium`: `RequestDispatch` and
/// the host rules after it). Every call names THIS host — the requester and
/// the reporter are always the machine the handle was built for — so a
/// caller cannot ask on another machine's behalf.
///
/// A request claims its task, so this is a task mutation: reachable from the
/// service layer, not from a read-only handle.
#[async_trait::async_trait]
pub trait DispatchRequestStore: DispatchRequestRead {
    /// `RequestDispatch`. `mode` `None` lets the module derive it (D3).
    async fn request_dispatch(
        &self,
        task: TaskId,
        origin: DispatchOrigin,
        mode: Option<DispatchMode>,
    ) -> Result<RequestDispatchAnswer>;
    /// `HostClaimsDispatch`. `false` is a refused claim — another board, or
    /// a redelivery, got it — and the caller provisions nothing.
    async fn claim_dispatch_request(&self, id: DispatchRequestId) -> Result<bool>;
    /// `HostReportsDispatched`. `false` is a refused report: the row failed
    /// under this host (the backstop, or the task's deletion), or its
    /// deadline passed first, and the caller owes the teardown of what it
    /// made. `reused_worktree` is provisioning's own answer, recorded on the
    /// row.
    async fn report_dispatch_provisioned(
        &self,
        id: DispatchRequestId,
        worktree: &str,
        tmux_window: &TmuxWindow,
        reused_worktree: bool,
    ) -> Result<bool>;
    /// `HostReportsDispatchFailed`. `false` when the row had already finished.
    async fn report_dispatch_failed(&self, id: DispatchRequestId, reason: &str) -> Result<bool>;
    /// `HostRestartFailsInterrupted`: fail this host's provisioning rows.
    async fn fail_interrupted_dispatch_requests(&self) -> Result<()>;

    /// Give `task` a current agent, as the dispatch claim does
    /// (`agents.allium: StartAgentOnFreshLaunch`), without the claim: the way
    /// a test sets up a task whose session holds a window or hook facts. Since
    /// agents A4 those facts live on the agent, so a patch that sets one needs
    /// an agent to land on. Only an in-memory handle has one to give.
    #[cfg(any(test, feature = "test-support"))]
    fn give_agent(&self, task: TaskId);
}

// ---------------------------------------------------------------------------
// Session requests (store-split M5)
// ---------------------------------------------------------------------------

/// What [`SessionRequestStore::request_wrap_up`] and
/// [`SessionRequestStore::request_exit_session`] answered. A refusal is an
/// ORDINARY answer, not an error (`store-split.allium: RequestWrapUp`,
/// `RequestExitSession`): it carries the module's reason, verbatim, and wrote
/// nothing. A store that could not be reached — or a call that got no answer,
/// which may or may not have landed — is the `Err` of the enclosing `Result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRequestAnswer {
    /// The row exists: a wrap_up row (its id is the exit token once done) or the
    /// exit_session row the close wrote.
    Requested(SessionRequestId),
    /// The module's reason, verbatim.
    Refused(String),
}

/// Reading this host's session request rows — the ones its subscription holds
/// (`sync.allium: SubscribeToThisHostsSessionRequests`), in every state. On
/// [`TaskReadStore`] so a read-only handle can follow a request's outcome; the
/// writes are on [`SessionRequestStore`].
#[async_trait::async_trait]
pub trait SessionRequestRead: Send + Sync {
    async fn session_request(&self, id: SessionRequestId) -> Option<SessionRequest>;
    /// Every row this host's subscription holds, oldest first.
    async fn session_requests(&self) -> Vec<SessionRequest>;
    /// Fires whenever the subscribed rows move — a session row among them. What
    /// a caller WAITING on a row's outcome waits on (`wrap_up`,
    /// `mcp-task-tools.allium: WrapUpViaMcp`), so the wait is a signal, never a
    /// poll. Subscribe before writing, then re-read on each wake.
    fn session_request_changes(&self) -> tokio::sync::watch::Receiver<u64>;
}

/// The session request reducers (`store-split.allium`: `RequestWrapUp` and the
/// host rules after it). Every call names THIS host, so a caller cannot ask or
/// report on another machine's behalf.
///
/// A close rewrites its task, so this is a task mutation: reachable from the
/// service layer, not from a read-only handle.
#[async_trait::async_trait]
pub trait SessionRequestStore: SessionRequestRead {
    /// `RequestWrapUp`. `done` and `pr` are answered with a row already done.
    async fn request_wrap_up(
        &self,
        task: TaskId,
        action: WrapUpMode,
    ) -> Result<SessionRequestAnswer>;
    /// `RequestExitSession`: the token check, the close, the epic chain and the
    /// exit_session row, in one module transaction. `pr_url` is only read for
    /// the `pr` action.
    async fn request_exit_session(
        &self,
        task: TaskId,
        token: SessionRequestId,
        action: WrapUpMode,
        pr_url: Option<&str>,
    ) -> Result<SessionRequestAnswer>;
    /// `RequestCloseTask` (`update_tasks`' done path): the module applies the
    /// Done close in one transaction and, when there is host work on this
    /// host, writes a `close` row for the board to serve. A refusal is an
    /// `Err`, as is a call that got no answer; either way nothing is known to
    /// have landed.
    async fn close_task(&self, task: TaskId) -> Result<()>;
    /// `HostClaimsSessionRequest`. `false` is a refused claim — another board,
    /// a redelivery or the deadline got there first — and the caller does
    /// nothing.
    async fn claim_session_request(&self, id: SessionRequestId) -> Result<bool>;
    /// `HostReportsSessionRequestDone`. `false` is a refused report: the row
    /// failed under this host, or a rebase finished past its deadline.
    async fn report_session_request_done(&self, id: SessionRequestId) -> Result<bool>;
    /// `HostReportsSessionRequestFailed`. `conflict` is true only for a genuine
    /// rebase conflict; the module then sets the Conflict sub_status. `false`
    /// when the row had already finished.
    async fn report_session_request_failed(
        &self,
        id: SessionRequestId,
        reason: &str,
        conflict: bool,
    ) -> Result<bool>;
    /// `HostRestartFailsInterruptedSessionRequests`: fail this host's running
    /// rows.
    async fn fail_interrupted_session_requests(&self) -> Result<()>;
}

/// PR state (store-split M6: `ReportPrStatus`, `ReportPrUnreadable`). The host
/// runs `gh` and posts what it read; the module writes the task. Every call
/// names THIS host. A task write, so reachable from the service layer only.
#[async_trait::async_trait]
pub trait PrStatusStore: Send + Sync {
    /// `ReportPrStatus`: the raw `state` and `reviewDecision` strings. `false`
    /// is a refusal (a string the module does not know), logged.
    async fn report_pr_status(
        &self,
        task: TaskId,
        state: &str,
        review_decision: &str,
    ) -> Result<bool>;
    /// `ReportPrUnreadable`: the host gave up on the task's PR.
    async fn report_pr_unreadable(&self, task: TaskId, reason: &str) -> Result<bool>;
}

/// Poll ownership (Phase 7): `core.allium: PollOwner`.
///
/// **Deliberately not on [`TaskCrud`].** Claiming or reassigning a
/// `PollOwner` row touches no `Task`/`Epic` row at all, so it does not belong
/// behind the task/epic mutation seal — a PR-poll tick handler (not a
/// sanctioned direct-mutation consumer; see the mutation-boundary section of
/// `docs/conventions.md`) needs to reach it through the plain read-only
/// `TaskReadStore` handle every general handler already holds, the same way
/// [`HostStore`] and [`RepoConfigStore`] do for their own non-CRUD writes.
#[async_trait::async_trait]
pub trait PollOwnershipStore: Send + Sync {
    /// Fill an absent claim. `claim_poll_owner` fills an absent row,
    /// `override_poll_owner` unconditionally reassigns an existing one. One
    /// method per operation rather than one per scope — `PollScopeId` already
    /// carries the type-safe task/epic distinction, so a second split here
    /// would only be the same fork twice.
    async fn claim_poll_owner(&self, target: PollScopeId) -> Result<()>;
    async fn override_poll_owner(&self, target: PollScopeId) -> Result<()>;
}

/// Read-only epic queries. Held (via [`TaskReadStore`]) by non-service consumers.
#[async_trait::async_trait]
pub trait EpicRead: Send + Sync {
    async fn get_epic(&self, id: EpicId) -> Option<Epic>;
    async fn list_epics(&self) -> Vec<Epic>;
    /// List direct children of the given epic.
    async fn list_sub_epics(&self, parent_id: EpicId) -> Vec<Epic>;
    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Vec<Task>;
    /// Ids of the epic's tasks the bulk read skipped because the row did not
    /// decode, ascending. Such a task is still in the epic's subtree with an
    /// unknown status (epics.allium: `DeleteEpicRefused`).
    async fn list_undecodable_task_ids_for_epic(&self, epic_id: EpicId) -> Vec<TaskId>;
    /// Fetch all tasks that have a non-null epic_id in a single query.
    /// Use instead of looping over epics and calling list_tasks_for_epic() per epic.
    async fn list_all_tasks_with_epic_id(&self) -> Vec<Task>;
}

/// The optional fields an epic can be created with.
#[derive(Debug, Default, Clone, Copy)]
pub struct EpicBirth<'a> {
    pub sort_order: Option<i64>,
    pub feed_command: Option<&'a str>,
    pub feed_interval_secs: Option<i64>,
}

/// Epic mutations, layered on top of the read surface. The
/// module's reducers own the epic rollup (store-split M7); reachable only by the service
/// layer and the sanctioned feed subsystem — non-service consumers hold
/// [`TaskReadStore`].
#[async_trait::async_trait]
pub trait EpicCrud: EpicRead {
    /// Create a new epic. `parent_epic_id` can be changed later via
    /// [`EpicCrud::patch_epic`]. The DB enforces `CHECK (parent_epic_id != id)`
    /// (migration v35) to prevent self-loops; cycle detection is handled in
    /// the service layer before any DB write.
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<Epic> {
        self.create_epic_configured(title, description, parent_epic_id, &EpicBirth::default())
            .await
    }
    /// [`EpicCrud::create_epic`] with the fields a caller may set at birth,
    /// carried in the one create row. A follow-up patch would leave a
    /// half-configured epic if it failed, and a retry would duplicate it.
    async fn create_epic_configured(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
        birth: &EpicBirth<'_>,
    ) -> Result<Epic>;
    /// `epics.allium: ProvisionManagedEpics`, decided in the module in one
    /// transaction (store-split M7, X4). `None` is a disabled feed: nothing is
    /// created or removed for it. An interval of `None` is no interval.
    async fn ensure_managed_epics(
        &self,
        reviews_command: Option<&str>,
        reviews_interval_secs: Option<i64>,
        cve_command: Option<&str>,
        cve_interval_secs: Option<i64>,
    ) -> Result<()>;
    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()>;
    async fn delete_epic(&self, id: EpicId) -> Result<()>;
    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()>;
    /// `epics.allium: RegroupEpic`, decided and applied by the store in one
    /// transaction: every direct task of `root` moves into its per-repo
    /// sub-epic.
    async fn regroup_epic(&self, root: EpicId) -> Result<()>;
    /// `epics.allium: FlattenEpic`, likewise: the tasks of every `repo-group`
    /// sub-epic move back onto `root` and the emptied sub-epics are deleted.
    async fn flatten_epic(&self, root: EpicId) -> Result<()>;
    /// `tasks.allium: SeeTasksByEnteringTheirEpic`: this board's person
    /// entered `epic`, so their unseen marks on the tasks directly in it end.
    /// A board with no settled identity has no marks to end and does nothing.
    async fn mark_epic_seen(&self, epic: EpicId) -> Result<()>;
}

/// Read-only work-package queries (`work-packages.allium`). Held (via
/// [`TaskReadStore`] and [`BoardReads`]) by non-service consumers.
#[async_trait::async_trait]
pub trait WorkPackageRead: Send + Sync {
    /// Every work package this board holds, oldest first.
    async fn list_work_packages(&self) -> Vec<WorkPackage>;
    /// Every link this board holds, ordered by package, then item.
    async fn list_work_package_links(&self) -> Vec<WorkPackageLink>;
}

/// Work-package mutations. Off [`TaskReadStore`], like [`EpicCrud`]: writes go
/// through the service layer.
#[async_trait::async_trait]
pub trait WorkPackageCrud: WorkPackageRead {
    /// Create a package stamped with this board's identity; answers its id.
    async fn create_work_package(&self, title: &str, description: &str) -> Result<WorkPackageId>;
    /// Change the title, description and/or manual order; `None` leaves a
    /// field alone.
    async fn patch_work_package(
        &self,
        id: WorkPackageId,
        title: Option<&str>,
        description: Option<&str>,
        sort_order: Option<i64>,
    ) -> Result<()>;
    /// Delete the package and its links; the linked items stay.
    async fn delete_work_package(&self, id: WorkPackageId) -> Result<()>;
    /// Link an item. Idempotent.
    async fn link_work_package_item(&self, id: WorkPackageId, item: WorkPackageItem) -> Result<()>;
    /// Unlink an item. Idempotent.
    async fn unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<()>;
}

/// Per-host preferences: key/value settings and the managed-feed config.
/// **Shared, not local** — a mutation is a reducer call scoped to this
/// install's own host id, and a read answers from the store's rows, so a board
/// that writes a setting reads it back from the same place; see
/// `docs/specs/settings.allium`.
#[async_trait::async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get_setting_bool(&self, key: &str) -> Result<Option<bool>>;
    async fn set_setting_bool(&self, key: &str, value: bool) -> Result<()>;
    async fn get_setting_string(&self, key: &str) -> Result<Option<String>>;
    async fn set_setting_string(&self, key: &str, value: &str) -> Result<()>;
    // -- Managed-feed config (WP5) --
    // Typed accessors over the `settings` table for the two managed feed
    // scripts and their poll intervals. `Some(..)` upserts; `None` clears the
    // key (so a subsequent get returns `None`). Intervals are stored as their
    // decimal string. See `epics.allium` ProvisionManagedEpics / the `config`
    // block for the authoritative semantics.
    async fn get_reviews_feed_command(&self) -> Result<Option<String>>;
    async fn set_reviews_feed_command(&self, value: Option<&str>) -> Result<()>;
    async fn get_reviews_feed_interval_secs(&self) -> Result<Option<i64>>;
    async fn set_reviews_feed_interval_secs(&self, value: Option<i64>) -> Result<()>;
    async fn get_cve_feed_command(&self) -> Result<Option<String>>;
    async fn set_cve_feed_command(&self, value: Option<&str>) -> Result<()>;
    async fn get_cve_feed_interval_secs(&self) -> Result<Option<i64>>;
    async fn set_cve_feed_interval_secs(&self, value: Option<i64>) -> Result<()>;
}

// ---------------------------------------------------------------------------
// RepoConfigRead, RepoConfigStore — the `repo_paths` and `repo_base_branches` shared tables
// ---------------------------------------------------------------------------

/// Repo registration and its per-repo config: the known repo paths, each one's
/// verify command, and the base-branch history. Shared tables
/// (`SharedTable::RepoPaths`, `SharedTable::RepoBaseBranches`) — see
/// [`TaskStore`].
/// The repo list's read surface, split from its writes the way
/// [`TaskRead`]/[`TaskCrud`] are, so a consumer that only draws the list
/// holds a handle that cannot write it.
#[async_trait::async_trait]
pub trait RepoConfigRead: Send + Sync {
    async fn list_repo_paths(&self) -> Vec<String>;

    /// The paths THIS host saved, most recent first. What a command that
    /// judges paths against this machine's disk may act on: a row another
    /// host saved, or one saved before hosts were recorded, is not in it.
    async fn list_repo_paths_saved_by_this_host(&self) -> Vec<String>;

    async fn get_verify_command(&self, path: &str) -> Option<String>;

    /// All `(repo_path, branch)` pairs across every repo, ordered by
    /// `last_used DESC, id DESC`.
    async fn list_all_base_branches(&self) -> Vec<(String, String)>;
}

#[async_trait::async_trait]
pub trait RepoConfigStore: RepoConfigRead {
    async fn save_repo_path(&self, path: &str) -> Result<()>;
    /// Remove the `repo_paths` row.
    async fn delete_repo_path(&self, path: &str) -> Result<()>;

    /// Set the verify command for a known repo path.
    ///
    /// If `command` is `Some(cmd)` and the path does not exist in `repo_paths`, a new
    /// row is inserted (with `last_used = now()`), equivalent to calling `save_repo_path`
    /// first. If `command` is `None`, the column is cleared to NULL; unknown paths are
    /// silently ignored (no row created).
    ///
    /// `command` must not contain a newline (`\n`) or carriage return (`\r`); returns
    /// an error if it does. Empty or whitespace-only commands are treated as `None`.
    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()>;

    // -- Base branch history (task #3422) --
    // Per-repo most-recently-used base_branch history. See
    // docs/specs/dispatch.allium (rule RecordBaseBranch, surface
    // BaseBranchPicker, config.max_base_branches_per_repo) and
    // docs/specs/core.allium (entity SavedRepoBranch).

    /// Upsert `(repo_path, branch)`, bumping `last_used` to now, then prune
    /// `repo_path`'s history down to the `max_base_branches_per_repo` (10)
    /// most-recently-used rows.
    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// TaskAndEpicStore — composite for consumers that need tasks + epics only
// ---------------------------------------------------------------------------

pub trait TaskAndEpicStore: TaskCrud + EpicCrud {}

impl<T: TaskCrud + EpicCrud> TaskAndEpicStore for T {}

// ---------------------------------------------------------------------------
// WorkPackageStore — composite for the work-package service
// ---------------------------------------------------------------------------

/// What `WorkPackageService` holds: the package writes, plus the task and epic
/// reads it checks a link's item against before writing.
pub trait WorkPackageStore: WorkPackageCrud + TaskRead + EpicRead {}

impl<T: WorkPackageCrud + TaskRead + EpicRead> WorkPackageStore for T {}

// ---------------------------------------------------------------------------
// LearningStore — narrow sub-trait for the learnings table
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait LearningStore: Send + Sync {
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId>;

    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>>;

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>>;

    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()>;

    async fn delete_learning(&self, id: LearningId) -> Result<bool>;

    /// Returns all approved, non-task-scoped learnings that have embeddings stored,
    /// with their raw embedding bytes. Used by the RAG pipeline.
    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>>;

    /// Returns approved, non-task-scoped learnings that have no embedding stored yet.
    /// Used by the backfill job to determine which learnings need to be embedded.
    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>>;

    /// Archive approved learnings that have proven unvalued and gone stale:
    /// status = approved AND upvote_count <= 0 AND updated_at <= cutoff.
    /// Sets status = archived and updated_at = now. Returns the number of rows
    /// affected. See docs/specs/learnings.allium: ArchiveStaleLearning.
    async fn archive_stale_learnings(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64>;

    /// Re-scope all epic-scoped learnings whose scope_ref = `from` to `to`.
    /// Used when a repo-group sub-epic is deleted, so its learnings are not
    /// left pointing at a deleted epic id. scope_ref is not an embedding
    /// input, so no re-embed obligation.
    ///
    /// Epic-shaped arguments, local-table write. It sat on [`EpicCrud`] until
    /// the store seam was drawn, which would have obliged a shared-table
    /// backend to implement a write against a table it does not hold.
    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<()>;
}

// ---------------------------------------------------------------------------
// LearningRetrievalStore — narrow sub-trait for retrievals + verdicts
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait LearningRetrievalStore: Send + Sync {
    /// Insert a row into `learning_retrievals` recording that `learning_id` was
    /// surfaced to `task_id` via the given source.
    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<()>;

    /// Return all retrievals recorded for `task_id`, ordered by id ascending.
    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>>;

    /// Apply a batch of verdicts atomically. Verdicts are not persisted; each
    /// only adjusts the learning's score: `Helped` bumps `upvote_count` (and
    /// sets `last_upvoted_at`), `Wrong` decrements it (a downvote; may go
    /// negative). Neither verdict changes status.
    async fn apply_verdicts_tx(&self, verdicts: &[(LearningId, LearningVerdict)]) -> Result<()>;
}

// ---------------------------------------------------------------------------
// UsageStore — narrow sub-trait for the usage_events table
// ---------------------------------------------------------------------------

/// Maximum number of events to keep in `usage_events`. Older rows are deleted
/// when the table exceeds this cap.
#[derive(Debug, Clone, Copy)]
pub struct UsageCap(u64);

impl UsageCap {
    pub fn new(n: u64) -> Self {
        assert!(n > 0, "UsageCap must be > 0");
        Self(n)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl Default for UsageCap {
    fn default() -> Self {
        Self(100_000)
    }
}

#[async_trait::async_trait]
pub trait UsageStore: Send + Sync {
    async fn record_usage_event(&self, event: &crate::models::UsageEvent) -> Result<()> {
        self.record_usage_event_with_cap(event, UsageCap::default())
            .await
    }
    async fn record_usage_event_with_cap(
        &self,
        event: &crate::models::UsageEvent,
        cap: UsageCap,
    ) -> Result<()>;
    /// Aggregated rows ordered by count ASC so the rarest features surface
    /// first — the primary use case is identifying candidates for pruning.
    async fn query_usage(&self, query: &UsageQuery) -> Result<Vec<crate::models::UsageSummary>>;
}

// ---------------------------------------------------------------------------
// TaskStore — the one complete store
// ---------------------------------------------------------------------------

/// Everything a board stores, in one trait.
///
/// **There is no shared/local seam any more** (task #4916). Phase 3 of the
/// SpacetimeDB migration split the store in two — a `SharedDomainStore` a
/// second backend would implement, and a `LocalStore` that stayed in SQLite —
/// so the store could be swapped half at a time. Phases 9 to 11 moved
/// settings, the knowledge base and usage to the shared side, and Phase 12
/// made the store mandatory, which left one backend and nothing for a seam to
/// separate. What stays on this machine — the Host row and the user identity
/// with its credential (`host.allium`) — is a property of what `Store`'s
/// methods do (they read and write `host.json`), not of which trait declares
/// them.
///
/// | Table | Reached through |
/// |---|---|
/// | `tasks`, `task_subagents` | [`TaskCrud`] / [`TaskRead`] |
/// | `epics` | [`EpicCrud`] / [`EpicRead`] |
/// | `repo_paths`, `repo_base_branches` | [`RepoConfigStore`] |
/// | `hosts` | [`HostStore`] |
/// | `subscriptions` | [`SubscriptionStore`] |
/// | `settings` | [`SettingsStore`] |
/// | `poll_owners` | [`PollOwnershipStore`] |
/// | `learnings` | [`LearningStore`] |
/// | `learning_retrievals` | [`LearningRetrievalStore`] |
/// | `usage_events` | [`UsageStore`] |
/// | `retired_feed_items` | [`TaskCrud`] (task #4971) |
/// | `work_packages`, `work_package_links` | [`WorkPackageCrud`] / [`WorkPackageRead`] |
/// | the user identity's credential | [`IdentityCredentialStore`] |
///
/// Every table is reachable through one handle:
///
/// ```
/// use dispatch_tui::store::TaskStore;
/// use dispatch_tui::models::TaskId;
/// async fn every_table_ok(db: &dyn TaskStore) {
///     let _ = db.get_task(TaskId(1)).await;       // TaskRead
///     let _ = db.list_epics().await;              // EpicRead
///     let _ = db.list_repo_paths().await;         // RepoConfigStore
///     let _ = db.ensure_host_identity().await;    // HostStore
///     let _ = db.get_setting_string("k").await; // SettingsStore
///     let _ = db.user_identity_token().await;     // IdentityCredentialStore
/// }
/// ```
///
/// The old local half is gone, so there is nothing to name:
///
/// ```compile_fail
/// use dispatch_tui::store::LocalStore;
/// ```
///
/// and a handle to only what it used to cover — settings and the credential —
/// is not a complete store:
///
/// ```compile_fail
/// use dispatch_tui::store::{IdentityCredentialStore, SettingsStore, TaskStore};
/// trait OldLocalHalf: SettingsStore + IdentityCredentialStore {}
/// fn needs_a_store(_: &dyn TaskStore) {}
/// fn old_local_half(db: &dyn OldLocalHalf) {
///     needs_a_store(db);
/// }
/// ```
///
/// `TaskReadStore` is named explicitly although the other members already
/// cover every method it has: a supertrait is what makes `Arc<dyn TaskStore>`
/// upcast to `Arc<dyn TaskReadStore>`, which is how the read-only handles are
/// built.
pub trait TaskStore:
    TaskAndEpicStore
    + WorkPackageStore
    + RepoConfigStore
    + HostStore
    + SubscriptionStore
    + SettingsStore
    + IdentityCredentialStore
    + LearningStore
    + LearningRetrievalStore
    + UsageStore
    + DispatchRequestStore
    + SessionRequestStore
    + PrStatusStore
    + TaskReadStore
{
}

impl<
        T: TaskAndEpicStore
            + WorkPackageStore
            + RepoConfigStore
            + HostStore
            + SubscriptionStore
            + SettingsStore
            + IdentityCredentialStore
            + LearningStore
            + LearningRetrievalStore
            + UsageStore
            + DispatchRequestStore
            + SessionRequestStore
            + PrStatusStore
            + TaskReadStore,
    > TaskStore for T
{
}

// ---------------------------------------------------------------------------
// TaskReadStore — task/epic-read-only handle held by non-service consumers
// ---------------------------------------------------------------------------

/// The handle held by non-service consumers (`McpState`, `TuiRuntime`). It
/// exposes the task/epic **read** surface ([`TaskRead`] + [`EpicRead`]) but not
/// [`TaskCrud`]/[`EpicCrud`] mutations, so a direct `state.db.patch_task(...)`
/// from a handler is a **compile error**. Task and epic writes must go through
/// `TaskServiceApi`/`EpicServiceApi`, which reach the module's
/// reducers — see the mutation-boundary section of `docs/conventions.md`.
///
/// The name is deliberately scoped: it seals **task/epic** writes, not every
/// write. Repo-config, host, settings, learning and usage writes remain
/// reachable here — they carry no cross-entity invariant, so sealing them would
/// add churn without protecting anything. `TaskStore: TaskReadStore` holds
/// transitively, so a write-capable `Arc<dyn TaskStore>` upcasts to
/// `Arc<dyn TaskReadStore>` for free.
///
/// It is the *mutation* boundary: which methods a read-only handle may reach.
///
/// Reads are reachable through the handle:
///
/// ```
/// use dispatch_tui::store::{TaskReadStore, TaskRead};
/// use dispatch_tui::models::TaskId;
/// async fn reads_ok(db: &dyn TaskReadStore) {
///     let _ = db.get_task(TaskId(1)).await; // a query — fine
/// }
/// ```
///
/// Task/epic mutations are a **compile error** — there is no way to reach
/// `patch_task` (or any `TaskCrud`/`EpicCrud` method) through a `TaskReadStore`:
///
/// ```compile_fail
/// use dispatch_tui::store::{TaskReadStore, TaskPatch};
/// use dispatch_tui::models::TaskId;
/// async fn mutation_rejected(db: &dyn TaskReadStore) {
///     // `patch_task` lives on `TaskCrud`, which `TaskReadStore` does not expose.
///     let _ = db.patch_task(TaskId(1), &TaskPatch::new()).await;
/// }
/// ```
pub trait TaskReadStore:
    TaskRead
    + EpicRead
    + WorkPackageRead
    + RepoConfigStore
    + HostStore
    + SettingsStore
    + IdentityCredentialStore
    + PollOwnershipStore
    + LearningStore
    + LearningRetrievalStore
    + UsageStore
    + DispatchRequestRead
    + SessionRequestRead
    + AgentRead
{
}

impl<
        T: TaskRead
            + EpicRead
            + WorkPackageRead
            + RepoConfigStore
            + HostStore
            + SettingsStore
            + IdentityCredentialStore
            + PollOwnershipStore
            + LearningStore
            + LearningRetrievalStore
            + UsageStore
            + DispatchRequestRead
            + SessionRequestRead
            + AgentRead,
    > TaskReadStore for T
{
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// The handle every service and handler holds, and the one implementation of
/// every store trait above.
///
/// It sits directly on the two halves of the shared store
/// (`storage.allium`, `sync.allium`):
///
/// - **reads** answer from [`SharedRows`](crate::sync::SharedRows), the live
///   view of this connection's subscriptions
///   (`sync.allium: BoardReadsFromTheSubscription`);
/// - **writes** are encoded here and sent as reducer calls through a
///   [`ReducerCaller`](crate::sync::ReducerCaller)
///   (`sync.allium: BoardWritesThroughTheStore`);
/// - **identity** is this install's `host.json` in `host_file_dir`
///   (`host.allium: IdentityLivesInHostFile`).
///
/// Every part is required, so there is no half-built handle: no "no store
/// attached" error, and no write that lands while its reads answer from
/// somewhere else. Nothing sits between a trait and the rows or reducers that
/// serve it: each method below is the implementation, not a hop to one.
pub struct Store {
    /// What this board can see. Every read, and the pre-reads a few writes
    /// take (the chain's candidates, a drain's prior status, a feed sync's
    /// removal candidates), answer from these rows — the same rows the board
    /// draws, so the chain takes the task the column shows as next.
    rows: Arc<crate::sync::SharedRows>,
    /// The transport every mutation goes through.
    caller: Arc<dyn crate::sync::ReducerCaller>,
    /// Who the board is writing as. Read per write, not resolved at
    /// construction — see [`crate::sync::WriterIdentity`].
    identity: Arc<dyn crate::sync::WriterIdentity>,
    clock: Arc<dyn crate::clock::Clock>,
    /// This machine's `Host` id, resolved once at bootstrap.
    ///
    /// Unlike the user identity this is known before any connection — it is
    /// minted locally on first run and immutable afterwards
    /// (`host.allium: MintHostIdentity`) — so it is a value rather than a cell.
    /// The claim needs it (a task whose worktree is on another machine is one
    /// this board must not take), and settings are scoped by it
    /// (`settings.allium`).
    host: String,
    /// The data directory whose `host.json` holds this install's identity
    /// (`host.allium: IdentityLivesInHostFile`).
    host_file_dir: std::path::PathBuf,
    /// The temporary data directory [`Store::open_in_memory`] keeps its host
    /// file in, removed with the handle.
    #[cfg(any(test, feature = "test-support"))]
    _memory_host_dir: Option<tempfile::TempDir>,
    /// The in-memory store's own caller, for [`Store::give_agent`].
    #[cfg(any(test, feature = "test-support"))]
    memory_caller: Option<Arc<crate::sync::memory_caller::MemoryReducerCaller>>,
}

/// The host id an in-memory handle's writes and host file both carry, and so
/// the host every poll-owner claim it makes is recorded under.
#[cfg(any(test, feature = "test-support"))]
pub const MEMORY_HOST_ID: &str = "test-host";

/// The user identity an in-memory handle writes as, and the owner its host is
/// registered to (`store-split.allium`, D11).
#[cfg(any(test, feature = "test-support"))]
pub const MEMORY_USER_ID: &str = "test-user";

impl Store {
    /// A handle over `rows`, writing through `caller`, with this install's
    /// identity in `<data_dir>/host.json`.
    ///
    /// `host` is this install's own `Host` id — known before any connection,
    /// because it is minted locally on first run (`host.allium:
    /// MintHostIdentity`).
    pub fn new(
        rows: Arc<crate::sync::SharedRows>,
        caller: Arc<dyn crate::sync::ReducerCaller>,
        identity: Arc<dyn crate::sync::WriterIdentity>,
        clock: Arc<dyn crate::clock::Clock>,
        host: String,
        data_dir: &Path,
    ) -> Self {
        Store {
            rows,
            caller,
            identity,
            clock,
            host,
            host_file_dir: data_dir.to_path_buf(),
            #[cfg(any(test, feature = "test-support"))]
            _memory_host_dir: None,
            #[cfg(any(test, feature = "test-support"))]
            memory_caller: None,
        }
    }

    /// A handle attached to a fresh in-memory store, which covers every
    /// reducer domain (spec: `spacetime-memory-store.allium`,
    /// `OpenInMemoryAttachesStoreOnceComplete`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_in_memory() -> Result<Self> {
        // Identity lives in a host file, as in production, so the handle gets
        // a private data directory holding one. It carries the host id the
        // memory writes settle as, so a claim and an identity read agree.
        let host_dir = tempfile::tempdir().context("creating the host file directory")?;
        let host_file = crate::host_file::host_file_path(host_dir.path());
        let identity = crate::host_file::HostIdentity {
            host_id: MEMORY_HOST_ID.to_string(),
            label: None,
            user_identity: None,
            credential: None,
            extra: Default::default(),
        };
        std::fs::write(&host_file, serde_json::to_vec(&identity)?)
            .context("writing the test host file")?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&host_file, std::fs::Permissions::from_mode(0o600))
                .context("restricting the test host file")?;
        }
        let mut db = Self::in_memory_with_host_file(host_dir.path());
        db._memory_host_dir = Some(host_dir);
        Ok(db)
    }

    /// A handle over a fresh, private in-process store — one `SharedRows` and
    /// one `MemoryReducerCaller` over it — whose identity lives in
    /// `<data_dir>/host.json`. The writes settle as a fixed test user and
    /// host. For a test whose subject is the host file itself; every other
    /// test wants [`Self::open_in_memory`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn in_memory_with_host_file(data_dir: &Path) -> Self {
        use crate::sync as s;
        let rows = Arc::new(s::SharedRows::new());
        let clock: Arc<dyn crate::clock::Clock> = Arc::new(crate::clock::SystemClock);
        let memory = Arc::new(
            s::memory_caller::MemoryReducerCaller::new(rows.clone(), clock.clone())
                .with_sender(MEMORY_USER_ID)
                .with_registered_host(MEMORY_HOST_ID, MEMORY_USER_ID),
        );
        let caller: Arc<dyn s::ReducerCaller> = memory.clone();
        let identity = Arc::new(s::SettledIdentity::default());
        identity.settle(MEMORY_USER_ID);
        let mut db = Self::new(
            rows,
            caller,
            identity,
            clock,
            MEMORY_HOST_ID.to_string(),
            data_dir,
        );
        db.memory_caller = Some(memory);
        db
    }

    /// Let [`Self::give_agent`] reach `caller`: for a test that builds its own
    /// handle over a caller it holds on to, as the fault-injection ones do.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_memory_caller(
        mut self,
        caller: Arc<crate::sync::memory_caller::MemoryReducerCaller>,
    ) -> Self {
        self.memory_caller = Some(caller);
        self
    }

    /// [`DispatchRequestStore::give_agent`] without the trait in scope.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(clippy::expect_used)]
    pub fn give_agent(&self, task: TaskId) {
        self.memory_caller
            .as_ref()
            .expect("give_agent needs an in-memory handle")
            .start_agent_for(task.0);
    }
}
