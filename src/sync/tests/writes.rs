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
use crate::sync::writes::{ReducerCaller, ReducerWriter, WriterIdentity};

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
    SaveRepoPath(String, String),
}

#[derive(Default)]
struct RecordingCaller {
    sent: Mutex<Vec<Sent>>,
    /// The store's refusal, when there is one. `sync.allium` makes no
    /// distinction between "unreachable" and "rejected" at this layer: both are
    /// a write that did not happen, carrying a reason.
    refuses_with: Option<String>,
}

impl RecordingCaller {
    fn refusing(why: &str) -> Self {
        Self {
            refuses_with: Some(why.to_string()),
            ..Self::default()
        }
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

    async fn patch_task(&self, id: TaskId, patch: bindings::TaskPatch) -> anyhow::Result<()> {
        self.record(Sent::PatchTask(id, Box::new(patch)))
    }

    async fn delete_task(&self, id: TaskId) -> anyhow::Result<()> {
        self.record(Sent::DeleteTask(id))
    }

    async fn save_repo_path(&self, path: String, last_used: String) -> anyhow::Result<()> {
        self.record(Sent::SaveRepoPath(path, last_used))
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
    );
    (writer, caller)
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
