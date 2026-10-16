use super::*;

// =======================================================================
// Notification flow tests
// =======================================================================

#[tokio::test]
async fn update_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": { "task_ids": [task_id.0], "status": "running" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    // Should have received a targeted TaskChanged(task_id) event
    let event = rx
        .try_recv()
        .expect("expected notification after update_task");
    assert!(
        matches!(event, crate::board_event::BoardEvent::TaskChanged(t) if t == task_id),
        "expected TaskChanged({task_id:?}), got {event:?}"
    );
}

#[tokio::test]
async fn create_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Notified Task", "repo_path": "/repo", "epic_id": null }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let event = rx
        .try_recv()
        .expect("expected notification after create_task");
    assert!(
        matches!(event, crate::board_event::BoardEvent::TaskChanged(_)),
        "expected TaskChanged, got {event:?}"
    );
}

#[tokio::test]
async fn failed_update_does_not_send_notification() {
    let (state, mut rx) = test_state_with_notify().await;
    let task_id = create_task_fixture(&state).await;

    // Invalid status should not trigger notification
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": { "task_ids": [task_id.0], "status": "bogus" }
        })),
    )
    .await;
    assert!(is_error(&resp));

    assert!(
        rx.try_recv().is_err(),
        "no notification should be sent on validation error"
    );
}
