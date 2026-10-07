mod context;
mod epics;
mod hooks;
mod keybindings;
mod learnings;
mod managed_feeds;
mod poll_ownership;
mod tasks;
mod usage;

use std::sync::Arc;

use axum::{
    body::to_bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};

use tokio::sync::mpsc;

use crate::db::{self, CreateLearningRow, CreateTaskRequest, Database};
use crate::mcp::identity::{CallerIdentity, IdentityError};
use crate::mcp::{BackgroundWrite, McpDeps, McpState};
use crate::models::{SubStatus, TaskStatus};
use crate::process::{MockProcessRunner, ProcessRunner};
use crate::service::embeddings::{serialize_embedding, EmbeddingService};

use super::dispatch::{handle_mcp, tool_definitions};
use super::types::{JsonRpcRequest, JsonRpcResponse};

/// The single `McpState` constructor the test module builds on: an in-memory DB
/// plus whichever of the three injectable seams a test cares about. Everything
/// else here (`test_state`, `test_state_with_db`, `state_with_mock_task_svc`,
/// `ChainFixture`) delegates, so the wiring exists once.
async fn test_state_with_overrides(
    runner: Arc<dyn ProcessRunner>,
    notify_tx: Option<mpsc::UnboundedSender<crate::mcp::McpEvent>>,
    task_svc: Option<Arc<dyn crate::service::TaskServiceApi>>,
) -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    test_state_with_overrides_and_bg_done(runner, notify_tx, task_svc, None).await
}

/// Like [`test_state_with_overrides`], but also installs a completion signal
/// for fire-and-forget background writes (usage, trajectory, the
/// `exit_session` tmux teardown), so a test can await one deterministically
/// instead of sleeping.
async fn test_state_with_overrides_and_bg_done(
    runner: Arc<dyn ProcessRunner>,
    notify_tx: Option<mpsc::UnboundedSender<crate::mcp::McpEvent>>,
    task_svc: Option<Arc<dyn crate::service::TaskServiceApi>>,
    bg_write_done_tx: Option<mpsc::UnboundedSender<BackgroundWrite>>,
) -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    state_over(db, runner, notify_tx, task_svc, bg_write_done_tx)
}

/// Build the `McpState` over `db`. Split from the constructor above so a test
/// can bring a SQLite-only handle.
fn state_over(
    db: Arc<dyn db::TaskStore>,
    runner: Arc<dyn ProcessRunner>,
    notify_tx: Option<mpsc::UnboundedSender<crate::mcp::McpEvent>>,
    task_svc: Option<Arc<dyn crate::service::TaskServiceApi>>,
    bg_write_done_tx: Option<mpsc::UnboundedSender<BackgroundWrite>>,
) -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    let mut state = McpState::new(
        McpDeps {
            db: db.clone(),
            runner,
            embedding_service: EmbeddingService::new_test(),
            data_dir: std::env::temp_dir(),
        },
        notify_tx,
    );
    if let Some(task_svc) = task_svc {
        state.task_svc = task_svc;
    }
    state.test_hooks.bg_write_done_tx = bg_write_done_tx;
    (Arc::new(state), db)
}

async fn test_state() -> Arc<McpState> {
    test_state_with_db().await.0
}

/// Like [`test_state`], but installs a completion signal that fires after each
/// fire-and-forget background write. Returns the receiver so the test can await
/// the write (e.g. usage recording) deterministically instead of sleeping.
async fn test_state_with_bg_done() -> (Arc<McpState>, mpsc::UnboundedReceiver<BackgroundWrite>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (state, _db) = test_state_with_overrides_and_bg_done(runner, None, None, Some(tx)).await;
    (state, rx)
}

async fn test_state_with_db() -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    test_state_with_overrides(Arc::new(MockProcessRunner::new(vec![])), None, None).await
}

async fn call(state: &Arc<McpState>, method: &str, params: Option<Value>) -> JsonRpcResponse {
    call_as(state, method, params, CallerIdentity::Session).await
}

async fn call_as(
    state: &Arc<McpState>,
    method: &str,
    params: Option<Value>,
    identity: CallerIdentity,
) -> JsonRpcResponse {
    call_with_identity(state, method, params, Ok(identity)).await
}

async fn call_with_identity(
    state: &Arc<McpState>,
    method: &str,
    params: Option<Value>,
    identity: Result<CallerIdentity, IdentityError>,
) -> JsonRpcResponse {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: method.to_string(),
        params,
    };
    let response: Response = handle_mcp(State(state.clone()), Extension(identity), Json(req))
        .await
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Send a JSON-RPC notification (no `id`) and return the raw (status, body) for inspection.
async fn call_notification(
    state: &Arc<McpState>,
    method: &str,
    params: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: None,
        method: method.to_string(),
        params,
    };
    let response: Response = handle_mcp(
        State(state.clone()),
        Extension(Ok(CallerIdentity::Session)),
        Json(req),
    )
    .await
    .into_response();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, bytes.to_vec())
}

// -- Shared helpers --------------------------------------------------------

/// Create a task with sensible defaults, returning the TaskId.
async fn create_task_fixture(state: &Arc<McpState>) -> crate::models::TaskId {
    create_task_fixture_at(state, "/repo").await
}

async fn create_task_fixture_at(state: &Arc<McpState>, repo_path: &str) -> crate::models::TaskId {
    state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Test Task",
            description: "test description",
            repo_path,
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
        .unwrap()
}

