use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::db::{self, Database};
use crate::mcp::McpState;
use crate::models::{SubStatus, TaskStatus};
use crate::process::{MockProcessRunner, ProcessRunner};

use super::dispatch::{handle_mcp, tool_definitions};
use super::epics::{CreateEpicArgs, GetEpicArgs, UpdateEpicArgs};
use super::review::{
    DispatchFixAgentArgs, DispatchReviewAgentArgs, GetReviewPrArgs, GetSecurityAlertArgs,
    ListReviewPrsArgs, ListSecurityAlertsArgs,
};
use super::tasks::{
    ClaimTaskArgs, CreateTaskWithEpicArgs, DispatchNextArgs, GetTaskArgs, ListTasksArgs,
    ReportUsageArgs, SendMessageArgs, UpdateTaskArgs, WrapUpArgs,
};
use super::types::{JsonRpcRequest, JsonRpcResponse};

fn test_state() -> Arc<McpState> {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    Arc::new(McpState {
        db,
        notify_tx: None,
        runner,
    })
}

async fn call(state: &Arc<McpState>, method: &str, params: Option<Value>) -> JsonRpcResponse {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params,
    };
    let (_, Json(response)) = handle_mcp(State(state.clone()), Json(req)).await;
    response
}

// -- Shared helpers --------------------------------------------------------

/// Create a task with sensible defaults, returning the TaskId.
fn create_task_fixture(state: &Arc<McpState>) -> crate::models::TaskId {
    state
        .db
        .create_task(
            "Test Task",
            "test description",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap()
}

/// Assert response is an error whose message contains `substr`.
fn assert_error(resp: &JsonRpcResponse, substr: &str) {
    let err = resp.error.as_ref().unwrap_or_else(|| {
        panic!(
            "expected error containing {substr:?}, got success: {:?}",
            resp.result
        )
    });
    assert!(
        err.message.contains(substr),
        "expected error containing {substr:?}, got: {:?}",
        err.message,
    );
}

/// Extract the text content from a successful MCP response.
fn extract_response_text(resp: &JsonRpcResponse) -> String {
    let result = resp
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("expected success, got error: {:?}", resp.error));
    result["content"][0]["text"]
        .as_str()
        .expect("missing text in response content")
        .to_string()
}

#[tokio::test]
async fn initialize_returns_capabilities() {
    let state = test_state();
    let resp = call(&state, "initialize", None).await;
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn tools_list_returns_tools() {
    let state = test_state();
    let resp = call(&state, "tools/list", None).await;
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in super::dispatch::TOOL_NAMES {
        assert!(
            names.contains(expected),
            "tools/list missing tool: {expected}"
        );
    }
    assert_eq!(names.len(), super::dispatch::TOOL_NAMES.len());
}

#[tokio::test]
async fn update_task_valid() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Running);
}

#[tokio::test]
async fn update_task_invalid_status() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "bogus" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

#[tokio::test]
async fn update_task_rejects_done_status() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
    )
    .await;
    assert_error(&resp, "Cannot set status to done or archived via MCP");

    // Verify task status unchanged
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_ne!(task.status, crate::models::TaskStatus::Done);

    // Also verify archived is rejected
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "archived" }
        })),
    )
    .await;
    assert_error(&resp, "Cannot set status to done or archived via MCP");
}

#[tokio::test]
async fn update_task_still_allows_other_statuses() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    for status in &["running", "review", "ready", "backlog"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "status": status }
            })),
        )
        .await;
        assert!(
            resp.error.is_none(),
            "status={status} should be allowed, got: {:?}",
            resp.error
        );
    }
}

#[tokio::test]
async fn update_task_missing_args() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "update_task", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_some());
}

#[tokio::test]
async fn get_task_found() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("My Task"));
}

#[tokio::test]
async fn get_task_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("not found"));
}

#[tokio::test]
async fn unknown_tool() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "bogus_tool", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("Unknown tool"));
}

#[tokio::test]
async fn unknown_method() {
    let state = test_state();
    let resp = call(&state, "bogus/method", None).await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("Method not found"));
}

#[tokio::test]
async fn create_task_minimal() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "New Task", "repo_path": "/my/repo" }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("created"));

    // Verify task was created in DB
    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "New Task");
    assert_eq!(tasks[0].status, TaskStatus::Backlog);
    assert!(tasks[0].plan_path.is_none());
}

#[tokio::test]
async fn create_task_with_plan_stays_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let plan_file = dir.path().join("plan.md");
    std::fs::write(&plan_file, "# Plan").unwrap();

    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Planned Task",
                "repo_path": "/my/repo",
                "plan_path": plan_file.to_string_lossy()
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Backlog);
    let stored = tasks[0].plan_path.as_deref().unwrap();
    assert!(
        std::path::Path::new(stored).is_absolute(),
        "plan path should be absolute, got: {stored}"
    );
    assert_eq!(
        stored,
        std::fs::canonicalize(&plan_file).unwrap().to_string_lossy()
    );
}

#[tokio::test]
async fn create_task_with_description() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Described Task",
                "repo_path": "/repo",
                "description": "Some details"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks[0].description, "Some details");
}

#[tokio::test]
async fn create_task_missing_title() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "repo_path": "/repo" }
        })),
    )
    .await;
    assert!(resp.error.is_some());
}

// -- String task_id coercion (Claude Code sends integers as strings) ------

#[tokio::test]
async fn update_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0.to_string(), "status": "running" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_task should accept string task_id, got: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Running);
}

#[tokio::test]
async fn get_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0.to_string() }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "get_task should accept string task_id, got: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("My Task"));
}

#[tokio::test]
async fn update_task_with_plan() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "ready", "plan_path": "/path/to/plan.md" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
    assert_eq!(task.plan_path.as_deref(), Some("/path/to/plan.md"));
}

#[tokio::test]
async fn update_task_title_only() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Old",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "New Title" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should succeed with title only: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.title, "New Title");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog); // unchanged
}

#[tokio::test]
async fn update_task_status_optional() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "Renamed" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.title, "Renamed");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
}

#[tokio::test]
async fn update_task_title_and_description() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Old",
            "old desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "New", "description": "new desc" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.title, "New");
    assert_eq!(task.description, "new desc");
}

#[tokio::test]
async fn update_task_repo_path() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/old/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "repo_path": "/new/repo" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should succeed with repo_path only: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.repo_path, "/new/repo");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog); // unchanged
}

#[tokio::test]
async fn update_task_no_fields_errors() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(
        resp.error.is_some(),
        "should error with no fields to update"
    );
}

#[tokio::test]
async fn patch_task_sets_multiple_fields() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "Desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "status": "ready",
                "title": "Updated Title"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.title, "Updated Title");
}

#[tokio::test]
async fn update_task_without_plan_preserves_existing() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            Some("/existing.md"),
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "ready" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.plan_path.as_deref(),
        Some("/existing.md"),
        "plan should be preserved when not provided"
    );
}

#[tokio::test]
async fn update_task_sets_pr_fields() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "PR test",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "pr_url": "https://github.com/org/repo/pull/99"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "Expected success, got: {:?}",
        resp.error
    );

    let updated = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        updated.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/99")
    );
}

// -- list_tasks tests -------------------------------------------------------

#[tokio::test]
async fn list_tasks_returns_all_when_no_filter() {
    let state = test_state();
    state
        .db
        .create_task(
            "Task A",
            "desc a",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Task B",
            "desc b",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Task A"));
    assert!(text.contains("Task B"));
}

#[tokio::test]
async fn list_tasks_filters_by_single_status() {
    let state = test_state();
    state
        .db
        .create_task(
            "Backlog Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Running Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "backlog" } })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Backlog Task"));
    assert!(!text.contains("Running Task"));
}

#[tokio::test]
async fn list_tasks_filters_by_multiple_statuses() {
    let state = test_state();
    state
        .db
        .create_task(
            "Backlog Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Running Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Review Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": ["backlog", "running"] } })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Backlog Task"));
    assert!(text.contains("Running Task"));
    assert!(!text.contains("Review Task"));
}

#[tokio::test]
async fn list_tasks_empty_result() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "running" } })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("No tasks found"));
}

// -- claim_task tests -------------------------------------------------------

#[tokio::test]
async fn claim_task_success() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Claimable",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other-task",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "claim should succeed: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(
        task.worktree.as_deref(),
        Some("/repo/.worktrees/5-other-task")
    );
    assert_eq!(task.tmux_window.as_deref(), Some("task-5"));
}

#[tokio::test]
async fn claim_task_rejects_running_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Running",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("already"));
}

#[tokio::test]
async fn claim_task_rejects_different_repo() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Other Repo",
            "desc",
            "/other-repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other-task",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("repo"));
}

#[tokio::test]
async fn claim_task_not_found() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": 9999,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("not found"));
}

#[tokio::test]
async fn report_usage_stores_and_accumulates() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    // First session
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "report_usage",
            "arguments": {
                "task_id": task_id.0,
                "cost_usd": 0.10,
                "input_tokens": 1000,
                "output_tokens": 500
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "first call failed: {:?}", resp.error);

    // Second session — should accumulate
    let resp2 = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "report_usage",
            "arguments": {
                "task_id": task_id.0,
                "cost_usd": 0.05,
                "input_tokens": 500,
                "output_tokens": 250,
                "cache_read_tokens": 100,
                "cache_write_tokens": 50
            }
        })),
    )
    .await;
    assert!(
        resp2.error.is_none(),
        "second call failed: {:?}",
        resp2.error
    );

    let all = state.db.get_all_usage().unwrap();
    assert_eq!(all.len(), 1);
    let u = &all[0];
    assert_eq!(u.task_id, task_id);
    assert!((u.cost_usd - 0.15).abs() < 1e-9);
    assert_eq!(u.input_tokens, 1_500);
    assert_eq!(u.output_tokens, 750);
    assert_eq!(u.cache_read_tokens, 100);
    assert_eq!(u.cache_write_tokens, 50);
}

#[tokio::test]
async fn report_usage_unknown_task_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "report_usage",
            "arguments": {
                "task_id": 9999,
                "cost_usd": 0.10,
                "input_tokens": 1000,
                "output_tokens": 500
            }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

