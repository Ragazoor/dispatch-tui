mod decode;
mod queries;

/// The `settings` keys naming an install's machine identity. Production keeps
/// that identity in `host.json` (`crate::host_file`); these are the names the
/// snapshot code (`src/spacetime/snapshot.rs`) gives the host registry's
/// columns. Spelled inline there instead, a rename would silently match
/// nothing rather than fail to compile.
pub(crate) use queries::{HOST_ID_KEY, HOST_LABEL_KEY, USER_IDENTITY_KEY};

pub use decode::decode_fallback_count;
pub(crate) use decode::{bump_decode_fallback, drop_undecodable, parse_datetime};

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
use anyhow::Context;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

use crate::models::{
    Epic, EpicId, FeedItem, FeedRole, Learning, LearningId, LearningKind, LearningRetrieval,
    LearningScope, LearningStatus, LearningVerdict, NotificationWrite, PollScopeId,
    RetrievalSource, StopOutcome, SubStatus, SubagentDrain, Task, TaskId, TaskStatus, TaskTag,
    UserPromptOutcome, WrapUpMode,
};

// ---------------------------------------------------------------------------
// patch_struct! — declarative macro for selective-update builder structs
// ---------------------------------------------------------------------------

/// Generates a lifetime-parameterised builder struct for partial DB updates.
///
/// Each field is wrapped in `Option<…>` (default `None` = don't touch).
/// Two field kinds:
/// - `plain    field: Type` — `Option<Type>` storage; setter takes `Type`.
/// - `nullable field: Type` — `Option<Option<Type>>` storage (double-Option);
///   setter takes `Option<Type>` (allows NULL vs value distinction).
///
/// Also generates `new()` (alias for `Default::default()`) and
/// `has_changes()` (true if any field is `Some`).
macro_rules! patch_struct {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident < $lt:lifetime > {
            $( $kind:ident $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Default)]
        $vis struct $name<$lt> {
            $( pub $field: patch_struct!(@field_type $kind $ty), )*
        }

        impl<$lt> $name<$lt> {
            pub fn new() -> Self { Self::default() }

            $( patch_struct!(@setter $kind $field $ty); )*

            pub fn has_changes(&self) -> bool {
                false $(|| self.$field.is_some())*
            }
        }
    };

    (@field_type plain    $ty:ty) => { Option<$ty> };
    (@field_type nullable $ty:ty) => { Option<Option<$ty>> };

    (@setter plain    $field:ident $ty:ty) => {
        pub fn $field(mut self, v: $ty) -> Self { self.$field = Some(v); self }
    };
    (@setter nullable $field:ident $ty:ty) => {
        pub fn $field(mut self, v: Option<$ty>) -> Self { self.$field = Some(v); self }
    };
}

// ---------------------------------------------------------------------------
// TaskPatch — builder for selective field updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective task field updates.
    ///
    /// `live_subagents` is deliberately **absent**: it is a denormalised
    /// `COUNT(*)` over `task_subagents`, owned exclusively by the transactional
    /// writes in [`queries::subagents`]. Leaving it out of the patch surface
    /// makes "no handler can desync the count" a compile-time property rather
    /// than a convention. `stop_pending` is patch-driven and stays — but for
    /// *clearing* only: the sole writer that may set it true is
    /// [`try_record_stop`](TaskCrud::try_record_stop)'s defer branch, which
    /// stamps `stop_pending_at` in the same statement. Setting it true through a
    /// patch would leave that timestamp stale or absent, and
    /// `record_user_prompt_submit` decides against it.
    pub struct TaskPatch<'a> {
        plain    status:       TaskStatus,
        nullable plan_path:    &'a str,
        plain    title:        &'a str,
        plain    description:  &'a str,
        plain    repo_path:    &'a str,
        nullable worktree:     &'a str,
        nullable tmux_window:  &'a crate::models::TmuxWindow,
        nullable host:         &'a str,
        plain    sub_status:   SubStatus,
        nullable url:          &'a crate::models::TaskUrl,
        nullable tag:          TaskTag,
        nullable sort_order:   i64,
        nullable completed_at: chrono::DateTime<chrono::Utc>,
        plain    base_branch:  &'a str,
        nullable external_id:  &'a str,
        plain    labels:       &'a [String],
        nullable last_pre_tool_use_at: chrono::DateTime<chrono::Utc>,
        nullable last_notification_at: chrono::DateTime<chrono::Utc>,
        nullable last_peer_message_sent_at: chrono::DateTime<chrono::Utc>,
        nullable last_peer_message_received_at: chrono::DateTime<chrono::Utc>,
        nullable wrap_up_mode: WrapUpMode,
        plain    auto_run_plan: bool,
        plain    phoenix:       bool,
        plain    stop_pending: bool,
    }
}

