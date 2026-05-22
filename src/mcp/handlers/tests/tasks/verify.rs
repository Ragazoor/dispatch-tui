#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn set_verify_command_stores_and_returns_confirmation() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "set_verify_command",
            "arguments": { "repo_path": "/my/repo", "command": "make test" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("make test"),
        "response should echo the command; got: {text}"
    );

    let stored = state.db.get_verify_command("/my/repo").await.unwrap();
    assert_eq!(stored, Some("make test".to_string()));
}

#[tokio::test]
async fn set_verify_command_rejects_multiline_command() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "set_verify_command",
            "arguments": { "repo_path": "/my/repo", "command": "cargo test\ncargo clippy" }
        })),
    )
    .await;

    assert_error(&resp, "single line");
}

#[tokio::test]
async fn set_verify_command_clears_when_command_omitted() {
    let state = test_state().await;
    state
        .db
        .set_verify_command("/my/repo", Some("old-cmd"))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "set_verify_command",
            "arguments": { "repo_path": "/my/repo" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("cleared"),
        "response should say cleared; got: {text}"
    );

    let stored = state.db.get_verify_command("/my/repo").await.unwrap();
    assert_eq!(stored, None);
}
