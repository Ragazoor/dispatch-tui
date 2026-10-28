//! `resume_task` (`mcp-task-tools.allium: ResumeTaskViaMcp`): the handler
//! checks what it can see, then hands the resume to the board as an event.

use super::*;
use crate::board_event::BoardEvent;

async fn test_state_with_notify() -> (
    Arc<McpState>,
    tokio::sync::mpsc::UnboundedReceiver<BoardEvent>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (state, _db) = test_state_with_host(
        Arc::new(crate::process::MockProcessRunner::new(vec![])),
        Some(tx),
        None,
    )
    .await;
    (state, rx)
}

/// A Running task with a worktree and no window: what a paused agent leaves.
async fn create_detached_task(state: &Arc<McpState>) -> crate::models::TaskId {
    let id = create_running_task_with_window(state).await;
    let patch = crate::store::TaskPatch::new().tmux_window(None);
    state.db_write().patch_task(id, &patch).await.unwrap();
    id
}

async fn resume(state: &Arc<McpState>, args: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "resume_task", "arguments": args })),
    )
    .await
}

#[tokio::test]
async fn resume_task_hands_a_detached_task_to_the_board() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_detached_task(&state).await;

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let text = extract_response_text(&resp);
    assert!(text.contains(&format!("#{}", id.0)), "got: {text}");
    assert!(text.contains("resum"), "got: {text}");
    match rx.try_recv().expect("expected a board event") {
        BoardEvent::ResumeTask { id: got, message } => {
            assert_eq!(got, id);
            assert_eq!(message, None);
        }
        other => panic!("expected ResumeTask, got {other:?}"),
    }
}

#[tokio::test]
async fn resume_task_carries_the_message_to_the_board() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_detached_task(&state).await;

    let resp = resume(
        &state,
        json!({ "task_id": id.0, "message": "rebase on main" }),
    )
    .await;

    assert!(!is_error(&resp), "{:?}", resp);
    match rx.try_recv().expect("expected a board event") {
        BoardEvent::ResumeTask { message, .. } => {
            assert_eq!(message.as_deref(), Some("rebase on main"));
        }
        other => panic!("expected ResumeTask, got {other:?}"),
    }
}

#[tokio::test]
async fn resume_task_refuses_a_task_that_never_ran() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_task_fixture(&state).await;

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let msg = error_message(&resp);
    assert!(msg.contains("no resumable session"), "{}", msg);
    assert!(msg.contains("dispatch_task"), "{}", msg);
    assert!(rx.try_recv().is_err(), "a refusal must not reach the board");
}

#[tokio::test]
async fn resume_task_refuses_a_running_task_without_a_worktree() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_detached_task(&state).await;
    let patch = crate::store::TaskPatch::new().worktree(None);
    state.db_write().patch_task(id, &patch).await.unwrap();

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let msg = error_message(&resp);
    assert!(msg.contains("no resumable session"), "{}", msg);
    assert!(msg.contains("worktree"), "{}", msg);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn resume_task_refuses_a_task_whose_agent_is_attached() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_running_task_with_window(&state).await;

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let msg = error_message(&resp);
    assert!(msg.contains("already"), "{}", msg);
    assert!(msg.contains("SendMessage"), "{}", msg);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn resume_task_refuses_an_unknown_task() {
    let (state, _rx) = test_state_with_notify().await;

    let resp = resume(&state, json!({ "task_id": 99999 })).await;

    assert!(is_error(&resp));
}
