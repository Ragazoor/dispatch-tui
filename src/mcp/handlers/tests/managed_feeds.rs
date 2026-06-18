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
