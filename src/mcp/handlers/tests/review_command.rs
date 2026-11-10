use serde_json::json;

use super::{assert_error, call, extract_response_text, test_state};

async fn call_tool(
    state: &std::sync::Arc<crate::mcp::McpState>,
    args: serde_json::Value,
) -> super::JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({"name": "set_review_command", "arguments": args})),
    )
    .await
}

/// `SetReviewCommandViaMcp` (docs/specs/mcp-task-tools.allium): omitting the
/// command is a read that names the default.
#[tokio::test]
async fn no_command_reads_the_default_and_writes_nothing() {
    let state = test_state().await;
    let resp = call_tool(&state, json!({})).await;
    let text = extract_response_text(&resp);
    assert!(text.contains("/code-review"), "got: {text}");
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
    let read = call_tool(&state, json!({})).await;
    assert!(extract_response_text(&read).contains("/review-pr"));
}

#[tokio::test]
async fn a_blank_command_restores_the_default() {
    let state = test_state().await;
    call_tool(&state, json!({"command": "/review-pr"})).await;
    let resp = call_tool(&state, json!({"command": "  "})).await;
    let text = extract_response_text(&resp);
    assert!(text.contains("/code-review"), "got: {text}");
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
