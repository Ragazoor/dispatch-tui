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

use anyhow::{Context, Result};
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
    /// `spacetime/module/src/tasks_epics.rs::batch_delete` for the shared-store path,
    /// which is the one that actually needs the joint guard (a stale
    /// subscription view is what "one operation, or nothing at all" has to
    /// hold up against; the local SQLite path has always had a single
    /// serialized writer and no such gap, matching [`Self::delete_task`]'s and
    /// [`EpicCrud::delete_epic`]'s own SQLite shape, which never re-validates
    /// there either).
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
    // The write routes through [`SharedWriter`] (`drop_closed_retired_feed_items`);
    // the read routes through [`SharedRetiredFeedItemReader`]
    // (`retired_without_task`) instead of `SharedWriter`, for the same reason
    // `query_usage` and the learning reads do: it is a join a subscription's
    // `WHERE` clause cannot express, so it has to run in Rust over the store's
    // rows rather than through a reducer. There is no `create_retired_feed_item`
    // port here — every real retirement writes the row inline, in the same
    // transaction as the delete (`delete_task`, `delete_epic`) or the v105/v106
    // migration, rather than through a separate call.
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
    /// Fill an absent claim. A no-op on a single-machine install — `PollOwner`
    /// has no SQLite counterpart, so there is no other host to contend a
    /// claim with. `claim_poll_owner`/`override_poll_owner` mirror
    /// [`SharedWriter`]'s own naming: `claim_poll_owner` fills an absent row,
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

/// Per-host preferences: key/value settings and the managed-feed config. **Routed, not local** (Phase 9) — a mutation goes
/// through [`SharedWriter`] when one is configured, scoped to this install's
/// own host id, and falls back to the local table otherwise; see
/// `docs/specs/settings.allium`. Reads route the same way, through
/// [`SharedReader`] (task #4916): a board that writes a setting to the store
/// reads it back from the store, never from a local table it no longer
/// writes.
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
/// Backed by two rows in SQLite's `settings` table rather than a dedicated one:
/// there is exactly one Host per install, so a key/value pair per field is
/// simpler than a one-row table. The trait says nothing about that — a backend
/// that holds a real `hosts` table satisfies it equally.
///
/// **The seam is declared here, not sealed.** Because SQLite stores the id and
/// label as ordinary `settings` rows, the same two rows are also readable and
/// writable through [`SettingsStore`]'s untyped key/value accessors — the local
/// half. Nothing stops a local-only consumer rewriting this install's host id
/// that way. Go through this trait; the key/value route is a leftover of the
/// backing, not a second supported API.
///
/// See `docs/specs/host.allium` (MintHostIdentity, RenameHost) and `core/Host`
/// in `docs/specs/core.allium`.
#[async_trait::async_trait]
pub trait HostStore: Send + Sync {
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
    async fn subscribed_epics(&self, subscriber: &str) -> Result<Vec<i64>>;

    /// Follow `epic_id`. Idempotent: subscribing twice is subscribing.
    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: i64) -> Result<()>;

    /// Stop following `epic_id`. Returns whether a subscription was actually
    /// removed, so a caller can refuse an unsubscribe from something
    /// unfollowed — the asymmetry with `subscribe_to_epic` is deliberate and
    /// `sync.allium: UnsubscribeFromEpic` says why.
    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: i64) -> Result<bool>;
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
/// with its credential (`host.allium`) — is a property of which methods
/// `Store` routes, not of which trait declares them.
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
// SharedLearningReader — where a learning read goes, when it does not go here
// ---------------------------------------------------------------------------

/// The read twin of [`SharedWriter`], scoped to the one table whose reads
/// need to leave SQLite: the knowledge base (Phase 10, task #4914).
///
/// Every other shared table's reads have their own dedicated seam already
/// (`crate::sync::BoardReads` for tasks/epics/repo config), because the
/// board's drawing needs them. Learnings have no TUI presence at all, so
/// there is no `BoardReads`-shaped consumer to fold this into — see
/// `crate::sync::learning_reads`'s header for why it is a sibling seam
/// rather than an addition to that one.
///
/// `db` defines this port and `sync` implements it
/// (`sync::SubscriptionLearningReads`), the same inversion `SharedWriter`
/// uses: nothing in `db` knows what a subscription is.
#[async_trait::async_trait]
pub trait SharedLearningReader: Send + Sync {
    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>>;
    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>>;
    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>>;
    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>>;
    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>>;
}