/// Validates that JSON schemas in `tool_definitions()` match the argument structs.
/// Catches drift when a field is added/removed from a struct but not the schema
/// (or vice versa). The `expected_props` lists must be kept in sync with struct
/// fields — if you add a field to a struct, add it here too; the test then fails
/// if the schema wasn't also updated.
#[test]
fn tool_schemas_match_arg_structs() {
    use std::collections::BTreeSet;

    let defs = tool_definitions();
    let tools_arr = defs["tools"].as_array().unwrap();
    let tools: std::collections::HashMap<&str, &Value> = tools_arr
        .iter()
        .map(|t| (t["name"].as_str().unwrap(), t))
        .collect();

    fn schema_props(tool: &Value) -> (BTreeSet<&str>, BTreeSet<&str>) {
        let schema = &tool["inputSchema"];
        let props: BTreeSet<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        let required: BTreeSet<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(|v| v.as_str().unwrap()).collect())
            .unwrap_or_default();
        (props, required)
    }

    // (tool_name, expected_properties, expected_required, full_payload)
    // expected_properties MUST match the struct fields — update when structs change.
    let cases: Vec<(&str, BTreeSet<&str>, BTreeSet<&str>, Value)> = vec![
        (
            "update_task",
            BTreeSet::from([
                "task_id",
                "status",
                "plan_path",
                "title",
                "description",
                "repo_path",
                "sort_order",
                "pr_url",
                "tag",
                "sub_status",
                "epic_id",
                "base_branch",
                "project_id",
            ]),
            BTreeSet::from(["task_id"]),
            json!({"task_id": 1, "status": "review", "plan_path": "/p.md", "title": "t", "description": "d", "repo_path": "/r", "sort_order": 100, "pr_url": "https://github.com/org/repo/pull/1", "tag": "bug", "sub_status": "awaiting_review", "epic_id": 5}),
        ),
        (
            "get_task",
            BTreeSet::from(["task_id"]),
            BTreeSet::from(["task_id"]),
            json!({"task_id": 1}),
        ),
        (
            "create_task",
            BTreeSet::from([
                "title",
                "repo_path",
                "description",
                "plan_path",
                "epic_id",
                "sort_order",
                "tag",
                "base_branch",
                "project_id",
            ]),
            BTreeSet::from(["title", "repo_path"]),
            json!({"title": "t", "repo_path": "/r", "description": "d", "plan_path": "/p.md", "sort_order": 10, "tag": "feature"}),
        ),
        (
            "list_tasks",
            BTreeSet::from(["status", "epic_id", "project_id", "repo_paths", "caller_task_id"]),
            BTreeSet::new(),
            json!({"status": "backlog", "epic_id": 1, "project_id": 1, "repo_paths": ["/r"], "caller_task_id": 1}),
        ),
        (
            "claim_task",
            BTreeSet::from(["task_id", "worktree", "tmux_window"]),
            BTreeSet::from(["task_id", "worktree", "tmux_window"]),
            json!({"task_id": 1, "worktree": "/w", "tmux_window": "tw"}),
        ),
        (
            "create_epic",
            BTreeSet::from([
                "title",
                "repo_path",
                "description",
                "sort_order",
                "parent_epic_id",
                "project_id",
            ]),
            BTreeSet::from(["title", "repo_path"]),
            json!({"title": "Epic", "repo_path": "/repo", "sort_order": 5}),
        ),
        (
            "get_epic",
            BTreeSet::from(["epic_id"]),
            BTreeSet::from(["epic_id"]),
            json!({"epic_id": 1}),
        ),
        ("list_epics", BTreeSet::new(), BTreeSet::new(), json!({})),
        (
            "update_epic",
            BTreeSet::from([
                "epic_id",
                "title",
                "description",
                "status",
                "plan_path",
                "sort_order",
                "repo_path",
                "project_id",
            ]),
            BTreeSet::from(["epic_id"]),
            json!({"epic_id": 1, "plan_path": "docs/plan.md", "sort_order": 42, "repo_path": "/new/path"}),
        ),
        (
            "wrap_up",
            BTreeSet::from(["task_id", "action"]),
            BTreeSet::from(["task_id", "action"]),
            json!({"task_id": 1, "action": "rebase"}),
        ),
        (
            "report_usage",
            BTreeSet::from([
                "task_id",
                "cost_usd",
                "input_tokens",
                "output_tokens",
                "cache_read_tokens",
                "cache_write_tokens",
            ]),
            BTreeSet::from(["task_id", "cost_usd", "input_tokens", "output_tokens"]),
            json!({"task_id": 1, "cost_usd": 0.42, "input_tokens": 1000,
                   "output_tokens": 500, "cache_read_tokens": 100, "cache_write_tokens": 50}),
        ),
        (
            "send_message",
            BTreeSet::from(["from_task_id", "to_task_id", "body"]),
            BTreeSet::from(["from_task_id", "to_task_id", "body"]),
            json!({"from_task_id": 1, "to_task_id": 2, "body": "Hello from task 1"}),
        ),
        (
            "dispatch_next",
            BTreeSet::from(["epic_id"]),
            BTreeSet::from(["epic_id"]),
            json!({"epic_id": 1}),
        ),
        (
            "dispatch_task",
            BTreeSet::from(["task_id"]),
            BTreeSet::from(["task_id"]),
            json!({"task_id": 1}),
        ),
        (
            "update_review_status",
            BTreeSet::from(["repo", "number", "status"]),
            BTreeSet::from(["repo", "number", "status"]),
            json!({"repo": "acme/app", "number": 42, "status": "findings_ready"}),
        ),
        (
            "list_review_prs",
            BTreeSet::from(["mode", "repo"]),
            BTreeSet::new(),
            json!({"mode": "reviewer"}),
        ),
        (
            "get_review_pr",
            BTreeSet::from(["repo", "number"]),
            BTreeSet::from(["repo", "number"]),
            json!({"repo": "acme/app", "number": 42}),
        ),
        (
            "dispatch_review_agent",
            BTreeSet::from(["repo", "number", "local_repo"]),
            BTreeSet::from(["repo", "number", "local_repo"]),
            json!({"repo": "acme/app", "number": 42, "local_repo": "/tmp/repo"}),
        ),
        (
            "list_security_alerts",
            BTreeSet::from(["repo", "severity", "kind"]),
            BTreeSet::new(),
            json!({"kind": "dependabot"}),
        ),
        (
            "get_security_alert",
            BTreeSet::from(["repo", "number", "kind"]),
            BTreeSet::from(["repo", "number", "kind"]),
            json!({"repo": "acme/api", "number": 7, "kind": "dependabot"}),
        ),
        (
            "dispatch_fix_agent",
            BTreeSet::from(["repo", "number", "kind", "local_repo"]),
            BTreeSet::from(["repo", "number", "kind", "local_repo"]),
            json!({"repo": "acme/api", "number": 7, "kind": "dependabot", "local_repo": "/tmp/repo"}),
        ),
        ("list_projects", BTreeSet::new(), BTreeSet::new(), json!({})),
        (
            "record_learning",
            BTreeSet::from([
                "task_id",
                "kind",
                "summary",
                "scope",
                "detail",
                "scope_ref",
                "tags",
            ]),
            BTreeSet::from(["task_id", "kind", "summary", "scope"]),
            json!({"task_id": 1, "kind": "pitfall", "summary": "Watch out", "scope": "user"}),
        ),
        (
            "query_learnings",
            BTreeSet::from(["task_id", "tag_filter", "limit"]),
            BTreeSet::from(["task_id"]),
            json!({"task_id": 1}),
        ),
        (
            "confirm_learning",
            BTreeSet::from(["learning_id", "task_id"]),
            BTreeSet::from(["learning_id", "task_id"]),
            json!({"learning_id": 1, "task_id": 1}),
        ),
    ];

    // Verify we cover exactly the tools that exist
    let expected_names: BTreeSet<&str> = cases.iter().map(|(name, _, _, _)| *name).collect();
    let actual_names: BTreeSet<&str> = tools.keys().copied().collect();
    assert_eq!(
        actual_names, expected_names,
        "Tool list mismatch — update this test when adding/removing tools"
    );

    for (name, exp_props, exp_required, payload) in &cases {
        let tool = tools[name];
        let (actual_props, actual_required) = schema_props(tool);

        assert_eq!(&actual_props, exp_props, "Property mismatch for '{name}'");
        assert_eq!(
            &actual_required, exp_required,
            "Required field mismatch for '{name}'"
        );

        // Verify the full payload deserializes into the struct
        match *name {
            "update_task" => {
                serde_json::from_value::<UpdateTaskArgs>(payload.clone()).unwrap();
            }
            "get_task" => {
                serde_json::from_value::<GetTaskArgs>(payload.clone()).unwrap();
            }
            "create_task" => {
                serde_json::from_value::<CreateTaskWithEpicArgs>(payload.clone()).unwrap();
            }
            "list_tasks" => {
                serde_json::from_value::<ListTasksArgs>(payload.clone()).unwrap();
            }
            "claim_task" => {
                serde_json::from_value::<ClaimTaskArgs>(payload.clone()).unwrap();
            }
            "report_usage" => {
                serde_json::from_value::<ReportUsageArgs>(payload.clone()).unwrap();
            }
            "create_epic" => {
                serde_json::from_value::<CreateEpicArgs>(payload.clone()).unwrap();
            }
            "get_epic" => {
                serde_json::from_value::<GetEpicArgs>(payload.clone()).unwrap();
            }
            "list_epics" => {} // no args
            "update_epic" => {
                serde_json::from_value::<UpdateEpicArgs>(payload.clone()).unwrap();
            }
            "wrap_up" => {
                serde_json::from_value::<WrapUpArgs>(payload.clone()).unwrap();
            }
            "send_message" => {
                serde_json::from_value::<SendMessageArgs>(payload.clone()).unwrap();
            }
            "dispatch_next" => {
                serde_json::from_value::<DispatchNextArgs>(payload.clone()).unwrap();
            }
            "update_review_status" => {
                serde_json::from_value::<super::tasks::UpdateReviewStatusArgs>(payload.clone())
                    .unwrap();
            }
            "list_review_prs" => {
                serde_json::from_value::<ListReviewPrsArgs>(payload.clone()).unwrap();
            }
            "get_review_pr" => {
                serde_json::from_value::<GetReviewPrArgs>(payload.clone()).unwrap();
            }
            "dispatch_review_agent" => {
                serde_json::from_value::<DispatchReviewAgentArgs>(payload.clone()).unwrap();
            }
            "list_security_alerts" => {
                serde_json::from_value::<ListSecurityAlertsArgs>(payload.clone()).unwrap();
            }
            "get_security_alert" => {
                serde_json::from_value::<GetSecurityAlertArgs>(payload.clone()).unwrap();
            }
            "dispatch_fix_agent" => {
                serde_json::from_value::<DispatchFixAgentArgs>(payload.clone()).unwrap();
            }
            "dispatch_task" => {
                serde_json::from_value::<super::tasks::DispatchTaskArgs>(payload.clone()).unwrap();
            }
            "list_projects" => {} // no args
            "record_learning" => {
                serde_json::from_value::<super::learnings::RecordLearningArgs>(payload.clone())
                    .unwrap();
            }
            "query_learnings" => {
                serde_json::from_value::<super::learnings::QueryLearningsArgs>(payload.clone())
                    .unwrap();
            }
            "confirm_learning" => {
                serde_json::from_value::<super::learnings::ConfirmLearningArgs>(payload.clone())
                    .unwrap();
            }
            other => panic!("No deserialization check for tool: {other}"),
        }
    }
}

