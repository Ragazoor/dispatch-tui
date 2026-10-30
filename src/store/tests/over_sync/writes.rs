//! Shared mutations as reducer calls.
//!
//! Spec: `docs/specs/sync.allium`'s three write rules.
//!
//! These exercise [`Store`] against a recording [`ReducerCaller`]. That
//! is the whole testable surface without a server: what the writer sends, what
//! it does with a refusal, and where the timestamps and the identity come from.
//! The transport itself — and the reducers on the far side — are
//! `tests/spacetime_module.rs`, against a live instance.

use crate::clock::{Clock, FixedClock};
use std::sync::{Arc, Mutex};

use crate::models::{
    EpicId, LearningId, NotificationWrite, PollScopeId, RetrievalSource, StopOutcome, SubStatus,
    SubagentDrain, TaskId, TaskStatus, UserPromptOutcome, WorkPackageId, WorkPackageItem,
};

use crate::spacetime::bindings;
use crate::store::{
    CreateTaskRequest, EpicCrud, LearningRetrievalStore, LearningStore, PollOwnershipStore,
    RepoConfigStore, SettingsStore, Store, SubscriptionStore, TaskCrud, TaskPatch, UsageStore,
    WorkPackageCrud,
};

use crate::sync::encode;
use crate::sync::writes::{
    push_host_registration, DrainReadBack, ReducerCaller, ReducerOutcome, WriterIdentity,
};

/// A data directory with no host file in it. The writes under test never read
/// this install's identity, so the store is given nowhere to read it from.
const NO_HOST_FILE: &str = "/nonexistent-test-path/dispatch-data";

use chrono::{DateTime, Utc};

/// A settled identity, without a store behind it.
struct FixedIdentity(Option<String>);

#[async_trait::async_trait]
impl WriterIdentity for FixedIdentity {
    async fn user(&self) -> anyhow::Result<Option<String>> {
        Ok(self.0.clone())
    }
}