// ---------------------------------------------------------------------------
// SharedUsageReader — where a usage read goes, when it does not go here
// ---------------------------------------------------------------------------

/// The read twin of [`SharedWriter`], scoped to `usage_events` (Phase 11, task
/// #4915).
///
/// `query_usage` groups and counts rows — a shape a subscription's `WHERE`
/// clause cannot express at all, let alone the dynamic category/actor/since
/// filters the MCP tool takes. So unlike every board-drawing table
/// (`crate::sync::BoardReads`), the aggregation has to run in Rust over the
/// rows a standing, unconditional subscription already holds in memory — the
/// same reasoning `SharedLearningReader` exists for, applied to a query
/// SQLite could answer with `GROUP BY` and a store cannot.
///
/// `db` defines this port and `sync` implements it
/// (`sync::SubscriptionUsageReads`), the same inversion `SharedWriter` and
/// `SharedLearningReader` use: nothing in `db` knows what a subscription is.
#[async_trait::async_trait]
pub trait SharedUsageReader: Send + Sync {
    async fn query_usage(&self, query: &UsageQuery) -> Result<Vec<crate::models::UsageSummary>>;
}

// ---------------------------------------------------------------------------
// SharedReader — where every other shared read goes
// ---------------------------------------------------------------------------

/// The read twin of [`SharedWriter`] for everything [`SharedLearningReader`]
/// and [`SharedUsageReader`] do not cover: tasks, epics, watchers, repo
/// configuration, subscriptions and settings.
///
/// Spec: `sync.allium`'s `BoardReadsFromTheSubscription`.
///
/// # Why this exists
///
/// A board with a store writes a shared row to the store and nowhere else
/// ("one copy, not two" — see [`SharedWriter`]). Until task #4916 its reads of
/// those same rows still went to SQLite, which from that moment held nothing
/// new: an MCP `get_task` on a just-created task answered `None`, a setting
/// read back after it was saved answered the old value, and the watcher
/// fan-out found nobody. Only the card-drawing reads
/// ([`crate::sync::BoardReads`]) had a store path. This port closes the rest.
///
/// Every derived query (live agents, a task by plan, an epic's tasks, an
/// epic's children) is a method here rather than a filter `Store` runs over
/// `list_all`: the implementation filters inside the rows' lock before
/// cloning, and each routed `Store` method is a one-line delegation. The
/// managed-feed getters are `get_setting` under a fixed key.
///
/// `db` defines this port and `sync` implements it
/// (`sync::SubscriptionBoardReads`, the adapter the board already draws
/// from), the same inversion the writer uses.
#[async_trait::async_trait]
pub trait SharedReader: Send + Sync {
    /// Every task, ordered `COALESCE(sort_order, id) ASC, id ASC`.
    async fn list_all(&self) -> Result<Vec<Task>>;
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>>;
    async fn task_exists(&self, id: TaskId) -> Result<bool>;
    /// Running or Review tasks with a tmux window, ordered by id.
    async fn list_live_agent_tasks(&self) -> Result<Vec<Task>>;
    /// The lowest-id task whose plan is `plan`.
    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>>;
    /// An epic's tasks, in `list_all`'s order.
    async fn list_tasks_for_epic(&self, epic: EpicId) -> Result<Vec<Task>>;
    /// An epic's tasks that did not decode, by ascending id.
    async fn list_undecodable_task_ids_for_epic(&self, epic: EpicId) -> Result<Vec<TaskId>>;
    /// Every task with an epic, ordered by epic, then as `list_all`.
    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<Task>>;
    /// The watcher task ids of `target`, ordered by watch id.
    async fn list_watchers_of(&self, target: TaskId) -> Result<Vec<TaskId>>;
    /// Every epic, ordered `COALESCE(sort_order, id) ASC, id ASC`.
    async fn list_epics(&self) -> Result<Vec<Epic>>;
    /// Epics whose parent is `parent` (`None` for the roots), in
    /// `list_epics`'s order.
    async fn list_epics_with_parent(&self, parent: Option<EpicId>) -> Result<Vec<Epic>>;
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>>;
    async fn list_repo_paths(&self) -> Result<Vec<String>>;
    async fn get_verify_command(&self, path: &str) -> Result<Option<String>>;
    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>>;
    /// `subscriber`'s followed epic ids, ascending.
    async fn subscribed_epics(&self, subscriber: &str) -> Result<Vec<i64>>;
    /// This host's setting `key`, if set.
    async fn get_setting(&self, key: &str) -> Result<Option<String>>;
}

