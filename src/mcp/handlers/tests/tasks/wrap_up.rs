use super::*;
use crate::dispatch::mock_sequence::DispatchScript;
use crate::mcp::handlers::tasks::WrapUpAction;
use crate::models::test_tmux_window;

/// Asserts a `wrap_up` response does not tell the agent to run `/retro`.
///
/// The `/wrap-up` skill runs retro before its commit step, well ahead of
/// `wrap_up`. A response that asks for retro here would run it a second time —
/// after the rebase has already fast-forwarded the base branch, so the edits
/// land nowhere — and re-file findings it filed minutes earlier. Asserted for
/// each action separately because they reach the response through two different
/// paths (the rebase's wait on its request row and the no-host-work actions' immediate answer).
fn assert_no_retro_instruction(action: &str, text: &str) {
    assert!(
        !text.to_lowercase().contains("retro"),
        "wrap_up({action}) response must not instruct the agent to run /retro; got: {text}"
    );
}

/// The exit token `wrap_up` last issued for `task_id`: the id of the task's
/// most recent wrap_up request row (`store-split.allium`, W3).
async fn latest_token(state: &Arc<McpState>, task_id: crate::models::TaskId) -> String {
    state
        .db
        .session_requests()
        .await
        .into_iter()
        .filter(|r| r.task_id == task_id && r.kind == crate::models::SessionRequestKind::WrapUp)
        .map(|r| r.id.0)
        .max()
        .expect("wrap_up must have written a request row")
        .to_string()
}

async fn make_state_with_runner(
    runner: Arc<dyn ProcessRunner>,
) -> (Arc<McpState>, Arc<dyn store::TaskStore>) {
    test_state_with_host(runner, None, None).await
}

/// A runner scripted for one clean `finish_task`: the preflight reads pass, no
/// remote so no pull, and the rebase and fast-forward both succeed. Declared via
/// `DispatchScript` rather than a hand-written queue — see `docs/conventions.md`,
/// "Driving a dispatch: `DispatchScript`, never a hand-written queue".
fn rebase_ok_runner() -> Arc<dyn ProcessRunner> {
    with_clean_status(DispatchScript::finish().no_remote())
}

async fn create_wrappable_task(db: &Arc<dyn store::TaskStore>) -> crate::models::TaskId {
    let task_id = db
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-t")),
    )
    .await
    .unwrap();
    task_id
}

/// Helper: creates a test state with a real notification channel.
async fn test_state_with_notify() -> (
    Arc<McpState>,
    tokio::sync::mpsc::UnboundedReceiver<crate::board_event::BoardEvent>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (state, _db) =
        test_state_with_host(Arc::new(MockProcessRunner::new(vec![])), Some(tx), None).await;
    (state, rx)
}

/// The commit the worktree is at, as `git rev-parse HEAD` prints it.
const HEAD_A: &str = "aaaaaaa\n";

const HEAD_B: &str = "bbbbbbb\n";

async fn wrap_up_call(
    state: &Arc<McpState>,
    task_id: crate::models::TaskId,
    action: &str,
) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": action } })),
    )
    .await
}

async fn assert_no_token(state: &McpState, task_id: crate::models::TaskId) {
    assert!(
        !state
            .db
            .session_requests()
            .await
            .iter()
            .any(|r| r.task_id == task_id),
        "a refused wrap_up must not write a request row, so mints no exit token"
    );
}

/// A runner scripted for the one check `wrap_up` makes before it acts (the
/// commit check; `status` is what `git status --porcelain` prints), then
/// whatever `after` answers.
fn checked_runner(
    status: &str,
    after: Vec<(
        Option<std::time::Duration>,
        anyhow::Result<std::process::Output>,
    )>,
) -> Arc<MockProcessRunner> {
    let mut responses = vec![(None, MockProcessRunner::ok_with_stdout(status.as_bytes()))];
    responses.extend(after);
    Arc::new(MockProcessRunner::new_with_delays(responses))
}

async fn wrappable_task_with_verify(
    runner: Arc<MockProcessRunner>,
    verify_command: Option<&str>,
) -> (Arc<McpState>, crate::models::TaskId) {
    let (state, db) = make_state_with_runner(runner).await;
    let task_id = create_wrappable_task(&db).await;
    if let Some(cmd) = verify_command {
        db.save_repo_path("/repo").await.unwrap();
        db.set_verify_command("/repo", Some(cmd)).await.unwrap();
    }
    (state, task_id)
}

mod epic_recalc;
mod exit_session;
mod notifications;
mod ready_checks;
mod request_flow;