/// What the caller was asked to do.
#[derive(Debug, Clone, PartialEq)]
enum Sent {
    CreateTask(Box<bindings::Task>),
    CreateTaskSignalled(Box<bindings::Task>),
    MarkEpicSeen(String, EpicId),
    SetDisplayName(String),
    /// Any of the workspace and super-admin reducers, by name.
    Workspace(&'static str),
    PatchTask(TaskId, Box<bindings::TaskPatch>),
    UpdateTask(TaskId, Option<i64>, String, Box<bindings::TaskPatch>),
    DeleteTask(TaskId),
    SetTaskEpic(TaskId, i64, String),
    CreateEpic(Box<bindings::Epic>),
    PatchEpic(i64),
    DeleteEpic(i64),
    BatchDelete(Vec<i64>, Vec<i64>),
    Regroup(i64),
    Flatten(i64),
    SaveRepoPath(String, String, String),
    DeleteRepoPath(String),
    SetVerifyCommand(String, String),
    RecordBaseBranch(String, String, String),
    Subscribe(String, i64),
    Unsubscribe(String, i64),
    SubagentStart(i64, String, String, String),
    SubagentStop(i64, String, String),
    SubagentClear(i64),
    SubagentClearAndVoidPendingStop(i64),
    TryRecordStop(i64, String),
    RecordPreToolUse(i64, String, String),
    RecordNotification(i64, String, String),
    RecordUserPromptSubmit(i64, String, String),
    MarkPrLearningsGateShown(i64, String),
    IngestFeed(i64, usize, bool, String),
    ClaimPollOwner(String, i64, String),
    OverridePollOwner(String, i64, String),
    BatchPatchSubStatus(usize),
    RespawnPhoenixSuccessor(i64, Box<bindings::Task>),
    RegisterHost(String, String, String),
    RequestDispatch(TaskId, String, String, String),
    ClaimDispatchRequest(i64, String),
    ReportDispatchProvisioned(i64, String, String, String, bool),
    ReportDispatchFailed(i64, String, String),
    FailInterruptedDispatchRequests(String),
    RequestWrapUp(TaskId, String, String),
    ClaimSessionRequest(i64, String),
    ReportSessionRequestDone(i64, String),
    ReportSessionRequestFailed(i64, String, String, bool),
    ReportPrStatus(i64, String, String, String),
    ReportPrUnreadable(i64, String, String),
    RequestExitSession(TaskId, i64, String, String),
    CloseTask(TaskId, String),
    EnsureManagedEpics(String, i64, String, i64),
    FailInterruptedSessionRequests(String),
    SaveSetting(String, String, String),
    ClearSetting(String, String),
    CreateLearning(Box<bindings::Learning>),
    PatchLearning(i64, Box<bindings::LearningPatch>),
    DeleteLearning(i64),
    RescopeEpicLearnings(i64, i64),
    RecordLearningRetrieval(i64, i64, String),
    ApplyLearningVerdicts(usize),
    ArchiveStaleLearnings(String),
    RecordUsageEvent(Box<bindings::UsageEvent>, i64),
    CreateWorkPackage(Box<bindings::WorkPackage>),
    CreateWorkPackageWithLinks(Box<bindings::WorkPackage>, Vec<(String, i64)>),
    PatchWorkPackage(i64, Box<bindings::WorkPackagePatch>),
    UpdateWorkPackageWithLinks {
        id: i64,
        patch: Box<bindings::WorkPackagePatch>,
        unlink: Vec<(String, i64)>,
        link: Vec<(String, i64)>,
    },
    DeleteWorkPackage(i64),
    LinkWorkPackageItem(i64, String, i64),
    UnlinkWorkPackageItem(i64, String, i64),
}

fn kind_ids(items: &[WorkPackageItem]) -> Vec<(String, i64)> {
    items
        .iter()
        .map(|i| (i.kind().to_string(), i.raw_id()))
        .collect()
}

#[derive(Default)]
struct RecordingCaller {
    sent: Mutex<Vec<Sent>>,
    /// The TRANSPORT failing — the store is unreachable. Distinct from the
    /// store refusing, below: one is an error and the other is an answer.
    refuses_with: Option<String>,
    /// The store answering "no". A claim reads this as "somebody else won";
    /// everything else reads it as an error.
    rejects: bool,
    /// The `Applied` payload `answer` hands back on success, for the methods
    /// still on `ReducerOutcome` (every pre-Phase-6b reducer, plus the four
    /// agent-session-state ones with no read-back ambiguity to type away).
    /// Empty for every test that never looks at it.
    returning: Vec<i64>,
    /// What `subagent_start` answers with.
    live_count: i64,
    /// What `subagent_stop`/`subagent_clear` answer with.
    drain: DrainReadBack,
    /// What `try_record_stop` answers with on success; `None` combined with
    /// `rejects`/`refuses_with` unset is itself a valid "deferred" answer, so
    /// `rejecting()` — not this field — is how a test asks for the refusal.
    stop_flag: bool,
    /// The subscription view a feed-upsert/stale-delete call should mutate as
    /// a side effect, simulating what a live reducer's delete does to the
    /// subscription cache — only set by the predict-then-verify tests, which
    /// need `Store`'s post-call verify-read to see a row actually
    /// gone. `None` elsewhere: every other test's mock call has no such
    /// effect.
    rows: Option<Arc<crate::sync::SharedRows>>,
    /// Task ids [`Self::rows`] removes when a feed write is answered.
    removes_on_feed_write: Vec<i64>,
}

impl RecordingCaller {
    fn refusing(why: &str) -> Self {
        Self {
            refuses_with: Some(why.to_string()),
            ..Self::default()
        }
    }

    /// A reachable store that says no.
    fn rejecting() -> Self {
        Self {
            rejects: true,
            ..Self::default()
        }
    }

    /// A reachable store that applies and answers with `ids` — the
    /// `ReducerOutcome`-based read-back payload.
    fn returning(ids: &[i64]) -> Self {
        Self {
            returning: ids.to_vec(),
            ..Self::default()
        }
    }

