#![allow(clippy::unwrap_used, clippy::expect_used)]
use serde_json::json;

// `call`, `test_state`, and `extract_response_text` are module-private helpers
// in `tests/mod.rs`; a child module can reach its ancestor's private items, so
// `super::` resolves them. `call` returns a typed `JsonRpcResponse` — it does
// NOT implement `Index`, so never write `resp["result"]`; use the helper or the
// `resp.result` / `resp.error` fields directly.
use super::{call, extract_response_text, test_state};

#[tokio::test]
async fn get_returns_current_config() {
    let state = test_state().await;
    state
        .db
        .set_reviews_feed_command(Some("/scripts/reviews.sh"))
        .await
        .unwrap();
    state
        .db
        .set_reviews_feed_interval_secs(Some(300))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "get_managed_feed_config", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("/scripts/reviews.sh"), "got: {text}");
    assert!(text.contains("300"), "got: {text}");
}

#[tokio::test]
async fn get_empty_reports_unset() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "get_managed_feed_config", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("unset"), "got: {text}");
}

use std::sync::Arc;

use crate::mcp::McpState;
use crate::models::FeedRole;

/// Issue a set_managed_feed_config call and discard the response (used by the
/// success-path tests that assert on persisted state, not the reply text).
async fn set(state: &Arc<McpState>, args: serde_json::Value) {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "set_managed_feed_config", "arguments": args })),
    )
    .await;
}

#[tokio::test]
async fn set_persists_all_four() {
    let state = test_state().await;
    set(
        &state,
        json!({
            "reviews_command": "/r.sh",
            "reviews_interval_secs": 120,
            "cve_command": "/c.sh",
            "cve_interval_secs": 600
        }),
    )
    .await;

    assert_eq!(
        state.db.get_reviews_feed_command().await.unwrap().as_deref(),
        Some("/r.sh")
    );
    assert_eq!(
        state.db.get_reviews_feed_interval_secs().await.unwrap(),
        Some(120)
    );
    assert_eq!(
        state.db.get_cve_feed_command().await.unwrap().as_deref(),
        Some("/c.sh")
    );
    assert_eq!(
        state.db.get_cve_feed_interval_secs().await.unwrap(),
        Some(600)
    );
}

#[tokio::test]
async fn set_omitted_field_leaves_existing() {
    let state = test_state().await;
    state
        .db
        .set_reviews_feed_command(Some("/existing.sh"))
        .await
        .unwrap();

    // Update only the CVE command; reviews_command is omitted entirely.
    set(&state, json!({ "cve_command": "/c.sh" })).await;

    assert_eq!(
        state.db.get_reviews_feed_command().await.unwrap().as_deref(),
        Some("/existing.sh"),
        "omitted field must not be touched"
    );
    assert_eq!(
        state.db.get_cve_feed_command().await.unwrap().as_deref(),
        Some("/c.sh")
    );
}

#[tokio::test]
async fn set_null_clears() {
    let state = test_state().await;
    state
        .db
        .set_reviews_feed_command(Some("/existing.sh"))
        .await
        .unwrap();

    set(&state, json!({ "reviews_command": null })).await;

    assert_eq!(
        state.db.get_reviews_feed_command().await.unwrap(),
        None,
        "explicit null must clear the value"
    );
}

#[tokio::test]
async fn set_negative_interval_rejected() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "set_managed_feed_config",
            "arguments": { "reviews_interval_secs": -5 }
        })),
    )
    .await;
    // Tool execution errors are re-wrapped as isError: true results (per MCP spec).
    let result = resp
        .result
        .as_ref()
        .expect("expected isError result, got no result");
    assert_eq!(
        result["isError"],
        json!(true),
        "expected isError: true, got: {resp:?}"
    );
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("reviews_interval_secs"),
        "expected error message about reviews_interval_secs, got: {text}"
    );
    // Nothing persisted.
    assert_eq!(
        state.db.get_reviews_feed_interval_secs().await.unwrap(),
        None
    );
}

#[tokio::test]
async fn set_provisions_managed_epics() {
    let state = test_state().await;
    set(
        &state,
        json!({ "reviews_command": "/r.sh", "cve_command": "/c.sh" }),
    )
    .await;

    let epics = state.db.list_epics().await.unwrap();
    let role_count = |role: FeedRole| epics.iter().filter(|e| e.feed_role == role).count();
    assert_eq!(role_count(FeedRole::ReviewsParent), 1, "epics: {epics:?}");
    assert_eq!(role_count(FeedRole::MyReviews), 1);
    assert_eq!(role_count(FeedRole::TeamReviews), 1);
    assert_eq!(role_count(FeedRole::Bots), 1);
    assert_eq!(role_count(FeedRole::Cve), 1);
}
