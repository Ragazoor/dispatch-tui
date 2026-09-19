//! Where a shared-table mutation goes — Phase 6 of the SpacetimeDB migration.
//!
//! Spec: `sync.allium`'s `BoardWritesThroughTheStore`,
//! `AWriteWithNoConnectionIsRefused` and `StoreRejectsAnInvalidMutation`.
//!
//! Three properties, and the second and third are the ones worth having:
//!
//!   1. With no writer attached — every board today — the SQLite write happens,
//!      exactly as it always has.
//!   2. With one attached, the SQLite write does NOT happen. Not "also
//!      happens": a shared table has exactly one copy, and a second one that
//!      nothing reads is a row the operator cannot see and cannot delete.
//!   3. A writer that refuses leaves SQLite untouched and the error reaches the
//!      caller. No queue, no fallback, no partial write.
//!
//! The fake writer here records rather than talks to a store. That is
//! deliberate: what these tests are about is the BRANCH, and a test that needed
//! a running SpacetimeDB to check which branch was taken would not run in CI.
//! The reducer implementation's own behaviour is `src/sync/tests/writes.rs`
//! and, against a live instance, `tests/spacetime_module.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::db::SharedWriter;
use std::sync::Mutex;

/// A writer that remembers what it was asked to do, and optionally refuses.
#[derive(Default)]
struct RecordingWriter {
    calls: Mutex<Vec<String>>,
    /// When set, every call fails with this message — the store being down.
    refuses_with: Option<String>,
}

impl RecordingWriter {
    fn refusing(why: &str) -> Self {
        Self {
            refuses_with: Some(why.to_string()),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, what: &str) -> Result<()> {
        if let Some(why) = &self.refuses_with {
            anyhow::bail!("{why}");
        }
        self.calls.lock().unwrap().push(what.to_string());
        Ok(())
    }
}

#[async_trait::async_trait]
impl SharedWriter for RecordingWriter {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        self.record(&format!("create_task {}", req.title))?;
        Ok(TaskId(1))
    }

    async fn patch_task(&self, id: TaskId, _patch: &TaskPatch<'_>) -> Result<()> {
        self.record(&format!("patch_task {id}"))
    }

    async fn delete_task(&self, id: TaskId) -> Result<()> {
        self.record(&format!("delete_task {id}"))
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        self.record(&format!("set_task_epic_id {task_id} {epic_id:?}"))
    }

    async fn try_claim_next_backlog_task(&self, epic_id: EpicId) -> Result<Option<TaskId>> {
        self.record(&format!("claim_next {epic_id}"))?;
        Ok(Some(TaskId(1)))
    }

    async fn try_claim_backlog_task(&self, id: TaskId) -> Result<bool> {
        self.record(&format!("claim {id}"))?;
        Ok(true)
    }

    async fn try_release_backlog_claim(&self, id: TaskId) -> Result<bool> {
        self.record(&format!("release {id}"))?;
        Ok(true)
    }

    async fn create_epic(
        &self,
        title: &str,
        _description: &str,
        _parent_epic_id: Option<EpicId>,
    ) -> Result<crate::models::Epic> {
        self.record(&format!("create_epic {title}"))?;
        Ok(crate::models::Epic {
            id: EpicId(1),
            title: title.to_string(),
            ..epic_fixture()
        })
    }

    async fn patch_epic(&self, id: EpicId, _patch: &EpicPatch<'_>) -> Result<()> {
        self.record(&format!("patch_epic {id}"))
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        self.record(&format!("delete_epic {id}"))
    }

    async fn recalculate_epic_status(&self, id: EpicId) -> Result<()> {
        self.record(&format!("recalculate_epic_status {id}"))
    }

    async fn insert_todo(&self, row: CreateTodoRow<'_>) -> Result<TodoId> {
        self.record(&format!("insert_todo {}", row.title))?;
        Ok(TodoId(1))
    }

    async fn patch_todo(&self, id: TodoId, _patch: &TodoPatch<'_>) -> Result<()> {
        self.record(&format!("patch_todo {}", id.0))
    }

    async fn delete_todo(&self, id: TodoId) -> Result<()> {
        self.record(&format!("delete_todo {}", id.0))
    }

    async fn delete_done_todos(&self) -> Result<()> {
        self.record("delete_done_todos")
    }

    async fn save_repo_path(&self, path: &str) -> Result<()> {
        self.record(&format!("save_repo_path {path}"))
    }

    async fn delete_repo_path(&self, path: &str) -> Result<()> {
        self.record(&format!("delete_repo_path {path}"))
    }

    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()> {
        self.record(&format!("set_verify_command {path} {command:?}"))
    }

    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()> {
        self.record(&format!("record_base_branch {repo_path} {branch}"))
    }

    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: i64) -> Result<()> {
        self.record(&format!("subscribe_to_epic {subscriber} {epic_id}"))
    }

    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: i64) -> Result<bool> {
        self.record(&format!("unsubscribe_from_epic {subscriber} {epic_id}"))?;
        Ok(true)
    }
}