    /// A reachable store that applies `subagent_start` and answers with `n`
    /// as the live count.
    fn returning_count(n: i64) -> Self {
        Self {
            live_count: n,
            ..Self::default()
        }
    }

    /// A reachable store that applies a drain (`subagent_stop`/
    /// `subagent_clear`) and answers with this read-back.
    fn returning_drain(live: i64, is_review: bool) -> Self {
        Self {
            drain: DrainReadBack { live, is_review },
            ..Self::default()
        }
    }

    /// A reachable store that applies `try_record_stop` and answers
    /// `Some(flipped)`.
    fn returning_stop_flag(flipped: bool) -> Self {
        Self {
            stop_flag: flipped,
            ..Self::default()
        }
    }

    /// A reachable store whose feed-upsert/stale-delete calls apply AND
    /// remove `ids` from `rows` as a side effect — simulating what the real
    /// reducer's delete does to the subscription cache, so a test can assert
    /// on `Store`'s post-call verify-read.
    fn removing_on_feed_write(rows: Arc<crate::sync::SharedRows>, ids: &[i64]) -> Self {
        Self {
            rows: Some(rows),
            removes_on_feed_write: ids.to_vec(),
            ..Self::default()
        }
    }

    /// Apply the side effect [`Self::removing_on_feed_write`] configured, if
    /// any. Called from every feed-upsert/stale-delete mock method after a
    /// successful `answer`.
    fn simulate_feed_removal(&self) {
        if let Some(rows) = &self.rows {
            for id in &self.removes_on_feed_write {
                rows.remove_task(TaskId(*id));
            }
        }
    }

    fn answer(&self, what: Sent) -> anyhow::Result<ReducerOutcome> {
        if let Some(why) = &self.refuses_with {
            anyhow::bail!("{why}");
        }
        if self.rejects {
            return Ok(ReducerOutcome::Refused("no".into()));
        }
        self.sent.lock().unwrap().push(what);
        Ok(ReducerOutcome::Applied(self.returning.clone()))
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    fn record(&self, what: Sent) -> anyhow::Result<()> {
        if let Some(why) = &self.refuses_with {
            anyhow::bail!("{why}");
        }
        self.sent.lock().unwrap().push(what);
        Ok(())
    }
}

#[async_trait::async_trait]
impl ReducerCaller for RecordingCaller {
    async fn create_task(&self, row: bindings::Task) -> anyhow::Result<TaskId> {
        self.record(Sent::CreateTask(Box::new(row)))?;
        Ok(TaskId(42))
    }

    async fn create_task_signalled(&self, row: bindings::Task) -> anyhow::Result<TaskId> {
        self.record(Sent::CreateTaskSignalled(Box::new(row)))?;
        Ok(TaskId(42))
    }

    async fn set_display_name(&self, name: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SetDisplayName(name))
    }

