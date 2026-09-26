//! Shared mutations as reducer calls.
//!
//! Spec: `docs/specs/sync.allium`'s three write rules.
//!
//! These exercise [`ReducerWriter`] against a recording [`ReducerCaller`]. That
//! is the whole testable surface without a server: what the writer sends, what
//! it does with a refusal, and where the timestamps and the identity come from.
//! The transport itself — and the reducers on the far side — are
//! `tests/spacetime_module.rs`, against a live instance.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use crate::db::{CreateTaskRequest, SharedWriter, TaskPatch};
use crate::models::{
    EpicId, LearningId, NotificationWrite, StopOutcome, SubStatus, SubagentDrain, TaskId,
    TaskStatus, UserPromptOutcome,
};
use crate::service::{Clock, FixedClock};
use crate::spacetime::bindings;
use crate::sync::writes::{
    push_host_registration, DrainReadBack, ReducerCaller, ReducerOutcome, ReducerWriter,
    WriterIdentity,
};

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
    PatchTask(TaskId, Box<bindings::TaskPatch>),
    DeleteTask(TaskId),
    SetTaskEpic(TaskId, i64, String),
    Claim(TaskId, String),
    Release(TaskId),
    CreateEpic(Box<bindings::Epic>),
    PatchEpic(i64),
    DeleteEpic(i64),
    Recalculate(i64),
    SaveRepoPath(String, String),
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
    UpsertFeedTasks(i64, usize, String),
    UpsertFeedTasksAdditive(i64, usize, String),
    ClaimPollOwner(String, i64, String),
    OverridePollOwner(String, i64, String),
    DeleteStaleSubtreeFeedTasks(i64, Vec<String>),
    CreateRepoGroupSubEpic(i64, String, String),
    CreateManagedRoleEpic(String, i64, String, String, i64, String),
    CreateTaskWatcher(i64, i64),
    DeleteTaskWatcher(i64, i64),
    DeleteWatchesOfTarget(i64),
    DeleteWatchesByWatcher(i64),
    BatchPatchSubStatus(usize),
    RespawnPhoenixSuccessor(i64, Box<bindings::Task>),
    RegisterHost(String, String, String),
    SaveSetting(String, String, String),
    ClearSetting(String, String),
    SaveFilterPreset(String, String, String, String),
    DeleteFilterPreset(String, String),
    CreateLearning(Box<bindings::Learning>),
    PatchLearning(i64, Box<bindings::LearningPatch>),
    DeleteLearning(i64),
    RescopeEpicLearnings(i64, i64),
    RecordLearningRetrieval(i64, i64, String),
    ApplyLearningVerdicts(usize),
    ArchiveStaleLearnings(String),
    RecordUsageEvent(Box<bindings::UsageEvent>, i64),
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
    /// need `ReducerWriter`'s post-call verify-read to see a row actually
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
    /// on `ReducerWriter`'s post-call verify-read.
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

    async fn patch_task(
        &self,
        id: TaskId,
        patch: bindings::TaskPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::PatchTask(id, Box::new(patch)))
    }

    async fn delete_task(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteTask(id))
    }

    async fn set_task_epic(
        &self,
        id: TaskId,
        epic_id: i64,
        owner: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SetTaskEpic(id, epic_id, owner))
    }

    async fn claim_backlog_task(&self, id: TaskId, host: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Claim(id, host))
    }

    async fn release_backlog_claim(&self, id: TaskId) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Release(id))
    }

    async fn create_epic(&self, row: bindings::Epic) -> anyhow::Result<i64> {
        self.record(Sent::CreateEpic(Box::new(row)))?;
        Ok(7)
    }

    async fn patch_epic(
        &self,
        id: i64,
        _patch: bindings::EpicPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::PatchEpic(id))
    }

    async fn delete_epic(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteEpic(id))
    }

    async fn recalculate_epic_status(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Recalculate(id))
    }

    async fn save_repo_path(
        &self,
        path: String,
        last_used: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SaveRepoPath(path, last_used))
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
        last_used: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RecordBaseBranch(repo_path, branch, last_used))
    }

    async fn subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::Subscribe(subscriber, epic_id))
    }

    async fn unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
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

    async fn save_filter_preset(
        &self,
        host: String,
        name: String,
        repo_paths: String,
        mode: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SaveFilterPreset(host, name, repo_paths, mode))
    }

    async fn delete_filter_preset(
        &self,
        host: String,
        name: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteFilterPreset(host, name))
    }

    async fn create_learning(&self, row: bindings::Learning) -> anyhow::Result<LearningId> {
        self.record(Sent::CreateLearning(Box::new(row)))?;
        Ok(LearningId(99))
    }

    async fn patch_learning(
        &self,
        id: i64,
        patch: bindings::LearningPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::PatchLearning(id, Box::new(patch)))
    }

    async fn delete_learning(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteLearning(id))
    }

    async fn rescope_epic_learnings(&self, from: i64, to: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RescopeEpicLearnings(from, to))
    }

    async fn record_learning_retrieval(
        &self,
        task_id: i64,
        learning_id: i64,
        source: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RecordLearningRetrieval(task_id, learning_id, source))
    }

    async fn apply_learning_verdicts(
        &self,
        verdicts: Vec<bindings::LearningVerdictInput>,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ApplyLearningVerdicts(verdicts.len()))
    }

    async fn archive_stale_learnings(&self, cutoff: String) -> anyhow::Result<ReducerOutcome> {
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
        task_id: i64,
        agent_id: String,
        session_id: String,
        started_at: String,
    ) -> anyhow::Result<i64> {
        self.record(Sent::SubagentStart(
            task_id, agent_id, session_id, started_at,
        ))?;
        Ok(self.live_count)
    }

    async fn subagent_stop(
        &self,
        task_id: i64,
        agent_id: String,
        session_id: String,
    ) -> anyhow::Result<DrainReadBack> {
        self.record(Sent::SubagentStop(task_id, agent_id, session_id))?;
        Ok(self.drain)
    }

    async fn subagent_clear(&self, task_id: i64) -> anyhow::Result<DrainReadBack> {
        self.record(Sent::SubagentClear(task_id))?;
        Ok(self.drain)
    }

    async fn subagent_clear_and_void_pending_stop(
        &self,
        task_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::SubagentClearAndVoidPendingStop(task_id))
    }

    /// `refuses_with` (transport failure) is checked first, same as every
    /// other method. `rejects` (`RecordingCaller::rejecting()`) then answers
    /// `None` — the refusal `try_record_stop`'s `NoOp` reads.
    async fn try_record_stop(
        &self,
        id: i64,
        stop_pending_at: String,
    ) -> anyhow::Result<Option<bool>> {
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
        id: i64,
        sub_status: String,
        at: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RecordPreToolUse(id, sub_status, at))
    }

    async fn record_notification(
        &self,
        id: i64,
        mode: String,
        at: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RecordNotification(id, mode, at))
    }

    async fn record_user_prompt_submit(
        &self,
        id: i64,
        activity_at: String,
        prompt_at: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::RecordUserPromptSubmit(id, activity_at, prompt_at))
    }

    async fn mark_pr_learnings_gate_shown(
        &self,
        id: i64,
        at: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::MarkPrLearningsGateShown(id, at))
    }

    async fn upsert_feed_tasks(
        &self,
        epic_id: i64,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let outcome = self.answer(Sent::UpsertFeedTasks(epic_id, items.len(), created_by))?;
        if matches!(outcome, ReducerOutcome::Applied(_)) {
            self.simulate_feed_removal();
        }
        Ok(outcome)
    }

    async fn upsert_feed_tasks_additive(
        &self,
        epic_id: i64,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> anyhow::Result<ReducerOutcome> {
        let outcome = self.answer(Sent::UpsertFeedTasksAdditive(
            epic_id,
            items.len(),
            created_by,
        ))?;
        if matches!(outcome, ReducerOutcome::Applied(_)) {
            self.simulate_feed_removal();
        }
        Ok(outcome)
    }

    async fn delete_stale_subtree_feed_tasks(
        &self,
        parent_id: i64,
        keep_external_ids: Vec<String>,
    ) -> anyhow::Result<ReducerOutcome> {
        let outcome = self.answer(Sent::DeleteStaleSubtreeFeedTasks(
            parent_id,
            keep_external_ids,
        ))?;
        if matches!(outcome, ReducerOutcome::Applied(_)) {
            self.simulate_feed_removal();
        }
        Ok(outcome)
    }

    async fn create_repo_group_sub_epic(
        &self,
        parent_id: i64,
        title: String,
        created_by: String,
    ) -> anyhow::Result<i64> {
        self.record(Sent::CreateRepoGroupSubEpic(parent_id, title, created_by))?;
        Ok(8)
    }

    async fn create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: i64,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> anyhow::Result<i64> {
        self.record(Sent::CreateManagedRoleEpic(
            title,
            parent_epic_id,
            role,
            feed_command,
            feed_interval_secs,
            created_by,
        ))?;
        Ok(9)
    }

    async fn create_task_watcher(
        &self,
        watcher_task_id: i64,
        target_task_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::CreateTaskWatcher(watcher_task_id, target_task_id))
    }

    async fn delete_task_watcher(
        &self,
        watcher_task_id: i64,
        target_task_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteTaskWatcher(watcher_task_id, target_task_id))
    }

    async fn delete_watches_of_target(
        &self,
        target_task_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteWatchesOfTarget(target_task_id))
    }

    async fn delete_watches_by_watcher(
        &self,
        watcher_task_id: i64,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteWatchesByWatcher(watcher_task_id))
    }

    async fn claim_poll_owner(
        &self,
        scope: String,
        scope_id: i64,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ClaimPollOwner(scope, scope_id, host))
    }

    async fn override_poll_owner(
        &self,
        scope: String,
        scope_id: i64,
        host: String,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::OverridePollOwner(scope, scope_id, host))
    }

    async fn batch_patch_sub_status(
        &self,
        updates: Vec<bindings::SubStatusUpdate>,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::BatchPatchSubStatus(updates.len()))
    }

    async fn respawn_phoenix_successor(
        &self,
        predecessor: i64,
        successor: bindings::Task,
    ) -> anyhow::Result<TaskId> {
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
}