/// Create a Running task with worktree and tmux_window set — ready for
/// exit_session. Uses the placeholder `/repo` path, which is fine for any test
/// that never provisions.
async fn create_running_task_with_window(state: &Arc<McpState>) -> crate::models::TaskId {
    create_running_task_with_window_in(state, "/repo", None).await
}

/// [`create_running_task_with_window`] for a task that must live in a real
/// on-disk repo (so a dispatch can provision against it) and/or belong to an
/// epic. The worktree and window are derived from the task id, so a fixture
/// holding several of them keeps them distinct.
async fn create_running_task_with_window_in(
    state: &Arc<McpState>,
    repo_path: &str,
    epic_id: Option<crate::models::EpicId>,
) -> crate::models::TaskId {
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Running Task",
            description: "description",
            repo_path,
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id,
            sort_order: Some(0),
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    let worktree = format!("{repo_path}/.worktrees/{}-running-task", task_id.0);
    let window = crate::models::TmuxWindow::for_task(task_id);
    let patch = crate::db::TaskPatch::new()
        .worktree(Some(worktree.as_str()))
        .tmux_window(Some(&window));
    state.db_write().patch_task(task_id, &patch).await.unwrap();
    task_id
}

/// The PR url [`close_session_via_mcp`] supplies for the `Pr` action.
const TEST_PR_URL: &str = "https://github.com/acme/repo/pull/1";

/// Seed the in-memory exit token `wrap_up` would have issued for `action`, and
/// return it. Shared because the token's shape is one struct in one map and
/// every close test needs it — inlining it made the shape a 15-site edit.
fn seed_exit_token(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
    action: crate::mcp::handlers::tasks::WrapUpAction,
) -> String {
    let token = "tok".to_string();
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: token.clone(),
            action,
        },
    );
    token
}

/// Close `task_id`'s session with `action`, seeding the exit token the same way
/// `wrap_up` would. `pr_url` is supplied for the `Pr` action, which requires it.
async fn close_session_via_mcp(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
    action: crate::mcp::handlers::tasks::WrapUpAction,
) -> JsonRpcResponse {
    let token = seed_exit_token(state, task_id, action);
    let mut arguments = json!({
        "task_id": task_id.0,
        "token": token,
        "action": action.as_str(),
    });
    if action == crate::mcp::handlers::tasks::WrapUpAction::Pr {
        arguments["pr_url"] = json!(TEST_PR_URL);
    }
    call(
        state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": arguments })),
    )
    .await
}

/// Returns `true` if the response is either a JSON-RPC protocol error or an
/// MCP tool-execution error result (`result.isError == true`).
fn is_error(resp: &JsonRpcResponse) -> bool {
    if resp.error.is_some() {
        return true;
    }
    resp.result
        .as_ref()
        .and_then(|r| r.get("isError"))
        .and_then(Value::as_bool)
        == Some(true)
}

/// Extract the error message from a response — works for protocol errors and
/// for MCP tool-execution error results (`isError: true` with a text content
/// block).
fn error_message(resp: &JsonRpcResponse) -> String {
    if let Some(err) = resp.error.as_ref() {
        return err.message.clone();
    }
    if let Some(result) = resp.result.as_ref() {
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return result["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
        }
    }
    panic!("expected error, got success: {:?}", resp.result);
}

/// Assert response is an error whose message contains `substr`.
fn assert_error(resp: &JsonRpcResponse, substr: &str) {
    let message = error_message(resp);
    assert!(
        message.contains(substr),
        "expected error containing {substr:?}, got: {message:?}",
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
    let state = test_state().await;
    let resp = call(&state, "initialize", None).await;
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert!(result["capabilities"]["tools"].is_object());
}

/// When the client offers a supported older protocol version, the server
/// echoes it back so the session downgrades gracefully.
#[tokio::test]
async fn initialize_echoes_supported_client_version() {
    let state = test_state().await;
    let resp = call(
        &state,
        "initialize",
        Some(json!({"protocolVersion": "2024-11-05"})),
    )
    .await;
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2024-11-05");
}

/// When the client offers an unknown version, the server replies with its
/// latest supported version (the client may then decide to abort).
#[tokio::test]
async fn initialize_falls_back_to_server_version_for_unknown_client_version() {
    let state = test_state().await;
    let resp = call(
        &state,
        "initialize",
        Some(json!({"protocolVersion": "1999-01-01"})),
    )
    .await;
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2025-06-18");
}

/// MCP defines `ping` for liveness probes. It must return an empty result —
/// not a `-32601 Method not found` protocol error.
#[tokio::test]
async fn ping_returns_empty_result() {
    let state = test_state().await;
    let resp = call(&state, "ping", None).await;
    assert!(
        resp.error.is_none(),
        "ping should not error: {:?}",
        resp.error
    );
    assert_eq!(resp.result.unwrap(), json!({}));
}

#[tokio::test]
async fn tools_list_returns_tools() {
    let state = test_state().await;
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

/// Per MCP spec, tool-execution failures (including "tool not found" inside
/// `tools/call`) must surface as `result.isError == true`, not as a JSON-RPC
/// protocol error. Strict clients reject the wrong shape and abort the session.
#[tokio::test]
async fn tools_call_unknown_tool_returns_is_error_result() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "bogus_tool", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none(), "should not be a protocol error");
    let result = resp.result.expect("expected isError result");
    assert_eq!(result["isError"], json!(true));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Unknown tool"), "got: {text}");
}

