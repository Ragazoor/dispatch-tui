#![allow(clippy::unwrap_used, clippy::expect_used)]
mod epics;
mod learnings;
mod projects;
mod review;
mod tasks;

use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::db::{self, CreateTaskRequest, Database};
use crate::mcp::McpState;
use crate::models::{ProjectId, SubStatus, TaskStatus};
use crate::process::{MockProcessRunner, ProcessRunner};

use super::dispatch::{handle_mcp, tool_definitions};
use super::epics::{CreateEpicArgs, GetEpicArgs, UpdateEpicArgs};
use super::review::{
    DispatchFixAgentArgs, DispatchReviewAgentArgs, GetReviewPrArgs, GetSecurityAlertArgs,
    ListReviewPrsArgs, ListSecurityAlertsArgs,
};
use super::tasks::{
    ClaimTaskArgs, CreateTaskWithEpicArgs, DispatchNextArgs, ExitSessionArgs, GetTaskArgs,
    ListTasksArgs, ReportUsageArgs, SendMessageArgs, UpdateTaskArgs, WrapUpArgs,
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

fn test_state_with_db() -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });
    (state, db)
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
        .create_task(CreateTaskRequest {
            title: "Test Task",
            description: "test description",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap()
}

/// Create a Running task with worktree and tmux_window set — ready for exit_session.
fn create_running_task_with_window(state: &Arc<McpState>) -> crate::models::TaskId {
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Running Task",
            description: "description",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let patch = crate::db::TaskPatch::new()
        .worktree(Some("/repo/.worktrees/task-123"))
        .tmux_window(Some("task-123"));
    state.db.patch_task(task_id, &patch).unwrap();
    task_id
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

// -- Dispatch-level tests --------------------------------------------------

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
            BTreeSet::from(["title", "repo_path", "project_id"]),
            json!({"title": "t", "repo_path": "/r", "project_id": 1, "description": "d", "plan_path": "/p.md", "sort_order": 10, "tag": "feature"}),
        ),
        (
            "list_tasks",
            BTreeSet::from([
                "status",
                "epic_id",
                "project_id",
                "repo_paths",
                "caller_task_id",
            ]),
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
                "feed_command",
                "feed_interval_secs",
            ]),
            BTreeSet::from(["epic_id"]),
            json!({"epic_id": 1, "plan_path": "docs/plan.md", "sort_order": 42, "repo_path": "/new/path"}),
        ),
        (
            "wrap_up",
            BTreeSet::from(["task_id", "action", "learning_verdicts"]),
            BTreeSet::from(["task_id", "action"]),
            json!({"task_id": 1, "action": "rebase", "learning_verdicts": [{"learning_id": 1, "verdict": "helped"}]}),
        ),
        (
            "report_usage",
            BTreeSet::from([
                "task_id",
                "input_tokens",
                "output_tokens",
                "cache_read_tokens",
                "cache_write_tokens",
            ]),
            BTreeSet::from(["task_id", "input_tokens", "output_tokens"]),
            json!({"task_id": 1, "input_tokens": 1000,
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
            "upvote_learning",
            BTreeSet::from(["learning_id", "task_id"]),
            BTreeSet::from(["learning_id", "task_id"]),
            json!({"learning_id": 1, "task_id": 1}),
        ),
        (
            "exit_session",
            BTreeSet::from(["task_id", "has_learnings"]),
            BTreeSet::from(["task_id"]),
            json!({"task_id": 1}),
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
            "upvote_learning" => {
                serde_json::from_value::<super::learnings::UpvoteLearningArgs>(payload.clone())
                    .unwrap();
            }
            "exit_session" => {
                serde_json::from_value::<ExitSessionArgs>(payload.clone()).unwrap();
            }
            other => panic!("No deserialization check for tool: {other}"),
        }
    }
}
