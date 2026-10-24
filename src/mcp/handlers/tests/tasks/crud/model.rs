use super::*;
use crate::models::{TaskId, TaskModel};

// ---------------------------------------------------------------------------
// model: create_task, update_tasks and get_task (core.allium: TaskModel,
// tasks.allium: SetModel)
// ---------------------------------------------------------------------------

async fn update(state: &Arc<McpState>, args: serde_json::Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "update_tasks", "arguments": args })),
    )
    .await
}

async fn get_task_text(state: &Arc<McpState>, id: TaskId) -> String {
    extract_response_text(
        &call(
            state,
            "tools/call",
            Some(json!({ "name": "get_task", "arguments": { "task_id": id.0 } })),
        )
        .await,
    )
}

async fn create(state: &Arc<McpState>, extra: serde_json::Value) -> JsonRpcResponse {
    let mut args = json!({ "title": "T", "repo_path": "/repo", "epic_id": null });
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    call(
        state,
        "tools/call",
        Some(json!({ "name": "create_task", "arguments": args })),
    )
    .await
}

#[tokio::test]
async fn create_task_stores_the_model() {
    let state = test_state().await;
    let id = extract_created_task_id(&create(&state, json!({ "model": "opus" })).await);
    let task = state.db.get_task(id).await.unwrap();
    assert_eq!(task.model, Some(TaskModel::Opus));
}

#[tokio::test]
async fn create_task_without_a_model_leaves_it_unset() {
    let state = test_state().await;
    let id = extract_created_task_id(&create(&state, json!({})).await);
    let task = state.db.get_task(id).await.unwrap();
    assert_eq!(task.model, None);
    assert_eq!(task.launch_model(), TaskModel::Sonnet);
}

#[tokio::test]
async fn create_task_refuses_an_unknown_model() {
    let state = test_state().await;
    let resp = create(&state, json!({ "model": "gpt-4" })).await;
    assert!(is_error(&resp));
    assert!(state.db.list_all().await.is_empty());
}

#[tokio::test]
async fn update_tasks_sets_and_clears_the_model() {
    let state = test_state().await;
    let id = extract_created_task_id(&create(&state, json!({})).await);

    let resp = update(&state, json!({ "task_ids": [id.0], "model": "haiku" })).await;
    assert!(!is_error(&resp), "{}", error_message(&resp));
    assert_eq!(
        state.db.get_task(id).await.unwrap().model,
        Some(TaskModel::Haiku)
    );

    let resp = update(&state, json!({ "task_ids": [id.0], "model": null })).await;
    assert!(!is_error(&resp), "{}", error_message(&resp));
    assert_eq!(state.db.get_task(id).await.unwrap().model, None);
}

#[tokio::test]
async fn get_task_shows_the_model_and_the_default() {
    let state = test_state().await;
    let unset = extract_created_task_id(&create(&state, json!({})).await);
    let fable = extract_created_task_id(&create(&state, json!({ "model": "fable" })).await);

    assert!(
        get_task_text(&state, unset)
            .await
            .contains("\nModel: sonnet (default)"),
        "{}",
        get_task_text(&state, unset).await
    );
    let text = get_task_text(&state, fable).await;
    assert!(text.contains("\nModel: fable\n"), "{text}");
}