// ---------------------------------------------------------------------------
// CreateTaskRequest — input struct for the create_task DB operation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct CreateTaskRequest<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub repo_path: &'a str,
    pub plan: Option<&'a str>,
    pub status: TaskStatus,
    pub base_branch: &'a str,
    pub epic_id: Option<EpicId>,
    pub sort_order: Option<i64>,
    pub tag: Option<TaskTag>,
    pub wrap_up_mode: Option<WrapUpMode>,
    pub auto_run_plan: bool,
    pub phoenix: bool,
}

// ---------------------------------------------------------------------------
// RemovedFeedTask — a row a feed stale-delete removed
// ---------------------------------------------------------------------------

/// A task row that a feed stale-delete removed and which still owns on-disk or
/// in-tmux state, so the caller has something to tear down. Returned by
/// [`TaskCrud::upsert_feed_tasks`] and
/// [`TaskCrud::delete_stale_subtree_feed_tasks`].
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

// ---------------------------------------------------------------------------
// EpicPatch — builder for selective epic field updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective epic field updates.
    pub struct EpicPatch<'a> {
        plain    title:              &'a str,
        plain    description:        &'a str,
        plain    status:             TaskStatus,
        nullable plan_path:          &'a str,
        nullable sort_order:         i64,
        nullable completed_at:       chrono::DateTime<chrono::Utc>,
        plain    auto_dispatch:      bool,
        plain    group_by_repo:      bool,
        plain    feed_append_only:   bool,
        plain    feed_role:          crate::models::FeedRole,
        plain    origin:             crate::models::EpicOrigin,
        nullable feed_command:       &'a str,
        nullable feed_interval_secs: i64,
        nullable parent_epic_id:     EpicId,
    }
}

// ---------------------------------------------------------------------------
// Sub-traits — focused slices of the database API
// ---------------------------------------------------------------------------

/// Read-only task queries. Held (via [`TaskReadStore`]) by non-service consumers so
/// a direct task *mutation* from a handler is a compile error — see the
/// "Service layer is the mutation boundary" section of `docs/conventions.md`.
#[async_trait::async_trait]
pub trait TaskRead: Send + Sync {
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>>;
    /// Whether a task row exists, without materialising it. For callers whose
    /// only question is existence — typically to honour a `NotFound` contract
    /// before a mutation — where [`get_task`](Self::get_task) would decode every
    /// column and discard the result.
    async fn task_exists(&self, id: TaskId) -> Result<bool>;
    async fn list_all(&self) -> Result<Vec<Task>>;
    /// The tasks [`Task::is_live_agent`] accepts, ordered by id — the
    /// agent-tree pane's agents section, read once a second per pane, so it
    /// filters in SQL rather than decoding every task ever created.
    async fn list_live_agent_tasks(&self) -> Result<Vec<Task>>;
    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>>;
}