// ---------------------------------------------------------------------------
// SharedWriter — where a shared-table mutation goes
// ---------------------------------------------------------------------------

/// The destination of a shared-table mutation on a board that has a store.
///
/// Spec: `sync.allium`'s `BoardWritesThroughTheStore`.
///
/// # Why a port here rather than a second store
///
/// The read side got a seam of its own ([`crate::sync::BoardReads`]) because
/// the board's reads are a small, self-contained set that a subscription can
/// serve whole. The write side is not like that. A mutation arrives through
/// [`Store`] — the same handle that also holds settings, learnings,
/// embeddings and usage, none of which are shared and none of which a store
/// would accept. Swapping the whole handle would mean a second implementation
/// of a hundred local methods that have nowhere else to go.
///
/// So the branch is here instead, at the one point every shared mutation
/// already passes through, and this trait is the port it branches to. `db`
/// defines it; `crate::sync` implements it over reducers. Nothing in `db`
/// knows what a reducer is.
///
/// # One copy, not two
///
/// A method implemented here is a method [`Store`] no longer performs
/// locally when a writer is attached. Not "also performs": a shared table has
/// exactly one copy, and a local one that nothing reads would diverge from the
/// store at the first mutation and leave the operator unable to tell which they
/// were looking at.
///
/// # The methods here are the ones cut over
///
/// As of task #4907, this is the whole shared mutation surface — see below.
///
/// # What was routed, and by which task
///
/// Task CRUD, the dispatch claim, epic CRUD and recalculation, repo
/// configuration and subscriptions — task #4864/#4905 and earlier Phase 6
/// work. Agent session state (`subagent_start`, `subagent_stop`,
/// `subagent_clear`, `subagent_clear_and_void_pending_stop`, `try_record_stop`,
/// `record_pre_tool_use`, `record_notification`, `record_user_prompt_submit`,
/// `mark_pr_learnings_gate_shown`) — task #4906. Feed ingestion
/// (`upsert_feed_tasks`, `upsert_feed_tasks_additive`,
/// `delete_stale_subtree_feed_tasks`, `create_repo_group_sub_epic`,
/// `create_managed_role_epic`), task watchers (`create_task_watcher`,
/// `delete_task_watcher`, `delete_watches_of_target`,
/// `delete_watches_by_watcher`), `batch_patch_sub_status` and
/// `respawn_phoenix_successor` — task #4907, this task.
///
/// # The host registry is a decision, not an omission
///
/// `ensure_host_identity`, `adopt_user_identity` and `rename_host` are NOT
/// here and never will be: the identity handshake writes this install's Host
/// row locally, before any connection exists, and that write must keep
/// happening unconditionally — it is the durable local credential, not a
/// shared row with one copy (the single-storage design doc's declared
/// permanent local exception). What DOES reach the store is a separate
/// best-effort mirror, `register_host` (not on this trait — see
/// [`crate::sync::push_host_registration`]), pushed on every connect/reconnect
/// and on a live rename — `sync.allium: RegisterHostOnConnect`/
/// `RegisterHostOnRename`. Decided on task #4907 rather than assumed.
///
/// # One path
///
/// Every board has a store (task #4916), so every shared mutation takes the
/// writer: the `Some` branch of each routed method is the production path,
/// and the SQLite branch remains only for handles built without a writer —
/// the test suite's in-memory database, until Phase 12b (#4975) replaces it.
/// The completeness flag that once gated `--spacetime-server` on this list
/// being finished went with the store-less board.
#[async_trait::async_trait]
pub trait SharedWriter: Send + Sync {
    // Tasks.
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId>;
    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()>;
    async fn delete_task(&self, id: TaskId) -> Result<()>;
    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()>;

    // The dispatch claim. Each returns whether it was WON, which on a store is
    // the reducer's acceptance rather than a row read back — see
    // `dispatch.allium: DispatchClaimExclusive`.
    async fn try_claim_next_backlog_task(&self, epic_id: EpicId) -> Result<Option<TaskId>>;
    async fn try_claim_backlog_task(&self, id: TaskId) -> Result<bool>;
    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool>;

