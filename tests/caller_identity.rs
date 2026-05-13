#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end integration tests for the MCP caller-identity header flow.
//!
//! Exercises the actual production router (middleware + handlers) over a
//! `tower::ServiceExt::oneshot` call to confirm that the header → identity
//! → handler chain works as a unit.

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
    })
    .await
    .unwrap()
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
    let (router, db) = test_router().await;
    let project_id = ProjectId(1);
    let parent = create_parent_task(&db, project_id).await;

    let resp = post_mcp(
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
async fn create_task_via_session_without_project_id_returns_32602() {
    let (router, _db) = test_router().await;
    let resp = post_mcp(
        router,
        &[(HEADER_KIND, "session")],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": { "title": "t", "repo_path": "/r" }
            }
        }),
    )
    .await;
    // tools/call returns the error inside the result envelope as JSON-RPC,
    // not at the top level — match either shape that the handler may emit.
    let err_code = resp["error"]["code"]
        .as_i64()
        .or_else(|| resp["result"]["error"]["code"].as_i64());
    assert_eq!(err_code, Some(-32602), "got: {resp}");
}

#[tokio::test]
async fn create_task_via_session_with_project_id_succeeds() {
    let (router, db) = test_router().await;
    let resp = post_mcp(
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
async fn missing_identity_headers_returns_32600() {
    let (router, _db) = test_router().await;
    let resp = post_mcp(
        router,
        &[],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "initialize",
            "params": {}
        }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32600, "got: {resp}");
}