    async fn claim_super_admin(&self) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("claim_super_admin"))
    }

    async fn grant_super_admin(&self, _user: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("grant_super_admin"))
    }

    async fn revoke_super_admin(&self, _user: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("revoke_super_admin"))
    }

    async fn set_epic_visibility(
        &self,
        _id: crate::models::EpicId,
        _visibility: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("set_epic_visibility"))
    }

    async fn move_epic_to_workspace(
        &self,
        _id: crate::models::EpicId,
        _workspace_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("move_epic_to_workspace"))
    }

    async fn set_work_package_visibility(
        &self,
        _id: crate::models::WorkPackageId,
        _visibility: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("set_work_package_visibility"))
    }

    async fn super_admin_delete_unseen_item(
        &self,
        _kind: String,
        _item_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("super_admin_delete_unseen_item"))
    }

    async fn create_workspace(&self, _name: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("create_workspace"))
    }

    async fn rename_workspace(&self, _id: i64, _name: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("rename_workspace"))
    }

    async fn delete_workspace(&self, _id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("delete_workspace"))
    }

    async fn add_member(
        &self,
        _id: i64,
        _user: String,
        _role: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("add_member"))
    }

    async fn set_member_role(&self, _id: i64, _role: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("set_member_role"))
    }

    async fn remove_member(&self, _id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Workspace("remove_member"))
    }

    async fn mark_epic_seen(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::MarkEpicSeen(subscriber, epic_id))
    }

    async fn patch_task(
        &self,
        id: TaskId,
        patch: bindings::TaskPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::PatchTask(id, Box::new(patch)))
    }

    async fn update_task(
        &self,
        id: TaskId,
        epic_id: Option<Option<EpicId>>,
        owner: String,
        patch: bindings::TaskPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        let epic_id = epic_id.map(encode::epic_ref);
        self.answer(Sent::UpdateTask(id, epic_id, owner, Box::new(patch)))
    }

    async fn delete_task(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteTask(id))
    }

    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: Option<EpicId>,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let epic_id = encode::epic_ref(epic_id);
        self.answer(Sent::SetTaskEpic(id, epic_id, owner))
    }

    async fn create_epic(&self, row: bindings::Epic) -> anyhow::Result<EpicId> {
        self.record(Sent::CreateEpic(Box::new(row)))?;
        Ok(EpicId(7))
    }

    async fn patch_epic(
        &self,
        id: EpicId,
        _patch: bindings::EpicPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        self.answer(Sent::PatchEpic(id))
    }

    async fn delete_epic(&self, id: EpicId) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        self.answer(Sent::DeleteEpic(id))
    }

    async fn batch_delete(
        &self,
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    ) -> anyhow::Result<ReducerOutcome> {
        let task_ids: Vec<i64> = task_ids.into_iter().map(|id| id.0).collect();
        let epic_ids: Vec<i64> = epic_ids.into_iter().map(|id| id.0).collect();
        self.answer(Sent::BatchDelete(task_ids, epic_ids))
    }

    async fn regroup_epic(&self, root: EpicId) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Regroup(root.0))
    }

    async fn flatten_epic(&self, root: EpicId) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Flatten(root.0))
    }

    async fn save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let last_used = encode::stamp(last_used);
        self.answer(Sent::SaveRepoPath(path, last_used, host))
    }

    async fn delete_repo_path(&self, path: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteRepoPath(path))
    }

    async fn set_verify_command(
        &self,
        path: String,
        command: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SetVerifyCommand(path, command))
    }

    async fn record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let last_used = encode::stamp(last_used);
        self.answer(Sent::RecordBaseBranch(repo_path, branch, last_used))
    }

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        self.answer(Sent::Subscribe(subscriber, epic_id))
    }

    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        self.answer(Sent::Unsubscribe(subscriber, epic_id))
    }

    async fn save_setting(
        &self,
        host: String,
        key: String,
        value: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SaveSetting(host, key, value))
    }

    async fn clear_setting(&self, host: String, key: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ClearSetting(host, key))
    }

    async fn create_learning(&self, row: bindings::Learning) -> anyhow::Result<LearningId> {
        self.record(Sent::CreateLearning(Box::new(row)))?;
        Ok(LearningId(99))
    }

    async fn patch_learning(
        &self,
        id: LearningId,
        patch: bindings::LearningPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        self.answer(Sent::PatchLearning(id, Box::new(patch)))
    }

    async fn delete_learning(&self, id: LearningId) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        self.answer(Sent::DeleteLearning(id))
    }

    async fn rescope_epic_learnings(
        &self,
        from: EpicId,
        to: EpicId,
    ) -> anyhow::Result<ReducerOutcome> {
        let from = from.0;
        let to = to.0;
        self.answer(Sent::RescopeEpicLearnings(from, to))
    }

    async fn record_learning_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> anyhow::Result<ReducerOutcome> {
        let task_id = task_id.0;
        let learning_id = learning_id.0;
        let source = source.as_str().to_string();
        self.answer(Sent::RecordLearningRetrieval(task_id, learning_id, source))
    }

    async fn apply_learning_verdicts(
        &self,
        _task_id: crate::models::TaskId,
        verdicts: Vec<(crate::models::LearningId, crate::models::LearningVerdict)>,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ApplyLearningVerdicts(verdicts.len()))
    }

    async fn archive_stale_learnings(
        &self,
        cutoff: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let cutoff = encode::stamp(cutoff);
        self.answer(Sent::ArchiveStaleLearnings(cutoff))
    }

    async fn record_usage_event(
        &self,
        row: bindings::UsageEvent,
        cap: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RecordUsageEvent(Box::new(row), cap))
    }

    async fn subagent_start(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        started_at: DateTime<Utc>,
    ) -> anyhow::Result<i64> {
        let task_id = task_id.0;
        let started_at = encode::subagent_started_at(started_at);
        self.record(Sent::SubagentStart(
            task_id, agent_id, session_id, started_at,
        ))?;
        Ok(self.live_count)
    }

    async fn subagent_stop(
        &self,
        task_id: TaskId,
        agent_id: String,
        session_id: String,
    ) -> anyhow::Result<DrainReadBack> {
        let task_id = task_id.0;
        self.record(Sent::SubagentStop(task_id, agent_id, session_id))?;
        Ok(self.drain)
    }

    async fn subagent_clear(&self, task_id: TaskId) -> anyhow::Result<DrainReadBack> {
        let task_id = task_id.0;
        self.record(Sent::SubagentClear(task_id))?;
        Ok(self.drain)
    }

    async fn subagent_clear_and_void_pending_stop(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<ReducerOutcome> {
        let task_id = task_id.0;
        self.answer(Sent::SubagentClearAndVoidPendingStop(task_id))
    }

    /// `refuses_with` (transport failure) is checked first, same as every
    /// other method. `rejects` (`RecordingCaller::rejecting()`) then answers
    /// `None` — the refusal `try_record_stop`'s `NoOp` reads.
    async fn try_record_stop(
        &self,
        id: TaskId,
        stop_pending_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<bool>> {
        let id = id.0;
        let stop_pending_at = encode::stamp(stop_pending_at);
        if let Some(why) = &self.refuses_with {
            anyhow::bail!("{why}");
        }
        if self.rejects {
            return Ok(None);
        }
        self.sent
            .lock()
            .unwrap()
            .push(Sent::TryRecordStop(id, stop_pending_at));
        Ok(Some(self.stop_flag))
    }

    async fn record_pre_tool_use(
        &self,
        id: TaskId,
        sub_status: SubStatus,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        let sub_status = sub_status.as_str().to_string();
        let at = encode::stamp(at);
        self.answer(Sent::RecordPreToolUse(id, sub_status, at))
    }

    async fn record_notification(
        &self,
        id: TaskId,
        mode: crate::models::NotificationWrite,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        let at = encode::stamp(at);
        let mode = encode::notification_mode(mode)
            .unwrap_or("ignore")
            .to_string();
        self.answer(Sent::RecordNotification(id, mode, at))
    }

    async fn record_user_prompt_submit(
        &self,
        id: TaskId,
        activity_at: DateTime<Utc>,
        prompt_at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        let activity_at = encode::stamp(activity_at);
        let prompt_at = encode::stamp(prompt_at);
        self.answer(Sent::RecordUserPromptSubmit(id, activity_at, prompt_at))
    }

    async fn mark_pr_learnings_gate_shown(
        &self,
        id: TaskId,
        at: DateTime<Utc>,
    ) -> anyhow::Result<ReducerOutcome> {
        let id = id.0;
        let at = encode::stamp(at);
        self.answer(Sent::MarkPrLearningsGateShown(id, at))
    }

    async fn ingest_feed(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedIngestItem>,
        additive: bool,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let outcome = self.answer(Sent::IngestFeed(
            epic_id.0,
            items.len(),
            additive,
            created_by,
        ))?;
        if matches!(outcome, ReducerOutcome::Applied(_)) {
            self.simulate_feed_removal();
        }
        Ok(outcome)
    }

    async fn ensure_managed_epics(
        &self,
        reviews_command: String,
        reviews_interval_secs: i64,
        cve_command: String,
        cve_interval_secs: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::EnsureManagedEpics(
            reviews_command,
            reviews_interval_secs,
            cve_command,
            cve_interval_secs,
        ))
    }

    async fn create_work_package(
        &self,
        row: bindings::WorkPackage,
    ) -> anyhow::Result<WorkPackageId> {
        self.record(Sent::CreateWorkPackage(Box::new(row)))?;
        Ok(WorkPackageId(9))
    }

    async fn create_work_package_with_links(
        &self,
        row: bindings::WorkPackage,
        links: Vec<WorkPackageItem>,
    ) -> anyhow::Result<WorkPackageId> {
        self.record(Sent::CreateWorkPackageWithLinks(
            Box::new(row),
            kind_ids(&links),
        ))?;
        Ok(WorkPackageId(9))
    }

    async fn patch_work_package(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::PatchWorkPackage(id.0, Box::new(patch)))
    }

    async fn update_work_package_with_links(
        &self,
        id: WorkPackageId,
        patch: bindings::WorkPackagePatch,
        unlink: Vec<WorkPackageItem>,
        link: Vec<WorkPackageItem>,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::UpdateWorkPackageWithLinks {
            id: id.0,
            patch: Box::new(patch),
            unlink: kind_ids(&unlink),
            link: kind_ids(&link),
        })
    }

    async fn delete_work_package(&self, id: WorkPackageId) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteWorkPackage(id.0))
    }

    async fn link_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::LinkWorkPackageItem(
            id.0,
            item.kind().to_string(),
            item.raw_id(),
        ))
    }

    async fn unlink_work_package_item(
        &self,
        id: WorkPackageId,
        item: WorkPackageItem,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::UnlinkWorkPackageItem(
            id.0,
            item.kind().to_string(),
            item.raw_id(),
        ))
    }

    async fn claim_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        let scope = scope.to_string();
        self.answer(Sent::ClaimPollOwner(scope, scope_id, host))
    }

    async fn override_poll_owner(
        &self,
        target: PollScopeId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let (scope, scope_id) = target.wire();
        let scope = scope.to_string();
        self.answer(Sent::OverridePollOwner(scope, scope_id, host))
    }

    async fn batch_patch_sub_status(
        &self,
        updates: Vec<(TaskId, SubStatus)>,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::BatchPatchSubStatus(updates.len()))
    }

    async fn respawn_phoenix_successor(
        &self,
        predecessor: TaskId,
        successor: bindings::Task,
    ) -> anyhow::Result<TaskId> {
        let predecessor = predecessor.0;
        self.record(Sent::RespawnPhoenixSuccessor(
            predecessor,
            Box::new(successor),
        ))?;
        Ok(TaskId(43))
    }

    async fn register_host(
        &self,
        id: String,
        label: String,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RegisterHost(id, label, owner))
    }

    async fn request_dispatch(
        &self,
        task: TaskId,
        requester: String,
        origin: String,
        mode: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RequestDispatch(task, requester, origin, mode))
    }

    async fn claim_dispatch_request(
        &self,
        id: crate::models::DispatchRequestId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ClaimDispatchRequest(id.0, host))
    }

    async fn report_dispatch_provisioned(
        &self,
        id: crate::models::DispatchRequestId,
        reporter: String,
        worktree: String,
        tmux_window: String,
        reused_worktree: bool,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ReportDispatchProvisioned(
            id.0,
            reporter,
            worktree,
            tmux_window,
            reused_worktree,
        ))
    }

    async fn report_dispatch_failed(
        &self,
        id: crate::models::DispatchRequestId,
        reporter: String,
        reason: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ReportDispatchFailed(id.0, reporter, reason))
    }

    async fn fail_interrupted_dispatch_requests(
        &self,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::FailInterruptedDispatchRequests(host))
    }

    async fn request_wrap_up(
        &self,
        task: TaskId,
        host: String,
        action: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RequestWrapUp(task, host, action))
    }

    async fn claim_session_request(
        &self,
        id: crate::models::SessionRequestId,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ClaimSessionRequest(id.0, host))
    }

    async fn report_session_request_done(
        &self,
        id: crate::models::SessionRequestId,
        reporter: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ReportSessionRequestDone(id.0, reporter))
    }

    async fn report_session_request_failed(
        &self,
        id: crate::models::SessionRequestId,
        reporter: String,
        reason: String,
        conflict: bool,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ReportSessionRequestFailed(
            id.0, reporter, reason, conflict,
        ))
    }

    async fn report_pr_status(
        &self,
        task: TaskId,
        host: String,
        state: String,
        review_decision: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ReportPrStatus(task.0, host, state, review_decision))
    }

    async fn report_pr_unreadable(
        &self,
        task: TaskId,
        host: String,
        reason: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ReportPrUnreadable(task.0, host, reason))
    }

    async fn request_exit_session(
        &self,
        task: TaskId,
        token: crate::models::SessionRequestId,
        action: String,
        pr_url: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RequestExitSession(task, token.0, action, pr_url))
    }

    async fn close_task(&self, task: TaskId, host: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::CloseTask(task, host))
    }

    async fn fail_interrupted_session_requests(
        &self,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::FailInterruptedSessionRequests(host))
    }
}

