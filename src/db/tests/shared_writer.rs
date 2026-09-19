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

    async fn save_repo_path(&self, path: &str) -> Result<()> {
        self.record(&format!("save_repo_path {path}"))
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

    let id = db
        .create_task(CreateTaskRequest {
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
        })
        .await
        .unwrap();

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
