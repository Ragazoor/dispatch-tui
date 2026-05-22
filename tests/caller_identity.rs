#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end integration tests for the MCP caller-identity header flow.
//!
//! Exercises the actual production router (middleware + handlers) over a
//! `tower::ServiceExt::oneshot` call to confirm that the header → identity
//! → handler chain works as a unit.

mod common;

use std::sync::Arc;

use serde_json::{json, Value};

use dispatch_tui::db::{self, CreateTaskRequest};
use dispatch_tui::mcp::identity::{HEADER_KIND, HEADER_TASK_ID};
use dispatch_tui::models::{ProjectId, TaskId, TaskStatus};

async fn create_parent_task(db: &Arc<dyn db::TaskStore>, project_id: ProjectId) -> TaskId {
    db.create_task(CreateTaskRequest {
        title: "parent",
        description: "",
        repo_path: "/r",
        plan: None,
        status: TaskStatus::Running,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id,
        wrap_up_mode: None,
    })
    .await
    .unwrap()
}

fn parse_created_task_id(resp: &Value) -> TaskId {
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("expected created-task text");
    let id_str = text
        .strip_prefix("Task ")
        .and_then(|s| s.strip_suffix(" created"))
        .expect("expected 'Task <id> created'");
    TaskId(id_str.parse().expect("numeric id"))
}

#[tokio::test]
async fn create_task_via_task_header_inherits_project() {
    let (router, db) = common::test_router().await;
    let project_id = ProjectId(1);
    let parent = create_parent_task(&db, project_id).await;

    let resp = common::post_mcp(
        router,
        &[(HEADER_TASK_ID, &parent.0.to_string())],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": { "title": "child", "repo_path": "/r" }
            }
        }),
    )
    .await;

    let new_id = parse_created_task_id(&resp);
    let new_task = db.get_task(new_id).await.unwrap().unwrap();
    assert_eq!(new_task.project_id, project_id);
}

#[tokio::test]
async fn create_task_via_session_with_project_id_succeeds() {
    let (router, db) = common::test_router().await;
    let resp = common::post_mcp(
        router,
        &[(HEADER_KIND, "session")],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": { "title": "t", "repo_path": "/r", "project_id": 1 }
            }
        }),
    )
    .await;
    let new_id = parse_created_task_id(&resp);
    let new_task = db.get_task(new_id).await.unwrap().unwrap();
    assert_eq!(new_task.project_id, ProjectId(1));
}

#[tokio::test]
async fn missing_identity_headers_still_allows_initialize() {
    let (router, _db) = common::test_router().await;
    let resp = common::post_mcp(
        router,
        &[],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "initialize",
            "params": {}
        }),
    )
    .await;
    assert!(resp.get("error").is_none(), "got: {resp}");
    assert_eq!(resp["id"], json!(1), "got: {resp}");
    assert_eq!(
        resp["result"]["protocolVersion"].as_str(),
        Some("2025-06-18"),
        "got: {resp}"
    );
}

#[tokio::test]
async fn missing_identity_headers_on_tools_call_returns_32600_with_request_id() {
    let (router, _db) = common::test_router().await;
    let resp = common::post_mcp(
        router,
        &[],
        json!({
            "jsonrpc": "2.0", "id": 42,
            "method": "tools/call",
            "params": {
                "name": "list_projects",
                "arguments": {}
            }
        }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32600, "got: {resp}");
    assert_eq!(resp["id"], json!(42), "id must echo request, not be null");
}
