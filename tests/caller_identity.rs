#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end integration tests for the MCP caller-identity header flow.
//!
//! Exercises the actual production router (middleware + handlers) over a
//! `tower::ServiceExt::oneshot` call to confirm that the header → identity
//! → handler chain works as a unit.

mod common;

use serde_json::{json, Value};

use dispatch_tui::db::CreateTaskRequest;
use dispatch_tui::mcp::identity::{HEADER_KIND, HEADER_TASK_ID};
use dispatch_tui::models::{TaskId, TaskStatus};

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

/// The message of a JSON-RPC error or of an `isError` tool result.
fn error_text(resp: &Value) -> String {
    if let Some(msg) = resp["error"]["message"].as_str() {
        return msg.to_string();
    }
    assert_eq!(
        resp["result"]["isError"],
        json!(true),
        "expected an error response, got: {resp}"
    );
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected error text, got: {resp}"))
        .to_string()
}

/// mcp-task-tools.allium: CreateTaskViaMcp / EveryTaskNamesItsEpicOrNull, over
/// the real router. The header identifies the caller as a task inside an epic,
/// and that epic is still not consulted: an omitted epic_id is refused and
/// nothing is created, while an explicit null files a standalone task.
#[tokio::test]
async fn create_task_via_task_header_does_not_inherit_epic() {
    let (router, db) = common::test_router().await;
    let epic = db.create_epic("e", "", None).await.unwrap();
    let parent_id = db
        .create_task(CreateTaskRequest {
            title: "parent",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let refused = common::post_mcp(
        router.clone(),
        &[(HEADER_TASK_ID, &parent_id.0.to_string())],
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
    let refusal = error_text(&refused);
    assert!(
        refusal.contains("epic_id") && refusal.contains(&format!("#{}", epic.id.0)),
        "refusal must name the argument and the caller's epic, got: {refusal}"
    );
    assert_eq!(
        db.list_all().await.unwrap().len(),
        1,
        "a refused create must leave only the caller's own task"
    );

    let resp = common::post_mcp(
        router,
        &[(HEADER_TASK_ID, &parent_id.0.to_string())],
        json!({
            "jsonrpc": "2.0", "id": 2,
            "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": { "title": "child", "repo_path": "/r", "epic_id": null }
            }
        }),
    )
    .await;

    let new_id = parse_created_task_id(&resp);
    let new_task = db.get_task(new_id).await.unwrap().unwrap();
    assert_eq!(new_task.epic_id, None, "the argument is the effective epic");
}

#[tokio::test]
async fn create_task_via_session_succeeds() {
    let (router, db) = common::test_router().await;
    let resp = common::post_mcp(
        router,
        &[(HEADER_KIND, "session")],
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "tools/call",
            "params": {
                "name": "create_task",
                "arguments": { "title": "t", "repo_path": "/r", "epic_id": null }
            }
        }),
    )
    .await;
    let new_id = parse_created_task_id(&resp);
    let new_task = db.get_task(new_id).await.unwrap().unwrap();
    assert_eq!(new_task.title, "t");
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
                "name": "create_task",
                "arguments": { "title": "t", "repo_path": "/r", "epic_id": null }
            }
        }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32600, "got: {resp}");
    assert_eq!(resp["id"], json!(42), "id must echo request, not be null");
}
