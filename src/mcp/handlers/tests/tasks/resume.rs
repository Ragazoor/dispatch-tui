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
/// The worktree directory exists on disk, in the returned temp dir.
async fn create_detached_task(state: &Arc<McpState>) -> (crate::models::TaskId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    let id = create_running_task_with_window_in(state, repo, None).await;
    let patch = crate::store::TaskPatch::new().tmux_window(None);
    state.db_write().patch_task(id, &patch).await.unwrap();
    let worktree = state.task_svc.get_task(id).await.unwrap().worktree.unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    (id, dir)
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
    let (id, _dir) = create_detached_task(&state).await;

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let text = extract_response_text(&resp);
    assert!(text.contains(&task_ref_of(&state, id).await), "got: {text}");
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
    let (id, _dir) = create_detached_task(&state).await;

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

/// `ResumeTaskViaMcp`: the reply precedes the board's live-window check, so a
/// call with a message warns that a live window drops it. A call without one
/// has nothing to drop and says nothing of it.
#[tokio::test]
async fn resume_task_with_a_message_warns_that_a_live_window_drops_it() {
    let (state, _rx) = test_state_with_notify().await;
    let (id, _dir) = create_detached_task(&state).await;

    let with = resume(&state, json!({ "task_id": id.0, "message": "hi" })).await;
    let text = extract_response_text(&with);
    assert!(text.contains("not delivered"), "got: {text}");

    let without = resume(&state, json!({ "task_id": id.0 })).await;
    let text = extract_response_text(&without);
    assert!(!text.contains("not delivered"), "got: {text}");
}

#[tokio::test]
async fn resume_task_refuses_a_task_that_never_ran() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_task_fixture(&state).await;

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let msg = error_message(&resp);
    assert!(msg.contains("no resumable session"), "{}", msg);
    assert!(
        msg.starts_with(&task_ref_of(&state, id).await),
        "the refusal names the task by id, name and title: {msg}"
    );
    assert!(msg.contains("dispatch_task"), "{}", msg);
    assert!(rx.try_recv().is_err(), "a refusal must not reach the board");
}

#[tokio::test]
async fn resume_task_refuses_a_running_task_without_a_worktree() {
    let (state, mut rx) = test_state_with_notify().await;
    let (id, _dir) = create_detached_task(&state).await;
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

#[tokio::test]
async fn resume_task_refuses_a_task_whose_worktree_is_on_another_machine() {
    let (state, mut rx) = test_state_with_notify().await;
    let (id, _dir) = create_detached_task(&state).await;
    let patch = crate::store::TaskPatch::new().host(Some("other-machine"));
    state.db_write().patch_task(id, &patch).await.unwrap();

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let msg = error_message(&resp);
    assert!(msg.contains("another machine"), "{}", msg);
    assert!(rx.try_recv().is_err(), "a refusal must not reach the board");
}

#[tokio::test]
async fn resume_task_refuses_a_task_whose_worktree_is_gone_from_disk() {
    let (state, mut rx) = test_state_with_notify().await;
    let (id, _dir) = create_detached_task(&state).await;
    let worktree = state.task_svc.get_task(id).await.unwrap().worktree.unwrap();
    std::fs::remove_dir_all(&worktree).unwrap();

    let resp = resume(&state, json!({ "task_id": id.0 })).await;

    let msg = error_message(&resp);
    assert!(msg.contains("no resumable session"), "{}", msg);
    assert!(msg.contains("no longer on disk"), "{}", msg);
    assert!(rx.try_recv().is_err(), "a refusal must not reach the board");
}

// ---- The agent-tree pane's view and resume (agent-tree.allium) ----------

async fn pane_view_for(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
) -> crate::hooks::wire::PaneView {
    let axum::Json(view) = crate::mcp::handlers::handle_pane_view(
        axum::extract::State(state.clone()),
        axum::Json(crate::hooks::wire::PaneViewRequest { task_id }),
    )
    .await
    .unwrap();
    view
}

async fn pane_resume(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
) -> crate::hooks::wire::PaneResumeAnswer {
    let axum::Json(answer) = crate::mcp::handlers::handle_pane_resume(
        axum::extract::State(state.clone()),
        axum::Json(crate::hooks::wire::PaneResumeRequest { task_id }),
    )
    .await;
    answer
}

/// DetachedIsTheBoardsOwnDefinition: a detached task is in detached_agents,
/// not live_agents; a Done task that kept its worktree is in neither.
#[tokio::test]
async fn the_pane_view_lists_detached_agents_apart_from_live_ones() {
    let (state, _rx) = test_state_with_notify().await;
    let (detached, _dir) = create_detached_task(&state).await;
    let (done, _done_dir) = create_detached_task(&state).await;
    state
        .db_write()
        .patch_task(
            done,
            &crate::store::TaskPatch::new().status(TaskStatus::Done),
        )
        .await
        .unwrap();
    let live = create_running_task_with_window_in(&state, "/repo", None).await;

    let view = pane_view_for(&state, live).await;

    let ids = |agents: &[crate::hooks::wire::PaneAgent]| -> Vec<_> {
        agents.iter().map(|a| a.id).collect()
    };
    assert_eq!(ids(&view.live_agents), vec![live]);
    assert_eq!(ids(&view.detached_agents), vec![detached]);
    assert_eq!(view.detached_agents[0].tmux_window, None);
}

/// ResumeIsTheBoardsOwnResume: a detached task's resume is handed to the
/// board as the same event resume_task sends, with no message.
#[tokio::test]
async fn a_pane_resume_of_a_detached_task_hands_it_to_the_board() {
    let (state, mut rx) = test_state_with_notify().await;
    let (id, _dir) = create_detached_task(&state).await;

    let answer = pane_resume(&state, id).await;

    assert_eq!(answer, crate::hooks::wire::PaneResumeAnswer::Accepted);
    match rx.try_recv().expect("expected a board event") {
        BoardEvent::ResumeTask { id: got, message } => {
            assert_eq!(got, id);
            assert_eq!(message, None);
        }
        other => panic!("expected ResumeTask, got {other:?}"),
    }
}

/// ResumeIsTheBoardsOwnResume: an attached task is refused with a reason,
/// and nothing is sent.
#[tokio::test]
async fn a_pane_resume_of_an_attached_task_is_refused() {
    let (state, mut rx) = test_state_with_notify().await;
    let id = create_running_task_with_window_in(&state, "/repo", None).await;

    let answer = pane_resume(&state, id).await;

    match answer {
        crate::hooks::wire::PaneResumeAnswer::Refused { reason } => {
            assert!(reason.contains("already has an agent attached"), "{reason}");
            assert!(!reason.contains(&format!("#{}", id.0)), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "a refusal sends nothing");
}
