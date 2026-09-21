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
    EpicId, NotificationWrite, StopOutcome, SubStatus, SubagentDrain, TaskId, TaskStatus,
    UserPromptOutcome,
};
use crate::service::{Clock, FixedClock};
use crate::spacetime::bindings;
use crate::sync::writes::{
    DrainReadBack, ReducerCaller, ReducerOutcome, ReducerWriter, WriterIdentity,
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
    CreateTodo(Box<bindings::Todo>),
    PatchTodo(i64),
    DeleteTodo(i64),
    DeleteDoneTodos(String),
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
    ShellStart(i64, String, String, String),
    ShellStop(i64, String, String),
    ShellClearNoDrain(i64),
    TryRecordStop(i64, String),
    RecordPreToolUse(i64, String, String),
    RecordNotification(i64, String, String),
    RecordUserPromptSubmit(i64, String, String),
    MarkPrLearningsGateShown(i64, String),
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
    /// What `subagent_start`/`shell_start` answer with.
    live_count: i64,
    /// What `subagent_stop`/`subagent_clear`/`shell_stop` answer with.
    drain: DrainReadBack,
    /// What `try_record_stop` answers with on success; `None` combined with
    /// `rejects`/`refuses_with` unset is itself a valid "deferred" answer, so
    /// `rejecting()` — not this field — is how a test asks for the refusal.
    stop_flag: bool,
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

    /// A reachable store that applies `subagent_start`/`shell_start` and
    /// answers with `n` as the live count.
    fn returning_count(n: i64) -> Self {
        Self {
            live_count: n,
            ..Self::default()
        }
    }

    /// A reachable store that applies a drain (`subagent_stop`/
    /// `subagent_clear`/`shell_stop`) and answers with this read-back.
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

    async fn create_todo(&self, row: bindings::Todo) -> anyhow::Result<i64> {
        self.record(Sent::CreateTodo(Box::new(row)))?;
        Ok(5)
    }

    async fn patch_todo(
        &self,
        id: i64,
        _patch: bindings::TodoPatch,
    ) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::PatchTodo(id))
    }

    async fn delete_todo(&self, id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteTodo(id))
    }

    async fn delete_done_todos(&self, owner: String) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::DeleteDoneTodos(owner))
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

    async fn shell_start(
        &self,
        task_id: i64,
        shell_id: String,
        session_id: String,
        started_at: String,
    ) -> anyhow::Result<i64> {
        self.record(Sent::ShellStart(task_id, shell_id, session_id, started_at))?;
        Ok(self.live_count)
    }

    async fn shell_stop(
        &self,
        task_id: i64,
        shell_id: String,
        session_id: String,
    ) -> anyhow::Result<DrainReadBack> {
        self.record(Sent::ShellStop(task_id, shell_id, session_id))?;
        Ok(self.drain)
    }

    async fn shell_clear_no_drain(&self, task_id: i64) -> anyhow::Result<ReducerOutcome> {
        self.answer(Sent::ShellClearNoDrain(task_id))
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
/// adversarial review of this plan's implementation caught twice
/// (`shell_start`, `try_record_stop`) before this file existed to catch it
/// again.
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

/// Clearing done todos is scoped to THIS PERSON. On one machine "every done
/// todo" was every done todo there was; on a shared store it would be every
/// colleague's completed checklist.
#[tokio::test]
async fn clearing_done_todos_names_whose_checklist() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer.delete_done_todos().await.unwrap();

    assert_eq!(caller.sent(), vec![Sent::DeleteDoneTodos("user-me".into())]);
}

/// ...and a board with no identity cannot do it at all, rather than clearing
/// everybody's.
#[tokio::test]
async fn a_board_with_no_identity_cannot_clear_a_checklist() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    assert!(writer.delete_done_todos().await.is_err());
    assert!(caller.sent().is_empty());
}

