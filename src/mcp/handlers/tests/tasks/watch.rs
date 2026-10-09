use super::*;

async fn make_task(
    state: &Arc<McpState>,
    title: &str,
    status: TaskStatus,
) -> crate::models::TaskId {
    state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "desc",
            status,
            ..CreateTaskRequest::fixture(title, "/repo")
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn watch_task_on_registers_watch() {
    let state = test_state().await;
    let watcher = make_task(&state, "Watcher", TaskStatus::Running).await;
    let target = make_task(&state, "Target", TaskStatus::Running).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "watch_task",
            "arguments": {"watcher_task_id": watcher.0, "target_task_id": target.0, "watch": true}
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Now watching"), "got: {text}");
    let watchers = state.db_write().list_watchers_of(target).await.unwrap();
    assert_eq!(watchers, vec![watcher]);
}

#[tokio::test]
async fn watch_task_on_already_finished_does_not_register() {
    let state = test_state().await;
    let watcher = make_task(&state, "Watcher", TaskStatus::Running).await;
    let target = make_task(&state, "Target", TaskStatus::Done).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "watch_task",
            "arguments": {"watcher_task_id": watcher.0, "target_task_id": target.0, "watch": true}
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("already finished"), "got: {text}");
    assert!(state
        .db_write()
        .list_watchers_of(target)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn watch_task_on_rejects_self_watch() {
    let state = test_state().await;
    let task = make_task(&state, "Solo", TaskStatus::Running).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "watch_task",
            "arguments": {"watcher_task_id": task.0, "target_task_id": task.0, "watch": true}
        })),
    )
    .await;

    // Self-watch is rejected as a domain validation error, which
    // `service_err_to_response` surfaces as a JSON-RPC tool error (see
    // `assert_error`/`error_message` usage elsewhere in this suite).
    assert_error(&resp, "watch itself");
}

#[tokio::test]
async fn watch_task_off_removes_watch() {
    let state = test_state().await;
    let watcher = make_task(&state, "Watcher", TaskStatus::Running).await;
    let target = make_task(&state, "Target", TaskStatus::Running).await;
    state
        .db_write()
        .create_task_watcher(watcher, target)
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "watch_task",
            "arguments": {"watcher_task_id": watcher.0, "target_task_id": target.0, "watch": false}
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("No longer watching"), "got: {text}");
    assert!(state
        .db_write()
        .list_watchers_of(target)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn watch_task_off_without_subscription_succeeds() {
    let state = test_state().await;
    let watcher = make_task(&state, "Watcher", TaskStatus::Running).await;
    let target = make_task(&state, "Target", TaskStatus::Running).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "watch_task",
            "arguments": {"watcher_task_id": watcher.0, "target_task_id": target.0, "watch": false}
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("No longer watching"), "got: {text}");
}

#[tokio::test]
async fn watch_task_requires_watch_argument() {
    let state = test_state().await;
    let watcher = make_task(&state, "Watcher", TaskStatus::Running).await;
    let target = make_task(&state, "Target", TaskStatus::Running).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "watch_task",
            "arguments": {"watcher_task_id": watcher.0, "target_task_id": target.0}
        })),
    )
    .await;

    assert_error(&resp, "watch");
    assert!(state
        .db_write()
        .list_watchers_of(target)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn old_watch_tools_are_gone() {
    let state = test_state().await;
    for name in ["subscribe_to_task", "unsubscribe_from_task"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": {"watcher_task_id": 1, "target_task_id": 2}
            })),
        )
        .await;
        assert_error(&resp, "Unknown tool");
    }
}