    // Epics.
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<Epic>;
    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()>;
    async fn delete_epic(&self, id: EpicId) -> Result<()>;
    async fn recalculate_epic_status(&self, id: EpicId) -> Result<()>;

    // Batch delete (tasks.allium: BatchDelete) — one atomic call over both
    // domains together, not `delete_task`/`delete_epic` looped per item. See
    // `TaskCrud::batch_delete`'s doc comment for why looping them independently
    // is exactly the gap this exists to close.
    async fn batch_delete(&self, task_ids: &[TaskId], epic_ids: &[EpicId]) -> Result<()>;

    // Repo configuration.
    async fn save_repo_path(&self, path: &str) -> Result<()>;
    async fn delete_repo_path(&self, path: &str) -> Result<()>;
    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()>;
    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()>;

    // Subscriptions.
    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: i64) -> Result<()>;
    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: i64) -> Result<bool>;

    // Settings (Phase 9). Scoped by THIS writer's own host
    // id, not passed as an argument — see `ReducerWriter.host`'s doc comment
    // for why that value is a field rather than a lookup, and
    // `docs/specs/settings.allium` for why the scope is host rather than
    // owner. host_id, host_label, the stored UserIdentity and its credential
    // are NOT here: `HostStore`/`IdentityCredentialStore` write those
    // unconditionally to the local store and are deliberately never routed —
    // see task #4907 and the `register_host` reducer's doc comment.
    async fn save_setting(&self, key: &str, value: &str) -> Result<()>;
    async fn clear_setting(&self, key: &str) -> Result<()>;

    // Learnings and retrievals (Phase 10, task #4914). Unlike settings above,
    // nothing here is scoped by host — a learning's visibility is governed
    // entirely by its own scope/scope_ref (`docs/specs/learnings.allium`'s
    // Storage Backend section).
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId>;
    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()>;
    async fn delete_learning(&self, id: LearningId) -> Result<bool>;
    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<()>;
    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<()>;
    async fn apply_learning_verdicts(
        &self,
        verdicts: &[(LearningId, LearningVerdict)],
    ) -> Result<()>;
    /// The archived count is best-effort telemetry, not a correctness signal
    /// — its one caller (`runtime::learnings::exec_archive_stale_learnings`)
    /// only logs it. A reducer cannot answer with a value
    /// (`sync.allium: EveryMutationIsAtomicAndAnswered`), so the routed path
    /// always reports `0` rather than reproducing the read-back machinery
    /// `create_task` needs for its id, which this count is not worth.
    async fn archive_stale_learnings(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64>;

    // Usage events (Phase 11, task #4915). Append-only telemetry with no
    // user-observable rule beyond "recorded" — no id readback, no patch, no
    // delete. `cap` is carried on every call rather than read from a stored
    // default, so the reducer's prune stays in step with whatever
    // `UsageCap` the caller passed, the same as the local SQL path does today.
    async fn record_usage_event_with_cap(
        &self,
        event: &crate::models::UsageEvent,
        cap: UsageCap,
    ) -> Result<()>;

    // Agent session state (Phase 6b). Mirrors the `TaskCrud` methods of the
    // same name exactly — same arguments, same return types — because this is
    // the same operation on a different backing, not a different operation.
    async fn subagent_start(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i64>;
    async fn subagent_stop(
        &self,
        id: TaskId,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SubagentDrain>;
    async fn subagent_clear(&self, id: TaskId) -> Result<SubagentDrain>;
    async fn subagent_clear_and_void_pending_stop(&self, id: TaskId) -> Result<()>;
    async fn try_record_stop(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<StopOutcome>;
    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    async fn record_notification(
        &self,
        id: TaskId,
        write: NotificationWrite,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserPromptOutcome>;
    async fn mark_pr_learnings_gate_shown(&self, id: TaskId) -> Result<bool>;

    // Feed ingestion (Phase 6c). Mirrors `TaskCrud`'s methods of the same
    // name exactly — see that trait's doc comments for the field-precedence
    // and reconciliation rules, unchanged by the move to a store.
    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>>;
    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<Vec<RemovedFeedTask>>;
    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<Vec<RemovedFeedTask>>;

    // Retired feed items (task #4971): the write side. See `TaskCrud`'s own
    // methods and the doc comment there for why this is here and
    // `retired_without_task` (a READ) is not.
    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: &[String],
    ) -> Result<()>;

    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId>;
    async fn create_managed_role_epic(
        &self,
        title: &str,
        parent_epic_id: Option<EpicId>,
        role: FeedRole,
        feed_command: Option<&str>,
        feed_interval_secs: Option<i64>,
    ) -> Result<EpicId>;

    // Task watchers.
    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()>;
    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<()>;
    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<()>;
    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<()>;

    // Poll ownership (Phase 7): `core.allium: PollOwner`. `claim_poll_owner`
    // fills an absent row; `override_poll_owner` unconditionally reassigns an
    // existing one. Only ever called when a store is attached — on a
    // single-machine install there is no `SharedWriter` to reach, so the
    // caller must check `shared_writer()` first, the same as every other
    // method here.
    async fn claim_poll_owner(&self, target: PollScopeId) -> Result<()>;
    async fn override_poll_owner(&self, target: PollScopeId) -> Result<()>;

    // Stragglers.
    async fn batch_patch_sub_status(&self, updates: &[(TaskId, SubStatus)]) -> Result<()>;
    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        req: CreateTaskRequest<'_>,
        labels: &[String],
    ) -> Result<TaskId>;
}

// ---------------------------------------------------------------------------
// SharedRetiredFeedItemReader — where a retired-feed-item READ goes, when it
// does not go here
// ---------------------------------------------------------------------------

/// The read twin of [`SharedWriter`], scoped to `retired_feed_items` (task
/// #4971).
///
/// `retired_without_task` joins across `retired_feed_items` AND `tasks`
/// (feed_epic_id's whole subtree) — a shape a subscription's `WHERE` clause
/// cannot express, so the join runs in Rust over the rows a standing,
/// unconditional subscription already holds in memory, the same reasoning
/// [`SharedUsageReader`] and [`SharedLearningReader`] exist for.
///
/// `db` defines this port and `sync` implements it
/// (`sync::SubscriptionRetiredFeedItemReads`), the same inversion the other
/// two readers and `SharedWriter` use: nothing in `db` knows what a
/// subscription is.
#[async_trait::async_trait]
pub trait SharedRetiredFeedItemReader: Send + Sync {
    async fn retired_without_task(
        &self,
        feed_epic_id: EpicId,
        external_ids: &[String],
    ) -> Result<Vec<String>>;
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

/// The handle every service and handler holds: a router over the attached
/// store ports. It keeps no data of its own and opens no local database —
/// every shared read and write goes to the port that serves it, and a handle
/// with no port attached refuses (`storage.allium`).
pub struct Store {
    /// Where shared-table mutations go.
    ///
    /// `Some` on every board and CLI process (`runtime::StoreParts::build`);
    /// `None` only on the empty placeholder base before
    /// [`Store::with_shared_store`] attaches the ports, or on a test handle
    /// built without one. See [`SharedWriter`].
    shared_writer: Option<Arc<dyn SharedWriter>>,
    /// Where a learning READ goes. The read twin of `shared_writer`: the
    /// knowledge base is team-shared, so a read must answer from the same rows
    /// a write lands in. See [`SharedLearningReader`].
    shared_learning_reader: Option<Arc<dyn SharedLearningReader>>,
    /// Where a usage READ goes, so `query_usage`'s aggregation runs over the
    /// store's rows. See [`SharedUsageReader`].
    shared_usage_reader: Option<Arc<dyn SharedUsageReader>>,
    /// Where every other shared READ goes — tasks, epics, watchers, repo
    /// configuration, subscriptions and settings. See [`SharedReader`].
    shared_reader: Option<Arc<dyn SharedReader>>,
    /// Where a `retired_without_task` READ goes. See
    /// [`SharedRetiredFeedItemReader`].
    shared_retired_feed_item_reader: Option<Arc<dyn SharedRetiredFeedItemReader>>,
    /// The data directory whose `host.json` holds this install's identity
    /// (`host.allium: IdentityLivesInHostFile`). `Some` on every board and CLI
    /// process; the [`HostStore`] and [`IdentityCredentialStore`] methods
    /// refuse without it.
    host_file_dir: Option<std::path::PathBuf>,
    /// The board's read seam over the in-memory store's own rows. Set only by
    /// [`Store::open_in_memory`] (spec: `spacetime-memory-store.allium`,
    /// `TestBoardReadsShareTheHandlesRows`).
    #[cfg(any(test, feature = "test-support"))]
    memory_board_reads: Option<Arc<dyn crate::sync::BoardReads>>,
    /// The temporary data directory [`Store::open_in_memory`] keeps its host
    /// file in, removed with the handle.
    #[cfg(any(test, feature = "test-support"))]
    _memory_host_dir: Option<tempfile::TempDir>,
}

/// Every port a store-backed [`Store`] routes through, attached together
/// by [`Store::with_shared_store`] so a handle is routed all or nothing: a
/// handle with a writer and no reader is the half-routed board task #4916
/// found (writes reaching the store, reads answering from a table nothing
/// writes), and this is what keeps it from being built again.
pub struct SharedStorePorts {
    pub writer: Arc<dyn SharedWriter>,
    pub reader: Arc<dyn SharedReader>,
    pub learning_reader: Arc<dyn SharedLearningReader>,
    pub usage_reader: Arc<dyn SharedUsageReader>,
    pub retired_feed_item_reader: Arc<dyn SharedRetiredFeedItemReader>,
}

/// The host id an in-memory handle's writer and host file both carry.
#[cfg(any(test, feature = "test-support"))]
const MEMORY_HOST_ID: &str = "test-host";

/// The error every routed call returns on a handle with no port for it.
fn no_store(port: &str) -> anyhow::Error {
    anyhow::anyhow!("no shared store attached: this handle has no {port}")
}

impl Store {
    /// A handle with no store and no host file attached. In production this is
    /// the empty placeholder base the store's ports are attached to
    /// (`runtime::placeholder_database`); it holds no data.
    pub fn unattached() -> Self {
        Store {
            shared_writer: None,
            shared_learning_reader: None,
            shared_usage_reader: None,
            shared_reader: None,
            shared_retired_feed_item_reader: None,
            host_file_dir: None,
            #[cfg(any(test, feature = "test-support"))]
            memory_board_reads: None,
            #[cfg(any(test, feature = "test-support"))]
            _memory_host_dir: None,
        }
    }

    /// Route every shared read and write through `ports`. The one production
    /// way to attach a store; the per-port builders below exist for the
    /// tests that exercise a single port.
    ///
    /// Consuming rather than a setter, so which backing a read or write goes
    /// to cannot change under a caller mid-operation.
    pub fn with_shared_store(mut self, ports: SharedStorePorts) -> Self {
        self.shared_writer = Some(ports.writer);
        self.shared_reader = Some(ports.reader);
        self.shared_learning_reader = Some(ports.learning_reader);
        self.shared_usage_reader = Some(ports.usage_reader);
        self.shared_retired_feed_item_reader = Some(ports.retired_feed_item_reader);
        self
    }

    /// Keep this install's identity in `<data_dir>/host.json`
    /// (`host.allium: IdentityLivesInHostFile`). The one production way to
    /// build a handle's identity half; consuming for the same reason
    /// [`Self::with_shared_store`] is.
    pub fn with_host_file(mut self, data_dir: &Path) -> Self {
        self.host_file_dir = Some(data_dir.to_path_buf());
        self
    }

    /// The data directory holding the host file.
    fn host_file_dir(&self) -> Result<&Path> {
        self.host_file_dir.as_deref().ok_or_else(|| {
            anyhow::anyhow!("no host file attached: this handle has no data directory")
        })
    }

    /// Route the [`SharedReader`] reads to `reader`.
    #[cfg(test)]
    pub(crate) fn with_shared_reader(mut self, reader: Arc<dyn SharedReader>) -> Self {
        self.shared_reader = Some(reader);
        self
    }

    /// The shared reader.
    fn shared_reader(&self) -> Result<&Arc<dyn SharedReader>> {
        self.shared_reader
            .as_ref()
            .ok_or_else(|| no_store("shared reader"))
    }

    /// Route shared-table mutations to `writer`.
    #[cfg(test)]
    pub(crate) fn with_shared_writer(mut self, writer: Arc<dyn SharedWriter>) -> Self {
        self.shared_writer = Some(writer);
        self
    }

    /// The writer — the single accessor every routed mutation reads.
    fn shared_writer(&self) -> Result<&Arc<dyn SharedWriter>> {
        self.shared_writer
            .as_ref()
            .ok_or_else(|| no_store("shared writer"))
    }

    /// Route learning reads to `reader`.
    #[cfg(test)]
    pub(crate) fn with_shared_learning_reader(
        mut self,
        reader: Arc<dyn SharedLearningReader>,
    ) -> Self {
        self.shared_learning_reader = Some(reader);
        self
    }

    /// The learning reader.
    fn shared_learning_reader(&self) -> Result<&Arc<dyn SharedLearningReader>> {
        self.shared_learning_reader
            .as_ref()
            .ok_or_else(|| no_store("learning reader"))
    }

    /// Route usage reads to `reader`.
    #[cfg(test)]
    pub(crate) fn with_shared_usage_reader(mut self, reader: Arc<dyn SharedUsageReader>) -> Self {
        self.shared_usage_reader = Some(reader);
        self
    }

    /// The usage reader.
    fn shared_usage_reader(&self) -> Result<&Arc<dyn SharedUsageReader>> {
        self.shared_usage_reader
            .as_ref()
            .ok_or_else(|| no_store("usage reader"))
    }

    /// Route `retired_without_task` reads to `reader`.
    #[cfg(test)]
    pub(crate) fn with_shared_retired_feed_item_reader(
        mut self,
        reader: Arc<dyn SharedRetiredFeedItemReader>,
    ) -> Self {
        self.shared_retired_feed_item_reader = Some(reader);
        self
    }

    /// The retired-feed-item reader.
    fn shared_retired_feed_item_reader(&self) -> Result<&Arc<dyn SharedRetiredFeedItemReader>> {
        self.shared_retired_feed_item_reader
            .as_ref()
            .ok_or_else(|| no_store("retired feed item reader"))
    }

    /// A handle attached to a fresh in-memory store, which covers every
    /// reducer domain (spec: `spacetime-memory-store.allium`,
    /// `OpenInMemoryAttachesStoreOnceComplete`). Async only so the many test
    /// call sites that `.await` it need no change.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn open_in_memory() -> Result<Self> {
        let (ports, board_reads) = Self::memory_store_ports();
        // Identity lives in a host file, as in production, so the handle gets
        // a private data directory holding one. It carries the host id the
        // memory writer settles as, so a claim and an identity read agree.
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
        let mut db = Self::unattached()
            .with_shared_store(ports)
            .with_host_file(host_dir.path());
        db.memory_board_reads = Some(board_reads);
        db._memory_host_dir = Some(host_dir);
        Ok(db)
    }

    /// The board's read seam over this handle's own in-memory rows, or `None`
    /// for a handle with no store attached.
    #[cfg(any(test, feature = "test-support"))]
    pub fn board_reads(&self) -> Option<Arc<dyn crate::sync::BoardReads>> {
        self.memory_board_reads.clone()
    }

    /// Ports over a fresh, private in-process store: one `SharedRows` and one
    /// `MemoryReducerCaller` over it, with every reader and the writer over
    /// those same rows. The writer settles as a fixed test user and host.
    #[cfg(any(test, feature = "test-support"))]
    fn memory_store_ports() -> (SharedStorePorts, Arc<dyn crate::sync::BoardReads>) {
        use crate::sync as s;
        let rows = Arc::new(s::SharedRows::new());
        let clock: Arc<dyn crate::service::Clock> = Arc::new(crate::service::SystemClock);
        let caller: Arc<dyn s::ReducerCaller> = Arc::new(
            s::memory_caller::MemoryReducerCaller::new(rows.clone(), clock.clone()),
        );
        let identity = Arc::new(s::SettledIdentity::default());
        identity.settle("test-user");
        let board_reads = Arc::new(s::SubscriptionBoardReads::new(rows.clone()));
        let ports = SharedStorePorts {
            writer: Arc::new(s::ReducerWriter::new(
                caller,
                identity,
                clock,
                MEMORY_HOST_ID.to_string(),
                board_reads.clone(),
            )),
            reader: board_reads.clone(),
            learning_reader: Arc::new(s::SubscriptionLearningReads::new(rows.clone())),
            usage_reader: Arc::new(s::SubscriptionUsageReads::new(rows.clone())),
            retired_feed_item_reader: Arc::new(s::SubscriptionRetiredFeedItemReads::new(rows)),
        };
        (ports, board_reads)
    }
}