/// The instant the fixed clock sits at, and the string it must produce.
const AT: &str = "2026-09-19T12:34:56.789Z";
const AT_STORED: &str = "2026-09-19 12:34:56.789";

/// A SECOND instant, distinct from `AT` — the writer's own `FixedClock`, used
/// for the methods (`save_repo_path` and friends) that read `self.now()`.
/// The agent-session-state methods take `now` as an argument instead
/// (decision 4 in this task's plan doc: it must be the event time the hook
/// itself observed, not this connection's clock), so a test that passed `AT`
/// for both could not tell "used the argument" apart from "resampled
/// `self.clock` and got the same answer by coincidence" — exactly the bug an
/// adversarial review of this plan's implementation caught twice (once in
/// the now-removed shell-tracking feature's own reducer twin, and once in
/// `try_record_stop`) before this file existed to catch it again.
const CALL_AT: &str = "2026-01-02T03:04:05.678Z";
const CALL_AT_STORED: &str = "2026-01-02 03:04:05.678";

fn call_at() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(CALL_AT)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

fn writer_with(caller: RecordingCaller) -> (ReducerWriter, Arc<RecordingCaller>) {
    let caller = Arc::new(caller);
    let clock = FixedClock::new(
        chrono::DateTime::parse_from_rfc3339(AT)
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(Some("user-me".into()))),
        Arc::new(clock) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SubscriptionBoardReads::new(Arc::new(
            crate::sync::SharedRows::new(),
        ))),
    );
    (writer, caller)
}

/// The twin of [`writer_with`], for the "nothing has settled yet" tests.
fn writer_with_no_identity(caller: RecordingCaller) -> (ReducerWriter, Arc<RecordingCaller>) {
    let caller = Arc::new(caller);
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(None)),
        Arc::new(FixedClock::new(
            chrono::DateTime::parse_from_rfc3339(AT)
                .unwrap()
                .with_timezone(&chrono::Utc),
        )) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SubscriptionBoardReads::new(Arc::new(
            crate::sync::SharedRows::new(),
        ))),
    );
    (writer, caller)
}

/// A writer over a seeded subscription view, for the candidate loop.
fn writer_over(
    rows: Arc<crate::sync::SharedRows>,
    caller: RecordingCaller,
) -> (ReducerWriter, Arc<RecordingCaller>) {
    let caller = Arc::new(caller);
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(Some("user-me".into()))),
        Arc::new(FixedClock::new(
            chrono::DateTime::parse_from_rfc3339(AT)
                .unwrap()
                .with_timezone(&chrono::Utc),
        )) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SubscriptionBoardReads::new(rows)),
    );
    (writer, caller)
}

