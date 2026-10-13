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
//! [`super::rows::SharedRows`] a real board's `Store` reads from —
//! `ReducerCallReachesSharedRows`. That is what lets every downstream read
//! (the store traits, `decode`, the board itself) run unchanged against this
//! store: nothing downstream of `SharedRows` can tell the difference between a
//! row that arrived over a subscription and one this caller just wrote.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::clock::Clock;
use crate::models::{
    DispatchRequestId, EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId,
    RetrievalSource, SessionRequestId, SubStatus, TaskId, WorkPackageId, WorkPackageItem,
};
use crate::spacetime::bindings;

use super::rows::SharedRows;
use super::writes::{DrainReadBack, ReducerCaller, ReducerOutcome};

mod agent_state;
mod config;
mod convert;
mod dispatch_requests;
mod feed;
mod grouping;
mod learnings;
mod pr_status;
mod session_requests;
mod tasks_epics;
mod work_packages;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// The store's own vocabulary
// ---------------------------------------------------------------------------
//
// Mirrors of the module's own private `DONE` string constant and friends
// (spacetime/module/src/lib.rs). Not reusable directly — they are private to
// that crate — so they are restated here, character for character, rather
// than invented independently. There is no `archived` any more — task #4971
// removed the status from the module, and this store must not resurrect it.

const DONE: &str = "done";
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

/// The timestamp format the store writes.
use super::encode::stamp;

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
    work_packages: BTreeMap<i64, module::WorkPackage>,
    /// Keyed by the module's derived link id (`module::work_package_link_id`).
    work_package_links: BTreeMap<String, module::WorkPackageLink>,
    /// store-split M4. The module's timer table has no counterpart: the
    /// backstop is [`MemoryReducerCaller::expire_dispatch_request`].
    dispatch_requests: BTreeMap<i64, module::DispatchRequest>,
    /// store-split M5. As above, the module's timer table has no counterpart:
    /// the backstop is [`MemoryReducerCaller::expire_session_request`].
    session_requests: BTreeMap<i64, module::SessionRequest>,
    next_task_id: i64,
    next_epic_id: i64,
    next_repo_path_id: i64,
    next_repo_base_branch_id: i64,
    next_usage_event_id: i64,
    next_poll_owner_id: i64,
    next_retired_feed_item_id: i64,
    next_learning_id: i64,
    next_learning_retrieval_id: i64,
    next_work_package_id: i64,
    next_dispatch_request_id: i64,
    next_session_request_id: i64,
}

impl Tables {
    fn new() -> Self {
        Self {
            next_task_id: 1,
            next_epic_id: 1,
            next_repo_path_id: 1,
            next_repo_base_branch_id: 1,
            next_usage_event_id: 1,
            next_poll_owner_id: 1,
            next_retired_feed_item_id: 1,
            next_learning_id: 1,
            next_learning_retrieval_id: 1,
            next_work_package_id: 1,
            next_dispatch_request_id: 1,
            next_session_request_id: 1,
            ..Self::default()
        }
    }
}

/// A transport failure a test makes the next call to one reducer answer
/// with ([`MemoryReducerCaller::fail_next`]): the two ways a real call can
/// end in `Err` without the module having refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectedFault {
    /// The call never reached the module: nothing was applied.
    Unsent,
    /// The module applied the call, but its reply was lost on the way back.
    ReplyLost,
}