#[tokio::test]
async fn claim_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Claimable",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0.to_string(),
                "worktree": "/repo/.worktrees/5-other-task",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string task_id: {:?}",
        resp.error
    );
}

// =======================================================================
// Epic tool tests
// =======================================================================

#[tokio::test]
async fn create_epic_minimal() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": { "title": "My Epic", "repo_path": "/repo" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Epic"));
    assert!(text.contains("created"));

    let epics = state.db.list_epics().unwrap();
    assert_eq!(epics.len(), 1);
    assert_eq!(epics[0].title, "My Epic");
    assert_eq!(epics[0].repo_path, "/repo");
}

#[tokio::test]
async fn create_epic_with_all_fields() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "Full Epic",
                "repo_path": "/repo",
                "description": "Epic desc"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let epics = state.db.list_epics().unwrap();
    assert_eq!(epics[0].description, "Epic desc");
}

#[tokio::test]
async fn create_epic_missing_title() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": { "repo_path": "/repo" }
        })),
    )
    .await;
    assert_error(&resp, "Invalid arguments");
}

#[tokio::test]
async fn create_epic_missing_repo_path() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": { "title": "No Repo" }
        })),
    )
    .await;
    assert_error(&resp, "Invalid arguments");
}

#[tokio::test]
async fn get_epic_found() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Get Me", "desc", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Get Me"));
    assert!(text.contains("desc"));
    assert!(text.contains("/repo"));
}

#[tokio::test]
async fn get_epic_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn get_epic_shows_subtask_summary() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("With Tasks", "", "/repo", None, 1)
        .unwrap();
    let t1 = state
        .db
        .create_task(
            "Sub 1",
            "",
            "/repo",
            None,
            TaskStatus::Done,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let t2 = state
        .db
        .create_task(
            "Sub 2",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();
    state.db.set_task_epic_id(t2, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("1/2 done"),
        "expected subtask summary, got: {text}"
    );
}

#[tokio::test]
async fn get_epic_accepts_string_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("String ID", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0.to_string() }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(text.contains("String ID"));
}

#[tokio::test]
async fn list_epics_empty() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No epics found"));
}

#[tokio::test]
async fn list_epics_with_items() {
    let state = test_state();
    state
        .db
        .create_epic("Epic A", "desc a", "/repo", None, 1)
        .unwrap();
    state
        .db
        .create_epic("Epic B", "desc b", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Epic A"));
    assert!(text.contains("Epic B"));
}

#[tokio::test]
async fn list_epics_shows_subtask_counts() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Tracked", "", "/repo", None, 1)
        .unwrap();
    let t1 = state
        .db
        .create_task(
            "Done",
            "",
            "/repo",
            None,
            TaskStatus::Done,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let t2 = state
        .db
        .create_task(
            "Pending",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();
    state.db.set_task_epic_id(t2, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("1/2 done"),
        "expected subtask counts, got: {text}"
    );
}

#[tokio::test]
async fn update_epic_title() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Old Title", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "title": "New Title" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("updated"));
    assert!(text.contains("title"));

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.title, "New Title");
}

#[tokio::test]
async fn update_epic_mark_done() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("To Finish", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "status": "done" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("status"),
        "response should mention status field: {text}"
    );

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.status, crate::models::TaskStatus::Done);
}

#[tokio::test]
async fn update_epic_multiple_fields() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Old", "old desc", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": {
                "epic_id": epic.id.0,
                "title": "New",
                "description": "new desc"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.title, "New");
    assert_eq!(updated.description, "new desc");
}

#[tokio::test]
async fn update_epic_accepts_string_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Str Epic", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0.to_string(), "title": "Updated" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );
}

#[tokio::test]
async fn update_epic_plan() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Planned Epic", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "plan_path": "docs/plans/epic-plan.md" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("plan"),
        "response should mention plan: {text}"
    );

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(
        updated.plan_path.as_deref(),
        Some("docs/plans/epic-plan.md")
    );
}

// =======================================================================
// Additional edge case tests
// =======================================================================

#[tokio::test]
async fn list_tasks_invalid_status_string() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "bogus" } })),
    )
    .await;
    assert_error(&resp, "Unknown status");
}

#[tokio::test]
async fn list_tasks_invalid_status_in_array() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": ["backlog", "bogus"] } })),
    )
    .await;
    assert_error(&resp, "Invalid status in array");
}

#[tokio::test]
async fn list_tasks_status_as_number_errors() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": 42 } })),
    )
    .await;
    assert_error(&resp, "string or array");
}

#[tokio::test]
async fn claim_task_rejects_done_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Done",
            "desc",
            "/repo",
            None,
            TaskStatus::Done,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert_error(&resp, "already");
}

#[tokio::test]
async fn claim_task_rejects_review_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Review",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert_error(&resp, "already");
}

#[tokio::test]
async fn claim_task_worktree_without_worktrees_dir() {
    let state = test_state();
    // Task repo is "/repo", worktree path has no /.worktrees/ segment
    // so the full path is used as the repo — should match when equal
    let task_id = state
        .db
        .create_task(
            "Direct",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should match when worktree equals repo: {:?}",
        resp.error
    );
}

#[tokio::test]
async fn create_task_with_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent Epic", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Epic Child",
                "repo_path": "/repo",
                "epic_id": epic.id.0
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let subtasks = state.db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].title, "Epic Child");
}

#[tokio::test]
async fn create_task_with_string_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent", "", "/repo", None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "String Epic Child",
                "repo_path": "/repo",
                "epic_id": epic.id.0.to_string()
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );

    let subtasks = state.db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(subtasks.len(), 1);
}

#[tokio::test]
async fn update_epic_no_fields_errors() {
    let state = test_state();
    let epic = state.db.create_epic("Test", "", "/repo", None, 1).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    assert_error(&resp, "At least one");
}

#[tokio::test]
async fn claim_task_updates_status_to_running() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Claim",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/1-claim",
                "tmux_window": "task-1"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state
        .db
        .get_task(crate::models::TaskId(task_id.0))
        .unwrap()
        .unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-claim"));
    assert_eq!(task.tmux_window.as_deref(), Some("task-1"));
}

// ---------------------------------------------------------------------------
// wrap_up tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_task_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": 9999, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn wrap_up_rejects_backlog_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "cannot be wrapped up");
}

#[tokio::test]
async fn wrap_up_accepts_running_blocked_task() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-my-task"))
            .sub_status(crate::models::SubStatus::NeedsInput),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_accepts_running_active_task() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_task_no_worktree() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "cannot be wrapped up");
}

#[tokio::test]
async fn wrap_up_invalid_action() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "teleport" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `teleport`");
}

#[tokio::test]
async fn wrap_up_rebase_returns_started() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-my-task")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_pr_succeeds_with_complete_message() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to create_pr; no symbolic-ref call
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/7\n"), // gh pr create
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "My Feature",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-my-feature")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

// ---------------------------------------------------------------------------
// sub_status tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_sets_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sub_status": "needs_input" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(
        text.contains("sub_status"),
        "response should mention sub_status: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.sub_status, crate::models::SubStatus::NeedsInput);
}

#[tokio::test]
async fn update_task_rejects_invalid_sub_status_for_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sub_status": "approved" }
        })),
    )
    .await;
    assert_error(&resp, "not valid for status");
}

#[tokio::test]
async fn update_task_rejects_bogus_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sub_status": "bogus" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

#[tokio::test]
async fn update_task_sub_status_with_status_change() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    // Change status to review and set sub_status to approved in one call
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "review", "sub_status": "approved" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, crate::models::SubStatus::Approved);
}

#[tokio::test]
async fn update_task_status_running_with_needs_input() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    // Set status=running and sub_status=needs_input in one call.
    // Before the fix, status() auto-reset sub_status to Active, which could
    // overwrite the explicit needs_input depending on builder call order.
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running", "sub_status": "needs_input" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sub_status, crate::models::SubStatus::NeedsInput);
}

#[tokio::test]
async fn update_task_sub_status_invalid_for_new_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    // Change status to review but set sub_status to active (valid for running, not review)
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "review", "sub_status": "active" }
        })),
    )
    .await;
    assert_error(&resp, "not valid for status");
}

#[tokio::test]
async fn list_tasks_shows_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Listed Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().sub_status(crate::models::SubStatus::NeedsInput),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("running/needs_input"),
        "expected running/needs_input in list output, got: {text}"
    );
}

#[tokio::test]
async fn get_task_shows_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Detail Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().sub_status(crate::models::SubStatus::ChangesRequested),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Sub-status: changes_requested"),
        "expected sub-status in detail, got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_conflict_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::fail("CONFLICT (content): Merge conflict in foo.rs"), // git rebase
        MockProcessRunner::ok(),                      // git rebase --abort
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Conflict Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-conflict-task")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "conflict");
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_not_on_main_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail(""), // git rev-parse (empty stdout → treated as non-main)
        MockProcessRunner::ok_with_stdout(b"feature\n"), // unused
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Wrong Branch",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-wrong-branch")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "not on main");
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on error"
    );
}