/// Tool handler failures (e.g. NotFound from the service layer) likewise
/// surface as `result.isError == true` rather than a JSON-RPC protocol error.
#[tokio::test]
async fn tools_call_handler_error_returns_is_error_result() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "get_task", "arguments": { "task_id": 999_999 } })),
    )
    .await;
    assert!(resp.error.is_none(), "should not be a protocol error");
    let result = resp.result.expect("expected isError result");
    assert_eq!(result["isError"], json!(true));
}

#[tokio::test]
async fn unknown_method() {
    let state = test_state().await;
    let resp = call(&state, "bogus/method", None).await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("Method not found"));
}

/// JSON-RPC 2.0 §4.1: the server MUST NOT reply to a notification. The MCP
/// streamable-HTTP transport spec maps that to HTTP 202 Accepted with an empty
/// body. Claude Code's strict response schema rejects `id: null`, so any body
/// here aborts the session.
#[tokio::test]
async fn notification_initialized_returns_202_with_no_body() {
    let state = test_state().await;
    let (status, body) =
        call_notification(&state, "notifications/initialized", Some(json!({}))).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(
        body.is_empty(),
        "expected empty body for notification, got: {:?}",
        String::from_utf8_lossy(&body)
    );
}

/// Even unknown notifications must be silently accepted — JSON-RPC forbids any
/// response (errors included) to messages without an `id`.
#[tokio::test]
async fn unknown_notification_returns_202_with_no_body() {
    let state = test_state().await;
    let (status, body) = call_notification(&state, "notifications/something_new", None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.is_empty());
}

#[tokio::test]
async fn initialize_succeeds_without_identity() {
    let state = test_state().await;
    let resp = call_with_identity(&state, "initialize", None, Err(IdentityError::Missing)).await;
    assert!(resp.error.is_none(), "got error: {:?}", resp.error);
    assert_eq!(resp.result.unwrap()["protocolVersion"], "2025-06-18");
}

#[tokio::test]
async fn ping_succeeds_without_identity() {
    let state = test_state().await;
    let resp = call_with_identity(&state, "ping", None, Err(IdentityError::Missing)).await;
    assert!(resp.error.is_none(), "got error: {:?}", resp.error);
    assert_eq!(resp.result.unwrap(), json!({}));
}

#[tokio::test]
async fn tools_list_succeeds_without_identity() {
    let state = test_state().await;
    let resp = call_with_identity(&state, "tools/list", None, Err(IdentityError::Missing)).await;
    assert!(resp.error.is_none(), "got error: {:?}", resp.error);
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().len();
    assert!(tools > 0);
}

#[tokio::test]
async fn tools_call_without_identity_returns_invalid_request_with_request_id() {
    let state = test_state().await;
    let resp = call_with_identity(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
        Err(IdentityError::Missing),
    )
    .await;
    let err = resp.error.expect("expected JSON-RPC error");
    assert_eq!(err.code, -32600);
    assert!(err.message.contains("missing"), "got: {}", err.message);
    // Strict MCP clients reject `id: null` on error responses; the handler must
    // echo back the request id (1) it parsed from the body.
    assert_eq!(resp.id, Some(json!(1)));
}

#[tokio::test]
async fn tools_call_with_conflict_identity_returns_invalid_request() {
    let state = test_state().await;
    let resp = call_with_identity(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
        Err(IdentityError::Conflict),
    )
    .await;
    let err = resp.error.expect("expected JSON-RPC error");
    assert_eq!(err.code, -32600);
    assert_eq!(resp.id, Some(json!(1)));
}

/// Every tool's schema is internally consistent: `required` only lists names
/// that are actually declared in `properties`. A required-but-undeclared field
/// (the shape of the `repo_path` defect: schema said "required" for a field no
/// struct backs) would slip past `deny_unknown_fields`, since that attribute
/// only rejects fields present in a *request*, not fields the schema
/// over-promises without a request ever mentioning them.
#[tokio::test]
async fn tool_schemas_have_consistent_required_fields() {
    let defs = tool_definitions();
    let tools_arr = defs["tools"].as_array().unwrap();

    for tool in tools_arr {
        let name = tool["name"].as_str().unwrap();
        let schema = &tool["inputSchema"];
        let props = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{name}: inputSchema.properties must be an object"));
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for field in &required {
            let field = field.as_str().unwrap();
            assert!(
                props.contains_key(field),
                "{name}: '{field}' is required but not declared in properties"
            );
        }
    }
}

/// The tool `name` as `tools/list` advertises it.
fn tool_def(name: &str) -> Value {
    tool_definitions()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("{name} must be advertised"))
        .clone()
}

/// The description of `name` as `tools/list` advertises it.
fn tool_description(name: &str) -> String {
    tool_def(name)["description"].as_str().unwrap().to_string()
}

/// rule-guidance.DispatchTaskViaMcp ("The tool description advertises both paths
/// and the recovery move"). Asserted on substrings rather than the whole string
/// so the prose can be reworded, but the load-bearing facts cannot quietly go
/// missing: that an existing worktree is REUSED, that its uncommitted changes
/// survive, and that the way back into a crashed task is update_task to backlog
/// then dispatch_task again.
#[test]
fn dispatch_task_description_advertises_worktree_reuse_and_the_recovery_move() {
    let desc = tool_description("dispatch_task");
    for needle in ["reused", "uncommitted", "update_task", "backlog"] {
        assert!(
            desc.contains(needle),
            "dispatch_task description must mention '{needle}', got: {desc}"
        );
    }
}