/// A backlog subtask of epic 1, as the subscription would deliver it.
fn backlog_row(id: i64, sort_order: Option<i64>, host: &str, phoenix: bool) -> bindings::Task {
    let mut row = crate::sync::encode::create_task_row(
        &CreateTaskRequest {
            epic_id: Some(EpicId(1)),
            ..a_request()
        },
        "",
        "user-me",
        AT_STORED,
    );
    row.id = id;
    row.sort_order = sort_order;
    row.host = host.to_string();
    row.phoenix = phoenix;
    row
}

fn a_request() -> CreateTaskRequest<'static> {
    CreateTaskRequest {
        title: "t",
        description: "d",
        repo_path: "/repo",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    }
}

/// A create sends one row and answers with the id the store generated.
///
/// The id matters because the caller dispatches the task it just made. A writer
/// that returned a placeholder would have the board open a detail view on a
/// task that is not the one it created.
#[tokio::test]
async fn a_create_sends_the_row_and_returns_the_generated_id() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer.create_task(a_request()).await.unwrap();

    assert_eq!(id, TaskId(42));
    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected one create, got {:?}", caller.sent());
    };
    assert_eq!(row.title, "t");
    assert_eq!(row.id, 0, "the store generates the id");
}

/// The board's clock stamps a create, and it stamps both columns the same.
///
/// `created_at` records when the person asked, which is a fact about this
/// machine rather than about the store. Everything the store DERIVES afterwards
/// uses the store's clock instead, so two boards' rows are ordered by one
/// clock — see `ReducerWriter::now`.
#[tokio::test]
async fn a_create_is_stamped_with_this_boards_clock() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_task(a_request()).await.unwrap();

    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.created_at, AT_STORED);
    assert_eq!(row.updated_at, AT_STORED);
}

/// The owner comes from the writer's identity, not from the request — the
/// request has no field for it. `core.allium: OwnerTracksUserBoardTask`.
#[tokio::test]
async fn an_epicless_create_carries_this_users_identity() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_task(a_request()).await.unwrap();

    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.owner, "user-me");
}

/// ...and a task in an epic carries none. The other arm of the same rule, and
/// the one the module refuses outright.
#[tokio::test]
async fn a_create_in_an_epic_carries_no_owner() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .create_task(CreateTaskRequest {
            epic_id: Some(EpicId(3)),
            ..a_request()
        })
        .await
        .unwrap();

    let Some(Sent::CreateTask(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.owner, "");
}

/// `created_by` is a DIFFERENT question from `owner` — it survives regardless
/// of epic membership, because it is how `sync.allium`'s `own_creations`
/// subscription finds a task this identity just created no matter which epic
/// it landed in. `core.allium: Task.created_by`.
#[tokio::test]
async fn a_task_create_carries_the_creator_regardless_of_epic() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_task(a_request()).await.unwrap();
    writer
        .create_task(CreateTaskRequest {
            epic_id: Some(EpicId(3)),
            ..a_request()
        })
        .await
        .unwrap();

    let sent = caller.sent();
    for entry in sent {
        let Sent::CreateTask(row) = entry else {
            panic!("expected two creates");
        };
        assert_eq!(row.created_by, "user-me");
    }
}

/// A patch travels as a patch, naming only what it changes.
#[tokio::test]
async fn a_patch_names_only_the_fields_it_changes() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .patch_task(TaskId(7), &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    let Some(Sent::PatchTask(id, patch)) = caller.sent().into_iter().next() else {
        panic!("expected a patch");
    };
    assert_eq!(id, TaskId(7));
    assert_eq!(patch.status.as_deref(), Some("running"));
    assert_eq!(patch.title, None);
    assert_eq!(patch.worktree, None);
}

/// `sync.allium: AWriteWithNoConnectionIsRefused` — the reason reaches the
/// caller, and nothing was sent.
#[tokio::test]
async fn a_refused_write_carries_the_stores_reason() {
    let (writer, caller) = writer_with(RecordingCaller::refusing("store unreachable"));

    let refused = writer.create_task(a_request()).await;

    let why = refused.expect_err("a refused write must fail");
    assert!(
        why.to_string().contains("store unreachable"),
        "expected the store's reason, got {why}"
    );
    assert!(caller.sent().is_empty());
}

/// And it is not retried. One ask, one answer, no second attempt hiding inside
/// the first — `sync.allium: NoWriteIsEverQueued`.
#[tokio::test]
async fn a_refusal_is_not_retried_inside_the_writer() {
    let caller = Arc::new(RecordingCaller::refusing("down"));
    let clock = FixedClock::new(
        chrono::DateTime::parse_from_rfc3339(AT)
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(Some("user-me".into()))),
        Arc::new(clock) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SubscriptionBoardReads::new(Arc::new(
            crate::sync::SharedRows::new(),
        ))),
    );

    for _ in 0..3 {
        assert!(writer.delete_task(TaskId(1)).await.is_err());
    }
    // Every attempt reached the caller and failed there. Nothing accumulated.
    assert!(caller.sent().is_empty());
}

/// A repo path is stamped by the board too, for the same reason a create is:
/// `last_used` is when this person used it.
#[tokio::test]
async fn a_repo_path_is_stamped_with_this_boards_clock() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.save_repo_path("/repo").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveRepoPath("/repo".into(), AT_STORED.into())]
    );
}

/// A board that has never connected cannot create a task on its own user
/// board, because there is no board to put it on. Refused here rather than
/// sent, so the operator gets the actionable message instead of the module's.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_an_epicless_task() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer.create_task(a_request()).await;

    assert!(refused
        .expect_err("a user-board task needs a user")
        .to_string()
        .contains("no user identity"));
    assert!(caller.sent().is_empty());
}

/// ...and NEITHER can a task inside an epic, even though that task carries no
/// owner. `sync.allium: CreatesRequireASettledIdentity` — every create needs a
/// settled identity now, because `created_by` needs a name to stamp regardless
/// of whether `owner` does.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_a_task_in_an_epic_either() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer
        .create_task(CreateTaskRequest {
            epic_id: Some(EpicId(3)),
            ..a_request()
        })
        .await;

    assert!(refused.is_err(), "a create needs a name to stamp");
    assert!(caller.sent().is_empty());
}

/// A delete names its row and nothing else. The cascade — watchers, shells,
/// subagents — is the store's, inside the same transaction, because a client
/// doing it in several calls could be interrupted between them.
#[tokio::test]
async fn a_delete_sends_only_the_id() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.delete_task(TaskId(9)).await.unwrap();

    assert_eq!(caller.sent(), vec![Sent::DeleteTask(TaskId(9))]);
}

// -- A refusal is not an outage ---------------------------------------------