#[tokio::test]
async fn wrap_up_pr_push_fails_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("remote: Permission denied"), // git push fails
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Push Fail",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-push-fail")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;

    assert_error(&resp, "wrap_up failed");
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on push failure"
    );
    assert!(task.pr_url.is_none(), "No PR URL should be set on failure");
}

#[tokio::test]
async fn update_task_status_recalculates_epic_status() {
    let state = test_state();
    let epic = state.db.create_epic("E", "", "/repo", None, 1).unwrap();
    let task_id = state
        .db
        .create_task(
            "T",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    // Move subtask to Running
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_task should succeed: {:?}",
        resp.error
    );

    // Epic should auto-advance to Running
    let epic = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

// ---------------------------------------------------------------------------
// send_message tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn send_message_writes_file_and_sends_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let worktree_path = tmp.path().to_str().unwrap().to_string();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux send-keys -l (notification text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    // Create sender and receiver tasks
    let sender_id = db
        .create_task(
            "Fix auth bug",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let receiver_id = db
        .create_task(
            "Review PR",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        receiver_id,
        &db::TaskPatch::new()
            .worktree(Some(&worktree_path))
            .tmux_window(Some("task-2")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": receiver_id.0,
                "body": "Can you review path/to/file.rs?"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("Message sent to task"),
        "Expected success message, got: {text}"
    );

    // Verify message file was written in .claude-messages/ directory
    let messages_dir = tmp.path().join(".claude-messages");
    assert!(
        messages_dir.is_dir(),
        ".claude-messages directory should exist"
    );
    let entries: Vec<_> = std::fs::read_dir(&messages_dir).unwrap().collect();
    assert_eq!(entries.len(), 1, "Should have exactly one message file");
    let message_path = entries[0].as_ref().unwrap().path();
    let file_name = message_path.file_name().unwrap().to_str().unwrap();
    assert!(
        file_name.starts_with(&format!("{}-", sender_id.0)),
        "Filename should start with sender task id"
    );
    assert!(file_name.ends_with(".md"), "Filename should end with .md");
    let content = std::fs::read_to_string(&message_path).unwrap();
    assert!(
        content.contains("Fix auth bug"),
        "Message should contain sender title"
    );
    assert!(
        content.contains("Can you review path/to/file.rs?"),
        "Message should contain body"
    );
}

#[tokio::test]
async fn send_message_target_not_found() {
    let state = test_state();

    let sender_id = state
        .db
        .create_task(
            "Sender",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": 9999,
                "body": "hello"
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_some(),
        "Should return error for missing target"
    );
    let err = resp.error.unwrap();
    assert!(
        err.message.contains("not found"),
        "Error should mention not found: {}",
        err.message
    );
}

#[tokio::test]
async fn send_message_target_no_worktree() {
    let state = test_state();

    let sender_id = state
        .db
        .create_task(
            "Sender",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let receiver_id = state
        .db
        .create_task(
            "Receiver",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": receiver_id.0,
                "body": "hello"
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_some(),
        "Should return error for target without worktree"
    );
    let err = resp.error.unwrap();
    assert!(
        err.message.contains("no worktree"),
        "Error should mention no worktree: {}",
        err.message
    );
}

#[tokio::test]
async fn send_message_target_no_tmux_window() {
    let state = test_state();

    let sender_id = state
        .db
        .create_task(
            "Sender",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let receiver_id = state
        .db
        .create_task(
            "Receiver",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .patch_task(
            receiver_id,
            &db::TaskPatch::new().worktree(Some("/some/worktree")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": receiver_id.0,
                "body": "hello"
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_some(),
        "Should return error for target without tmux window"
    );
    let err = resp.error.unwrap();
    assert!(
        err.message.contains("no tmux window"),
        "Error should mention no tmux window: {}",
        err.message
    );
}

// =======================================================================
// Notification flow tests
// =======================================================================

/// Helper: creates a test state with a real notification channel.
fn test_state_with_notify() -> (
    Arc<McpState>,
    tokio::sync::mpsc::UnboundedReceiver<crate::mcp::McpEvent>,
) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let state = Arc::new(McpState {
        db,
        notify_tx: Some(tx),
        runner,
    });
    (state, rx)
}

#[tokio::test]
async fn update_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    // Should have received a Refresh event
    let event = rx
        .try_recv()
        .expect("expected notification after update_task");
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn create_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Notified Task", "repo_path": "/repo" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let event = rx
        .try_recv()
        .expect("expected notification after create_task");
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn claim_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/1-test",
                "tmux_window": "task-1"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let event = rx
        .try_recv()
        .expect("expected notification after claim_task");
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn failed_update_does_not_send_notification() {
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_task_fixture(&state);

    // Invalid status should not trigger notification
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "bogus" }
        })),
    )
    .await;
    assert!(resp.error.is_some());

    assert!(
        rx.try_recv().is_err(),
        "no notification should be sent on validation error"
    );
}

// =======================================================================
// update_task: additional validation and edge cases
// =======================================================================

#[tokio::test]
async fn update_task_nonexistent_task_returns_error() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": 9999, "status": "running" }
        })),
    )
    .await;
    assert_error(&resp, "Database error");
}

#[tokio::test]
async fn update_task_invalid_tag() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "tag": "invalid_tag" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `invalid_tag`");
}

#[tokio::test]
async fn update_task_valid_tag() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    for tag in &["bug", "feature", "chore", "epic"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "tag": tag }
            })),
        )
        .await;
        assert!(
            resp.error.is_none(),
            "tag={tag} should be valid, got: {:?}",
            resp.error
        );
    }

    // Verify last tag persisted
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.tag, Some(crate::models::TaskTag::Epic));
}

#[tokio::test]
async fn update_task_sets_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent", "", "/repo", None, 1)
        .unwrap();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "epic_id": epic.id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(
        text.contains("epic_id"),
        "response should list epic_id: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));
}

#[tokio::test]
async fn update_task_sort_order() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sort_order": 42 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.sort_order, Some(42));
}

// =======================================================================
// create_task: additional validation and edge cases
// =======================================================================

#[tokio::test]
async fn create_task_invalid_tag() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Tagged", "repo_path": "/repo", "tag": "bogus" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

#[tokio::test]
async fn create_task_valid_tag() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Bug Task", "repo_path": "/repo", "tag": "bug" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::Bug));
}

#[tokio::test]
async fn create_task_with_sort_order() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Ordered Task", "repo_path": "/repo", "sort_order": 99 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks[0].sort_order, Some(99));
}

#[tokio::test]
async fn create_task_with_nonexistent_epic() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Orphan", "repo_path": "/repo", "epic_id": 9999 }
        })),
    )
    .await;
    // Should fail because the epic FK doesn't exist
    assert!(resp.error.is_some(), "should error with invalid epic_id");
}

// =======================================================================
// list_tasks: filtering edge cases
// =======================================================================

#[tokio::test]
async fn list_tasks_filters_by_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("My Epic", "", "/repo", None, 1)
        .unwrap();
    let t1 = state
        .db
        .create_task(
            "Epic Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Standalone Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "epic_id": epic.id.0 } })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Epic Task"),
        "should include task linked to epic"
    );
    assert!(
        !text.contains("Standalone Task"),
        "should exclude task not linked to epic"
    );
}

#[tokio::test]
async fn list_tasks_filters_by_status_and_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Combined Filter", "", "/repo", None, 1)
        .unwrap();
    let t1 = state
        .db
        .create_task(
            "Backlog Epic",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let t2 = state
        .db
        .create_task(
            "Running Epic",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();
    state.db.set_task_epic_id(t2, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "status": "backlog", "epic_id": epic.id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Backlog Epic"),
        "should include backlog task in epic"
    );
    assert!(
        !text.contains("Running Epic"),
        "should exclude running task when filtering by backlog"
    );
}

#[tokio::test]
async fn list_tasks_epic_filter_no_match() {
    let state = test_state();
    state
        .db
        .create_task(
            "No Epic",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "epic_id": 9999 } })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("No tasks found"));
}

#[tokio::test]
async fn list_tasks_done_status_filter() {
    let state = test_state();
    state
        .db
        .create_task(
            "Done Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Done,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Backlog Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "done" } })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("Done Task"));
    assert!(!text.contains("Backlog Task"));
}

// =======================================================================
// wrap_up: verify DB state after successful operations
// =======================================================================

#[tokio::test]
async fn wrap_up_rebase_sets_task_to_done() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Rebase Done",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-rebase-done")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("wrap_up complete"));

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Done,
        "Task should be Done after successful rebase"
    );
}

#[tokio::test]
async fn wrap_up_pr_sets_review_and_pr_url() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "PR Review",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-pr-review")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("wrap_up complete"));
    assert!(
        text.contains("https://github.com/org/repo/pull/42"),
        "response should include PR URL"
    );

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should be Review after PR creation"
    );
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/42"),
        "PR URL should be stored"
    );
}

#[tokio::test]
async fn wrap_up_pr_does_not_inject_review_command() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
                                 // No send-keys mock — injection must not occur
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner: mock.clone() as Arc<dyn ProcessRunner>,
    });

    let task_id = db
        .create_task(
            "Feature",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-feature"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);

    // Give any spawned background tasks time to complete so their calls are recorded.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(cmd, args)| cmd == "tmux" && args.iter().any(|a| a == "send-keys")),
        "wrap_up pr must not inject a review command via send-keys; got calls: {calls:?}"
    );
}

#[tokio::test]
async fn wrap_up_pr_returns_existing_url_on_duplicate() {
    // When gh pr create fails because the PR already exists, wrap_up should treat
    // it as success and return the URL from the error message.
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::fail(
            "a pull request for branch '1-feature' already exists:\nhttps://github.com/org/repo/pull/7",
        ), // gh pr create — already exists
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Feature",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-feature")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success when PR already exists, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("https://github.com/org/repo/pull/7"),
        "response should include existing PR URL, got: {text}"
    );

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should move to Review even when PR already exists"
    );
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/7"),
        "Existing PR URL should be saved"
    );
}

#[tokio::test]
async fn wrap_up_rebase_recalculates_epic_status() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db.create_epic("E", "", "/repo", None, 1).unwrap();
    let task_id = db
        .create_task(
            "Only Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-only-task")),
    )
    .unwrap();
    // Move epic to Running to simulate in-progress state
    db.recalculate_epic_status(epic.id).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(
        epic.status,
        TaskStatus::Done,
        "Epic should auto-advance to Done when all subtasks complete"
    );
}