/// Task mutations, layered on top of the read surface. Reachable only by the
/// service layer (which owns invariants like epic-status recalculation) and by
/// the sanctioned feed subsystem — non-service consumers hold [`TaskReadStore`].
#[async_trait::async_trait]
pub trait TaskCrud: TaskRead {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId>;
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
    /// Atomically select *and* claim `epic_id`'s next backlog subtask for
    /// dispatch: the first one ordered by `COALESCE(sort_order, id)` then `id`
    /// moves to `Running` with the default running sub-status and
    /// `last_pre_tool_use_at = now`. Returns the id it claimed, or `None` when
    /// the epic has no backlog subtask left.
    ///
    /// Selection and claim are one statement — the ordering predicate lives in
    /// the `WHERE id = (SELECT … LIMIT 1)` subquery — so there is no
    /// select-then-claim window for a concurrent caller to win, and hence no
    /// retry. Exclusivity comes from that plus the single writer connection all
    /// mutations serialise through, not from a lock. Backs the epic-chaining
    /// claim described by `AutoDispatchNextSubtask` in `docs/specs/epics.allium`.
    async fn try_claim_next_backlog_task(
        &self,
        epic_id: EpicId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<TaskId>>;
    /// The by-id twin of [`Self::try_claim_next_backlog_task`]: claim `id` for
    /// dispatch if and only if it is still `Backlog`, applying the same
    /// `Running` + default sub-status + `last_pre_tool_use_at` write. Returns
    /// whether the claim was won.
    ///
    /// For callers handed a specific task rather than selecting one — the MCP
    /// `dispatch_task` tool and every TUI dispatch path. One statement, so a
    /// claim never half-applies: `Ok(false)` means someone else holds it (or it
    /// is gone) and `Err` means nothing was written, which is what lets callers
    /// treat a failed claim as "provision nothing" with no unwind to do. Backs
    /// `DispatchClaimExclusive` in `docs/specs/dispatch.allium`.
    async fn try_claim_backlog_task(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool>;
    /// The inverse of [`Self::try_claim_next_backlog_task`]: return a claimed but
    /// still-unprovisioned task to `Backlog` and clear the activity stamp the
    /// claim seeded. Conditional on `status = running AND worktree IS NULL`, so
    /// it cannot stomp a task that has since been provisioned or moved by
    /// someone else. Returns whether the release applied.
    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool>;
    /// Upsert tasks from a feed. Inserts new tasks; on conflict (epic_id, external_id)
    /// updates title and description only — status and other user-managed fields are preserved.
    ///
    /// `repo_paths` is a parallel slice: `repo_paths[i]` is the resolved local path for
    /// `items[i]`. Pass `""` when the path could not be resolved — dispatch will be blocked
    /// until the user sets it via the task editor.
    ///
    /// `base_branches` is a parallel slice: `base_branches[i]` is the base branch the
    /// inserted task should use for its worktree. The caller is expected to resolve the
    /// repo's default branch (typically via [`crate::git::detect_default_branch`]),
    /// falling back to `"main"` when no path is known.
    ///
    /// Returns the rows its stale-delete pass removed that still own a worktree
    /// or tmux window — see [`RemovedFeedTask`]. The delete itself is unchanged:
    /// every stale feed task in the epic is removed whether or not it is
    /// reported.
    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>>;
    /// The insert/update half of [`TaskCrud::upsert_feed_tasks`] WITHOUT its
    /// stale-delete pass: items present in `items` are inserted or refreshed
    /// exactly as they would be, and feed tasks absent from `items` are left
    /// alone rather than deleted.
    ///
    /// For a partially degraded emission — a feed command that wrote to stderr
    /// while still emitting items — whose omissions are not trustworthy
    /// evidence that the tasks are gone (feeds.allium:
    /// `DegradedNonEmptyEmission`). An empty `items` is therefore a no-op here,
    /// not the "clear the epic" that [`TaskCrud::upsert_feed_tasks`] treats it
    /// as.
    ///
    /// Always returns an EMPTY [`RemovedFeedTask`] list, because it removes
    /// nothing — there is, by construction, nothing for the caller to tear
    /// down. The return type matches [`TaskCrud::upsert_feed_tasks`] on purpose:
    /// callers pick between the two by mode and then share one success/failure
    /// tail (the `removed_or_warn` + recalculate shape), rather than forking
    /// into two arms that each restate it.
    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>>;
    /// Delete stale feed tasks across the WHOLE subtree of `parent_id` (all its
    /// direct child epics), keeping only those whose `external_id` is in
    /// `keep_external_ids`. Unlike [`TaskCrud::upsert_feed_tasks`]'s per-epic
    /// delete pass, this is subtree-scoped with a global keep-set, so a task
    /// that was just *moved* between role sub-epics (absent from its losing
    /// epic's group) is not deleted. Manual tasks (`external_id IS NULL`) are
    /// always preserved. Used by the role-routed reviews reconciler.
    ///
    /// Returns the removed rows that still own a worktree or tmux window — see
    /// [`RemovedFeedTask`]. The delete predicate is unchanged: every stale feed
    /// task in the subtree is removed whether or not it is reported.
    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<Vec<RemovedFeedTask>>;
    /// Atomically update `sub_status` for multiple tasks in a single transaction.
    /// Used by the tick to batch all per-task reclassifications into one DB round-trip.
    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()>;
    /// Insert a watch: `watcher_task_id` wants to be notified when
    /// `target_task_id` finishes (`Done`) or is deleted first.
    /// Idempotent — inserting an existing (watcher, target) pair is a no-op.
    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()>;
    /// Remove a specific watch. Idempotent — no-op if it doesn't exist.
    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()>;
    /// List every task currently watching `target_task_id`.
    async fn list_watchers_of(&self, target_task_id: TaskId) -> Result<Vec<TaskId>>;
    /// Remove every watch row where `target_task_id` is the target. Called
    /// after firing finish/delete notifications for that target.
    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<()>;
    /// Remove every watch row where `watcher_task_id` is the watcher. Called
    /// when the watcher itself is deleted.
    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<()>;

    // Retired feed items (`docs/specs/core.allium`: `RetiredFeedItem`).
    //
    // The write is a reducer call (`drop_closed_retired_feed_items`); the read
    // (`retired_without_task`) is a join a subscription's `WHERE` clause cannot
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
    /// `DropClosedRetiredFeedItems` (`docs/specs/feeds.allium`): drop every
    /// `retired_feed_items` row keyed on `feed_epic_id` whose `external_id` is
    /// absent from `keep_external_ids`. Called only after a TRUSTED (mirroring,
    /// non-additive) cycle — an additive cycle must not call this.
    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<()>;
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
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>>;
    async fn list_epics(&self) -> Result<Vec<Epic>>;
    /// List only root epics (no parent). Used for the main board view.
    async fn list_root_epics(&self) -> Result<Vec<Epic>>;
    /// List direct children of the given epic.
    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>>;
    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<Task>>;
    /// Ids of the epic's tasks the bulk read skipped because the row did not
    /// decode, ascending. Such a task is still in the epic's subtree with an
    /// unknown status (epics.allium: `DeleteEpicRefused`).
    async fn list_undecodable_task_ids_for_epic(&self, epic_id: EpicId) -> Result<Vec<TaskId>>;
    /// Fetch all tasks that have a non-null epic_id in a single query.
    /// Use instead of looping over epics and calling list_tasks_for_epic() per epic.
    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<Task>>;
}

/// Epic mutations, layered on top of the read surface. The
/// `recalculate_epic_status` invariant lives here; reachable only by the service
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
    ) -> Result<Epic>;
    /// Find-or-create the `RepoGroup` sub-epic of `parent_id` titled `title`.
    /// Race-safe via the partial unique index; reuses an existing match
    /// rather than creating a duplicate.
    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId>;
    /// Create a managed-feed-role epic in a single insert, `feed_role` set from
    /// the start. Race-safe via the partial unique index on
    /// `(parent_epic_id, feed_role)`: a lost race re-selects and returns the
    /// winner's id rather than leaving an orphaned, untagged epic behind — see
    /// [`Self::create_repo_group_sub_epic`] for the same pattern.
    async fn create_managed_role_epic(
        &self,
        title: &str,
        parent_epic_id: Option<EpicId>,
        role: FeedRole,
        feed_command: Option<&str>,
        feed_interval_secs: Option<i64>,
    ) -> Result<EpicId>;
    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()>;
    async fn delete_epic(&self, id: EpicId) -> Result<()>;
    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()>;
    /// Recalculate an epic's status from its active children (tasks + sub-epics).
    /// Propagates upward to the parent epic if one exists.
    async fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()>;
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
    async fn list_repo_paths(&self) -> Result<Vec<String>>;