/// mcp-task-tools.allium: UpdateTaskViaMcp. The `status` argument lists the
/// statuses and nothing more. The resume-a-crashed-task story is
/// dispatch_task's (DispatchTaskViaMcp tells it once), and the aside that no MCP
/// path deletes a task describes a tool that does not exist.
#[test]
fn update_task_status_description_tells_no_resume_story_and_no_delete_aside() {
    let update = tool_def("update_task");
    let status = update["inputSchema"]["properties"]["status"]["description"]
        .as_str()
        .unwrap()
        .to_lowercase();
    for needle in ["resum", "reuse", "worktree", "delete"] {
        assert!(
            !status.contains(needle),
            "update_task.status must not carry {needle:?} — the resume story is \
dispatch_task's and there is no delete tool, got: {status}"
        );
    }
    // TheNextCallIsNamedJustInTime: the next-call instruction lives in the
    // wrap_up response, not in an argument description.
    assert!(
        !status.contains("exit_session"),
        "update_task.status must not restate the wrap_up/exit_session sequence, got: {status}"
    );
}

/// rule-guidance.SetVerifyCommandViaMcp ("The tool description does not claim
/// prompt injection"). dispatch-prompt.allium's
/// `ThePromptCarriesNoVerifyCommand` is explicit that the prompt deliberately
/// carries no verify command; the description used to say the opposite, which
/// sends a reader looking for the command on the one
/// surface that never carries it. Assert both halves: the false claim is gone,
/// and the two surfaces that do carry it are named.
#[test]
fn set_verify_command_description_names_the_surfaces_that_carry_the_command() {
    let desc = tool_description("set_verify_command");
    assert!(
        desc.contains("never appears in a dispatch prompt"),
        "set_verify_command must state that the command reaches no dispatch prompt, got: {desc}"
    );
    assert!(
        !desc.contains("injected into"),
        "set_verify_command must not claim the command is injected into prompts, got: {desc}"
    );
    for needle in ["get_task", "wrap_up"] {
        assert!(
            desc.contains(needle),
            "set_verify_command description must name the '{needle}' surface, got: {desc}"
        );
    }
}

/// An `update_*` tool's description opens by listing what it can change, and
/// that list is the first thing a caller reads. Derived from each tool's own
/// schema rather than hardcoded, so a field added to the argument set fails
/// here until the description learns about it — the drift that left seven of
/// `update_task`'s fifteen fields unadvertised.
///
/// Applies to every `update_*` tool rather than one hand-picked tool, so which
/// of them carries the guarantee is not a judgement call.
#[test]
fn every_update_tool_description_names_every_field_it_accepts() {
    let defs = tool_definitions();
    let updaters = defs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["name"].as_str().unwrap().starts_with("update_"));
    let mut checked = 0;
    for tool in updaters {
        let name = tool["name"].as_str().unwrap();
        let desc = tool["description"].as_str().unwrap();
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        // `update_task` selects with task_id, `update_epic` with epic_id: the
        // tool's own name gives the selector, which is not a mutable field.
        let selector = format!("{}_id", name.trim_start_matches("update_"));
        for field in props.keys() {
            if *field == selector {
                continue;
            }
            assert!(
                desc.contains(field.as_str()),
                "{name}'s description must name the '{field}' field it accepts, got: {desc}"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 2,
        "expected update_task and update_epic, saw {checked}"
    );
}

/// rule-guidance.GetTaskViaMcp ("The tool description is one sentence"). The
/// format is shown by what the call returns; naming the lines a wrapping-up
/// agent reads belongs to the /wrap-up skill, the caller that reads them.
#[test]
fn get_task_description_is_one_sentence() {
    let desc = tool_description("get_task");
    assert_eq!(
        sentence_count(&desc),
        1,
        "get_task's description must be one sentence, got: {desc}"
    );
    assert!(
        !desc.contains("JSON") && !desc.contains("Verify command"),
        "get_task's description must not describe the response format or name its lines, \
got: {desc}"
    );
}

/// Sentences in `text`, split on terminal punctuation.
fn sentence_count(text: &str) -> usize {
    text.split_terminator(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count()
}

/// `list_tasks` renders a task's url under its own type label, so a
/// security-alert task prints "Security alert:" and never "PR:". The
/// description promised a PR URL, which sends a caller scanning for the wrong
/// word on three of the four url types.
#[test]
fn list_tasks_description_does_not_promise_a_pr_only_url_label() {
    let desc = tool_description("list_tasks");
    assert!(
        !desc.contains("PR URL"),
        "list_tasks must not describe the url output as a PR URL, got: {desc}"
    );
    // Derived from the labels the output actually prints, so a renamed or added
    // url type fails here rather than quietly going unadvertised.
    for url_type in crate::models::UrlType::ALL {
        let label = url_type.type_word();
        assert!(
            desc.contains(label),
            "list_tasks description must name the '{label}' url label, got: {desc}"
        );
    }
}

/// No tool description names the code that implements it. A description is
/// agent-facing prose with the same rot problem `check-doc-symbols.sh` exists
/// to catch in `docs/` — and it slips through that checker: the script does
/// scan `src/`, but harvests only doc-comment lines (`///`, `//!`), so a
/// description, being a string literal, is invisible to it. Public interface
/// names are fine: every MCP tool name, and every argument name any tool
/// declares, are what a caller actually passes.
///
/// Detects multi-segment `snake_case` words, which is the shape an internal
/// Rust item takes and the shape ordinary prose does not. That is one of the
/// five shapes the script rejects, not all five — a PascalCase-and-method
/// citation, a file-path-and-symbol one, an empty call, and a macro invocation
/// all pass this test and have not yet appeared in a description.
#[test]
fn no_tool_description_cites_internal_symbol_names() {
    let defs = tool_definitions();
    let tools = defs["tools"].as_array().unwrap();

    // Public vocabulary: tool names plus every declared argument name.
    let mut allowed: std::collections::HashSet<&str> =
        super::dispatch::TOOL_NAMES.iter().copied().collect();
    for tool in tools {
        if let Some(props) = tool["inputSchema"]["properties"].as_object() {
            allowed.extend(props.keys().map(String::as_str));
        }
    }
    // Settings and enum values a caller sets or reads, not internal items.
    // `caller_task_id` is here because create_task's description exists partly
    // to say that argument does NOT exist — naming it is the point.
    allowed.extend([
        "auto_dispatch",
        "saved_repo_paths",
        "verify_command",
        "caller_task_id",
    ]);

    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let desc = tool["description"].as_str().unwrap();
        for word in desc.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            // An empty segment covers a leading, trailing or doubled `_`.
            let is_multi_segment_snake_case = word.contains('_')
                && word.split('_').all(|seg| {
                    !seg.is_empty()
                        && seg
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                });
            if is_multi_segment_snake_case {
                assert!(
                    allowed.contains(word),
                    "{name} description cites '{word}', which is neither a tool name nor an \
                     argument any tool declares — name the behaviour, not the code"
                );
            }
        }
    }
}