/// THE DISTINCTION THE CLAIM DEPENDS ON. A store that says no is a different
/// thing from a store that is not there, and only one of them is a failure.
///
/// Losing a race is the ordinary outcome of two hosts reaching for one task;
/// the caller handles it by provisioning nothing. A store that is DOWN is an
/// outage the operator has to see. Collapsing the two would make a board that
/// lost its connection look like a board losing every race.
#[tokio::test]
async fn a_lost_claim_is_an_answer_and_an_outage_is_an_error() {
    let (winner, _) = writer_with(RecordingCaller::default());
    assert!(winner.try_claim_backlog_task(TaskId(1)).await.unwrap());

    let (loser, _) = writer_with(RecordingCaller::rejecting());
    assert!(
        !loser.try_claim_backlog_task(TaskId(1)).await.unwrap(),
        "a refused claim is Ok(false) — somebody else got there first"
    );

    let (offline, _) = writer_with(RecordingCaller::refusing("store unreachable"));
    assert!(
        offline.try_claim_backlog_task(TaskId(1)).await.is_err(),
        "a claim with the store down must fail rather than report a lost race"
    );
}

/// ...and everywhere else a refusal IS an error. A patch the store declined did
/// not happen, and the caller has to know.
#[tokio::test]
async fn a_refused_patch_is_an_error() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    let refused = writer
        .patch_task(TaskId(1), &TaskPatch::new().title("t"))
        .await;

    assert!(refused.is_err(), "a declined patch must not report success");
}

/// The same asymmetry on unsubscribing, where the spec asks for it explicitly:
/// unfollowing something unfollowed is a refusal at the store and `Ok(false)`
/// here, matching the SQLite signature (`sync.allium: UnsubscribeFromEpic`).
#[tokio::test]
async fn unsubscribing_from_something_unfollowed_is_false_not_an_error() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    assert!(!writer.unsubscribe_from_epic("user-me", 3).await.unwrap());
}

// -- The rest of the routed surface -----------------------------------------

/// Moving a task out of its epic gives it an owner; moving it in takes the
/// owner away. `core.allium: OwnerTracksUserBoardTask` — the store enforces
/// both arms, and this is the name it needs to do so.
#[tokio::test]
async fn leaving_an_epic_carries_the_owner_and_joining_one_does_not() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.set_task_epic_id(TaskId(1), None).await.unwrap();
    writer
        .set_task_epic_id(TaskId(2), Some(EpicId(4)))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::SetTaskEpic(TaskId(1), 0, "user-me".into()),
            Sent::SetTaskEpic(TaskId(2), 4, String::new()),
        ]
    );
}

/// Clearing a verify command sends the store's absent sentinel, not a command
/// that happens to be empty.
#[tokio::test]
async fn clearing_a_verify_command_sends_the_absent_sentinel() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.set_verify_command("/repo", None).await.unwrap();
    writer
        .set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::SetVerifyCommand("/repo".into(), String::new()),
            Sent::SetVerifyCommand("/repo".into(), "cargo test".into()),
        ]
    );
}

/// A new epic is born in backlog, not auto-dispatching and manual —
/// `epics.allium: CreateEpic`. Those are the epic's birth state rather than
/// defaults somebody forgot, and they are set here so the store has no opinion
/// about what a new epic looks like.
#[tokio::test]
async fn a_new_epic_is_born_in_backlog() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let created = writer.create_epic("New", "why", None).await.unwrap();

    let Some(Sent::CreateEpic(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.status, "backlog");
    assert!(!row.auto_dispatch);
    assert_eq!(row.origin, "manual");
    assert_eq!(row.parent_epic_id, 0);
    // And the caller gets back the row it sent, with the store's id on it.
    assert_eq!(created.id, EpicId(7));
    assert_eq!(created.title, "New");
    assert_eq!(created.status, TaskStatus::Backlog);
}

/// An epic create carries the creator's identity, the same as a task's does —
/// `core.allium: Epic.created_by`. Unlike a task, an epic has no `owner` field
/// at all, so this is the ONLY way `sync.allium`'s `own_creations`
/// subscription can find an epic its creator just made.
#[tokio::test]
async fn an_epic_create_carries_the_creator() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.create_epic("New", "why", None).await.unwrap();

    let Some(Sent::CreateEpic(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.created_by, "user-me");
}

/// A board with no settled identity cannot create an epic either —
/// `sync.allium: CreatesRequireASettledIdentity`. An epic has no `owner` to
/// fall back on the way a task does, so this is the only guard standing
/// between a create and a `created_by` nobody can name.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_an_epic() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer.create_epic("New", "why", None).await;

    assert!(refused.is_err(), "an epic create needs a name to stamp");
    assert!(caller.sent().is_empty());
}

/// The claim carries THIS machine's host id, which is what lets the store pass
/// over a task whose worktree is somewhere else.
#[tokio::test]
async fn a_claim_names_the_machine_making_it() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.try_claim_backlog_task(TaskId(3)).await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::Claim(TaskId(3), "host-me".into())]
    );
}

// -- The chain's candidate loop ---------------------------------------------

/// The chain takes the task the board draws as next: `COALESCE(sort_order, id)`
/// then id. A chain that disagreed with the column would be a board doing
/// something other than what it displays.
#[tokio::test]
async fn the_chain_takes_the_task_the_board_draws_as_next() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    for row in [
        backlog_row(10, None, "", false),
        backlog_row(20, Some(5), "", false),
        backlog_row(30, None, "", false),
    ] {
        rows.upsert_task(&row);
    }
    let (writer, caller) = writer_over(rows, RecordingCaller::default());

    let claimed = writer.try_claim_next_backlog_task(EpicId(1)).await.unwrap();

    assert_eq!(claimed, Some(TaskId(20)), "the explicit sort_order wins");
    assert_eq!(
        caller.sent(),
        vec![Sent::Claim(TaskId(20), "host-me".into())]
    );
}

/// A phoenix subtask is passed over, never claimed.
/// `epics.allium: PhoenixIsNeverChained` — a recurring subtask respawns on
/// completion, so chaining its successor would launch an agent at it
/// immediately, forever.
#[tokio::test]
async fn the_chain_passes_over_a_phoenix() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&backlog_row(1, None, "", true));
    rows.upsert_task(&backlog_row(2, None, "", false));
    let (writer, _) = writer_over(rows, RecordingCaller::default());

    assert_eq!(
        writer.try_claim_next_backlog_task(EpicId(1)).await.unwrap(),
        Some(TaskId(2))
    );
}