    async fn get_verify_command(&self, path: &str) -> Result<Option<String>>;

    /// All `(repo_path, branch)` pairs across every repo, ordered by
    /// `last_used DESC, id DESC`.
    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>>;
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
// HostStore — the `hosts` shared table
// ---------------------------------------------------------------------------

/// This install's entry in the host registry (`SharedTable::Hosts`) — see
/// [`TaskStore`]. Its reads and writes stay on this machine; the shared
/// registry gets a mirror (`sync.allium: RegisterHostOnConnect`).
///
/// Backed by this install's `host.json` (`host.allium:
/// IdentityLivesInHostFile`), not by a store table. [`SettingsStore`]'s
/// key/value accessors refuse the identity keys, so this trait is the only way
/// to read or change them.
///
/// See `docs/specs/host.allium` (MintHostIdentity, RenameHost) and `core/Host`
/// in `docs/specs/core.allium`.
#[async_trait::async_trait]
pub trait HostStore: Send + Sync {
    /// This install's `Host` id, as the handle was built with it. Resolved
    /// once per process before the handle exists (`host.allium:
    /// MintHostIdentity` at a board's launch, the host file for a one-shot
    /// command), so it cannot fail here. The claim's `is_locally_owned` reads
    /// the same value, so the claim and the dispatch's host stamp agree
    /// (`dispatch.allium: DispatchTask`).
    fn host_id(&self) -> &str;