/// Every tool description carries at least three sentences. Description detail
/// is the largest single factor in whether a tool gets called correctly, and
/// the common failure is under-description, not bloat: a one-line description
/// leaves the caller guessing at what the tool returns, when not to reach for
/// it, and how it differs from its neighbours. Six tools sat at one or two
/// sentences — `list_epics` was 35 characters.
///
/// Deliberately a floor and not a ceiling: no test caps a description's
/// length. A long description that states contract is doing its job.
#[test]
fn every_tool_description_meets_the_detail_floor() {
    const MIN_SENTENCES: usize = 3;
    // The two descriptions the spec makes one sentence by design:
    // GetTaskViaMcp ("The tool description is one sentence") and
    // learnings.allium's TheAuthoringRulesHaveOneHome.
    const ONE_SENTENCE_BY_SPEC: [&str; 2] = ["get_task", "record_learning"];
    let defs = tool_definitions();
    for tool in defs["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if ONE_SENTENCE_BY_SPEC.contains(&name) {
            continue;
        }
        let desc = tool["description"].as_str().unwrap();
        let sentences = desc
            .split_terminator(['.', '!', '?'])
            .filter(|s| !s.trim().is_empty())
            .count();
        assert!(
            sentences >= MIN_SENTENCES,
            "{name}'s description is {sentences} sentence(s); the floor is {MIN_SENTENCES}. \
             Say what it does, when to use it, and what it does not return. Got: {desc}"
        );
    }
}

/// `create_epic` accepts `parent_epic_id`, so sub-epics are reachable from the
/// create call and not only from `update_epic`. The description is where a
/// caller finds that out.
#[test]
fn create_epic_description_mentions_sub_epics() {
    let desc = tool_description("create_epic").to_lowercase();
    assert!(
        desc.contains("sub-epic") || desc.contains("parent"),
        "create_epic description must mention creating a sub-epic, got: {desc}"
    );
}

/// learnings.allium: TheAuthoringRulesHaveOneHome. The record_learning
/// description is one sentence: it records a non-obvious finding and points at
/// the /learnings skill. Every authoring rule lives in the skill alone; the
/// validator's rejection is the just-in-time copy for a caller without it.
#[test]
fn record_learning_description_is_one_sentence_pointing_at_the_skill() {
    let desc = tool_description("record_learning");
    assert!(
        desc.contains("/learnings"),
        "record_learning description must point at the /learnings skill, got: {desc}"
    );
    assert_eq!(
        sentence_count(&desc),
        1,
        "record_learning's description must be one sentence, got: {desc}"
    );
    let lower = desc.to_lowercase();
    for rule in ["procedural", "lint rule", "failing check", "must not name"] {
        assert!(
            !lower.contains(rule),
            "record_learning's description must not carry the authoring rule {rule:?} — \
it lives in the /learnings skill, got: {desc}"
        );
    }
}

/// mcp-task-tools.allium: WrapUpViaMcp / TheNextCallIsNamedJustInTime. The
/// wrap_up description says what the tool does and returns. It carries no
/// MUST and no "never stop after" — the next call is named by the wrap_up
/// response, at the moment it applies.
#[test]
fn wrap_up_description_carries_no_next_call_mandate() {
    let desc = tool_description("wrap_up");
    assert!(
        !desc.contains("MUST"),
        "wrap_up's description must not carry MUST, got: {desc}"
    );
    assert!(
        !desc.to_lowercase().contains("never stop"),
        "wrap_up's description must not carry never-stop wording, got: {desc}"
    );
    assert!(
        desc.contains("token"),
        "wrap_up's description must still say it returns an exit token, got: {desc}"
    );
}

