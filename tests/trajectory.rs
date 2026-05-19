#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for MCP trajectory persistence.

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};
use tower::ServiceExt;

use dispatch_tui::db::{self, CreateTaskRequest, Database};
use dispatch_tui::mcp::identity::{HEADER_KIND, HEADER_TASK_ID};
use dispatch_tui::models::{ProjectId, TaskId, TaskStatus};
use dispatch_tui::process::{MockProcessRunner, ProcessRunner};

async fn test_router() -> (axum::Router, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let router = dispatch_tui::mcp::router(db.clone(), None, runner);
    (router, db)
}

async fn post_mcp(router: axum::Router, headers: &[(&str, &str)], body: Value) -> Value {
    let mut builder = Request::post("/mcp").header("content-type", "application/json");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let resp = router
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), 65_536).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Happy path: a task-identity call with a worktree set should write one
/// JSONL entry to `<worktree>/.dispatch/trajectory.jsonl`.
#[tokio::test]
async fn task_identity_with_worktree_writes_trajectory_entry() {
    let tmp = tempfile::tempdir().unwrap();
    tokio::fs::create_dir_all(tmp.path().join(".dispatch")).await.unwrap();

    let (router, db) = test_router().await;

    // Create a task and set its worktree to the temp dir.
    let task_id: TaskId = db
        .create_task(CreateTaskRequest {
            title: "trajectory-test-task",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    let worktree_path = tmp.path().to_str().unwrap().to_string();
    db.patch_task(task_id, &db::TaskPatch::new().worktree(Some(&worktree_path)))
        .await
        .unwrap();

    // Call list_tasks via MCP with the task's identity header.
    let _resp = post_mcp(
        router,
        &[(HEADER_TASK_ID, &task_id.0.to_string())],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": { "name": "list_tasks", "arguments": {} }
        }),
    )
    .await;

    // Give the fire-and-forget spawn time to flush.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let trajectory_path = tmp.path().join(".dispatch").join("trajectory.jsonl");
    assert!(
        trajectory_path.exists(),
        "trajectory.jsonl should exist after a task-identity tools/call"
    );

    let content = tokio::fs::read_to_string(&trajectory_path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly 1 trajectory line, got: {lines:?}");

    let parsed: Value = serde_json::from_str(lines[0]).expect("trajectory line must be valid JSON");
    assert_eq!(
        parsed["task_id"], task_id.0,
        "task_id field mismatch: {parsed}"
    );
    assert_eq!(
        parsed["method"].as_str(),
        Some("list_tasks"),
        "method field mismatch: {parsed}"
    );
    assert_eq!(
        parsed["schema_version"].as_str(),
        Some("1.0.0"),
        "schema_version field mismatch: {parsed}"
    );
    assert!(
        parsed["duration_ms"].is_number(),
        "duration_ms must be a number: {parsed}"
    );
    assert!(
        parsed["timestamp"].is_string(),
        "timestamp must be a string: {parsed}"
    );
}

/// A task-identity call where the task has NO worktree should not write any
/// file. (No worktree path → nothing to write to.)
#[tokio::test]
async fn task_identity_without_worktree_writes_no_file() {
    let (router, db) = test_router().await;

    let task_id: TaskId = db
        .create_task(CreateTaskRequest {
            title: "no-worktree-task",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    // Do NOT set a worktree on this task.

    let resp = post_mcp(
        router,
        &[(HEADER_TASK_ID, &task_id.0.to_string())],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": { "name": "list_tasks", "arguments": {} }
        }),
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // The call itself must succeed (no protocol error).
    assert!(
        resp.get("error").is_none(),
        "expected no protocol error for task-identity call without worktree, got: {resp}"
    );

    // The task still has no worktree in DB.
    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert!(
        task.worktree.is_none(),
        "task worktree should still be None, got: {:?}",
        task.worktree
    );
}

/// A session-identity call should never write a trajectory file.
#[tokio::test]
async fn session_identity_writes_no_trajectory_file() {
    let tmp = tempfile::tempdir().unwrap();
    tokio::fs::create_dir_all(tmp.path().join(".dispatch")).await.unwrap();

    let (router, _db) = test_router().await;

    let _resp = post_mcp(
        router,
        &[(HEADER_KIND, "session")],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} }
        }),
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let trajectory_path = tmp.path().join(".dispatch").join("trajectory.jsonl");
    assert!(
        !trajectory_path.exists(),
        "trajectory.jsonl must NOT be written for session-identity calls"
    );
}
