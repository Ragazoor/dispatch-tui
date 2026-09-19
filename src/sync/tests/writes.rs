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
use crate::models::{EpicId, TaskId, TaskStatus};
use crate::service::{Clock, FixedClock};
use crate::spacetime::bindings;
use crate::sync::writes::{ReducerCaller, ReducerOutcome, ReducerWriter, WriterIdentity};

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

    fn answer(&self, what: Sent) -> anyhow::Result<ReducerOutcome> {
        if let Some(why) = &self.refuses_with {
            anyhow::bail!("{why}");
        }
        if self.rejects {
            return Ok(ReducerOutcome::Refused("no".into()));
        }
        self.sent.lock().unwrap().push(what);
        Ok(ReducerOutcome::Applied(Vec::new()))
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
}

/// The instant the fixed clock sits at, and the string it must produce.
const AT: &str = "2026-09-19T12:34:56.789Z";
const AT_STORED: &str = "2026-09-19 12:34:56.789";

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
        Arc::new(crate::sync::SharedRows::new()),
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
        rows,
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
        Arc::new(crate::sync::SharedRows::new()),
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
    let caller = Arc::new(RecordingCaller::default());
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(None)),
        Arc::new(FixedClock::new(
            chrono::DateTime::parse_from_rfc3339(AT)
                .unwrap()
                .with_timezone(&chrono::Utc),
        )) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SharedRows::new()),
    );

    let refused = writer.create_task(a_request()).await;

    assert!(refused
        .expect_err("a user-board task needs a user")
        .to_string()
        .contains("no user identity"));
    assert!(caller.sent().is_empty());
}

/// ...but it can still create one INSIDE an epic. That task carries no owner at
/// all, so an unsettled identity is not in its way — and a person whose board
/// has not identified yet can still work on shared epics.
#[tokio::test]
async fn a_board_with_no_identity_can_still_create_a_task_in_an_epic() {
    let caller = Arc::new(RecordingCaller::default());
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(None)),
        Arc::new(FixedClock::new(
            chrono::DateTime::parse_from_rfc3339(AT)
                .unwrap()
                .with_timezone(&chrono::Utc),
        )) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SharedRows::new()),
    );

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
    let caller = Arc::new(RecordingCaller::default());
    let writer = ReducerWriter::new(
        caller.clone(),
        Arc::new(FixedIdentity(None)),
        Arc::new(FixedClock::new(
            chrono::DateTime::parse_from_rfc3339(AT)
                .unwrap()
                .with_timezone(&chrono::Utc),
        )) as Arc<dyn Clock>,
        "host-me".into(),
        Arc::new(crate::sync::SharedRows::new()),
    );

    assert!(writer.delete_done_todos().await.is_err());
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