#[tokio::test]
async fn wrap_up_accepts_string_task_id() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse
        MockProcessRunner::fail(""),                  // git remote get-url
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0.to_string(), "action": "rebase" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "wrap_up should accept string task_id: {:?}",
        resp.error
    );
}

// =======================================================================
// get_task: additional formatting checks
// =======================================================================

#[tokio::test]
async fn get_task_shows_all_fields() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent Epic", "", "/repo", None, 1)
        .unwrap();
    let task_id = state
        .db
        .create_task(
            "Full Task",
            "detailed desc",
            "/repo",
            Some("/plan.md"),
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new()
                .worktree(Some("/repo/.worktrees/1-full"))
                .tmux_window(Some("task-1"))
                .pr_url(Some("https://github.com/org/repo/pull/5"))
                .tag(Some(crate::models::TaskTag::Feature))
                .sort_order(Some(10)),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Full Task"), "should show title");
    assert!(text.contains("detailed desc"), "should show description");
    assert!(text.contains("/repo"), "should show repo path");
    assert!(text.contains("/plan.md"), "should show plan");
    assert!(text.contains("Parent Epic"), "should show epic title");
    assert!(
        text.contains("/repo/.worktrees/1-full"),
        "should show worktree"
    );
    assert!(text.contains("task-1"), "should show tmux window");
    assert!(text.contains("pull/5"), "should show PR URL");
    assert!(text.contains("feature"), "should show tag");
    assert!(text.contains("Sort order: 10"), "should show sort order");
}

#[tokio::test]
async fn get_task_without_epic_omits_epic_line() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Solo Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        !text.contains("Epic:"),
        "should not show Epic line for task without epic: {text}"
    );
}

// =======================================================================
// list_tasks: format verification
// =======================================================================

#[tokio::test]
async fn list_tasks_shows_tag_and_plan_indicators() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Tagged Planned",
            "desc",
            "/repo",
            Some("/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().tag(Some(crate::models::TaskTag::Bug)),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    // [plan] indicator replaced by | Goal: <goal text> when plan is readable;
    // when the plan file doesn't exist the description is shown as fallback.
    assert!(
        !text.contains("[plan]"),
        "old [plan] badge should no longer appear: {text}"
    );
    assert!(text.contains("[bug]"), "should show tag indicator: {text}");
}

#[tokio::test]
async fn list_tasks_shows_epic_indicator() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Sprint 1", "", "/repo", None, 1)
        .unwrap();
    let task_id = state
        .db
        .create_task(
            "Epic Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Sprint 1"),
        "should show epic title in list: {text}"
    );
}

#[tokio::test]
async fn list_tasks_truncates_long_descriptions() {
    let state = test_state();
    let long_desc = "x".repeat(300);
    state
        .db
        .create_task(
            "Long Desc",
            &long_desc,
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("..."),
        "should truncate long description: {text}"
    );
    assert!(
        text.len() < long_desc.len() + 100,
        "truncated output should be shorter than full description"
    );
}

#[tokio::test]
async fn list_tasks_excludes_archived_by_default() {
    let state = test_state();
    state
        .db
        .create_task(
            "Active Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state
        .db
        .create_task(
            "Archived Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Archived,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("Active Task"), "should show active task");
    assert!(
        !text.contains("Archived Task"),
        "should not show archived task: {text}"
    );
}

#[tokio::test]
async fn list_epics_excludes_archived() {
    let state = test_state();
    state
        .db
        .create_epic("Active Epic", "desc", "/repo", None, 1)
        .unwrap();
    let archived_epic = state
        .db
        .create_epic("Archived Epic", "desc", "/repo", None, 1)
        .unwrap();
    state
        .db
        .patch_epic(
            archived_epic.id,
            &db::EpicPatch::new().status(TaskStatus::Archived),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Active Epic"), "should show active epic");
    assert!(
        !text.contains("Archived Epic"),
        "should not show archived epic: {text}"
    );
}

// -- dispatch_next tests ------------------------------------------------------

#[tokio::test]
async fn dispatch_next_epic_not_found_returns_error() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_next_no_backlog_returns_success_noop() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Test Epic", "desc", "/repo", None, 1)
        .unwrap();

    // Add a task that's already Running (not Backlog)
    let task_id = state
        .db
        .create_task(
            "Running Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("no backlog tasks"),
        "Expected noop message, got: {text}"
    );
}

#[tokio::test]
async fn dispatch_next_picks_first_backlog_subtask() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, 1)
        .unwrap();
    let task1_id = db
        .create_task(
            "Task 1",
            "first",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let task2_id = db
        .create_task(
            "Task 2",
            "second",
            &repo_path,
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.set_task_epic_id(task1_id, Some(epic.id)).unwrap();
    db.set_task_epic_id(task2_id, Some(epic.id)).unwrap();

    // Pre-create the worktree directory (mocked git won't create it)
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-task-1", task1_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", task1_id.0)),
        "Expected first task ID in response, got: {text}"
    );

    // Wait for spawn_blocking to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify the task was dispatched
    let task1 = db.get_task(task1_id).unwrap().unwrap();
    assert_eq!(task1.status, TaskStatus::Running);
    assert!(task1.worktree.is_some());
    assert!(task1.tmux_window.is_some());

    // task2 should still be Backlog
    let task2 = db.get_task(task2_id).unwrap().unwrap();
    assert_eq!(task2.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn dispatch_next_respects_sort_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, 1)
        .unwrap();

    // task1 has higher ID but lower sort_order — should be picked second
    let task1_id = db
        .create_task(
            "Task A",
            "first by id",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let task2_id = db
        .create_task(
            "Task B",
            "second by id",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.set_task_epic_id(task1_id, Some(epic.id)).unwrap();
    db.set_task_epic_id(task2_id, Some(epic.id)).unwrap();

    // Give task2 a lower sort_order so it should be picked first
    db.patch_task(task2_id, &db::TaskPatch::new().sort_order(Some(1)))
        .unwrap();
    db.patch_task(task1_id, &db::TaskPatch::new().sort_order(Some(2)))
        .unwrap();

    // Pre-create worktree dir for task2 (the one that should be dispatched)
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-task-b", task2_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", task2_id.0)),
        "Expected task2 (lower sort_order) to be dispatched, got: {text}"
    );
}

#[tokio::test]
async fn dispatch_next_respects_tag_routing() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, 1)
        .unwrap();

    // Create a feature-tagged task with no plan — should use Plan mode
    let task_id = db
        .create_task(
            "Feature Task",
            "a feature",
            &repo_path,
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().tag(Some(crate::models::TaskTag::Feature)),
    )
    .unwrap();

    // Pre-create worktree dir
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-feature-task", task_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", task_id.0)),
        "Expected feature task to be dispatched, got: {text}"
    );

    // Wait for spawn_blocking
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

// ---------------------------------------------------------------------------
// update_review_status tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_review_status_updates_pr() {
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let state = test_state();
    let pr = ReviewPr {
        number: 42,
        title: "Test PR".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(crate::db::PrKind::Review, &[pr]).unwrap();
    state
        .db
        .set_pr_agent(
            crate::db::PrKind::Review,
            "acme/app",
            42,
            "dispatch:review-42",
            "/tmp/wt",
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/app", "number": 42, "status": "findings_ready" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let status = state
        .db
        .pr_agent_status("review_prs", "acme/app", 42)
        .unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::FindingsReady));
}

#[tokio::test]
async fn update_review_status_no_match_errors() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/unknown", "number": 999, "status": "idle" }
        })),
    )
    .await;
    assert!(resp.error.is_some());
}

#[tokio::test]
async fn update_review_status_invalid_status_errors() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/app", "number": 1, "status": "bogus" }
        })),
    )
    .await;
    assert!(resp.error.is_some());
}

#[tokio::test]
async fn update_review_status_findings_ready_sets_action_required() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr, WorkflowItemKind};
    use chrono::Utc;

    let state = test_state();

    // Insert a PR and set an active review agent so update_agent_status succeeds
    let pr = ReviewPr {
        number: 42,
        title: "Test PR".to_string(),
        author: "alice".to_string(),
        repo: "org/repo".to_string(),
        url: "https://github.com/org/repo/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(crate::db::PrKind::Review, &[pr]).unwrap();
    state
        .db
        .set_pr_agent(
            crate::db::PrKind::Review,
            "org/repo",
            42,
            "dispatch:review-42",
            "/tmp/wt",
        )
        .unwrap();

    // Pre-insert a workflow row in Ongoing/Reviewing
    state
        .db
        .insert_pr_workflow_if_absent("org/repo", 42, WorkflowItemKind::ReviewerPr)
        .unwrap();
    state
        .db
        .upsert_pr_workflow(
            "org/repo",
            42,
            WorkflowItemKind::ReviewerPr,
            "ongoing",
            Some("reviewing"),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "org/repo", "number": 42, "status": "findings_ready" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let row = state
        .db
        .get_pr_workflow("org/repo", 42, WorkflowItemKind::ReviewerPr)
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "action_required");
    assert_eq!(row.sub_state.as_deref(), Some("findings_ready"));
}

#[tokio::test]
async fn update_review_status_findings_ready_without_workflow_row() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr, WorkflowItemKind};
    use chrono::Utc;

    let state = test_state();

    // Insert a PR and set an active review agent so update_agent_status succeeds
    let pr = ReviewPr {
        number: 88,
        title: "Test PR No Workflow".to_string(),
        author: "bob".to_string(),
        repo: "acme/product".to_string(),
        url: "https://github.com/acme/product/pull/88".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(crate::db::PrKind::Review, &[pr]).unwrap();
    state
        .db
        .set_pr_agent(
            crate::db::PrKind::Review,
            "acme/product",
            88,
            "dispatch:review-88",
            "/tmp/wt",
        )
        .unwrap();

    // NOTE: NO workflow row is inserted — find_pr_workflow_kind will return None

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/product", "number": 88, "status": "findings_ready" }
        })),
    )
    .await;
    // Should succeed even though there's no workflow row
    // (find_workflow_kind_for returns None, so upsert_pr_workflow is skipped)
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    // Agent status should be updated
    let status = state
        .db
        .pr_agent_status("review_prs", "acme/product", 88)
        .unwrap();
    assert_eq!(status.map(|s| s.as_db_str()), Some("findings_ready"));

    // No workflow row should exist since find_pr_workflow_kind found none
    let no_workflow = state
        .db
        .get_pr_workflow("acme/product", 88, WorkflowItemKind::ReviewerPr)
        .unwrap();
    assert!(no_workflow.is_none());
}