/// ReviewTasksAreNotWrappedUp: the reason a review task is refused, and the
/// retag escape hatch, live in the refusal message. The description may say
/// in passing that a review-tagged task is refused, and no more.
#[test]
fn wrap_up_description_leaves_the_review_refusal_reason_to_the_error() {
    let desc = tool_description("wrap_up").to_lowercase();
    for needle in ["retag", "merges", "hand it back"] {
        assert!(
            !desc.contains(needle),
            "wrap_up's description must leave {needle:?} to the refusal message, got: {desc}"
        );
    }
}

/// TheNextCallIsNamedJustInTime: the exit_session description says what the
/// tool does — closes the session with the token and, when the epic
/// auto-dispatches, starts its next backlog subtask. It carries no MUST, no
/// "final step" instruction, no "must not dispatch it yourself", and nothing
/// about how to react to its responses (the failed-close response carries
/// that itself).
#[test]
fn exit_session_description_says_only_what_the_tool_does() {
    let desc = tool_description("exit_session");
    let lower = desc.to_lowercase();
    assert!(
        !desc.contains("MUST"),
        "exit_session's description must not carry MUST, got: {desc}"
    );
    for needle in [
        "must not",
        "final step",
        "did not take effect",
        "not treat",
        "never stop",
    ] {
        assert!(
            !lower.contains(needle),
            "exit_session's description must not carry {needle:?}, got: {desc}"
        );
    }
    assert!(
        lower.contains("next backlog subtask") && lower.contains("auto"),
        "exit_session's description must state the positive fact that it starts the \
epic's next backlog subtask when the epic auto-dispatches, got: {desc}"
    );
}

/// keybindings.allium: ListKeybindingsViaMcp. The description says what the
/// call returns and carries no worked example — a real response shows the
/// output better than a sample does.
#[test]
fn list_keybindings_description_carries_no_worked_example() {
    let desc = tool_description("list_keybindings");
    let lower = desc.to_lowercase();
    assert!(
        !lower.contains("for example") && !lower.contains("e.g."),
        "list_keybindings' description must not carry a worked example, got: {desc}"
    );
}

/// The shared-argument rule at the head of mcp-task-tools.allium: an argument
/// that means the same thing on every tool carries one short wording of that
/// meaning, the same on each tool. A tool adds text only where its meaning
/// differs, so the shared wording opens every tool's description of it.
#[test]
fn a_shared_argument_carries_one_wording_on_every_tool() {
    let defs = tool_definitions();
    let tools = defs["tools"].as_array().unwrap();
    for arg in [
        "base_branch",
        "wrap_up_mode",
        "auto_run_plan",
        "phoenix",
        "tag",
        "sort_order",
    ] {
        let wordings: Vec<(&str, &str)> = tools
            .iter()
            .filter_map(|t| {
                let d = t["inputSchema"]["properties"][arg]["description"].as_str()?;
                Some((t["name"].as_str().unwrap(), d))
            })
            .collect();
        assert!(
            wordings.len() >= 2,
            "{arg} is a shared argument, expected it on at least two tools, got: {wordings:?}"
        );
        let (_, shortest) = wordings
            .iter()
            .min_by_key(|(_, d)| d.len())
            .copied()
            .unwrap();
        // The shared meaning: the shortest wording up to its first sentence
        // break, so a tool-specific tail ("Defaults to …", "Pass null to
        // clear.") does not count against it.
        let core = shortest.split(". ").next().unwrap_or(shortest);
        for (tool, d) in &wordings {
            assert!(
                d.starts_with(core),
                "{arg}: {tool}'s wording does not start with the shared one {core:?}, \
got: {d:?} (all: {wordings:?})"
            );
        }
    }
}

/// The same rule's other half: an omitted argument defaults on create but
/// means "no change" on update, so an update tool's argument does not promise
/// a default.
#[test]
fn an_update_tool_promises_no_default_for_an_omitted_argument() {
    let defs = tool_definitions();
    for tool in defs["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if !name.starts_with("update_") {
            continue;
        }
        let Some(props) = tool["inputSchema"]["properties"].as_object() else {
            continue;
        };
        for (arg, schema) in props {
            let d = schema["description"].as_str().unwrap_or("");
            assert!(
                !d.contains("Defaults to"),
                "{name}.{arg}: an omitted argument means no change on an update tool, so it \
must not promise a default, got: {d}"
            );
        }
    }
}

/// Human-UI trivia does not belong in a description: which tags the TUI tag
/// picker offers is nothing a caller acts on.
#[test]
fn no_tag_description_carries_tui_trivia() {
    let defs = tool_definitions();
    for tool in defs["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if let Some(d) = tool["inputSchema"]["properties"]["tag"]["description"].as_str() {
            assert!(
                !d.contains("TUI") && !d.contains("tag picker"),
                "{name}.tag must not carry TUI trivia, got: {d}"
            );
        }
    }
}

/// Implementation detail (how a self-reference is guarded) does not belong in
/// an argument description either.
#[test]
fn no_argument_description_explains_how_the_database_guards_it() {
    let defs = tool_definitions();
    for tool in defs["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let Some(props) = tool["inputSchema"]["properties"].as_object() else {
            continue;
        };
        for (arg, schema) in props {
            let d = schema["description"].as_str().unwrap_or("");
            assert!(
                !d.to_lowercase().contains("database"),
                "{name}.{arg} must not describe how the database guards it, got: {d}"
            );
        }
    }
}