    /// Mint this install's Host id if it does not already exist — an opaque
    /// generated id, nothing else — and return the current `(id, label)`
    /// either way. `label` is `None` until an operator names this machine via
    /// `rename_host` (see `docs/specs/startup.allium`: `HostLabelPrompt`,
    /// which asks for exactly that before the board's first launch draws).
    ///
    /// Idempotent by construction: the insert is `ON CONFLICT DO NOTHING`, so
    /// a second call (or a race between several dispatch processes on first
    /// run) never re-mints the id. Never call anything that mutates `id`
    /// after this — `RenameHost` only ever touches the label.
    async fn ensure_host_identity(&self) -> Result<(String, Option<String>)>;

    /// Change this install's Host label. Rejects an empty (or
    /// whitespace-only) string; the id is untouched.
    async fn rename_host(&self, label: &str) -> Result<()>;

    /// This install's UserIdentity — `core/Host.owner` for the local row — or
    /// `None` if it has never connected to a shared store.
    ///
    /// `None` until the first connection settles one. Since the store became
    /// mandatory (task #4916) a drawn board has always connected, but a
    /// process that has not — a failed first attempt, a test — still sees
    /// `None`, so every caller handles the absence rather than unwrapping it.
    async fn user_identity(&self) -> Result<Option<String>>;

    /// Store the identity a shared store issued.
    ///
    /// **Writes once.** A second call with a DIFFERENT identity leaves the
    /// stored one alone rather than overwriting it — the decision about a
    /// changed identity belongs to `crate::sync::settle_identity`, which
    /// refuses it, and this method must not quietly do the opposite underneath.
    ///
    /// Rejects an empty identity. The CREDENTIAL that proves this identity is
    /// deliberately not written here: it is local and secret, so it lives on
    /// the other half of the seam — see
    /// [`IdentityCredentialStore::set_user_identity_token`].
    async fn adopt_user_identity(&self, identity: &str) -> Result<()>;