/// A todo create carries THIS CONNECTION's own settled identity, not whatever
/// `CreateTodoRow.owner` says — `sync.allium: CreatesRequireASettledIdentity`.
/// Passing `owner: None` here and still getting "user-me" out is the point:
/// `TodoService`'s pre-resolved value can predate this connection's own
/// handshake (see `require_identity`'s doc comment), so the writer asks the
/// live cell itself rather than trusting it.
#[tokio::test]
async fn a_todo_create_carries_this_connections_settled_identity() {
    let (writer, caller) = writer_with(RecordingCaller::default());

    writer
        .insert_todo(crate::db::CreateTodoRow {
            title: "buy milk",
            task_id: None,
            epic_id: None,
            owner: None,
        })
        .await
        .unwrap();

    let Some(Sent::CreateTodo(row)) = caller.sent().into_iter().next() else {
        panic!("expected a create");
    };
    assert_eq!(row.owner, "user-me");
}

/// A board with no settled identity cannot create a todo either —
/// `sync.allium: CreatesRequireASettledIdentity`. Closes the gap where a
/// shared-store todo could previously land unowned and permanently invisible
/// to its own creator.
#[tokio::test]
async fn a_board_with_no_identity_cannot_create_a_todo() {
    let (writer, caller) = writer_with_no_identity(RecordingCaller::default());

    let refused = writer
        .insert_todo(crate::db::CreateTodoRow {
            title: "buy milk",
            task_id: None,
            epic_id: None,
            owner: None,
        })
        .await;

    assert!(refused.is_err(), "a todo create needs a name to stamp");
    assert!(caller.sent().is_empty());
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
// (subagent_start/shell_start, and the plain applied()/won() wrappers).
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

/// `subagent_start`/`shell_start` never refuse and answer with the live count
/// the module read back, not a placeholder.
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

/// `started_at` is RFC 3339 for a subagent — this column is never compared,
/// unlike the shell twin, which uses the module's fixed-width millis format
/// instead (see `shell_start_stamps_the_fixed_width_format_of_the_passed_now` below).
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

#[tokio::test]
async fn shell_start_reads_the_live_count_back() {
    let (writer, caller) = writer_with(RecordingCaller::returning_count(2));

    let live = writer
        .shell_start(TaskId(1), "shell-1", "session-1", at())
        .await
        .unwrap();

    assert_eq!(live, 2);
    assert_eq!(
        caller.sent(),
        vec![Sent::ShellStart(
            1,
            "shell-1".into(),
            "session-1".into(),
            AT_STORED.to_string(),
        )]
    );
}

/// The module compares `oldest_live_shell_started_at` lexicographically
/// (`MIN`), so this column's fixed-width format is load-bearing — unlike the
/// subagent twin's RFC 3339, which is never compared.
#[tokio::test]
async fn shell_start_stamps_the_fixed_width_format_of_the_passed_now() {
    let (writer, caller) = writer_with(RecordingCaller::returning_count(1));

    writer
        .shell_start(TaskId(1), "shell-1", "session-1", call_at())
        .await
        .unwrap();

    let Some(Sent::ShellStart(_, _, _, started_at)) = caller.sent().into_iter().next() else {
        panic!("expected a shell start");
    };
    assert_eq!(started_at, CALL_AT_STORED);
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

/// `shell_stop` shares the exact same drain-answer decoding as `subagent_stop`
/// — see `apply_pending_stop_if_drained`, the shared predicate both route
/// through in the module.
#[tokio::test]
async fn a_shell_stop_that_drains_a_running_task_reports_the_flip() {
    let (writer, _) = writer_seeded_with(
        1,
        TaskStatus::Running,
        RecordingCaller::returning_drain(0, true),
    );

    let drain = writer
        .shell_stop(TaskId(1), "shell-1", "session-1")
        .await
        .unwrap();

    assert!(drain.applied_pending_stop);
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

#[tokio::test]
async fn shell_clear_no_drain_is_a_plain_applied_call() {
    let (writer, caller) = writer_with(RecordingCaller::returning(&[]));

    writer.shell_clear_no_drain(TaskId(1)).await.unwrap();

    assert_eq!(caller.sent(), vec![Sent::ShellClearNoDrain(1)]);
}

/// The instant the fixed clock in `writer_with`/`writer_seeded_with` sits at.
fn at() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(AT)
        .unwrap()
        .with_timezone(&chrono::Utc)
}