/// The instant the fixed clock sits at, and the string it must produce.
const AT: &str = "2026-09-19T12:34:56.789Z";

const AT_STORED: &str = "2026-09-19 12:34:56.789";

/// A store over `rows`, writing through `caller` as `identity` with the
/// clock fixed at [`AT`]. Every writer fixture below is this with a default
/// filled in.
fn store_over(
    rows: Arc<crate::sync::SharedRows>,
    identity: Option<&str>,
    caller: RecordingCaller,
) -> (Store, Arc<RecordingCaller>) {
    let caller = Arc::new(caller);
    let clock = FixedClock::new(
        chrono::DateTime::parse_from_rfc3339(AT)
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    let writer = Store::new(
        rows,
        caller.clone(),
        Arc::new(FixedIdentity(identity.map(str::to_string))),
        Arc::new(clock) as Arc<dyn Clock>,
        "host-me".into(),
        std::path::Path::new(NO_HOST_FILE),
    );
    (writer, caller)
}

fn writer_with(caller: RecordingCaller) -> (Store, Arc<RecordingCaller>) {
    store_over(
        Arc::new(crate::sync::SharedRows::new()),
        Some("user-me"),
        caller,
    )
}

/// The twin of [`writer_with`], for the "nothing has settled yet" tests.
fn writer_with_no_identity(caller: RecordingCaller) -> (Store, Arc<RecordingCaller>) {
    store_over(Arc::new(crate::sync::SharedRows::new()), None, caller)
}

/// A writer over a seeded subscription view, for the candidate loop.
fn writer_over(
    rows: Arc<crate::sync::SharedRows>,
    caller: RecordingCaller,
) -> (Store, Arc<RecordingCaller>) {
    store_over(rows, Some("user-me"), caller)
}

fn a_request() -> CreateTaskRequest<'static> {
    CreateTaskRequest {
        description: "d",
        ..CreateTaskRequest::fixture("t", "/repo")
    }
}

// -- Agent session state (Phase 6b) ------------------------------------------
//
// `writer_with` seeds no rows, so `prior_running`/`prior_review`'s pre-read
// always answers `None` there — fine for the methods that don't need it
// (subagent_start, and the plain applied()/won() wrappers).
// The drain and resume/refresh tests need `writer_over` with a seeded row
// instead, exactly as the claim tests already do for the same reason.

fn seeded_row(id: i64, status: TaskStatus) -> bindings::Task {
    let mut row =
        crate::sync::encode::create_task_row(&a_request(), "user-me", "user-me", AT_STORED);
    row.id = id;
    row.status = status.as_str().to_string();
    row
}

fn writer_seeded_with(
    id: i64,
    status: TaskStatus,
    caller: RecordingCaller,
) -> (Store, Arc<RecordingCaller>) {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&seeded_row(id, status));
    writer_over(rows, caller)
}

/// The instant the fixed clock in `writer_with`/`writer_seeded_with` sits at.
fn at() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(AT)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

mod creates;
mod host_and_settings;
mod requests;
mod session_hooks;