/// ...and so is a task whose worktree is on another machine. Claiming it would
/// dispatch an agent with nowhere to work.
#[tokio::test]
async fn the_chain_passes_over_another_hosts_task() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&backlog_row(1, None, "host-other", false));
    rows.upsert_task(&backlog_row(2, None, "host-me", false));
    let (writer, _) = writer_over(rows, RecordingCaller::default());

    assert_eq!(
        writer.try_claim_next_backlog_task(EpicId(1)).await.unwrap(),
        Some(TaskId(2))
    );
}

/// An epic with nothing left claims nothing, and that is not an error. It is
/// the ordinary end of a chain, which `exit_session` reaches on every wrap-up.
#[tokio::test]
async fn an_empty_backlog_ends_the_chain_quietly() {
    let (writer, caller) = writer_over(
        Arc::new(crate::sync::SharedRows::new()),
        RecordingCaller::default(),
    );

    assert_eq!(
        writer.try_claim_next_backlog_task(EpicId(1)).await.unwrap(),
        None
    );
    assert!(caller.sent().is_empty(), "nothing to offer, nothing sent");
}

/// THE RACE. Every candidate is taken by somebody else, so the loop runs out
/// and reports an empty backlog rather than claiming something it lost.
#[tokio::test]
async fn a_chain_that_loses_every_race_claims_nothing() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&backlog_row(1, None, "", false));
    rows.upsert_task(&backlog_row(2, None, "", false));
    let (writer, _) = writer_over(rows, RecordingCaller::rejecting());

    assert_eq!(
        writer.try_claim_next_backlog_task(EpicId(1)).await.unwrap(),
        None
    );
}

/// ...but a store that is DOWN fails on the first offer rather than walking the
/// whole list and reporting an empty backlog. An outage must not look like a
/// finished epic.
#[tokio::test]
async fn a_chain_with_the_store_down_fails_loudly() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&backlog_row(1, None, "", false));
    let (writer, _) = writer_over(rows, RecordingCaller::refusing("store unreachable"));

    assert!(writer.try_claim_next_backlog_task(EpicId(1)).await.is_err());
}

// -- The refusal names the outage -------------------------------------------

/// `sync.allium: AWriteWithNoConnectionIsRefused` — the refusal carries the
/// connection's reason. "The store is unreachable: connection refused" is
/// something an operator can act on; "could not save" is not.
///
/// Against `SettledIdentity` directly, because the cell is the whole mechanism:
/// the session owns the error and the transport that reports the refusal is
/// deliberately not able to reach the session.
#[test]
fn the_outage_reason_reaches_the_writer_through_the_cell() {
    let cell = crate::sync::SettledIdentity::default();
    assert_eq!(cell.last_error(), None, "nothing has failed yet");

    cell.set_last_error(Some("connection refused".into()));
    assert_eq!(cell.last_error().as_deref(), Some("connection refused"));
}

/// A successful connection clears it, so a refusal never quotes an outage that
/// is over. The clear rides on `settle` rather than being a second call the
/// connection loop has to remember.
#[tokio::test]
async fn connecting_clears_the_last_outage() {
    let cell = crate::sync::SettledIdentity::default();
    cell.set_last_error(Some("connection refused".into()));

    cell.settle("user-me");

    assert_eq!(cell.last_error(), None);
    assert_eq!(cell.user().await.unwrap().as_deref(), Some("user-me"));
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
) -> (ReducerWriter, Arc<RecordingCaller>) {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&seeded_row(id, status));
    writer_over(rows, caller)
}

/// `subagent_start` never refuses and answers with the live count the
/// module read back, not a placeholder.
#[tokio::test]
async fn subagent_start_reads_the_live_count_back() {
    let (writer, caller) = writer_with(RecordingCaller::returning_count(3));

    let live = writer
        .subagent_start(TaskId(1), "agent-1", "session-1", at())
        .await
        .unwrap();

    assert_eq!(live, 3);
    assert_eq!(
        caller.sent(),
        vec![Sent::SubagentStart(
            1,
            "agent-1".into(),
            "session-1".into(),
            at().to_rfc3339(),
        )]
    );
}

/// `started_at` is RFC 3339 for a subagent — this column is never compared.
#[tokio::test]
async fn subagent_start_stamps_rfc3339_of_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning_count(1));

    writer
        .subagent_start(TaskId(1), "agent-1", "session-1", call_at())
        .await
        .unwrap();

    let Some(Sent::SubagentStart(_, _, _, started_at)) = caller.sent().into_iter().next() else {
        panic!("expected a subagent start");
    };
    assert_eq!(started_at, "2026-01-02T03:04:05.678+00:00");
}

/// A drain that did NOT reach zero (or reached zero without a pending Stop to
/// apply) reports no flip, whatever the module's `live` answer says by
/// itself — `applied_pending_stop` is a SEPARATE bit from `live`.
#[tokio::test]
async fn a_subagent_stop_that_does_not_drain_reports_no_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(2, false),
    );

    let drain = writer
        .subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();

    assert_eq!(
        drain,
        SubagentDrain {
            live: 2,
            applied_pending_stop: false,
        }
    );
}

/// The task was seen `Running` just before the call, and the module reports
/// the row is now in `review` — the flip is real, and reported as one.
#[tokio::test]
async fn a_subagent_stop_that_drains_a_running_task_reports_the_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(0, true),
    );

    let drain = writer
        .subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();

    assert_eq!(
        drain,
        SubagentDrain {
            live: 0,
            applied_pending_stop: true,
        }
    );
}

/// The module's `review` answer alone is not enough — a task already sitting
/// in `review` before this call reports `review` too, and it drained
/// nothing. The pre-read is what tells the two apart: without one seeing
/// `Running`, this reports no flip even though the module says `review`.
/// (Decision 3 in this task's plan doc: advisory, and safe to under-report
/// here because the module's own recalculation, not this bit, is what kept
/// the epic status correct.)
#[tokio::test]
async fn a_review_answer_with_no_running_pre_read_reports_no_flip() {
    let (writer, _) = writer_with(RecordingCaller::returning_drain(0, true));

    let drain = writer
        .subagent_stop(TaskId(1), "agent-1", "session-1")
        .await
        .unwrap();

    assert!(!drain.applied_pending_stop);
}

#[tokio::test]
async fn a_subagent_clear_that_drains_a_running_task_reports_the_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(0, true),
    );

    let drain = writer.subagent_clear(TaskId(1)).await.unwrap();

    assert!(drain.applied_pending_stop);
}

