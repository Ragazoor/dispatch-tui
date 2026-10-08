use super::*;

async fn make_task(state: &Arc<McpState>, title: &str) -> crate::models::TaskId {
    state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture(title, "/repo")
        })
        .await
        .unwrap()
}

/// `mcp-task-tools.allium: OverridePollOwnerViaMcp`, task scope. On a
/// single-machine test harness (no shared store attached) the underlying
/// `PollOwnershipStore::override_poll_owner` is a harmless no-op — see
/// `store::PollOwnershipStore` — so this asserts the call succeeds and reports
/// the reassignment, not that a row changed (there is none to read back on
/// this backing).
#[tokio::test]
async fn override_poll_owner_reassigns_a_tasks_pr_poll_claim() {
    let state = test_state().await;
    let task = make_task(&state, "Reviewed").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "override_poll_owner",
            "arguments": {"task_id": task.0}
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("PR-poll ownership") && text.contains("reassigned"),
        "got: {text}"
    );
}

#[tokio::test]
async fn override_poll_owner_reassigns_an_epics_feed_poll_claim() {
    let state = test_state().await;
    let epic = state
        .db_write()
        .create_epic("Feed Epic", "", None)
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "override_poll_owner",
            "arguments": {"epic_id": epic.id.0}
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("Feed-poll ownership") && text.contains("reassigned"),
        "got: {text}"
    );
}

#[tokio::test]
async fn override_poll_owner_rejects_neither_id() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "override_poll_owner",
            "arguments": {}
        })),
    )
    .await;

    assert_error(&resp, "provide one of task_id or epic_id");
}

#[tokio::test]
async fn override_poll_owner_rejects_an_unresolved_task_id() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "override_poll_owner",
            "arguments": {"task_id": 999}
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn override_poll_owner_rejects_an_unresolved_epic_id() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "override_poll_owner",
            "arguments": {"epic_id": 999}
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn override_poll_owner_rejects_both_ids() {
    let state = test_state().await;
    let task = make_task(&state, "Reviewed").await;
    let epic = state
        .db_write()
        .create_epic("Feed Epic", "", None)
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "override_poll_owner",
            "arguments": {"task_id": task.0, "epic_id": epic.id.0}
        })),
    )
    .await;

    assert_error(&resp, "provide exactly one of task_id or epic_id");
}
