#![allow(clippy::unwrap_used)]
use super::*;

#[tokio::test]
async fn mcp_tool_call_records_usage_event() {
    let (state, mut bg_done) = test_state_with_bg_done().await;

    call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    // Await the fire-and-forget usage write deterministically (Session identity
    // spawns only the usage write, so exactly one signal arrives).
    let which = tokio::time::timeout(std::time::Duration::from_secs(5), bg_done.recv())
        .await
        .expect("timed out waiting for usage write")
        .expect("bg_done channel closed");
    assert_eq!(which, BackgroundWrite::Usage);

    let results = state
        .db
        .query_usage(&crate::db::UsageQuery::default())
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].action, "list_tasks");
    assert_eq!(results[0].category, "mcp_tool");
}

#[tokio::test]
async fn query_usage_mcp_tool_returns_aggregated_counts() {
    use crate::models::{UsageActor, UsageCategory, UsageEvent};

    let state = test_state().await;
    let event = UsageEvent {
        category: UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: Some("d".to_string()),
        actor: UsageActor::Human,
    };
    state.db.record_usage_event(&event).await.unwrap();
    state.db.record_usage_event(&event).await.unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "query_usage", "arguments": {} })),
    )
    .await;

    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let text = resp.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let arr: serde_json::Value = serde_json::from_str(&text).unwrap();
    let items = arr.as_array().unwrap();
    let kb: Vec<_> = items
        .iter()
        .filter(|r| r["category"] == "keybinding")
        .collect();
    assert_eq!(kb.len(), 1);
    assert_eq!(kb[0]["action"], "dispatch_task");
    assert_eq!(kb[0]["count"], 2);
}