/// `create_task` used to spell out the transport headers that carry caller
/// identity. A caller cannot set, read or change them; the only actionable
/// half is that there is no `caller_task_id` argument to pass.
#[test]
fn create_task_description_does_not_leak_transport_headers() {
    let desc = tool_description("create_task");
    assert!(
        !desc.contains("X-Caller"),
        "create_task description must not name transport headers a caller cannot set, got: {desc}"
    );
    assert!(
        desc.contains("caller_task_id"),
        "create_task description must still say there is no caller_task_id argument, got: {desc}"
    );
}

/// mcp-task-tools.allium: CreateTaskViaMcp / EveryTaskNamesItsEpicOrNull.
/// `epic_id` is a REQUIRED argument whose VALUE may be null, so the schema must
/// advertise it in `required` and admit null as a type. A caller reading the
/// schema is the one place the requirement can be learned before it is hit.
#[test]
fn create_task_schema_requires_a_nullable_epic_id() {
    let def = tool_def("create_task");
    let required = def["inputSchema"]["required"]
        .as_array()
        .expect("create_task must declare required fields")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"epic_id"),
        "epic_id must be a required argument, got: {required:?}"
    );

    let ty = &def["inputSchema"]["properties"]["epic_id"]["type"];
    let members = ty
        .as_array()
        .unwrap_or_else(|| panic!("epic_id must admit null as well as integer, got: {ty}"))
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        members.contains(&"null") && members.contains(&"integer"),
        "epic_id must be [integer, null], got: {members:?}"
    );
}

/// The description and the epic_id field doc must not promise inheritance any
/// more — it is the surface an agent reads before deciding to omit the
/// argument, and the old copy told it omission was the normal case.
#[test]
fn create_task_description_promises_no_epic_inheritance() {
    let def = tool_def("create_task");
    let desc = def["description"].as_str().unwrap().to_string();
    let field = def["inputSchema"]["properties"]["epic_id"]["description"]
        .as_str()
        .expect("epic_id must be documented")
        .to_string();
    for (surface, text) in [("tool description", &desc), ("epic_id field", &field)] {
        assert!(
            !text.to_lowercase().contains("inherit"),
            "{surface} must not promise epic inheritance, got: {text}"
        );
    }
    assert!(
        field.to_lowercase().contains("null"),
        "epic_id field doc must say null means a standalone task, got: {field}"
    );
}

/// `WrapUpAction::ALL` backs the wrap_up/exit_session MCP schema's action
/// enum (dispatch.rs) — a variant added there without updating `ALL` would
/// silently under-advertise it.
#[test]
fn wrap_up_action_all_has_every_variant() {
    assert_eq!(crate::mcp::handlers::tasks::WrapUpAction::ALL.len(), 3);
}

/// `update_task.status` and `update_task.sub_status` are the two schema
/// fields that deliberately advertise a SUBSET of their backing enum
/// (status excludes `archived` — `done` is advertised, but only reachable
/// through the dedicated close-only path, MarkTaskDoneViaMcp; sub_status
/// excludes the system-derived `pr_closed` and `pr_unreachable`) rather than
/// the full `::ALL`. Each subset
/// is its own named const — `TaskStatus::MCP_UPDATABLE` /
/// `SubStatus::MCP_ADVERTISED` — derived in the schema exactly like every
/// full-set field, so a variant silently missing from either (not just a
/// typo in an already-present string) fails here instead of just not being
/// advertised.
#[test]
fn subset_enum_consts_have_every_intended_variant() {
    assert_eq!(TaskStatus::MCP_UPDATABLE.len(), 4);
    assert_eq!(SubStatus::MCP_ADVERTISED.len(), 9);
}

