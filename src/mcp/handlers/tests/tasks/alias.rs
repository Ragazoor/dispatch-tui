//! mcp-task-tools.allium: AliasResolvesToTaskId.

use super::*;

async fn task_with_alias(state: &Arc<McpState>, alias: &str) -> crate::models::TaskId {
    let id = state
        .db_write()
        .create_task(CreateTaskRequest::fixture("T", "/repo"))
        .await
        .unwrap();
    state
        .db_write()
        .patch_task(id, &crate::store::TaskPatch::new().alias(alias))
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn get_task_accepts_the_alias_in_place_of_the_id() {
    let state = test_state().await;
    let id = task_with_alias(&state, "scar").await;

    let by_alias = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": "scar"}})),
    )
    .await;
    assert!(by_alias.error.is_none(), "{:?}", by_alias.error);
    let text = extract_response_text(&by_alias);
    assert!(text.contains(&id.0.to_string()), "{text}");
}

#[tokio::test]
async fn update_tasks_accepts_aliases_in_task_ids() {
    let state = test_state().await;
    let id = task_with_alias(&state, "scar").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": {"task_ids": ["scar"], "title": "by alias"}
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(state.db.get_task(id).await.unwrap().title, "by alias");
}

#[tokio::test]
async fn a_numeric_string_is_still_an_id() {
    let state = test_state().await;
    let id = task_with_alias(&state, "scar").await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": id.0.to_string()}})),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
}

#[tokio::test]
async fn an_unknown_alias_is_not_found_like_an_unknown_id() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": "nope"}})),
    )
    .await;
    let err = resp.error.expect("unknown alias must be an error");
    assert_eq!(err.code, super::super::super::types::NOT_FOUND_CODE);
}

#[tokio::test]
async fn update_tasks_renames_the_alias() {
    let state = test_state().await;
    let id = task_with_alias(&state, "scar").await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": {"task_ids": ["scar"], "alias": "moss"}
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(state.db.get_task(id).await.unwrap().alias, "moss");
}

#[tokio::test]
async fn update_tasks_refuses_a_digit_only_alias() {
    let state = test_state().await;
    let id = task_with_alias(&state, "scar").await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": {"task_ids": [id.0], "alias": "12345"}
        })),
    )
    .await;
    assert_eq!(resp.result.as_ref().unwrap()["isError"], json!(true));
    assert_eq!(state.db.get_task(id).await.unwrap().alias, "scar");
}
