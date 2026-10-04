//! A native, in-process stand-in for a real SpacetimeDB reducer call.
//!
//! Spec: `docs/specs/spacetime-memory-store.allium`.
//!
//! [`MemoryReducerCaller`] implements [`super::writes::ReducerCaller`] the same
//! way [`super::sdk_connector::SdkReducerCaller`] does, but against a plain
//! in-process `Mutex`-guarded table set rather than a real server. It never
//! runs inside SpacetimeDB and never touches a `ReducerContext` — it is an
//! independent reimplementation of the module's reducers, not the module code
//! relocated, and `spacetime-memory-store.allium`'s `ReducerConformance`
//! contract is what obligates the two to agree.
//!
//! # What is reused, and what is not
//!
//! The module's pure, ctx-free helpers — [`module::derive_epic_status`],
//! [`module::stamps_completion`], [`module::apply_task_patch`],
//! [`module::apply_epic_patch`], [`module::validate_task_ownership`],
//! [`module::subscription_id`], [`module::claimable_by`] — are called directly
//! rather than re-derived here, per `ReducerConformance`'s `@guidance`.
//! Everything else a reducer does — row storage, lookup, delete
//! cascades, id generation, orchestration order — has no such shared source
//! and is this file's own reimplementation of what
//! `spacetime/module/src/lib.rs` does with a `ReducerContext` in hand.
//!
//! # After a call: pushed into `SharedRows`, not held separately
//!
//! Every mutation below pushes its resulting rows into the same
//! [`super::rows::SharedRows`] a real board's `SubscriptionBoardReads` wraps —
//! `ReducerCallReachesSharedRows`. That is what lets every downstream read
//! (`SharedReader`, `decode`, the board itself) run unchanged against this
//! store: nothing downstream of `SharedRows` can tell the difference between a
//! row that arrived over a subscription and one this caller just wrote.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::models::{
    EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId, RetrievalSource,
    SubStatus, TaskId,
};
use crate::service::Clock;
use crate::spacetime::bindings;

use super::rows::SharedRows;
use super::writes::{DrainReadBack, ReducerCaller, ReducerOutcome};

mod agent_state;
mod config;
mod convert;
mod feed;
mod learnings;
mod tasks_epics;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

// ---------------------------------------------------------------------------
// The store's own vocabulary
// ---------------------------------------------------------------------------
//
// Mirrors of the module's own private `DONE`/`BACKLOG` string constants
// (spacetime/module/src/lib.rs). Not reusable directly — they are private to
// that crate — so they are restated here, character for character, rather
// than invented independently. There is no `archived` any more — task #4971
// removed the status from the module, and this store must not resurrect it.

const DONE: &str = "done";
const BACKLOG: &str = "backlog";
const RUNNING: &str = "running";
const REVIEW: &str = "review";
const ACTIVE: &str = "active";
const AWAITING_REVIEW: &str = "awaiting_review";
const NEEDS_INPUT: &str = "needs_input";
const POLL_SCOPE_EPIC: &str = "epic";

/// Mirrors the module's own `MAX_EPIC_DEPTH`: a corrupt `parent_epic_id` cycle
/// is unreachable through any writer here, so this is a stack-overflow guard
/// rather than a correctness mechanism, on the same reasoning as the module's
/// copy.
const MAX_EPIC_DEPTH: usize = 64;

/// The timestamp format both stores write — see `super::encode::stamp`'s own
/// doc comment for why this is restated rather than imported.
fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    super::encode::stamp(at)
}

// ---------------------------------------------------------------------------
// Native row storage
// ---------------------------------------------------------------------------

/// The native tables this caller covers, plus the next id each `#[auto_inc]`
/// column would hand out. Ids start at 1, matching SpacetimeDB's own
/// convention that 0 means "generate one" (see the module's header, "Absence
/// is a sentinel, not a null").
#[derive(Default)]
struct Tables {
    tasks: BTreeMap<i64, module::Task>,
    epics: BTreeMap<i64, module::Epic>,
    repo_paths: BTreeMap<i64, module::RepoPath>,
    repo_base_branches: BTreeMap<i64, module::RepoBaseBranch>,
    subscriptions: BTreeMap<String, module::Subscription>,
    settings: BTreeMap<String, module::Setting>,
    usage_events: BTreeMap<i64, module::UsageEvent>,
    task_watchers: BTreeMap<i64, module::TaskWatcher>,
    poll_owners: BTreeMap<i64, module::PollOwner>,
    hosts: BTreeMap<String, module::Host>,
    retired_feed_items: BTreeMap<i64, module::RetiredFeedItem>,
    learnings: BTreeMap<i64, module::Learning>,
    learning_retrievals: BTreeMap<i64, module::LearningRetrieval>,
    /// No primary key on the module's own `task_subagents` table either (see
    /// `TaskSubagent`'s doc comment) — a live set of rows, not entities with
    /// an identity of their own, so a plain `Vec` mirrors it exactly. Nothing
    /// here models `task_shells`: it is a dead table upstream too (no
    /// reducer writes it any more — see the module's `TaskShell` doc
    /// comment), so its cascade is always a no-op and there is nothing to
    /// gain by storing an empty table natively.
    task_subagents: Vec<module::TaskSubagent>,
    next_task_id: i64,
    next_epic_id: i64,
    next_repo_path_id: i64,
    next_repo_base_branch_id: i64,
    next_usage_event_id: i64,
    next_task_watcher_id: i64,
    next_poll_owner_id: i64,
    next_retired_feed_item_id: i64,
    next_learning_id: i64,
    next_learning_retrieval_id: i64,
}