#[tokio::test]
async fn wrap_up_rebase_clears_tmux_window() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
        MockProcessRunner::ok(),                      // tmux kill-window
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Rebase Clear Window",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-rebase-clear"))
            .tmux_window(Some("task-99")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("wrap_up complete"));

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(
        task.tmux_window.is_none(),
        "tmux_window should be cleared in DB after successful rebase"
    );
}

#[tokio::test]
async fn wrap_up_rebase_conflict_sets_conflict_substatus() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::fail("CONFLICT (content): Merge conflict in foo.rs"), // git rebase
        MockProcessRunner::ok(),                      // git rebase --abort
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Conflict Sub",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-conflict-sub")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "conflict");
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
    assert_eq!(
        task.sub_status,
        SubStatus::Conflict,
        "sub_status should be Conflict after rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_clears_conflict_substatus_on_non_conflict_error() {
    // When a task has Conflict sub_status from a previous rebase attempt,
    // and a new rebase fails with a non-conflict error (e.g. Other), the
    // stale Conflict sub_status should be cleared — matching TUI behavior.
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail(""), // detect_default_branch (symbolic-ref)
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""), // git remote get-url (no remote)
        MockProcessRunner::fail("fatal: some other git error"), // git rebase (non-conflict failure)
        MockProcessRunner::ok(),     // git rebase --abort
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Stale Conflict",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-stale-conflict"))
            .sub_status(SubStatus::Conflict),
    )
    .unwrap();

    // Verify conflict is set before wrap_up
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Conflict);

    let _resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_ne!(
        task.sub_status,
        SubStatus::Conflict,
        "Stale Conflict sub_status should be cleared even on non-conflict rebase error"
    );
}

// ---------------------------------------------------------------------------
// base_branch: create_task and update_task MCP schema tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_with_base_branch_stores_it() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "My Feature",
                "repo_path": "/repo",
                "base_branch": "develop"
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let tasks = state.db.list_all().unwrap();
    let task = tasks.iter().find(|t| t.title == "My Feature").unwrap();
    assert_eq!(task.base_branch, "develop");
}

#[tokio::test]
async fn create_task_without_base_branch_defaults_to_main() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Default Branch Task",
                "repo_path": "/repo"
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let tasks = state.db.list_all().unwrap();
    let task = tasks
        .iter()
        .find(|t| t.title == "Default Branch Task")
        .unwrap();
    assert_eq!(task.base_branch, "main");
}

#[tokio::test]
async fn update_task_with_base_branch_updates_it() {
    let state = test_state();

    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "base_branch": "release/2.0"
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.base_branch, "release/2.0");
}

#[tokio::test]
async fn dispatch_next_returns_disabled_when_auto_dispatch_off() {
    let state = test_state();

    // Create epic with auto_dispatch = false
    let epic = state.db.create_epic("E", "desc", "/repo", None, 1).unwrap();
    state
        .db
        .patch_epic(epic.id, &db::EpicPatch::new().auto_dispatch(false))
        .unwrap();

    // Create a backlog subtask linked to the epic
    let task_id = state
        .db
        .create_task(
            "Sub",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    // Should return informational message, not dispatch
    let text = extract_response_text(&resp);
    assert!(
        text.contains("auto dispatch is disabled"),
        "Expected disabled message, got: {text}"
    );

    // Task must still be in backlog — not dispatched
    let task_after = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task_after.status, TaskStatus::Backlog);
}

// ---------------------------------------------------------------------------
// Step 6: MCP sub-epic creation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_create_sub_epic() {
    let state = test_state();

    // Create parent epic first
    let parent = state
        .db
        .create_epic("Parent Epic", "desc", "/tmp", None, 1)
        .unwrap();

    // Create sub-epic via MCP with parent_epic_id
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "Sub Epic",
                "repo_path": "/tmp",
                "description": "child",
                "parent_epic_id": parent.id.0
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got: {:?}",
        resp.error
    );

    // Verify the sub-epic has the correct parent
    let epics = state.db.list_epics().unwrap();
    let sub = epics
        .iter()
        .find(|e| e.title == "Sub Epic")
        .expect("sub epic should be created");
    assert_eq!(
        sub.parent_epic_id,
        Some(parent.id),
        "sub epic should have parent_epic_id set"
    );
}

#[tokio::test]
async fn create_epic_tool_schema_includes_parent_epic_id() {
    let state = test_state();
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
    let create_epic = tools
        .iter()
        .find(|t| t["name"] == "create_epic")
        .expect("create_epic not in tool list");
    let props = &create_epic["inputSchema"]["properties"];
    assert!(
        props.get("parent_epic_id").is_some(),
        "create_epic schema is missing parent_epic_id property"
    );
}

// ---------------------------------------------------------------------------
// Fixtures for review/security tests
// ---------------------------------------------------------------------------

fn insert_my_pr_fixture(state: &Arc<McpState>, number: i64, repo: &str) {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    let pr = ReviewPr {
        number,
        title: format!("My PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 5,
        deletions: 1,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    let mut existing = state.db.load_prs(PrKind::My).unwrap_or_default();
    existing.retain(|p| !(p.repo == repo && p.number == number));
    existing.push(pr);
    state.db.save_prs(PrKind::My, &existing).unwrap();
}

fn insert_review_pr_fixture(state: &Arc<McpState>, number: i64, repo: &str) {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    let pr = ReviewPr {
        number,
        title: format!("PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    // Load existing PRs and append to avoid batch-replace deleting prior inserts.
    let mut existing = state.db.load_prs(PrKind::Review).unwrap_or_default();
    existing.retain(|p| !(p.repo == repo && p.number == number));
    existing.push(pr);
    state.db.save_prs(PrKind::Review, &existing).unwrap();
}

fn insert_security_alert_fixture(
    state: &Arc<McpState>,
    number: i64,
    repo: &str,
    kind: crate::models::AlertKind,
) {
    use crate::models::{AlertSeverity, SecurityAlert};
    let alert = SecurityAlert {
        number,
        repo: repo.to_string(),
        severity: AlertSeverity::High,
        kind,
        title: format!("Alert #{number}"),
        package: Some("some-pkg".to_string()),
        vulnerable_range: Some("< 1.0".to_string()),
        fixed_version: Some("1.0.0".to_string()),
        cvss_score: Some(7.5),
        url: format!("https://github.com/{repo}/security/dependabot/{number}"),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "A vulnerability".to_string(),
    };
    // Load existing alerts and append to avoid batch-replace deleting prior inserts.
    let mut existing = state.db.load_security_alerts().unwrap_or_default();
    existing.retain(|a| !(a.repo == repo && a.number == number && a.kind == kind));
    existing.push(alert);
    state.db.save_security_alerts(&existing).unwrap();
}

// ---------------------------------------------------------------------------
// list_review_prs tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_review_prs_empty() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No PRs found"));
}

#[tokio::test]
async fn list_review_prs_returns_stored_prs() {
    let state = test_state();
    insert_review_pr_fixture(&state, 42, "acme/app");
    insert_review_pr_fixture(&state, 99, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "reviewer"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("42"));
    assert!(text.contains("99"));
}

#[tokio::test]
async fn list_review_prs_filters_by_repo() {
    let state = test_state();
    insert_review_pr_fixture(&state, 1, "acme/app");
    insert_review_pr_fixture(&state, 2, "acme/other");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"repo": "acme/app"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(!text.contains("acme/other"));
}

// ---------------------------------------------------------------------------
// get_review_pr tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_review_pr_found() {
    let state = test_state();
    insert_review_pr_fixture(&state, 42, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 42}})),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(text.contains("42"));
}

#[tokio::test]
async fn get_review_pr_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 999}})),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn get_review_pr_found_in_my_prs() {
    let state = test_state();
    insert_my_pr_fixture(&state, 55, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 55}})),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(text.contains("55"));
}

// ---------------------------------------------------------------------------
// list_security_alerts tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_security_alerts_empty() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No alerts found"));
}

#[tokio::test]
async fn list_security_alerts_returns_stored_alerts() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot);
    insert_security_alert_fixture(&state, 2, "acme/api", AlertKind::CodeScanning);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Alert #1"));
    assert!(text.contains("Alert #2"));
}

#[tokio::test]
async fn list_security_alerts_filters_by_kind() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot);
    insert_security_alert_fixture(&state, 2, "acme/api", AlertKind::CodeScanning);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"kind": "dependabot"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Alert #1"));
    assert!(!text.contains("Alert #2"));
}

// ---------------------------------------------------------------------------
// get_security_alert tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_security_alert_found() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 7, "acme/api", AlertKind::Dependabot);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_security_alert",
            "arguments": {"repo": "acme/api", "number": 7, "kind": "dependabot"}
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/api"));
    assert!(text.contains("Alert #7"));
}

#[tokio::test]
async fn get_security_alert_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_security_alert",
            "arguments": {"repo": "acme/api", "number": 999, "kind": "dependabot"}
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

// ---------------------------------------------------------------------------
// dispatch_review_agent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_review_agent_pr_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 999, "local_repo": "/tmp/repo"}
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_review_agent_already_reviewing() {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    let state = test_state();
    let pr = ReviewPr {
        number: 42,
        title: "PR #42".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(PrKind::Review, &[pr]).unwrap();
    // Persist the agent tracking fields (save_prs does not write these).
    state
        .db
        .set_pr_agent(
            PrKind::Review,
            "acme/app",
            42,
            "review-42",
            "/repo/.worktrees/review-42",
        )
        .unwrap();
    let _ = ReviewAgentStatus::Reviewing; // confirm variant exists

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 42, "local_repo": "/tmp/repo"}
        })),
    )
    .await;
    assert_error(&resp, "already has an active review agent");
}