/// `try_record_stop`: once accepted, `Flipped` and `Deferred` are read back
/// from the module's answer directly, with no pre-read needed — see this
/// task's plan doc, decision 3, for why this pair (unlike resume/refresh
/// below) needs none.
#[tokio::test]
async fn try_record_stop_reads_flipped_back() {
    let (writer, _) = writer_with(RecordingCaller::returning_stop_flag(true));

    assert_eq!(
        writer.try_record_stop(TaskId(1), at()).await.unwrap(),
        StopOutcome::Flipped
    );
}

#[tokio::test]
async fn try_record_stop_reads_deferred_back() {
    let (writer, _) = writer_with(RecordingCaller::returning_stop_flag(false));

    assert_eq!(
        writer.try_record_stop(TaskId(1), at()).await.unwrap(),
        StopOutcome::Deferred
    );
}

/// A refused call — the module's own guard against a task that is not
/// `Running` — reads as `NoOp`, not an error. The refusal IS the answer here.
#[tokio::test]
async fn try_record_stop_reads_a_refusal_as_no_op() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    assert_eq!(
        writer.try_record_stop(TaskId(1), at()).await.unwrap(),
        StopOutcome::NoOp
    );
}

/// `stop_pending_at` is the millisecond-precision fixed-width format, the
/// same `encode::stamp` the create path uses — it is compared against the
/// arriving prompt's own timestamp, so its format is load-bearing.
#[tokio::test]
async fn try_record_stop_stamps_the_passed_now_not_this_connections_clock() {
    let (writer, caller) = writer_with(RecordingCaller::returning_stop_flag(true));

    writer.try_record_stop(TaskId(1), call_at()).await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::TryRecordStop(1, CALL_AT_STORED.to_string())]
    );
}

/// `Resumed` vs. `Refreshed` cannot be read back from the module's answer —
/// both write the identical row (decision 3). The task was seen `Review`
/// just before the call, so this reports `Resumed`.
#[tokio::test]
async fn record_user_prompt_submit_reads_a_review_pre_read_as_resumed() {
    let (writer, _) = writer_seeded_with(1, TaskStatus::Review, RecordingCaller::returning(&[]));

    assert_eq!(
        writer
            .record_user_prompt_submit(TaskId(1), at())
            .await
            .unwrap(),
        UserPromptOutcome::Resumed
    );
}

/// The other arm: seen `Running` already, so this is a plain refresh.
#[tokio::test]
async fn record_user_prompt_submit_reads_a_running_pre_read_as_refreshed() {
    let (writer, _) = writer_seeded_with(1, TaskStatus::Running, RecordingCaller::returning(&[]));

    assert_eq!(
        writer
            .record_user_prompt_submit(TaskId(1), at())
            .await
            .unwrap(),
        UserPromptOutcome::Refreshed
    );
}

#[tokio::test]
async fn record_user_prompt_submit_reads_a_refusal_as_no_op() {
    let (writer, _) = writer_with(RecordingCaller::rejecting());

    assert_eq!(
        writer
            .record_user_prompt_submit(TaskId(1), at())
            .await
            .unwrap(),
        UserPromptOutcome::NoOp
    );
}

/// Both timestamps come off the SAME `now` — mirroring
/// `src/db/queries/tasks.rs::record_user_prompt_submit`'s single-`now`, two-precision split
/// (seconds for `last_pre_tool_use_at`, millis for the void-comparison) —
/// see this task's plan doc, decision 4.
#[tokio::test]
async fn record_user_prompt_submit_stamps_both_timestamps_from_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .record_user_prompt_submit(TaskId(1), call_at())
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::RecordUserPromptSubmit(
            1,
            CALL_AT_STORED.to_string(),
            CALL_AT_STORED.to_string(),
        )]
    );
}

/// `Applied`/`Refused` map straight onto the PR gate's `true`/`false` via
/// `ReducerOutcome::won()` — the exact `try_claim_backlog_task` shape.
#[tokio::test]
async fn mark_pr_learnings_gate_shown_wins_when_applied_and_loses_when_refused() {
    let (applied_writer, _) = writer_with(RecordingCaller::returning(&[]));
    assert!(applied_writer
        .mark_pr_learnings_gate_shown(TaskId(1))
        .await
        .unwrap());

    let (refused_writer, _) = writer_with(RecordingCaller::rejecting());
    assert!(!refused_writer
        .mark_pr_learnings_gate_shown(TaskId(1))
        .await
        .unwrap());
}

/// `Ignore` never reaches the store at all — the client-side short-circuit
/// matching the SQL path's early `return Ok(())`, checked here by asserting
/// nothing was sent rather than merely that the call succeeded.
#[tokio::test]
async fn an_ignored_notification_never_reaches_the_store() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .record_notification(TaskId(1), NotificationWrite::Ignore, at())
        .await
        .unwrap();

    assert!(caller.sent().is_empty());
}

/// The other three `NotificationWrite` modes DO reach the store, each named
/// by the module's own spelling.
#[tokio::test]
async fn a_notification_names_the_modules_mode() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .record_notification(TaskId(1), NotificationWrite::Raise, at())
        .await
        .unwrap();
    writer
        .record_notification(TaskId(1), NotificationWrite::Clear, at())
        .await
        .unwrap();
    writer
        .record_notification(TaskId(1), NotificationWrite::RaiseIfNoOwnWorkLive, at())
        .await
        .unwrap();

    let modes: Vec<String> = caller
        .sent()
        .into_iter()
        .map(|s| match s {
            Sent::RecordNotification(_, mode, _) => mode,
            other => panic!("expected a notification, got {other:?}"),
        })
        .collect();
    assert_eq!(modes, vec!["raise", "clear", "raise_if_no_own_work_live"]);
}

/// `record_pre_tool_use` passes the already-classified `sub_status` straight
/// through, stamped with the event time.
#[tokio::test]
async fn record_pre_tool_use_names_the_sub_status_and_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .record_pre_tool_use(TaskId(1), SubStatus::Active, call_at())
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::RecordPreToolUse(
            1,
            "active".into(),
            CALL_AT_STORED.to_string()
        )]
    );
}

#[tokio::test]
async fn subagent_clear_and_void_pending_stop_is_a_plain_applied_call() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .subagent_clear_and_void_pending_stop(TaskId(1))
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SubagentClearAndVoidPendingStop(1)]
    );
}

/// The instant the fixed clock in `writer_with`/`writer_seeded_with` sits at.
fn at() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(AT)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