/// Every MCP arg struct carries `#[serde(deny_unknown_fields)]`, so a stray or
/// stale argument (like the discarded `repo_path` on `create_epic`) surfaces
/// as a JSON-RPC error instead of being silently dropped. This exercises every
/// registered tool with a minimal valid payload plus one bogus field — the
/// per-field/per-value behaviour of each struct is covered by that handler's
/// own tests elsewhere in this suite.
///
/// `list_epics` and `get_managed_feed_config` are intentionally absent: both
/// take zero arguments and their handlers never parse `args` at all, so there
/// is no struct for `deny_unknown_fields` to guard.
#[tokio::test]
async fn every_tool_with_args_rejects_unknown_field() {
    let state = test_state().await;

    let payloads: &[(&str, Value)] = &[
        ("update_task", json!({"task_id": 1})),
        ("get_task", json!({"task_id": 1})),
        (
            "create_task",
            json!({"title": "t", "repo_path": "/r", "epic_id": null}),
        ),
        ("list_tasks", json!({})),
        ("create_epic", json!({"title": "t"})),
        ("get_epic", json!({"epic_id": 1})),
        ("update_epic", json!({"epic_id": 1})),
        ("wrap_up", json!({"task_id": 1, "action": "rebase"})),
        ("dispatch_task", json!({"task_id": 1})),
        (
            "subscribe_to_task",
            json!({"watcher_task_id": 1, "target_task_id": 2}),
        ),
        (
            "unsubscribe_from_task",
            json!({"watcher_task_id": 1, "target_task_id": 2}),
        ),
        (
            "record_learning",
            json!({"task_id": 1, "kind": "pitfall", "summary": "s", "scope": "user"}),
        ),
        ("query_learnings", json!({"task_id": 1})),
        (
            "rate_learning",
            json!({"learning_id": 1, "task_id": 1, "verdict": "helped"}),
        ),
        ("delete_learning", json!({"learning_id": 1})),
        ("set_verify_command", json!({"repo_path": "/r"})),
        (
            "exit_session",
            json!({"task_id": 1, "token": "t", "action": "rebase"}),
        ),
        ("set_managed_feed_config", json!({})),
        ("query_usage", json!({})),
        ("list_keybindings", json!({})),
        ("read_context", json!({"uri": "dispatch://task/self"})),
        ("override_poll_owner", json!({"task_id": 1})),
    ];

    let no_arg_tools = ["list_epics", "get_managed_feed_config"];
    let covered: std::collections::BTreeSet<&str> = payloads
        .iter()
        .map(|(n, _)| *n)
        .chain(no_arg_tools)
        .collect();
    let all_tools: std::collections::BTreeSet<&str> =
        super::dispatch::TOOL_NAMES.iter().copied().collect();
    assert_eq!(
        covered, all_tools,
        "every registered tool must appear either in `payloads` or `no_arg_tools`"
    );

    for (name, base_args) in payloads {
        let mut args = base_args.clone();
        args.as_object_mut()
            .unwrap()
            .insert("__bogus_unknown_field__".to_string(), json!(true));

        let resp = call(
            &state,
            "tools/call",
            Some(json!({ "name": name, "arguments": args })),
        )
        .await;
        assert!(
            is_error(&resp),
            "tool '{name}' should reject an unknown argument, got: {:?}",
            resp.result
        );
    }
}

#[tokio::test]
async fn list_projects_tool_is_removed() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_projects", "arguments": {} })),
    )
    .await;
    // Per MCP spec, unknown tools surface as isError: true
    let result = resp.result.expect("expected isError result");
    assert_eq!(result["isError"], json!(true));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Unknown tool"), "got: {text}");
}

/// `claim_task` was the agent-facing "adopt this backlog task into the worktree
/// I already have" call, removed in #3808: it had zero recorded invocations
/// across 627 agent trajectories, no skill told an agent to call it, and it was
/// the last Backlog -> Running path still doing a read-then-write status check
/// instead of an atomic claim.
///
/// `TOOL_NAMES` is the whole guard, rather than a `tools/call` round-trip or
/// `tools_list_returns_tools`. That test compares `tools/list` *against*
/// `TOOL_NAMES`, so it is self-consistent by construction and would stay green
/// if the tool came back. And `TOOL_NAMES` and `dispatch_tool`'s match arms
/// expand from the same `$name` literals in `mcp_tools!`, so a name absent here
/// is unroutable by construction — a live-dispatch check could not fail
/// independently of this assert.
///
/// #3824 tracks folding this and the two sibling tool-absence tests into one
/// registry test that pins `TOOL_NAMES` against an explicit list.
#[test]
fn claim_task_tool_is_removed() {
    assert!(
        !super::dispatch::TOOL_NAMES.contains(&"claim_task"),
        "claim_task must not be a registered tool"
    );
}

#[tokio::test]
async fn create_task_from_session_succeeds() {
    let (state, db) = test_state_with_db().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "T", "repo_path": "/r", "epic_id": null }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let result = resp.result.as_ref().expect("expected result");
    assert!(
        result.get("isError").is_none() || result["isError"] != json!(true),
        "unexpected isError: {:?}",
        resp.result
    );
    let tasks = db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "T");
}

/// tasks.allium: DoneRequiresHumanConfirmation — "there is no MCP delete
/// tool, so removing a task from the board stays a human TUI action", and
/// mcp-task-tools.allium's matching note for epics. Nor is there an archive
/// tool any more.
#[test]
fn tools_list_advertises_no_task_or_epic_delete_or_archive_tool() {
    let defs = tool_definitions();
    let names: Vec<&str> = defs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for forbidden in [
        "delete_task",
        "delete_epic",
        "archive_task",
        "archive_epic",
        "batch_delete",
    ] {
        assert!(
            !names.contains(&forbidden),
            "{forbidden} must not be an MCP tool"
        );
    }
}

/// With `archived` gone from TaskStatus (core.allium), no task or epic
/// status field in any tool schema may advertise it. Learnings keep their own,
/// unrelated `archived` status, so learning tools are skipped.
#[test]
fn no_task_or_epic_status_schema_advertises_archived() {
    fn status_enums(v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Object(map) => {
                for (k, child) in map {
                    if k == "status" || k == "statuses" {
                        if let Some(e) = child.get("enum") {
                            out.push(e.clone());
                        }
                        if let Some(e) = child.get("items").and_then(|i| i.get("enum")) {
                            out.push(e.clone());
                        }
                    }
                    status_enums(child, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|i| status_enums(i, out)),
            _ => {}
        }
    }
    let defs = tool_definitions();
    for tool in defs["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if name.contains("learning") {
            continue;
        }
        let mut enums = vec![];
        status_enums(&tool["inputSchema"], &mut enums);
        for e in enums {
            assert!(
                !e.as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v.as_str() == Some("archived")),
                "{name} advertises 'archived' in a status enum: {e}"
            );
        }
    }
}