// ---------------------------------------------------------------------------
// dispatch_fix_agent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_fix_agent_alert_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 999,
                "kind": "dependabot", "local_repo": "/tmp/repo"
            }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_fix_agent_already_reviewing() {
    use crate::models::{AlertKind, AlertSeverity, ReviewAgentStatus, SecurityAlert};
    let state = test_state();
    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-9999".to_string(),
        package: Some("pkg".to_string()),
        vulnerable_range: None,
        fixed_version: Some("1.0.0".to_string()),
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "A vuln".to_string(),
    };
    state.db.save_security_alerts(&[alert]).unwrap();
    // Persist the agent tracking fields (save_security_alerts does not write these).
    state
        .db
        .set_alert_agent(
            "acme/api",
            7,
            AlertKind::Dependabot,
            "fix-7",
            "/repo/.worktrees/fix-vuln-7",
        )
        .unwrap();
    let _ = ReviewAgentStatus::Reviewing; // confirm variant exists

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 7,
                "kind": "dependabot", "local_repo": "/tmp/repo"
            }
        })),
    )
    .await;
    assert_error(&resp, "already has an active fix agent");
}

#[tokio::test]
async fn list_review_prs_mode_author() {
    let state = test_state();
    insert_my_pr_fixture(&state, 55, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "author"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("55"), "PR #55 should appear in author mode");
}

#[tokio::test]
async fn list_review_prs_mode_all() {
    let state = test_state();
    insert_review_pr_fixture(&state, 10, "acme/app");
    insert_my_pr_fixture(&state, 20, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "all"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("10"),
        "reviewer PR #10 should appear in all mode"
    );
    assert!(
        text.contains("20"),
        "author PR #20 should appear in all mode"
    );
}

#[tokio::test]
async fn list_security_alerts_filters_by_severity() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    let state = test_state();

    let high_alert = SecurityAlert {
        number: 1,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "High Alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/1".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    let critical_alert = SecurityAlert {
        number: 2,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::Critical,
        kind: AlertKind::Dependabot,
        title: "Critical Alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/2".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    state
        .db
        .save_security_alerts(&[high_alert, critical_alert])
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"severity": "high"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("High Alert"), "High alert should appear");
    assert!(
        !text.contains("Critical Alert"),
        "Critical alert should not appear"
    );
}

#[tokio::test]
async fn list_security_alerts_filters_by_repo() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot);
    insert_security_alert_fixture(&state, 2, "acme/web", AlertKind::Dependabot);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"repo": "acme/api"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/api"), "acme/api alert should appear");
    assert!(
        !text.contains("acme/web"),
        "acme/web alert should not appear"
    );
}

#[tokio::test]
async fn dispatch_review_agent_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    // Pre-create worktree dir so git worktree add is skipped.
    std::fs::create_dir_all(dir.path().join(".worktrees").join("review-42")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window → false, empty stdout)
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git fetch origin feature/branch
        // git worktree add skipped (dir pre-exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l (claude cmd)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    insert_review_pr_fixture(&state, 42, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 42, "local_repo": repo_path}
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Review agent dispatched"),
        "expected dispatch confirmation: {text}"
    );

    let status = db.pr_agent_status("review_prs", "acme/app", 42).unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be reviewing after dispatch"
    );
}

#[tokio::test]
async fn dispatch_fix_agent_success() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    // Pre-create worktree dir so git worktree add is skipped.
    std::fs::create_dir_all(dir.path().join(".worktrees").join("fix-vuln-7")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window)
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"), // git symbolic-ref (detect default branch)
        MockProcessRunner::ok(),                                          // git fetch origin main
        // git worktree add skipped (dir pre-exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l (claude cmd)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-0001".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.21".to_string()),
        cvss_score: None,
        url: "https://example.com/7".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
    };
    db.save_security_alerts(&[alert]).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 7,
                "kind": "dependabot", "local_repo": repo_path
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Fix agent dispatched"),
        "expected dispatch confirmation: {text}"
    );

    let status = db
        .alert_agent_status("acme/api", 7, AlertKind::Dependabot)
        .unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be reviewing after dispatch"
    );
}

// ---------------------------------------------------------------------------
// dispatch_task tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_task_dispatches_backlog_task() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text / write prompt file)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "My Backlog Task",
            "do the thing",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    // Pre-create worktree dir (mocked git won't create it)
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-my-backlog-task", task_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected 'dispatched' in response, got: {text}"
    );

    // dispatch_task is synchronous — no sleep needed
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(
        task.worktree.is_some(),
        "worktree should be set after dispatch"
    );
    assert!(
        task.tmux_window.is_some(),
        "tmux_window should be set after dispatch"
    );
}

#[tokio::test]
async fn dispatch_task_returns_error_for_non_backlog_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Running Task",
            "already running",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    assert_error(&resp, "not in backlog");
}

#[tokio::test]
async fn dispatch_task_unknown_task_id_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_task_respects_tag_routing() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text / write prompt file)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    // Feature-tagged task with no plan → should route to Plan mode
    let task_id = db
        .create_task(
            "Feature Task",
            "a new feature",
            &repo_path,
            None, // no plan
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().tag(Some(crate::models::TaskTag::Feature)),
    )
    .unwrap();

    // Pre-create worktree dir
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-feature-task", task_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected dispatch confirmation, got: {text}"
    );

    // Task should be Running — plan mode still dispatches an agent
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn dispatch_task_returns_error_when_dispatch_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    // First mock call fails (tmux new-window fails) → dispatch errors out
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("tmux: no server running"), // tmux new-window fails
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = db
        .create_task(
            "Backlog Task",
            "will fail to dispatch",
            &repo_path,
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    assert!(resp.error.is_some(), "expected error when dispatch fails");

    // Task status must remain Backlog — dispatch failure must not leave it as Running
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Backlog,
        "task should remain Backlog after dispatch failure"
    );
}

// ---------------------------------------------------------------------------
// create_task project_id tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_without_project_id_assigns_to_default() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "T",
                "description": "",
                "repo_path": "/r"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let tasks = db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    let default_id = db.get_default_project().unwrap().id;
    assert_eq!(tasks[0].project_id, default_id);
}

#[tokio::test]
async fn create_task_with_project_id_assigns_correctly() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Other", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "T",
                "description": "",
                "repo_path": "/r",
                "project_id": other.id
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let tasks = db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].project_id, other.id);
}

// ---------------------------------------------------------------------------
// create_epic project_id tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_epic_without_project_id_assigns_to_default() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "E",
                "repo_path": "/r"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let epics = db.list_epics().unwrap();
    assert_eq!(epics.len(), 1);
    let default_id = db.get_default_project().unwrap().id;
    assert_eq!(epics[0].project_id, default_id);
}

#[tokio::test]
async fn create_epic_with_project_id_assigns_correctly() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Other", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "E",
                "repo_path": "/r",
                "project_id": other.id
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let epics = db.list_epics().unwrap();
    assert_eq!(epics.len(), 1);
    assert_eq!(epics[0].project_id, other.id);
}

// ---------------------------------------------------------------------------
// list_projects
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_projects_returns_all_projects() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    db.create_project("Dispatch", 1).unwrap();
    db.create_project("wizard_game", 2).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_projects", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let text = extract_response_text(&resp);
    assert!(text.contains("Default"), "expected Default project in list");
    assert!(
        text.contains("Dispatch"),
        "expected Dispatch project in list"
    );
    assert!(
        text.contains("wizard_game"),
        "expected wizard_game project in list"
    );
}

// ---------------------------------------------------------------------------
// update_task project_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_project_id_moves_task() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Dispatch", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = create_task_fixture(&state);
    let default_id = db.get_default_project().unwrap().id;
    let task_before = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task_before.project_id, default_id);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "project_id": other.id }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let task_after = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task_after.project_id, other.id);
}

#[tokio::test]
async fn update_task_invalid_project_id_returns_error() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "project_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "project");
    assert_eq!(resp.error.as_ref().unwrap().code, -32602);
}

// ---------------------------------------------------------------------------
// update_epic project_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_epic_project_id_moves_epic() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Dispatch", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic(
            "Test Epic",
            "",
            "/repo",
            None,
            db.get_default_project().unwrap().id,
        )
        .unwrap();
    let default_id = db.get_default_project().unwrap().id;
    assert_eq!(epic.project_id, default_id);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "project_id": other.id }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let epics = db.list_epics().unwrap();
    let updated = epics.iter().find(|e| e.id == epic.id).unwrap();
    assert_eq!(updated.project_id, other.id);
}

#[tokio::test]
async fn update_epic_invalid_project_id_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic("E", "", "/r", None, db.get_default_project().unwrap().id)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "project_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "project");
    assert_eq!(resp.error.as_ref().unwrap().code, -32602);
}

// ---------------------------------------------------------------------------
// Learning tool tests
// ---------------------------------------------------------------------------

fn default_project_id(state: &Arc<McpState>) -> i64 {
    state.db.get_default_project().unwrap().id
}

fn create_task_in_repo(state: &Arc<McpState>, repo: &str) -> crate::models::TaskId {
    let pid = default_project_id(state);
    state
        .db
        .create_task(
            "Test task",
            "",
            repo,
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            pid,
        )
        .unwrap()
}

fn create_approved_learning(
    state: &Arc<McpState>,
    summary: &str,
    scope: crate::models::LearningScope,
    scope_ref: Option<&str>,
    tags: &[&str],
) -> crate::models::LearningId {
    let tag_strings: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    let id = state
        .db
        .create_learning(
            crate::models::LearningKind::Convention,
            summary,
            None,
            scope,
            scope_ref,
            &tag_strings,
            None,
        )
        .unwrap();
    state
        .db
        .patch_learning(
            id,
            &crate::db::LearningPatch::new().status(crate::models::LearningStatus::Approved),
        )
        .unwrap();
    id
}

// --- record_learning ---------------------------------------------------------

#[tokio::test]
async fn record_learning_creates_proposed_entry() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "Always use cargo fmt before committing",
                "scope": "repo",
                "scope_ref": "/repo/foo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(text.contains("proposed"), "expected 'proposed' in: {text}");

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Proposed),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(
        learnings[0].summary,
        "Always use cargo fmt before committing"
    );
    assert_eq!(learnings[0].scope, crate::models::LearningScope::Repo);
    assert_eq!(learnings[0].source_task_id, Some(task_id));
}