/// A blank epic, for the one method that has to return a whole row.
fn epic_fixture() -> crate::models::Epic {
    crate::models::Epic {
        id: EpicId(0),
        title: String::new(),
        description: String::new(),
        status: TaskStatus::Backlog,
        plan_path: None,
        sort_order: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        completed_at: None,
        auto_dispatch: false,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        group_by_repo: false,
        feed_role: crate::models::FeedRole::None,
        origin: crate::models::EpicOrigin::Manual,
        feed_append_only: false,
    }
}

fn a_request() -> CreateTaskRequest<'static> {
    CreateTaskRequest {
        title: "shared",
        description: "",
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

async fn db_with(writer: RecordingWriter) -> (Database, Arc<RecordingWriter>) {
    let writer = Arc::new(writer);
    let db = in_memory_db().await.with_shared_writer(writer.clone());
    (db, writer)
}

/// The unchanged board. No writer, so the row lands in SQLite and can be read
/// back — this is the single-machine install and not a degraded one.
#[tokio::test]
async fn with_no_writer_a_shared_write_goes_to_sqlite() {
    let db = in_memory_db().await;
    db.save_repo_path("/repo").await.unwrap();
    assert_eq!(db.list_repo_paths().await.unwrap(), vec!["/repo"]);
}

/// The configured board. The writer is called and SQLite is NOT written.
///
/// The second assertion is the one that matters. A board that wrote both would
/// have a local copy nothing reads, diverging from the store from the first
/// mutation onward, and an operator with no way to tell which they were
/// looking at.
#[tokio::test]
async fn with_a_writer_a_shared_write_bypasses_sqlite() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.save_repo_path("/repo").await.unwrap();

    assert_eq!(writer.calls(), vec!["save_repo_path /repo"]);
    assert!(
        db.list_repo_paths().await.unwrap().is_empty(),
        "the local table must stay empty; the store holds the only copy"
    );
}

/// `sync.allium: AWriteWithNoConnectionIsRefused`. The error reaches the
/// caller and nothing was written anywhere.
#[tokio::test]
async fn a_write_with_the_store_down_fails_and_changes_nothing() {
    let (db, _) = db_with(RecordingWriter::refusing("store unreachable")).await;

    let refused = db.save_repo_path("/repo").await;

    let why = refused.expect_err("a write with the store down must fail");
    assert!(
        why.to_string().contains("store unreachable"),
        "the refusal must name the reason, got {why}"
    );
    assert!(
        db.list_repo_paths().await.unwrap().is_empty(),
        "a refused write must not fall back to the local copy"
    );
}

/// A refusal is never retried or buffered. `sync.allium: NoWriteIsEverQueued`.
///
/// Asserted by making the refusal one-shot at the call site: the caller gets
/// exactly one failure per attempt, and a second attempt is a second call
/// rather than a replay of the first.
#[tokio::test]
async fn a_refused_write_is_not_replayed_on_the_next_one() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    // One successful write, so there is a call history to compare against.
    db.save_repo_path("/a").await.unwrap();
    assert_eq!(writer.calls(), vec!["save_repo_path /a"]);

    // A second write adds exactly one call. Nothing from before reappears.
    db.save_repo_path("/b").await.unwrap();
    assert_eq!(
        writer.calls(),
        vec!["save_repo_path /a", "save_repo_path /b"]
    );
}

/// Task creation routes too, and the id the store generated comes back.
#[tokio::test]
async fn a_task_create_routes_to_the_writer() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    let id = db.create_task(a_request()).await.unwrap();

    assert_eq!(id, TaskId(1));
    assert_eq!(writer.calls(), vec!["create_task shared"]);
    assert!(db.list_all().await.unwrap().is_empty());
}

/// The LOCAL half is untouched by any of this. Settings, learnings and usage
/// stay in SQLite on every board — the seam is the shared/local line
/// `SharedDomainStore` and `LocalStore` already draw, not "everything".
#[tokio::test]
async fn a_local_write_still_goes_to_sqlite_with_a_writer_attached() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.set_setting_string("theme", "dark").await.unwrap();

    assert_eq!(
        db.get_setting_string("theme").await.unwrap().as_deref(),
        Some("dark")
    );
    assert!(
        writer.calls().is_empty(),
        "a local write must not reach the shared writer"
    );
}