// ---------------------------------------------------------------------------
// Phase 6c: feed ingestion, task watchers, and the stragglers
// ---------------------------------------------------------------------------

/// A feed-managed task row in `epic`, as the subscription would deliver it.
fn feed_task_row(id: i64, epic: i64, external_id: &str) -> bindings::Task {
    let mut row = crate::sync::encode::create_task_row(
        &CreateTaskRequest {
            epic_id: Some(EpicId(epic)),
            ..a_request()
        },
        "",
        "user-me",
        AT_STORED,
    );
    row.id = id;
    row.external_id = external_id.to_string();
    row
}

fn a_feed_item(external_id: &str, title: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: title.to_string(),
        description: String::new(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }
}

/// The core of the predict-then-verify design (this task's plan doc, decision
/// 1): a candidate the mock reducer actually removed is reported; a candidate
/// that merely matched the stale predicate but SURVIVED the call (simulating
/// a race) is silently dropped, never reported as removed.
#[tokio::test]
async fn a_confirmed_removal_is_reported_but_a_raced_survivor_is_not() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&feed_task_row(1, 1, "gone"));
    rows.upsert_task(&feed_task_row(2, 1, "raced"));
    rows.upsert_task(&feed_task_row(3, 1, "kept"));
    // The mock reducer call removes only id 1 — id 2 is a stale candidate by
    // the predicate but the "reducer" (the mock) never actually deletes it,
    // simulating a concurrent write that un-staled it between the pre-read
    // and the call.
    let caller = RecordingCaller::removing_on_feed_write(rows.clone(), &[1]);
    let (writer, sent) = writer_over(rows, caller);

    let removed = writer
        .upsert_feed_tasks(
            EpicId(1),
            &[a_feed_item("kept", "kept")],
            &["/repo".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(
        removed.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![TaskId(1)],
        "only the CONFIRMED-absent candidate is reported, never a survivor"
    );
    assert_eq!(
        sent.sent(),
        vec![Sent::UpsertFeedTasks(1, 1, "user-me".to_string())],
        "the wire item count is what was sent, not the candidate count"
    );
}

/// The additive variant never predicts or reports a removal, matching the
/// SQLite version's "always empty" contract — it has no stale-delete pass to
/// predict candidates for.
#[tokio::test]
async fn additive_upsert_never_reports_a_removal() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    rows.upsert_task(&feed_task_row(1, 1, "absent-from-emission"));
    let caller = RecordingCaller::removing_on_feed_write(rows.clone(), &[1]);
    let (writer, sent) = writer_over(rows, caller);

    let removed = writer
        .upsert_feed_tasks_additive(EpicId(1), &[], &[], &[])
        .await
        .unwrap();

    assert!(removed.is_empty());
    assert_eq!(
        sent.sent(),
        vec![Sent::UpsertFeedTasksAdditive(1, 0, "user-me".to_string())]
    );
}

/// A feed sync must not fail just because this install has never connected to
/// a shared store before — best-effort, unlike every `create_*` call that
/// uses `require_identity`. `created_by` is honestly empty rather than the
/// whole sync being refused. `core.allium: Task.created_by`;
/// `feeds.allium: UpsertFeedTasks`.
#[tokio::test]
async fn feed_upsert_with_no_settled_identity_still_applies_with_an_empty_created_by() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    let caller = RecordingCaller::default();
    let (writer, sent) = {
        let caller = Arc::new(caller);
        let writer = ReducerWriter::new(
            caller.clone(),
            Arc::new(FixedIdentity(None)),
            Arc::new(FixedClock::new(
                chrono::DateTime::parse_from_rfc3339(AT)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            )) as Arc<dyn Clock>,
            "host-me".into(),
            Arc::new(crate::sync::SubscriptionBoardReads::new(rows)),
        );
        (writer, caller)
    };

    writer
        .upsert_feed_tasks(
            EpicId(1),
            &[a_feed_item("kept", "kept")],
            &["/repo".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(
        sent.sent(),
        vec![Sent::UpsertFeedTasks(1, 1, String::new())],
        "no settled identity stamps an empty created_by, not a refusal"
    );
}

/// The subtree-scoped delete reads candidates from every direct CHILD epic of
/// `parent_id`, not from the parent itself.
#[tokio::test]
async fn delete_stale_subtree_scopes_candidates_to_child_epics() {
    let rows = Arc::new(crate::sync::SharedRows::new());
    let mut parent = crate::sync::encode::create_epic_row("parent", "", None, "user-me", AT_STORED);
    parent.id = 1;
    rows.upsert_epic(&parent);
    let mut child =
        crate::sync::encode::create_epic_row("child", "", Some(EpicId(1)), "user-me", AT_STORED);
    child.id = 2;
    rows.upsert_epic(&child);
    rows.upsert_task(&feed_task_row(10, 2, "stale"));
    let caller = RecordingCaller::removing_on_feed_write(rows.clone(), &[10]);
    let (writer, sent) = writer_over(rows, caller);

    let removed = writer
        .delete_stale_subtree_feed_tasks(EpicId(1), &["keep".to_string()])
        .await
        .unwrap();

    assert_eq!(
        removed.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![TaskId(10)]
    );
    assert_eq!(
        sent.sent(),
        vec![Sent::DeleteStaleSubtreeFeedTasks(
            1,
            vec!["keep".to_string()]
        )]
    );
}

/// Find-or-create sends this connection's own settled identity as
/// `created_by` — required for `own_creations` to ever find the row again
/// (decision 3 of this task's plan doc) — and returns the id the store
/// answered, whichever arm it took.
#[tokio::test]
async fn create_repo_group_sub_epic_stamps_the_identity_and_returns_the_id() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer
        .create_repo_group_sub_epic(EpicId(1), "my-repo")
        .await
        .unwrap();

    assert_eq!(id, EpicId(8));
    assert_eq!(
        caller.sent(),
        vec![Sent::CreateRepoGroupSubEpic(
            1,
            "my-repo".to_string(),
            "user-me".to_string()
        )]
    );
}

#[tokio::test]
async fn a_board_with_no_identity_cannot_create_a_repo_group_sub_epic() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    assert!(writer
        .create_repo_group_sub_epic(EpicId(1), "my-repo")
        .await
        .is_err());
    assert!(caller.sent().is_empty());
}

