#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: feed sync end-to-end via `FeedRunner::tick()`.
//!
//! Drives a real shell `feed_command` that emits a JSON `FeedItem` array,
//! and asserts the upsert behaviour through the public DB API.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use dispatch_tui::db::{Database, EpicCrud, EpicPatch};
use dispatch_tui::feed::FeedRunner;
use dispatch_tui::mcp::McpEvent;

use dispatch_tui::process::{MockProcessRunner, ProcessRunner};

/// Always-failing runner: each `git symbolic-ref` call falls back to "main".
struct AlwaysFailRunner;

impl ProcessRunner for AlwaysFailRunner {
    fn run(&self, _program: &str, _args: &[&str]) -> anyhow::Result<std::process::Output> {
        MockProcessRunner::fail("not a git repo")
    }
}

async fn wait_for_refresh(rx: &mut mpsc::UnboundedReceiver<McpEvent>) {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for McpEvent::Refresh")
        .expect("channel closed");
}

#[tokio::test]
async fn feed_sync_creates_then_updates_tasks_via_external_id() {
    let db = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db.create_epic("Feed Epic", "", None).await.unwrap();

    // First feed: 3 items.
    let cmd_v1 = r#"echo '[
        {"external_id":"a","title":"A","description":"","status":"backlog","tag":"bug"},
        {"external_id":"b","title":"B","description":"","status":"backlog","tag":"bug"},
        {"external_id":"c","title":"C","description":"","status":"backlog","tag":"bug"}
    ]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd_v1)))
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let proc_runner: Arc<dyn ProcessRunner> = Arc::new(AlwaysFailRunner);
    let mut runner = FeedRunner::new(db.clone(), tx, proc_runner);

    runner.tick().await;
    wait_for_refresh(&mut rx).await;

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 3, "expected 3 tasks after first feed sync");
    let mut external_ids: Vec<&str> = tasks
        .iter()
        .filter_map(|t| t.external_id.as_deref())
        .collect();
    external_ids.sort();
    assert_eq!(external_ids, vec!["a", "b", "c"]);

    // Second feed: drop "c", change title of "b". The feed command must be
    // changed and `last_run` cleared so the runner re-executes immediately.
    let cmd_v2 = r#"echo '[
        {"external_id":"a","title":"A","description":"","status":"backlog","tag":"bug"},
        {"external_id":"b","title":"B updated","description":"","status":"backlog","tag":"bug"}
    ]'"#;
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some(cmd_v2)))
        .await
        .unwrap();

    // The feed runner enforces a per-epic interval. Build a fresh runner so
    // the interval check passes — the persistent state is in the DB only.
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let proc_runner2: Arc<dyn ProcessRunner> = Arc::new(AlwaysFailRunner);
    let mut runner2 = FeedRunner::new(db.clone(), tx2, proc_runner2);
    runner2.tick().await;
    wait_for_refresh(&mut rx2).await;

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let titles_by_ext: std::collections::HashMap<String, String> = tasks
        .iter()
        .map(|t| (t.external_id.clone().unwrap_or_default(), t.title.clone()))
        .collect();
    assert_eq!(titles_by_ext.get("a").map(String::as_str), Some("A"));
    assert_eq!(
        titles_by_ext.get("b").map(String::as_str),
        Some("B updated"),
        "b should reflect the updated title"
    );
    // Stale feed item "c" is removed (matches `upsert_feed_tasks_removes_stale_items`).
    assert!(
        !titles_by_ext.contains_key("c"),
        "stale feed task c should be removed"
    );
}