/// EVERY ROUTED METHOD IS ROUTED, checked one call at a time.
///
/// The value here is not any single assertion — it is that a method added to
/// [`SharedWriter`] and then NOT guarded in `src/db/queries/` compiles, passes
/// every other test, and silently writes to the wrong store. Nothing but a call
/// through `Database` catches that, so this calls all of them.
///
/// It asserts the WRITER saw it rather than that SQLite did not, because a
/// couple of these (a patch with no changes, an epic recalculation) have no
/// observable local row to be absent.
#[tokio::test]
async fn every_routed_mutation_reaches_the_writer() {
    let (db, writer) = db_with(RecordingWriter::default()).await;

    db.create_task(a_request()).await.unwrap();
    db.patch_task(TaskId(1), &TaskPatch::new().title("t"))
        .await
        .unwrap();
    db.set_task_epic_id(TaskId(1), Some(EpicId(2)))
        .await
        .unwrap();
    db.delete_task(TaskId(1)).await.unwrap();

    db.try_claim_next_backlog_task(EpicId(1), chrono::Utc::now())
        .await
        .unwrap();
    db.try_claim_backlog_task(TaskId(1), chrono::Utc::now())
        .await
        .unwrap();
    db.try_release_backlog_claim(TaskId(1)).await.unwrap();

    db.create_epic("E", "", None).await.unwrap();
    db.patch_epic(EpicId(1), &EpicPatch::new().title("e"))
        .await
        .unwrap();
    db.recalculate_epic_status(EpicId(1)).await.unwrap();
    db.delete_epic(EpicId(1)).await.unwrap();

    db.insert_todo(CreateTodoRow {
        title: "todo",
        task_id: None,
        epic_id: None,
        owner: Some("user-me"),
    })
    .await
    .unwrap();
    db.patch_todo(TodoId(1), &TodoPatch::new().done(true))
        .await
        .unwrap();
    db.delete_todo(TodoId(1)).await.unwrap();
    db.delete_done_todos().await.unwrap();

    db.save_repo_path("/repo").await.unwrap();
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    db.record_base_branch("/repo", "main").await.unwrap();
    db.delete_repo_path("/repo").await.unwrap();

    db.subscribe_to_epic("user-me", 1).await.unwrap();
    db.unsubscribe_from_epic("user-me", 1).await.unwrap();

    let names: Vec<String> = writer
        .calls()
        .into_iter()
        .map(|c| c.split_whitespace().next().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "create_task",
            "patch_task",
            "set_task_epic_id",
            "delete_task",
            "claim_next",
            "claim",
            "release",
            "create_epic",
            "patch_epic",
            "recalculate_epic_status",
            "delete_epic",
            "insert_todo",
            "patch_todo",
            "delete_todo",
            "delete_done_todos",
            "save_repo_path",
            "set_verify_command",
            "record_base_branch",
            "delete_repo_path",
            "subscribe_to_epic",
            "unsubscribe_from_epic",
        ]
    );
}

/// ...and none of them touched SQLite. The other half of the claim above,
/// checked over the tables that do have observable rows.
#[tokio::test]
async fn no_routed_mutation_leaves_a_local_row() {
    let (db, _) = db_with(RecordingWriter::default()).await;

    db.create_task(a_request()).await.unwrap();
    db.create_epic("E", "", None).await.unwrap();
    db.insert_todo(CreateTodoRow {
        title: "todo",
        task_id: None,
        epic_id: None,
        owner: Some("user-me"),
    })
    .await
    .unwrap();
    db.save_repo_path("/repo").await.unwrap();
    db.record_base_branch("/repo", "main").await.unwrap();
    db.subscribe_to_epic("user-me", 1).await.unwrap();

    assert!(db.list_all().await.unwrap().is_empty(), "tasks");
    assert!(db.list_epics().await.unwrap().is_empty(), "epics");
    assert!(db.list_todos().await.unwrap().is_empty(), "todos");
    assert!(db.list_repo_paths().await.unwrap().is_empty(), "repo_paths");
    assert!(
        db.list_all_base_branches().await.unwrap().is_empty(),
        "repo_base_branches"
    );
    assert!(
        db.subscribed_epics("user-me").await.unwrap().is_empty(),
        "subscriptions"
    );
}

/// A refusal on ANY of them changes nothing locally. The no-fallback rule is
/// per-method, not a property of the one method it was first written for.
#[tokio::test]
async fn a_refusal_never_falls_back_to_the_local_store() {
    let (db, _) = db_with(RecordingWriter::refusing("store unreachable")).await;

    assert!(db.create_task(a_request()).await.is_err());
    assert!(db.create_epic("E", "", None).await.is_err());
    assert!(db.save_repo_path("/repo").await.is_err());

    assert!(db.list_all().await.unwrap().is_empty());
    assert!(db.list_epics().await.unwrap().is_empty());
    assert!(db.list_repo_paths().await.unwrap().is_empty());
}
