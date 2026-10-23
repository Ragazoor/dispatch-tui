use super::*;
use crate::models::TaskId;

// ---------------------------------------------------------------------------
// schedule: create_task, update_tasks and get_task (schedules.allium)
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

#[tokio::test]
async fn create_task_with_a_schedule_is_armed_by_the_store() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Weekly dep audit",
                "repo_path": "/repo",
                "epic_id": null,
                "schedule": "0 9 * * 1"
            }
        })),
    )
    .await;
    let id = extract_created_task_id(&resp);
    let task = state.db.get_task(id).await.unwrap();
    assert_eq!(task.schedule.as_deref(), Some("0 9 * * 1"));
    let due = task.next_due.expect("the store arms next_due");
    assert_eq!(due.format("%a %H:%M").to_string(), "Mon 09:00");
}

#[tokio::test]
async fn create_task_with_an_invalid_cron_is_refused_and_creates_nothing() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Bad",
                "repo_path": "/repo",
                "epic_id": null,
                "schedule": "every monday"
            }
        })),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("invalid cron schedule"),
        "{}",
        error_message(&resp)
    );
    assert!(state.db.list_all().await.is_empty());
}

#[tokio::test]
async fn update_tasks_sets_and_clears_a_schedule() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = update(
        &state,
        json!({ "task_ids": [task_id.0], "schedule": "30 6 * * *" }),
    )
    .await;
    assert!(!is_error(&resp), "{}", extract_response_text(&resp));
    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(task.schedule.as_deref(), Some("30 6 * * *"));
    assert!(task.next_due.is_some());

    update(&state, json!({ "task_ids": [task_id.0], "schedule": "" })).await;
    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!((task.schedule, task.next_due), (None, None));
}

#[tokio::test]
async fn update_tasks_refuses_a_one_off_schedule_outside_backlog() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    update(
        &state,
        json!({ "task_ids": [task_id.0], "status": "review" }),
    )
    .await;

    let resp = update(
        &state,
        json!({ "task_ids": [task_id.0], "schedule": "0 9 * * 1" }),
    )
    .await;
    let text = format!("{}{}", extract_response_text(&resp), error_message(&resp));
    assert!(
        text.contains("a schedule on a non-phoenix task needs it in backlog"),
        "{text}"
    );
    assert_eq!(state.db.get_task(task_id).await.unwrap().schedule, None);
}

/// One call arms both: the schedule sees the phoenix flag the same call sets.
#[tokio::test]
async fn update_tasks_arms_phoenix_and_schedule_together_outside_backlog() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    update(
        &state,
        json!({ "task_ids": [task_id.0], "status": "review" }),
    )
    .await;

    let resp = update(
        &state,
        json!({ "task_ids": [task_id.0], "phoenix": true, "schedule": "0 9 * * 1" }),
    )
    .await;
    assert!(!is_error(&resp), "{}", extract_response_text(&resp));
    let task = state.db.get_task(task_id).await.unwrap();
    assert!(task.phoenix);
    assert_eq!(task.schedule.as_deref(), Some("0 9 * * 1"));
    assert_eq!(task.next_due, None, "the successor arms it, not this run");
}

#[tokio::test]
async fn done_cannot_travel_with_a_schedule() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    let resp = update(
        &state,
        json!({ "task_ids": [task_id.0], "status": "done", "schedule": "0 9 * * 1" }),
    )
    .await;
    let text = format!("{}{}", extract_response_text(&resp), error_message(&resp));
    assert!(text.contains("schedule"), "{text}");
    assert_ne!(
        state.db.get_task(task_id).await.unwrap().status,
        TaskStatus::Done
    );
}

#[tokio::test]
async fn get_task_shows_the_schedule_only_when_set() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    assert!(!get_task_text(&state, task_id).await.contains("Schedule"));

    update(
        &state,
        json!({ "task_ids": [task_id.0], "schedule": "0 9 * * 1" }),
    )
    .await;
    let text = get_task_text(&state, task_id).await;
    assert!(text.contains("Schedule: 0 9 * * 1 (UTC)"), "{text}");
    assert!(text.contains("next due "), "{text}");
    assert!(!text.contains("Schedule error"), "{text}");
}