    /// Adopt `identity` together with the `credential` that proves it, as ONE
    /// write: on a handle that keeps its identity in the host file, one
    /// whole-file replacement, so the file never holds the one without the
    /// other (`host.allium: AdoptUserIdentity`). The owner keeps write-once
    /// semantics; the credential is refreshed. Rejects an empty identity or
    /// credential.
    async fn adopt_user_identity_with_credential(
        &self,
        identity: &str,
        credential: &str,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// IdentityCredentialStore — the local secret that proves the shared identity
// ---------------------------------------------------------------------------

/// The credential this install presents to prove it is
/// [`HostStore::user_identity`]. Never routed to the store — see
/// [`TaskStore`].
///
/// **Deliberately not on [`HostStore`]**, although it is about the same
/// identity. `HostStore` is the shared half, and a second backend implementing
/// it is the shared store itself; a credential kept there is a credential every
/// person on the board can read and use. There is no column for it in the
/// SpacetimeDB module and it never travels in a snapshot, so the only honest
/// place for it is here.
///
/// The split has a practical consequence worth knowing: adopting an identity is
/// two writes on two halves, not one. They are not atomic, and they do not need
/// to be — an identity with no credential presents as a conflict on the next
/// connection, which is a state the system already refuses loudly.
#[async_trait::async_trait]
pub trait IdentityCredentialStore: Send + Sync {
    /// The stored credential, or `None` on an install that has never connected.
    async fn user_identity_token(&self) -> Result<Option<String>>;

    /// Store or refresh the credential.
    ///
    /// Overwrites, unlike the identity it proves: refreshing the proof of the
    /// SAME identity is ordinary. Rejects an empty credential — an identity
    /// this install cannot prove again survives until the next connection and
    /// then presents as a conflict, which is worse than not storing it.
    async fn set_user_identity_token(&self, token: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// SubscriptionStore — the `subscriptions` shared table
// ---------------------------------------------------------------------------

/// Which epics a person follows (`SharedTable::Subscriptions`) — see
/// [`TaskStore`].
///
/// A subscription belongs to the PERSON, not the machine, which is why every
/// method here takes a subscriber rather than reading the local host: following
/// an epic on the desktop is already true on the laptop.
///
/// There is deliberately no method for subscribing to a user board. Your own
/// needs no row — the identity implies it — and somebody else's is theirs. The
/// absence is the enforcement: an operation that could be called and refused
/// would be an operation whose refusal could be got wrong. See
/// `docs/specs/sync.allium` (SubscribeToEpic) and `core/Subscription`.
#[async_trait::async_trait]
pub trait SubscriptionStore: Send + Sync {
    /// The epic ids `subscriber` follows, ascending.
    async fn subscribed_epics(&self, subscriber: &str) -> Result<Vec<EpicId>>;

    /// Follow `epic_id`. Idempotent: subscribing twice is subscribing.
    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: EpicId) -> Result<()>;

    /// Stop following `epic_id`. Returns whether a subscription was actually
    /// removed, so a caller can refuse an unsubscribe from something
    /// unfollowed — the asymmetry with `subscribe_to_epic` is deliberate and
    /// `sync.allium: UnsubscribeFromEpic` says why.
    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: EpicId) -> Result<bool>;
}

// ---------------------------------------------------------------------------
// TaskAndEpicStore — composite for consumers that need tasks + epics only
// ---------------------------------------------------------------------------

pub trait TaskAndEpicStore: TaskCrud + EpicCrud {}

impl<T: TaskCrud + EpicCrud> TaskAndEpicStore for T {}

// ---------------------------------------------------------------------------
// LearningPatch — builder for partial learning updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective learning field updates.
    ///
    /// Deliberately narrow: `embedding` is the only field with a production
    /// writer (the startup backfill in `runtime::bootstrap`), and `status` is
    /// written by tests seeding archived/rejected rows. There is no field-editing
    /// path for a learning's content — `detail`/`kind`/`tags` setters were
    /// removed with the TUI overlay, which keeps a stale embedding
    /// unrepresentable (see `docs/specs/learnings.allium`: EmbeddingConsistency).
    pub struct LearningPatch<'a> {
        plain    status:    LearningStatus,
        plain    summary:   &'a str,
        plain    embedding: &'a [u8],
    }
}

// ---------------------------------------------------------------------------
// LearningFilter — optional filter for list_learnings
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct LearningFilter {
    pub status: Option<LearningStatus>,
    pub scope: Option<LearningScope>,
    pub scope_ref: Option<String>,
    /// Return only learnings whose tags intersect this set (OR match).
    pub tags: Vec<String>,
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// CreateLearningRow — DB-layer params for inserting a learning row
// ---------------------------------------------------------------------------

pub struct CreateLearningRow<'a> {
    pub kind: LearningKind,
    pub summary: &'a str,
    pub detail: Option<&'a str>,
    pub scope: LearningScope,
    pub scope_ref: Option<&'a str>,
    pub tags: &'a [String],
    pub source_task_id: Option<TaskId>,
    pub embedding: Option<&'a [u8]>,
}

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

#[derive(Debug, Default)]
pub struct UsageQuery {
    pub category: Option<String>,
    pub actor: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<usize>,
}

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
/// | `tasks`, `task_watchers`, `task_subagents` | [`TaskCrud`] / [`TaskRead`] |
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
    + RepoConfigStore
    + HostStore
    + SubscriptionStore
    + SettingsStore
    + IdentityCredentialStore
    + LearningStore
    + LearningRetrievalStore
    + UsageStore
    + TaskReadStore
{
}

impl<
        T: TaskAndEpicStore
            + RepoConfigStore
            + HostStore
            + SubscriptionStore
            + SettingsStore
            + IdentityCredentialStore
            + LearningStore
            + LearningRetrievalStore
            + UsageStore
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
/// `TaskServiceApi`/`EpicServiceApi`, which own the `recalculate_epic_status`
/// invariant — see the mutation-boundary section of `docs/conventions.md`.
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
    + RepoConfigStore
    + HostStore
    + SettingsStore
    + IdentityCredentialStore
    + PollOwnershipStore
    + LearningStore
    + LearningRetrievalStore
    + UsageStore
{
}

impl<
        T: TaskRead
            + EpicRead
            + RepoConfigStore
            + HostStore
            + SettingsStore
            + IdentityCredentialStore
            + PollOwnershipStore
            + LearningStore
            + LearningRetrievalStore
            + UsageStore,
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
}

/// The host id an in-memory handle's writes and host file both carry, and so
/// the host every poll-owner claim it makes is recorded under.
#[cfg(any(test, feature = "test-support"))]
pub const MEMORY_HOST_ID: &str = "test-host";

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
        }
    }

    /// A handle attached to a fresh in-memory store, which covers every
    /// reducer domain (spec: `spacetime-memory-store.allium`,
    /// `OpenInMemoryAttachesStoreOnceComplete`). Async only so the many test
    /// call sites that `.await` it need no change.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn open_in_memory() -> Result<Self> {
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
        let caller: Arc<dyn s::ReducerCaller> = Arc::new(
            s::memory_caller::MemoryReducerCaller::new(rows.clone(), clock.clone()),
        );
        let identity = Arc::new(s::SettledIdentity::default());
        identity.settle("test-user");
        Self::new(
            rows,
            caller,
            identity,
            clock,
            MEMORY_HOST_ID.to_string(),
            data_dir,
        )
    }
}