impl Tables {
    fn new() -> Self {
        Self {
            next_task_id: 1,
            next_epic_id: 1,
            next_repo_path_id: 1,
            next_repo_base_branch_id: 1,
            next_usage_event_id: 1,
            next_task_watcher_id: 1,
            next_poll_owner_id: 1,
            next_retired_feed_item_id: 1,
            next_learning_id: 1,
            next_learning_retrieval_id: 1,
            ..Self::default()
        }
    }
}

/// The native in-process stand-in for a real SpacetimeDB reducer call.
pub struct MemoryReducerCaller {
    tables: Mutex<Tables>,
    rows: Arc<SharedRows>,
    clock: Arc<dyn Clock>,
}

impl MemoryReducerCaller {
    /// `rows` is the SAME `SharedRows` a board's reads run against — pushing
    /// into it here is what `ReducerCallReachesSharedRows` means. `clock`
    /// supplies every timestamp a real reducer would take from
    /// `ctx.timestamp`.
    pub fn new(rows: Arc<SharedRows>, clock: Arc<dyn Clock>) -> Self {
        Self {
            tables: Mutex::new(Tables::new()),
            rows,
            clock,
        }
    }

    fn now(&self) -> String {
        stamp(self.clock.now())
    }

    /// A task's raw `owner` column, bypassing `SharedRows`/`crate::models::Task`
    /// (which does not carry this field at all). The one way the conformance
    /// suite can compare `owner` — the field `set_task_epic` mutates natively
    /// on this store — against the real reducer's column, read over SQL there.
    /// `None` if the task does not exist.
    pub fn task_owner(&self, id: i64) -> Option<String> {
        self.lock().tasks.get(&id).map(|t| t.owner.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A row's given id if it brought one, otherwise the next value off
    /// `counter` — mirroring the module's own auto-increment: a nonzero id is
    /// an explicit choice (seeding, a round-trip), and only a zero id asks
    /// this store to generate one.
    fn assign_id(given: i64, counter: &mut i64) -> i64 {
        if given != 0 {
            return given;
        }
        let id = *counter;
        *counter += 1;
        id
    }
}

#[async_trait]
impl ReducerCaller for MemoryReducerCaller {
    async fn create_task(&self, row: bindings::Task) -> Result<TaskId> {
        self.apply_create_task(row)
    }

    async fn patch_task(&self, id: TaskId, patch: bindings::TaskPatch) -> Result<ReducerOutcome> {
        self.apply_patch_task(id, patch)
    }

    async fn delete_task(&self, id: TaskId) -> Result<ReducerOutcome> {
        self.apply_delete_task(id)
    }

    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: Option<EpicId>,
        owner: String,
    ) -> Result<ReducerOutcome> {
        self.apply_set_task_epic(id, epic_id, owner)
    }

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> Result<ReducerOutcome> {
        self.apply_claim_backlog_task(id, host)
    }

    async fn release_backlog_claim(&self, id: TaskId) -> Result<ReducerOutcome> {
        self.apply_release_backlog_claim(id)
    }

    async fn create_epic(&self, row: bindings::Epic) -> Result<EpicId> {
        self.apply_create_epic(row)
    }

    async fn patch_epic(&self, id: EpicId, patch: bindings::EpicPatch) -> Result<ReducerOutcome> {
        self.apply_patch_epic(id, patch)
    }

    async fn delete_epic(&self, id: EpicId) -> Result<ReducerOutcome> {
        self.apply_delete_epic(id)
    }

    async fn recalculate_epic_status(&self, id: EpicId) -> Result<ReducerOutcome> {
        self.apply_recalculate_epic_status(id)
    }

    async fn batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> Result<ReducerOutcome> {
        self.apply_batch_delete(task_ids, epic_ids)
    }

    async fn save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        self.apply_save_repo_path(path, last_used)
    }

    async fn delete_repo_path(&self, path: String) -> Result<ReducerOutcome> {
        self.apply_delete_repo_path(path)
    }

    async fn set_verify_command(&self, path: String, command: String) -> Result<ReducerOutcome> {
        self.apply_set_verify_command(path, command)
    }