/// The native in-process stand-in for a real SpacetimeDB reducer call.
pub struct MemoryReducerCaller {
    tables: Mutex<Tables>,
    rows: Arc<SharedRows>,
    clock: Arc<dyn Clock>,
    /// The identity a real reducer reads off `ctx.sender`. Stamped as
    /// `created_by` on the sub-epics a routed write creates. Empty unless set.
    sender: String,
    /// Faults queued by [`Self::fail_next`], each consumed by the next call
    /// to the reducer it names.
    faults: Mutex<Vec<(&'static str, InjectedFault)>>,
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
            sender: String::new(),
            faults: Mutex::new(Vec::new()),
        }
    }

    /// Make the next call to `reducer` end in a transport error, `fault`
    /// saying whether the module applied it first. Only the request reducers a
    /// host calls honour it (`claim_dispatch_request`,
    /// `report_dispatch_provisioned`, `report_dispatch_failed`,
    /// `claim_session_request`, `report_session_request_done`,
    /// `report_session_request_failed`, `request_exit_session`,
    /// `report_pr_status`, `report_pr_unreadable`); queue it twice
    /// to fail two calls.
    pub fn fail_next(&self, reducer: &'static str, fault: InjectedFault) {
        self.faults
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((reducer, fault));
    }

    /// Run `apply` as `reducer` would, unless a fault is queued for it.
    fn faulted(
        &self,
        reducer: &'static str,
        apply: impl FnOnce() -> Result<ReducerOutcome>,
    ) -> Result<ReducerOutcome> {
        let fault = {
            let mut faults = self.faults.lock().unwrap_or_else(|e| e.into_inner());
            faults
                .iter()
                .position(|(r, _)| *r == reducer)
                .map(|i| faults.remove(i).1)
        };
        match fault {
            None => apply(),
            Some(InjectedFault::Unsent) => {
                anyhow::bail!("injected transport failure: {reducer} was never sent")
            }
            Some(InjectedFault::ReplyLost) => {
                let _ = apply();
                anyhow::bail!("injected transport failure: the reply to {reducer} was lost")
            }
        }
    }

    /// Name the identity this caller writes as, where a reducer would read
    /// `ctx.sender` — the only place the real module learns who is asking.
    pub fn with_sender(mut self, sender: impl Into<String>) -> Self {
        self.sender = sender.into();
        self
    }

    /// Register `id` in the host registry, owned by `owner` — a fixture's
    /// way to give a host the owner the D11 checks compare its calls
    /// against (`store-split.allium`). Pair it with [`Self::with_sender`].
    pub fn with_registered_host(self, id: impl Into<String>, owner: impl Into<String>) -> Self {
        let id = id.into();
        let row = module::Host {
            id: id.clone(),
            label: String::new(),
            owner: owner.into(),
        };
        self.lock().hosts.insert(id, row.clone());
        self.rows.upsert_host(&row.into());
        self
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

    async fn regroup_epic(&self, root: EpicId) -> Result<ReducerOutcome> {
        self.apply_regroup_epic(root)
    }

    async fn flatten_epic(&self, root: EpicId) -> Result<ReducerOutcome> {
        self.apply_flatten_epic(root)
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

    async fn ingest_feed(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedIngestItem>,
        additive: bool,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        self.apply_ingest_feed(epic_id, items, additive, created_by)
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

    async fn create_work_package(&self, row: bindings::WorkPackage) -> Result<WorkPackageId> {
        self.apply_create_work_package(row)
    }

    async fn patch_work_package(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
    ) -> Result<ReducerOutcome> {
        self.apply_patch_work_package(id, patch)
    }

    async fn delete_work_package(&self, id: WorkPackageId) -> Result<ReducerOutcome> {
        self.apply_delete_work_package(id)
    }

    async fn link_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome> {
        self.apply_link_work_package_item(id, item)
    }

    async fn unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> Result<ReducerOutcome> {
        self.apply_unlink_work_package_item(id, item)
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

    async fn request_dispatch(
        &self,
        task: TaskId,
        requester: String,
        origin: String,
        mode: String,
    ) -> Result<ReducerOutcome> {
        self.apply_request_dispatch(task, requester, origin, mode)
    }

    async fn claim_dispatch_request(
        &self,
        id: DispatchRequestId,
        host: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("claim_dispatch_request", || {
            self.apply_claim_dispatch_request(id, host)
        })
    }

    async fn report_dispatch_provisioned(
        &self,
        id: DispatchRequestId,
        reporter: String,
        worktree: String,
        tmux_window: String,
        reused_worktree: bool,
    ) -> Result<ReducerOutcome> {
        self.faulted("report_dispatch_provisioned", || {
            self.apply_report_dispatch_provisioned(
                id,
                reporter,
                worktree,
                tmux_window,
                reused_worktree,
            )
        })
    }

    async fn report_dispatch_failed(
        &self,
        id: DispatchRequestId,
        reporter: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("report_dispatch_failed", || {
            self.apply_report_dispatch_failed(id, reporter, reason)
        })
    }

    async fn fail_interrupted_dispatch_requests(&self, host: String) -> Result<ReducerOutcome> {
        self.apply_fail_interrupted_dispatch_requests(host)
    }

    async fn request_wrap_up(
        &self,
        task: TaskId,
        host: String,
        action: String,
    ) -> Result<ReducerOutcome> {
        self.apply_request_wrap_up(task, host, action)
    }

    async fn claim_session_request(
        &self,
        id: SessionRequestId,
        host: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("claim_session_request", || {
            self.apply_claim_session_request(id, host)
        })
    }

    async fn report_session_request_done(
        &self,
        id: SessionRequestId,
        reporter: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("report_session_request_done", || {
            self.apply_report_session_request_done(id, reporter)
        })
    }

    async fn report_session_request_failed(
        &self,
        id: SessionRequestId,
        reporter: String,
        reason: String,
        conflict: bool,
    ) -> Result<ReducerOutcome> {
        self.faulted("report_session_request_failed", || {
            self.apply_report_session_request_failed(id, reporter, reason, conflict)
        })
    }

    async fn report_pr_status(
        &self,
        task: TaskId,
        host: String,
        state: String,
        review_decision: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("report_pr_status", || {
            self.apply_report_pr_status(task, host, state, review_decision)
        })
    }

    async fn report_pr_unreadable(
        &self,
        task: TaskId,
        host: String,
        reason: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("report_pr_unreadable", || {
            self.apply_report_pr_unreadable(task, host, reason)
        })
    }

    async fn request_exit_session(
        &self,
        task: TaskId,
        token: SessionRequestId,
        action: String,
        pr_url: String,
    ) -> Result<ReducerOutcome> {
        self.faulted("request_exit_session", || {
            self.apply_request_exit_session(task, token, action, pr_url)
        })
    }

    async fn fail_interrupted_session_requests(&self, host: String) -> Result<ReducerOutcome> {
        self.apply_fail_interrupted_session_requests(host)
    }
}