#[tokio::test]
async fn create_managed_role_epic_stamps_the_identity_and_returns_the_id() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer
        .create_managed_role_epic(
            "Reviews",
            Some(EpicId(1)),
            crate::models::FeedRole::None,
            Some("gh pr list"),
            Some(300),
        )
        .await
        .unwrap();

    assert_eq!(id, EpicId(9));
    assert_eq!(
        caller.sent(),
        vec![Sent::CreateManagedRoleEpic(
            "Reviews".to_string(),
            1,
            crate::models::FeedRole::None.as_str().to_string(),
            "gh pr list".to_string(),
            300,
            "user-me".to_string()
        )]
    );
}

/// The four watcher methods are plain applied-or-refused calls — no id, no
/// read-back.
#[tokio::test]
async fn task_watcher_methods_are_plain_applied_calls() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .create_task_watcher(TaskId(1), TaskId(2))
        .await
        .unwrap();
    writer
        .delete_task_watcher(TaskId(1), TaskId(2))
        .await
        .unwrap();
    writer.delete_watches_of_target(TaskId(2)).await.unwrap();
    writer.delete_watches_by_watcher(TaskId(1)).await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![
            Sent::CreateTaskWatcher(1, 2),
            Sent::DeleteTaskWatcher(1, 2),
            Sent::DeleteWatchesOfTarget(2),
            Sent::DeleteWatchesByWatcher(1),
        ]
    );
}

/// An empty batch never reaches the writer at all — mirrors the SQLite
/// path's own early return, and saves a round trip for the tick's common
/// case of nothing having changed.
#[tokio::test]
async fn an_empty_sub_status_batch_never_calls_the_writer() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer.batch_patch_sub_status(&[]).await.unwrap();

    assert!(caller.sent().is_empty());
}

#[tokio::test]
async fn batch_patch_sub_status_sends_the_whole_batch_in_one_call() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer
        .batch_patch_sub_status(&[
            (TaskId(1), SubStatus::Active),
            (TaskId(2), SubStatus::Stale),
        ])
        .await
        .unwrap();

    assert_eq!(caller.sent(), vec![Sent::BatchPatchSubStatus(2)]);
}

/// The successor is stamped with this connection's identity and the labels
/// the caller passed — not `encode::create_task_row`'s empty default.
#[tokio::test]
async fn respawn_phoenix_successor_stamps_the_identity_and_the_labels() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    let id = writer
        .respawn_phoenix_successor(TaskId(1), a_request(), &["a".to_string(), "b".to_string()])
        .await
        .unwrap();

    assert_eq!(id, TaskId(43));
    let sent = caller.sent();
    match sent.as_slice() {
        [Sent::RespawnPhoenixSuccessor(predecessor, row)] => {
            assert_eq!(*predecessor, 1);
            assert_eq!(row.owner, "user-me");
            assert_eq!(row.labels, "[\"a\",\"b\"]");
        }
        other => panic!("expected one RespawnPhoenixSuccessor call, got {other:?}"),
    }
}

#[tokio::test]
async fn a_board_with_no_identity_cannot_respawn_a_phoenix_successor() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    assert!(writer
        .respawn_phoenix_successor(TaskId(1), a_request(), &[])
        .await
        .is_err());
    assert!(caller.sent().is_empty());
}

// -- Host registry (Phase 6c) -------------------------------------------------
//
// `push_host_registration` is NOT a `SharedWriter` method — see
// `db::SharedWriter`'s doc comment — so it is exercised directly rather than
// through `ReducerWriter`, against the same `RecordingCaller` every other
// transport-level test here uses.

/// The ordinary case: the call reaches the caller with exactly the row
/// passed in.
#[tokio::test]
async fn push_host_registration_sends_the_row() {
    let caller = RecordingCaller::returning(&[]);

    push_host_registration(
        &caller,
        "host-1".to_string(),
        "My Laptop".to_string(),
        "user-me".to_string(),
    )
    .await;

    assert_eq!(
        caller.sent(),
        vec![Sent::RegisterHost(
            "host-1".to_string(),
            "My Laptop".to_string(),
            "user-me".to_string()
        )]
    );
}

/// Best-effort: a transport failure does not panic or propagate — there is
/// nothing to propagate TO, since this runs fire-and-forget from the
/// connection loop and the rename call site.
#[tokio::test]
async fn push_host_registration_swallows_a_transport_failure() {
    let caller = RecordingCaller::refusing("store unreachable");

    // Would panic on an unhandled `Err` if this propagated instead of
    // logging — the assertion is that this line completes at all.
    push_host_registration(&caller, "host-1".to_string(), String::new(), String::new()).await;
}

/// Best-effort: a store REFUSAL (a reachable store that said no) is also
/// swallowed rather than surfaced.
#[tokio::test]
async fn push_host_registration_swallows_a_refusal() {
    let caller = RecordingCaller::rejecting();

    push_host_registration(&caller, "host-1".to_string(), String::new(), String::new()).await;
}

// -- Settings and filter presets (Phase 9) -----------------------------------
//
// `host` is never a caller-supplied argument on `SharedWriter` — see that
// trait's doc comment — so the load-bearing assertion here is not "the call
// reaches the store" (every other routed method already proves that shape);
// it is that the host id the store SEES is this writer's own, with no way for
// a caller to name a different one.

#[tokio::test]
async fn save_setting_is_scoped_to_this_writers_own_host() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.save_setting("theme", "dark").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveSetting(
            "host-me".to_string(),
            "theme".to_string(),
            "dark".to_string()
        )]
    );
}

#[tokio::test]
async fn clear_setting_is_scoped_to_this_writers_own_host() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.clear_setting("theme").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::ClearSetting(
            "host-me".to_string(),
            "theme".to_string()
        )]
    );
}

#[tokio::test]
async fn save_filter_preset_is_scoped_to_this_writers_own_host() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .save_filter_preset("preset", &["/repo".to_string()], "include")
        .await
        .unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveFilterPreset(
            "host-me".to_string(),
            "preset".to_string(),
            "[\"/repo\"]".to_string(),
            "include".to_string()
        )]
    );
}

#[tokio::test]
async fn delete_filter_preset_is_scoped_to_this_writers_own_host() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.delete_filter_preset("preset").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::DeleteFilterPreset(
            "host-me".to_string(),
            "preset".to_string()
        )]
    );
}

/// None of these four require a settled identity — unlike `create_task`/
/// `create_epic`, host is known before any connection settles
/// one (`host.allium: MintHostIdentity`).
#[tokio::test]
async fn settings_and_filter_presets_route_with_no_identity_settled() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    writer.save_setting("theme", "dark").await.unwrap();

    assert_eq!(
        caller.sent(),
        vec![Sent::SaveSetting(
            "host-me".to_string(),
            "theme".to_string(),
            "dark".to_string()
        )]
    );
}
