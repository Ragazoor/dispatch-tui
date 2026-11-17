use serde_json::json;

use super::{assert_error, call, extract_response_text, test_state};

async fn call_named(
    state: &std::sync::Arc<crate::mcp::McpState>,
    name: &str,
    args: serde_json::Value,
) -> super::JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({"name": name, "arguments": args})),
    )
    .await
}

async fn call_tool(
    state: &std::sync::Arc<crate::mcp::McpState>,
    args: serde_json::Value,
) -> super::JsonRpcResponse {
    call_named(state, "review-command_set", args).await
}

async fn read(state: &std::sync::Arc<crate::mcp::McpState>) -> super::JsonRpcResponse {
    call_named(state, "review-command_read", json!({})).await
}

/// `ReadReviewCommandViaMcp` (docs/specs/mcp-task-tools.allium): the read tool
/// names the default and writes nothing.
#[tokio::test]
async fn the_read_tool_names_the_default_and_writes_nothing() {
    let state = test_state().await;
    let resp = read(&state).await;
    let text = extract_response_text(&resp);
    assert!(text.contains("/dispatch:preference-review"), "got: {text}");
    assert_eq!(
        state.db.get_setting_string("review_command").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn a_command_is_saved_and_named_in_the_reply() {
    let state = test_state().await;
    let resp = call_tool(&state, json!({"command": "/review-pr"})).await;
    assert!(extract_response_text(&resp).contains("/review-pr"));
    assert_eq!(
        state.db.get_setting_string("review_command").await.unwrap(),
        Some("/review-pr".to_string())
    );
    // A later read names the saved command.
    let read = read(&state).await;
    assert!(extract_response_text(&read).contains("/review-pr"));
}

/// `SetReviewCommandViaMcp`: the command is required. A call without it is
/// refused and does not read, because reading is review-command_read.
#[tokio::test]
async fn the_set_tool_requires_a_command() {
    let state = test_state().await;
    let resp = call_tool(&state, json!({})).await;
    assert_error(&resp, "command");
    assert_eq!(
        state.db.get_setting_string("review_command").await.unwrap(),
        None
    );
}

/// `ReadReviewCommandViaMcp`: the read tool takes no arguments.
#[tokio::test]
async fn the_read_tool_takes_no_arguments() {
    let state = test_state().await;
    let resp = call_named(&state, "review-command_read", json!({"command": "/x"})).await;
    assert_error(&resp, "command");
    assert_eq!(
        state.db.get_setting_string("review_command").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn a_blank_command_restores_the_default() {
    let state = test_state().await;
    call_tool(&state, json!({"command": "/review-pr"})).await;
    let resp = call_tool(&state, json!({"command": "  "})).await;
    let text = extract_response_text(&resp);
    assert!(text.contains("/dispatch:preference-review"), "got: {text}");
}

#[tokio::test]
async fn a_multiline_command_is_refused() {
    let state = test_state().await;
    let resp = call_tool(&state, json!({"command": "/a\n/b"})).await;
    assert_error(&resp, "single line");
    assert_eq!(
        state.db.get_setting_string("review_command").await.unwrap(),
        None
    );
}