    async fn record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        self.apply_record_base_branch(repo_path, branch, last_used)
    }

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome> {
        self.apply_subscribe_to_epic(subscriber, epic_id)
    }

    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome> {
        self.apply_unsubscribe_from_epic(subscriber, epic_id)
    }

    async fn save_setting(
        &self,
        host: String,
        key: String,
        value: String,
    ) -> Result<ReducerOutcome> {
        self.apply_save_setting(host, key, value)
    }

    async fn clear_setting(&self, host: String, key: String) -> Result<ReducerOutcome> {
        self.apply_clear_setting(host, key)
    }

    async fn create_learning(&self, row: bindings::Learning) -> Result<LearningId> {
        self.apply_create_learning(row)
    }

    async fn patch_learning(
        &self,
        id: LearningId,
        patch: bindings::LearningPatch,
    ) -> Result<ReducerOutcome> {
        self.apply_patch_learning(id, patch)
    }

    async fn delete_learning(&self, id: LearningId) -> Result<ReducerOutcome> {
        self.apply_delete_learning(id)
    }

    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<ReducerOutcome> {
        self.apply_rescope_epic_learnings(from, to)
    }

    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<ReducerOutcome> {
        self.apply_record_learning_retrieval(task_id, learning_id, source)
    }

    async fn apply_learning_verdicts(
        &self,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<ReducerOutcome> {
        self.apply_apply_learning_verdicts(verdicts)
    }

    async fn archive_stale_learnings(&self, cutoff: DateTime<Utc>) -> Result<ReducerOutcome> {
        self.apply_archive_stale_learnings(cutoff)
    }

    async fn record_usage_event(
        &self,
        row: bindings::UsageEvent,
        cap: i64,
    ) -> Result<ReducerOutcome> {
        self.apply_record_usage_event(row, cap)
    }

    async fn subagent_start(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> Result<i64> {
        self.apply_subagent_start(task_id, agent_id, session_id, started_at)
    }

    async fn subagent_stop(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
    ) -> Result<DrainReadBack> {
        self.apply_subagent_stop(task_id, agent_id, session_id)
    }

    async fn subagent_clear(&self, task_id: TaskId) -> Result<DrainReadBack> {
        self.apply_subagent_clear(task_id)
    }

    async fn subagent_clear_and_void_pending_stop(
        &self,
        task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        self.apply_subagent_clear_and_void_pending_stop(task_id)
    }

    async fn try_record_stop(
        &self,
        id: TaskId,
        stop_pending_at: DateTime<Utc>,
    ) -> Result<Option<bool>> {
        self.apply_try_record_stop(id, stop_pending_at)
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        self.apply_record_pre_tool_use(id, sub_status, at)
    }

    async fn record_notification(
        &self,
        id: TaskId,
        mode: NotificationWrite,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        self.apply_record_notification(id, mode, at)
    }

    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        activity_at: DateTime<Utc>,
        prompt_at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        self.apply_record_user_prompt_submit(id, activity_at, prompt_at)
    }

    async fn mark_pr_learnings_gate_shown(
        &self,
        id: TaskId,
        at: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        self.apply_mark_pr_learnings_gate_shown(id, at)
    }

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        self.apply_upsert_feed_tasks(epic_id, items, created_by)
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        self.apply_upsert_feed_tasks_additive(epic_id, items, created_by)
    }

    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome> {
        self.apply_delete_stale_subtree_feed_tasks(parent_id, keep_external_ids)
    }

    async fn drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome> {
        self.apply_drop_closed_retired_feed_items(feed_epic_id, keep_external_ids)
    }

    async fn create_repo_group_sub_epic(
        &self,
        parent_id: EpicId,
        title: String,
        created_by: String,
    ) -> Result<EpicId> {
        self.apply_create_repo_group_sub_epic(parent_id, title, created_by)
    }

    async fn create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: Option<EpicId>,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> Result<EpicId> {
        self.apply_create_managed_role_epic(
            title,
            parent_epic_id,
            role,
            feed_command,
            feed_interval_secs,
            created_by,
        )
    }

    async fn create_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        self.apply_create_task_watcher(watcher_task_id, target_task_id)
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: TaskId,
        target_task_id: TaskId,
    ) -> Result<ReducerOutcome> {
        self.apply_delete_task_watcher(watcher_task_id, target_task_id)
    }

    async fn delete_watches_of_target(&self, target_task_id: TaskId) -> Result<ReducerOutcome> {
        self.apply_delete_watches_of_target(target_task_id)
    }

    async fn delete_watches_by_watcher(&self, watcher_task_id: TaskId) -> Result<ReducerOutcome> {
        self.apply_delete_watches_by_watcher(watcher_task_id)
    }

    async fn claim_poll_owner(&self, target: PollScopeId, host: String) -> Result<ReducerOutcome> {
        self.apply_claim_poll_owner(target, host)
    }

    async fn override_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> Result<ReducerOutcome> {
        self.apply_override_poll_owner(target, host)
    }

    async fn batch_patch_sub_status(
        &self,
        updates: Vec<(TaskId, SubStatus)>,
    ) -> Result<ReducerOutcome> {
        self.apply_batch_patch_sub_status(updates)
    }

    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        successor: bindings::Task,
    ) -> Result<TaskId> {
        self.apply_respawn_phoenix_successor(predecessor, successor)
    }

    async fn register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> Result<ReducerOutcome> {
        self.apply_register_host(id, label, owner)
    }
}