#[tokio::test]
async fn record_learning_derives_scope_ref_for_repo() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/bar");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "pitfall",
                "summary": "Watch out for integer overflow",
                "scope": "repo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Proposed),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(learnings[0].scope_ref.as_deref(), Some("/repo/bar"));
}

#[tokio::test]
async fn record_learning_derives_scope_ref_for_epic() {
    let state = test_state();
    let pid = default_project_id(&state);
    let epic = state.db.create_epic("E", "", "/r", None, pid).unwrap();
    let task_id = state
        .db
        .create_task(
            "T",
            "",
            "/r",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            Some(epic.id),
            None,
            None,
            pid,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "episodic",
                "summary": "Epic-level outcome",
                "scope": "epic"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Proposed),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(
        learnings[0].scope_ref.as_deref(),
        Some(epic.id.0.to_string().as_str())
    );
}

#[tokio::test]
async fn record_learning_epic_scope_no_epic_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/baz");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "episodic",
                "summary": "Epic outcome but no epic",
                "scope": "epic"
            }
        })),
    )
    .await;
    assert_error(&resp, "epic");
}

#[tokio::test]
async fn record_learning_user_scope_no_scope_ref() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "preference",
                "summary": "I prefer verbose variable names",
                "scope": "user"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Proposed),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert!(learnings[0].scope_ref.is_none());
}

#[tokio::test]
async fn record_learning_empty_summary_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "pitfall",
                "summary": "   ",
                "scope": "user"
            }
        })),
    )
    .await;
    assert_error(&resp, "summary");
}

#[tokio::test]
async fn record_learning_unknown_task_fails() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": 9999,
                "kind": "pitfall",
                "summary": "Some learning",
                "scope": "user"
            }
        })),
    )
    .await;
    assert_error(&resp, "9999");
}

// --- query_learnings ---------------------------------------------------------

#[tokio::test]
async fn query_learnings_returns_approved_for_task() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/myproject");
    create_approved_learning(
        &state,
        "Use anyhow for errors",
        crate::models::LearningScope::Repo,
        Some("/repo/myproject"),
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Use anyhow for errors"),
        "expected learning in: {text}"
    );
}

#[tokio::test]
async fn query_learnings_excludes_proposed() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/proj");
    state
        .db
        .create_learning(
            crate::models::LearningKind::Convention,
            "Proposed only",
            None,
            crate::models::LearningScope::Repo,
            Some("/repo/proj"),
            &[],
            None,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(
        !text.contains("Proposed only"),
        "proposed learning should not appear: {text}"
    );
}

#[tokio::test]
async fn query_learnings_tag_filter_narrows_results() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/tagged");
    create_approved_learning(
        &state,
        "Rust tips",
        crate::models::LearningScope::Repo,
        Some("/repo/tagged"),
        &["rust"],
    );
    create_approved_learning(
        &state,
        "Testing tips",
        crate::models::LearningScope::Repo,
        Some("/repo/tagged"),
        &["testing"],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0, "tag_filter": "rust" }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("Rust tips"), "expected rust learning");
    assert!(
        !text.contains("Testing tips"),
        "should not see testing learning"
    );
}

#[tokio::test]
async fn query_learnings_respects_limit() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/limited");
    for i in 0..5 {
        create_approved_learning(
            &state,
            &format!("Learning {i}"),
            crate::models::LearningScope::Repo,
            Some("/repo/limited"),
            &[],
        );
    }

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0, "limit": 2 }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    // Each entry starts with "[<id>]", count those occurrences
    let count = text.matches('[').count();
    assert_eq!(count, 2, "expected exactly 2 learnings, got text: {text}");
}

#[tokio::test]
async fn query_learnings_unknown_task_fails() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "9999");
}

// --- confirm_learning --------------------------------------------------------

#[tokio::test]
async fn confirm_learning_increments_count() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo");
    let learning_id = create_approved_learning(
        &state,
        "Useful tip",
        crate::models::LearningScope::User,
        None,
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "confirm_learning",
            "arguments": { "learning_id": learning_id, "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let learning = state.db.get_learning(learning_id).unwrap().unwrap();
    assert_eq!(learning.confirmed_count, 1);
}

#[tokio::test]
async fn confirm_learning_proposed_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo");
    let learning_id = state
        .db
        .create_learning(
            crate::models::LearningKind::Pitfall,
            "Not yet approved",
            None,
            crate::models::LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "confirm_learning",
            "arguments": { "learning_id": learning_id, "task_id": task_id.0 }
        })),
    )
    .await;
    assert_error(&resp, "approved");
}

#[tokio::test]
async fn confirm_learning_unknown_learning_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "confirm_learning",
            "arguments": { "learning_id": 9999, "task_id": task_id.0 }
        })),
    )
    .await;
    assert_error(&resp, "9999");
}

// -- list_tasks caller_task_id / scope derivation tests ---------------------

#[tokio::test]
async fn list_tasks_caller_task_id_scopes_to_epic() {
    let state = test_state();

    // Create epics directly via DB
    let epic = state.db.create_epic("My Epic", "", "/repo", None, 1).unwrap();
    let epic2 = state.db.create_epic("Other Epic", "", "/repo", None, 1).unwrap();

    // Task A (caller) in epic
    let id_a = state
        .db
        .create_task("Task A", "", "/repo", None, TaskStatus::Backlog, "main", Some(epic.id), None, None, 1)
        .unwrap();

    // Task B (sibling) in the same epic
    state
        .db
        .create_task("Task B", "", "/repo", None, TaskStatus::Backlog, "main", Some(epic.id), None, None, 1)
        .unwrap();

    // Task C in a different epic (should NOT appear)
    state
        .db
        .create_task("Task C", "", "/repo", None, TaskStatus::Backlog, "main", Some(epic2.id), None, None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": id_a.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Task B"), "should include sibling Task B");
    assert!(!text.contains("Task A"), "should exclude the caller Task A");
    assert!(!text.contains("Task C"), "should exclude Task C from other epic");
}

#[tokio::test]
async fn list_tasks_caller_task_id_scopes_to_project_when_no_epic() {
    let state = test_state();

    // Task A (caller) in project 1, no epic
    let id_a = state
        .db
        .create_task("Task A", "", "/repo", None, TaskStatus::Backlog, "main", None, None, None, 1)
        .unwrap();

    // Task B sibling in project 1
    state
        .db
        .create_task("Task B", "", "/repo", None, TaskStatus::Backlog, "main", None, None, None, 1)
        .unwrap();

    // Task C in project 2 (should NOT appear)
    state
        .db
        .create_task("Task C", "", "/repo", None, TaskStatus::Backlog, "main", None, None, None, 2)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": id_a.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Task B"), "should include project sibling Task B");
    assert!(!text.contains("Task A"), "should exclude caller Task A");
    assert!(!text.contains("Task C"), "should exclude Task C from project 2");
}

#[tokio::test]
async fn list_tasks_explicit_scope_overrides_caller_derived_scope() {
    let state = test_state();

    // Create epic directly via DB
    let epic = state.db.create_epic("Epic", "", "/repo", None, 1).unwrap();

    // Caller is in the epic
    let id_a = state
        .db
        .create_task("Task A", "", "/repo", None, TaskStatus::Backlog, "main", Some(epic.id), None, None, 1)
        .unwrap();

    // Task B also in the epic
    state
        .db
        .create_task("Task B", "", "/repo", None, TaskStatus::Backlog, "main", Some(epic.id), None, None, 1)
        .unwrap();

    // Task C in project 2, no epic — explicit project_id=2 should match this
    state
        .db
        .create_task("Task C", "", "/repo", None, TaskStatus::Backlog, "main", None, None, None, 2)
        .unwrap();

    // Pass caller_task_id (which has epic) BUT also explicit project_id=2 → project wins
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": id_a.0, "project_id": 2 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Task C"), "explicit project_id=2 should show Task C");
    assert!(!text.contains("Task B"), "Task B is in epic/project1, should not appear");
    assert!(!text.contains("Task A"), "caller excluded");
}

#[tokio::test]
async fn list_tasks_repo_paths_filter() {
    let state = test_state();

    state
        .db
        .create_task("Repo A task", "", "/repo/a", None, TaskStatus::Backlog, "main", None, None, None, 1)
        .unwrap();
    state
        .db
        .create_task("Repo B task", "", "/repo/b", None, TaskStatus::Backlog, "main", None, None, None, 1)
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "repo_paths": ["/repo/a"] }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Repo A task"));
    assert!(!text.contains("Repo B task"));
}

#[tokio::test]
async fn list_tasks_unknown_caller_task_id_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "Unknown caller_task_id");
}

#[tokio::test]
async fn list_tasks_includes_pr_url_in_output() {
    let state = test_state();

    let task_id = create_task_fixture(&state);
    state
        .db
        .patch_task(
            task_id,
            &crate::db::TaskPatch::new().pr_url(Some("https://github.com/org/repo/pull/42")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("| PR: https://github.com/org/repo/pull/42"),
        "PR URL should appear in output; got: {text}"
    );
}

#[tokio::test]
async fn list_tasks_includes_plan_goal_in_output() {
    let state = test_state();

    let plan_path = std::env::temp_dir().join("dispatch_test_plan_345.md");
    std::fs::write(
        &plan_path,
        "# My Feature — Implementation Plan\n\n**Goal:** Implement the learning enrichment.\n",
    )
    .unwrap();
    let plan_path_str = plan_path.to_string_lossy().to_string();

    state
        .db
        .create_task(
            "Feature task",
            "desc",
            "/repo",
            Some(&plan_path_str),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("| Goal: Implement the learning enrichment."),
        "Plan goal should appear in output; got: {text}"
    );

    let _ = std::fs::remove_file(&plan_path);
}

#[tokio::test]
async fn list_tasks_falls_back_to_description_when_no_plan() {
    let state = test_state();

    state
        .db
        .create_task(
            "No Plan Task",
            "A task without a plan file",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("A task without a plan file"),
        "Description should appear as fallback; got: {text}"
    );
}

#[tokio::test]
async fn list_tasks_omits_pr_segment_when_no_pr_url() {
    let state = test_state();
    create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("| PR:"),
        "No PR segment should appear when pr_url is null; got: {text}"
    );
}
