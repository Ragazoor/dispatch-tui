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

// ---------------------------------------------------------------------------
// wrap_up tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_task_not_found() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": 9999, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn wrap_up_rejects_backlog_task() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "cannot be wrapped up");
}

#[tokio::test]
async fn wrap_up_accepts_running_blocked_task() {
    let (state, db) = test_state_with_host(rebase_ok_runner(), None, None).await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("My Task", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-my-task"))
            .sub_status(crate::models::SubStatus::NeedsInput),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_accepts_running_active_task() {
    let (state, db) = test_state_with_host(rebase_ok_runner(), None, None).await;

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

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_response_demands_exit_session_imperatively() {
    // The wrap_up rebase response is the agent's primary cue to call
    // exit_session. It must:
    //   - name exit_session as the next call,
    //   - be imperative (not advisory like "when ready"),
    //   - say the session is not yet closed so the agent does not stop.
    let (state, db) = test_state_with_host(rebase_ok_runner(), None, None).await;

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

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);

    assert!(
        text.contains("exit_session"),
        "response must name exit_session as the next call; got: {text}"
    );
    assert!(
        text.contains("MUST"),
        "response must be imperative (contain 'MUST'); got: {text}"
    );
    assert!(
        text.contains("NOT") || text.contains("not yet"),
        "response must clearly say the session is not yet closed; got: {text}"
    );
    assert!(
        !text.contains("when ready"),
        "response must not be advisory ('when ready'); got: {text}"
    );
    assert!(
        text.contains("action=\"rebase\""),
        "response must tell the agent which action to pass to exit_session; got: {text}"
    );

    let row = state
        .db
        .session_requests()
        .await
        .into_iter()
        .find(|r| r.task_id == task_id)
        .expect("the wrap_up request row is the token after a successful rebase");
    assert!(
        text.contains(&row.id.0.to_string()),
        "response must include the exit token; got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_task_no_worktree() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "cannot be wrapped up");
}

#[tokio::test]
async fn wrap_up_invalid_action() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    state
        .db_write()
        .patch_task(
            task_id,
            &store::TaskPatch::new()
                .host(Some(crate::store::MEMORY_HOST_ID))
                .worktree(Some("/repo/.worktrees/1-t")),
        )
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "teleport" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `teleport`");
}

#[tokio::test]
async fn wrap_up_rebase_returns_started() {
    let (state, db) = test_state_with_host(rebase_ok_runner(), None, None).await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("My Task", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-my-task")),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("wrap_up complete"),
        "Expected 'wrap_up complete', got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_returns_exit_token() {
    let (state, db) = test_state_with_host(
        Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b""), // wrap_up: clean worktree
            MockProcessRunner::ok_with_stdout(b"main\n"),
            MockProcessRunner::ok_with_stdout(b""),
            MockProcessRunner::fail(""),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ])),
        None,
        None,
    )
    .await;
    let task_id = create_wrappable_task(&db).await;

    let resp = call(
        &state,
        "tools/call",
        Some(
            json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "rebase" } }),
        ),
    )
    .await;

    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);

    let row = state
        .db
        .session_requests()
        .await
        .into_iter()
        .find(|r| r.task_id == task_id)
        .expect("the wrap_up request row is the token after wrap_up rebase");
    assert_eq!(
        row.action,
        crate::models::WrapUpMode::Rebase,
        "the row must record the action the token was issued for"
    );
    assert_eq!(token_in_response(&text), row.id.0.to_string());
    assert_no_retro_instruction("rebase", &text);
}

#[tokio::test]
async fn wrap_up_done_returns_exit_token() {
    let (state, db) = wrap_up_state_with_db().await;

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
            .worktree(Some("/repo/.worktrees/1-t"))
            .tmux_window(Some(&test_tmux_window("task-1"))),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "done" } })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);

    {
        let row = state
            .db
            .session_requests()
            .await
            .into_iter()
            .find(|r| r.task_id == task_id)
            .expect("the wrap_up request row is the token after wrap_up done");
        assert_eq!(
            row.action,
            crate::models::WrapUpMode::Done,
            "the row must record the action the token was issued for"
        );
        assert_eq!(token_in_response(&text), row.id.0.to_string());
        assert_no_retro_instruction("done", &text);
    }

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "wrap_up(done) must defer the Done transition to exit_session"
    );
}

// ---------------------------------------------------------------------------
// wrap_up PR action tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_pr_defers_review_and_url_to_exit_session() {
    // wrap_up(pr) no longer takes pr_url and no longer mutates task.status/url —
    // it only issues a token recording action=pr. The terminal mutation moves
    // to exit_session(action="pr", pr_url=...).
    let state = wrap_up_state_with_db().await.0;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("PR Task", "/repo")
        })
        .await
        .unwrap();
    state
        .db_write()
        .patch_task(
            task_id,
            &store::TaskPatch::new()
                .host(Some(crate::store::MEMORY_HOST_ID))
                .worktree(Some("/repo/.worktrees/1-pr-task")),
        )
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "task must stay running until exit_session closes"
    );
    assert!(task.url.is_none(), "url must not be set yet");

    let row = state
        .db
        .session_requests()
        .await
        .into_iter()
        .find(|r| r.task_id == task_id)
        .expect("the wrap_up request row is the token after wrap_up pr");
    assert_eq!(
        row.action,
        crate::models::WrapUpMode::Pr,
        "the row must record action=pr"
    );
}

#[tokio::test]
async fn wrap_up_pr_response_contains_token_and_no_retro_instruction() {
    let state = wrap_up_state_with_db().await.0;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    state
        .db_write()
        .patch_task(
            task_id,
            &store::TaskPatch::new()
                .host(Some(crate::store::MEMORY_HOST_ID))
                .worktree(Some("/repo/.worktrees/1-t")),
        )
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr" }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    // A token IS issued now — the pr path is no longer exit_session-free.
    let row = state
        .db
        .session_requests()
        .await
        .into_iter()
        .find(|r| r.task_id == task_id)
        .expect("wrap_up(pr) must write a request row, which is the exit token");

    let text = extract_response_text(&resp);
    assert_eq!(
        token_in_response(&text),
        row.id.0.to_string(),
        "response must include the exit token; got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "response should direct the agent to exit_session; got: {text}"
    );
    assert!(
        !text.contains("do not call exit_session") && !text.contains("do not call `exit_session`"),
        "response must no longer tell the agent to skip exit_session; got: {text}"
    );
    assert_no_retro_instruction("pr", &text);
    // PR-action wrap_up still nudges the agent to rate any unrated retrieved learnings.
    assert!(
        text.contains("rate_learning"),
        "PR wrap_up response should nudge rate_learning; got: {text}"
    );
}

// ---------------------------------------------------------------------------
// wrap_up rebase helpers + tests
// ---------------------------------------------------------------------------

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

#[tokio::test]
async fn wrap_up_without_verdicts_still_succeeds() {
    let (state, db) = make_state_with_runner(rebase_ok_runner()).await;
    let task_id = create_wrappable_task(&db).await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    if let Some(err) = resp.error.as_ref() {
        assert!(
            !err.message.contains("verdict"),
            "verdict path should not trigger when none provided: {}",
            err.message
        );
    }
}

// ---------------------------------------------------------------------------
// wrap_up verify reminder tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_success_omits_verify_reminder_when_unconfigured() {
    let (state, db) = make_state_with_runner(rebase_ok_runner()).await;
    let task_id = create_wrappable_task(&db).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("Verify before"),
        "Expected no 'Verify before' in unconfigured response, got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "Expected 'exit_session' in response, got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_conflict_returns_error() {
    let (state, db) = test_state_with_host(
        with_clean_status(
            DispatchScript::finish()
                .no_remote()
                .rebase_conflicts_in_stderr(&["foo.rs"]),
        ),
        None,
        None,
    )
    .await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Conflict Task", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-conflict-task")),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "conflict");
    assert_error(&resp, "foo.rs");
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_dirty_primary_worktree_returns_error() {
    // A dirty primary worktree must be reported as its own distinct error —
    // not conflated with a rebase conflict — and must not flip the task's
    // sub_status to Conflict, since no rebase was ever attempted.
    let (state, db) = test_state_with_host(
        with_clean_status(DispatchScript::finish().dirty_primary(&["unrelated.rs"])),
        None,
        None,
    )
    .await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Dirty Primary Task", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-dirty-primary-task")),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "uncommitted");
    assert_error(&resp, "/repo");
    let error_text = error_message(&resp);
    assert!(
        !error_text.to_lowercase().contains("rebase conflict"),
        "dirty primary worktree must not be reported as a rebase conflict, got: {error_text}"
    );
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on dirty primary worktree"
    );
    assert_ne!(
        task.sub_status,
        SubStatus::Conflict,
        "dirty primary worktree must not set the Conflict sub_status"
    );
}

#[tokio::test]
async fn wrap_up_rebase_not_on_main_returns_error() {
    // HEAD is on something other than the base branch, so the finish refuses at
    // its first call — the script declares that as its whole sequence, which is
    // what makes the stale trailing response this queue used to carry
    // impossible to leave behind.
    let (state, db) = test_state_with_host(
        with_clean_status(DispatchScript::finish().head_branch("feature")),
        None,
        None,
    )
    .await;

    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Wrong Branch", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-wrong-branch")),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    assert_error(&resp, "not on main");
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on error"
    );
}

#[tokio::test]
async fn update_task_status_recalculates_epic_status() {
    let state = test_state().await;
    let epic = state.db_write().create_epic("E", "", None).await.unwrap();
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest::fixture("T", "/repo"))
        .await
        .unwrap();
    state
        .db_write()
        .set_task_epic_id(task_id, Some(epic.id))
        .await
        .unwrap();

    // Move subtask to Running
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": { "task_ids": [task_id.0], "status": "running" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_task should succeed: {:?}",
        resp.error
    );

    // Epic stays in backlog (running tasks do not auto-advance)
    let epic = state.db.get_epic(epic.id).await.unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}
// =======================================================================
// Notification flow tests
// =======================================================================

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
// =======================================================================
// wrap_up: reflection nudge
// =======================================================================

async fn make_rebase_state() -> (Arc<dyn store::TaskStore>, Arc<McpState>) {
    let (state, db) = test_state_with_host(rebase_ok_runner(), None, None).await;
    (db, state)
}

async fn seed_task_with_worktree(
    db: &Arc<dyn store::TaskStore>,
    suffix: &str,
) -> crate::models::TaskId {
    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture(&format!("Task {suffix}"), "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some(&format!(
                "/repo/.worktrees/{}-task-{suffix}",
                task_id.0
            ))),
    )
    .await
    .unwrap();
    task_id
}

#[tokio::test]
async fn wrap_up_rebase_directs_to_exit_session_not_reflection_nudge() {
    // After the behavioral change, wrap_up(rebase) no longer emits the
    // reflection nudge inline. Instead it tells the agent to call exit_session,
    // which handles the reflection prompt on first call.
    let (db, state) = make_rebase_state().await;
    let task_id = seed_task_with_worktree(&db, "nudge-default").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("exit_session"),
        "response should direct agent to call exit_session; got: {text}"
    );
    assert!(
        !text.contains("record_learning"),
        "reflection nudge must not appear in wrap_up(rebase) response; got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_omits_reflection_nudge_regardless_of_setting() {
    // The learning_reflection_enabled setting no longer affects wrap_up(rebase)
    // — the nudge has moved to exit_session.
    let (db, state) = make_rebase_state().await;
    db.set_setting_bool("learning_reflection_enabled", true)
        .await
        .unwrap();
    let task_id = seed_task_with_worktree(&db, "nudge-setting-irrelevant").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear in wrap_up(rebase) even when setting=true; got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "response should direct to exit_session; got: {text}"
    );
}

// -- exit_session tests -------------------------------------------------------
//
// exit_session is now a single call: exit_session(task_id, token, action, pr_url?).
// There is no more reflect-then-close two-phase dance — the mandatory
// reflection is the /retro skill, run before exit_session is ever called.

#[tokio::test]
async fn exit_session_rebase_closes_session_in_one_call() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let resp = close_session_via_mcp(&state, task_id, WrapUpAction::Rebase).await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let task = state.db.get_task(task_id).await.unwrap();
    assert!(task.tmux_window.is_none());
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Done));
}

#[tokio::test]
async fn exit_session_done_closes_session_in_one_call() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let resp = close_session_via_mcp(&state, task_id, WrapUpAction::Done).await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let task = state.db.get_task(task_id).await.unwrap();
    assert!(task.tmux_window.is_none());
    assert_eq!(task.status, TaskStatus::Done);
}

#[tokio::test]
async fn exit_session_pr_sets_review_and_url() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let resp = close_session_via_mcp(&state, task_id, WrapUpAction::Pr).await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let task = state.db.get_task(task_id).await.unwrap();
    assert!(task.tmux_window.is_none());
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Review));
    assert_eq!(task.url.as_ref().map(|u| u.url.as_str()), Some(TEST_PR_URL));
    assert_eq!(
        task.url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Pr)
    );
}

#[tokio::test]
async fn exit_session_pr_without_pr_url_errors() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let token = seed_exit_token(&state, task_id, WrapUpAction::Pr).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "pr" }
        })),
    )
    .await;
    assert_error(&resp, "pr_url");

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "a rejected close must not mutate the task"
    );
}

#[tokio::test]
async fn exit_session_action_mismatch_errors() {
    // Token was issued for wrap_up(action="rebase"); closing with a different
    // action must be rejected — this stops an agent from doing one thing and
    // closing as another.
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let token = seed_exit_token(&state, task_id, WrapUpAction::Rebase).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": {
                "task_id": task_id.0,
                "token": token,
                "action": "pr",
                "pr_url": "https://github.com/owner/repo/pull/1"
            }
        })),
    )
    .await;
    assert_error(&resp, "rebase");

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "mismatch must not mutate the task"
    );
    assert!(
        task.tmux_window.is_some(),
        "mismatch must not clear the window"
    );
}

#[tokio::test]
async fn exit_session_without_action_errors() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token }
        })),
    )
    .await;
    assert_error(&resp, "action");
}

#[tokio::test]
async fn exit_session_after_close_token_is_gone() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;

    // Close.
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;
    assert_eq!(extract_response_text(&resp), "Session closed.");

    // Second call — token is gone, and there's no window left either.
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;
    assert_error(&resp, "wrap_up first");
}

#[tokio::test]
async fn exit_session_full_flow_rebase() {
    let (state, db) = test_state_with_host(
        Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b""), // wrap_up: clean worktree
            MockProcessRunner::ok_with_stdout(b"main\n"),
            MockProcessRunner::ok_with_stdout(b""),
            MockProcessRunner::fail(""),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ])),
        None,
        None,
    )
    .await;
    let task_id = create_wrappable_task(&db).await;
    db.patch_task(
        task_id,
        &store::TaskPatch::new().tmux_window(Some(&test_tmux_window("task-1"))),
    )
    .await
    .unwrap();

    // Rebase
    let wrap_resp = call(
        &state,
        "tools/call",
        Some(
            json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "rebase" } }),
        ),
    )
    .await;
    assert!(!is_error(&wrap_resp));

    let token = latest_token(&state, task_id).await;

    // Single exit_session call closes the session.
    let close_resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "rebase" }
        })),
    )
    .await;
    assert_eq!(extract_response_text(&close_resp), "Session closed.");

    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn wrap_up_second_call_overwrites_token() {
    let (state, db) = test_state_with_host(
        Arc::new(MockProcessRunner::new(vec![
            // First rebase
            MockProcessRunner::ok_with_stdout(b""), // wrap_up: clean worktree
            MockProcessRunner::ok_with_stdout(b"main\n"),
            MockProcessRunner::ok_with_stdout(b""),
            MockProcessRunner::fail(""),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
            // Second rebase
            MockProcessRunner::ok_with_stdout(b""), // wrap_up: clean worktree
            MockProcessRunner::ok_with_stdout(b"main\n"),
            MockProcessRunner::ok_with_stdout(b""),
            MockProcessRunner::fail(""),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ])),
        None,
        None,
    )
    .await;
    let task_id = create_wrappable_task(&db).await;
    db.patch_task(
        task_id,
        &store::TaskPatch::new().tmux_window(Some(&test_tmux_window("task-1"))),
    )
    .await
    .unwrap();

    call(
        &state,
        "tools/call",
        Some(
            json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "rebase" } }),
        ),
    )
    .await;
    let first_token = latest_token(&state, task_id).await;

    call(
        &state,
        "tools/call",
        Some(
            json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "rebase" } }),
        ),
    )
    .await;
    let second_token = latest_token(&state, task_id).await;

    assert_ne!(
        first_token, second_token,
        "second wrap_up should generate a new token"
    );

    // Old token rejected
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": first_token, "action": "rebase" }
        })),
    )
    .await;
    assert_error(&resp, "invalid exit token");
}

#[tokio::test]
async fn exit_session_unknown_task_returns_error() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": 9999, "token": "any" }
        })),
    )
    .await;

    assert_error(&resp, "9999");
}

#[tokio::test]
async fn exit_session_task_without_window_returns_error() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;

    // A matching token+action gets past those checks; the window is gone by the
    // time the closing call arrives (some other path tore the session down).
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;
    state
        .db_write()
        .patch_task(task_id, &crate::store::TaskPatch::new().tmux_window(None))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;

    assert_error(&resp, "no active session");
}

#[tokio::test]
async fn exit_session_without_token_errors() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "action": "done" } })),
    )
    .await;

    assert_error(&resp, "wrap_up first");
}

#[tokio::test]
async fn exit_session_with_wrong_token_errors() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;

    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;
    assert_ne!(token, "wrong-token", "the token below must not match");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "wrong-token", "action": "done" }
        })),
    )
    .await;

    assert_error(&resp, "invalid exit token");
}

#[tokio::test]
async fn wrap_up_rebase_does_not_kill_window() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "description",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("Rebase Task", "/repo")
        })
        .await
        .unwrap();

    // Set up worktree + tmux_window so is_wrappable passes.
    let window = test_tmux_window("task-rebase-window");
    let patch = crate::store::TaskPatch::new()
        .host(Some(crate::store::MEMORY_HOST_ID))
        .worktree(Some("/repo/.worktrees/task-rebase"))
        .tmux_window(Some(&window));
    state.db_write().patch_task(task_id, &patch).await.unwrap();

    // wrap_up calls finish_task which runs git commands. With MockProcessRunner
    // the git operations will fail, but the key assertion holds for BOTH paths:
    // neither the success path nor the error path should clear tmux_window
    // after this change.
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).await.unwrap();
    // tmux_window must NOT be cleared — exit_session owns the window kill.
    assert!(
        task.tmux_window.is_some(),
        "wrap_up(rebase) must not clear tmux_window — exit_session is responsible"
    );
}

// -- exit_session: Done transition (added with the wrap_up/exit_session alignment) ---

#[tokio::test]
async fn exit_session_marks_task_done_with_no_epic() {
    // No-epic branch: the closing call must mark the task Done even when
    // there is no epic to recalculate. Pins the `is_some()` guard around
    // recalculate_epic_status.
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Done));
    assert!(task.tmux_window.is_none());
    assert!(task.epic_id.is_none(), "fixture should have no epic");
}

#[tokio::test]
async fn exit_session_already_done_task_stays_done() {
    // Idempotency: a task that is somehow already Done before exit_session
    // closes must remain Done after the closing call.
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    // The token is issued while the task is still running (the module only
    // wraps up a running or review task); the task is Done by the time the
    // closing call arrives.
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;
    state
        .db_write()
        .patch_task(
            task_id,
            &crate::store::TaskPatch::new().status(TaskStatus::Done),
        )
        .await
        .unwrap();

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn exit_session_recalculates_epic_status() {
    let state = test_state().await;
    let epic = state.db_write().create_epic("E", "", None).await.unwrap();
    let task_id = create_running_task_with_window(&state).await;
    state
        .db_write()
        .set_task_epic_id(task_id, Some(epic.id))
        .await
        .unwrap();
    state
        .db_write()
        .recalculate_epic_status(epic.id)
        .await
        .unwrap();
    let epic_before = state.db.get_epic(epic.id).await.unwrap();
    assert_ne!(
        epic_before.status,
        TaskStatus::Done,
        "precondition: epic should be in-progress before exit_session"
    );
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;

    let epic_after = state.db.get_epic(epic.id).await.unwrap();
    assert_eq!(
        epic_after.status,
        TaskStatus::Done,
        "epic should auto-advance to Done once its only subtask is Done"
    );
}

#[tokio::test]
async fn exit_session_resets_sub_status_to_default_for_done() {
    // A task that carries a non-default sub_status (e.g. Stale) into the
    // closing call must have it reset to the Done default.
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state
        .db_write()
        .patch_task(
            task_id,
            &crate::store::TaskPatch::new().sub_status(SubStatus::Stale),
        )
        .await
        .unwrap();
    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(
        task.sub_status,
        SubStatus::default_for(TaskStatus::Done),
        "closing exit_session must reset sub_status to default_for(Done)"
    );
}

#[tokio::test]
async fn exit_session_emits_refresh_after_done_patch() {
    // Pin the notify ordering: a Refresh event fires after the closing call
    // commits the Done patch, so the TUI re-renders the task in Done.
    let (state, mut rx) = test_state_with_notify().await;
    let task_id = create_running_task_with_window(&state).await;

    let token = seed_exit_token(&state, task_id, WrapUpAction::Done).await;
    while rx.try_recv().is_ok() {}

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    // DB must already be Done by the time the Refresh fires.
    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);

    let event = rx
        .try_recv()
        .expect("expected TaskChanged after closing exit_session");
    assert!(
        matches!(event, crate::board_event::BoardEvent::TaskChanged(t) if t == task_id),
        "expected TaskChanged({task_id:?}), got {event:?}"
    );
}

#[tokio::test]
async fn wrap_up_then_exit_session_end_to_end() {
    // The finish's own calls come from the script; the two tmux calls after it
    // belong to `exit_session`, which the script has no vocabulary to model, so
    // they are appended — the pattern documented on `DispatchScript::responses`.
    let mut responses = DispatchScript::finish().no_remote().responses();
    responses.insert(0, (None, MockProcessRunner::ok_with_stdout(b"")));
    responses.extend([
        (None, MockProcessRunner::ok()), // tmux has-session
        (None, MockProcessRunner::ok()), // tmux kill-window
    ]);
    let (state, db) = test_state_with_host(
        Arc::new(MockProcessRunner::new_with_delays(responses)),
        None,
        None,
    )
    .await;

    let epic = db.create_epic("E2E Epic", "", None).await.unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("E2E Task", "/repo")
        })
        .await
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).await.unwrap();
    db.patch_task(
        task_id,
        &crate::store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/e2e"))
            .tmux_window(Some(&test_tmux_window("e2e-window"))),
    )
    .await
    .unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic_before = db.get_epic(epic.id).await.unwrap();
    assert_ne!(epic_before.status, TaskStatus::Done);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let after_wrap_up = db.get_task(task_id).await.unwrap();
    assert_eq!(
        after_wrap_up.status,
        TaskStatus::Running,
        "after wrap_up: status must still be Running"
    );
    assert!(
        after_wrap_up.tmux_window.is_some(),
        "after wrap_up: tmux_window must be preserved"
    );
    let epic_after_wrap_up = db.get_epic(epic.id).await.unwrap();
    assert_eq!(
        epic_after_wrap_up.status, epic_before.status,
        "after wrap_up: epic status must not change"
    );

    // The token wrap_up issued: its request row's id.
    let token = latest_token(&state, task_id).await;

    // Single exit_session call closes the session.
    let close_resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "rebase" }
        })),
    )
    .await;
    assert!(close_resp.error.is_none(), "{:?}", close_resp.error);

    let final_task = db.get_task(task_id).await.unwrap();
    assert_eq!(final_task.status, TaskStatus::Done);
    assert_eq!(
        final_task.sub_status,
        SubStatus::default_for(TaskStatus::Done)
    );
    assert!(final_task.tmux_window.is_none());

    let final_epic = db.get_epic(epic.id).await.unwrap();
    assert_eq!(
        final_epic.status,
        TaskStatus::Done,
        "epic auto-advances once its only subtask completes via exit_session"
    );
}

#[tokio::test]
async fn wrap_up_done_defers_done_transition_to_exit_session() {
    let (state, db) = wrap_up_state_with_db().await;

    let task_id = db
        .create_task(CreateTaskRequest {
            status: TaskStatus::Running,
            wrap_up_mode: Some(crate::models::WrapUpMode::Done),
            ..CreateTaskRequest::fixture("Done Task", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-done-task"))
            .tmux_window(Some(&test_tmux_window("task-1"))),
    )
    .await
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "done" }
        })),
    )
    .await;
    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    let text = extract_response_text(&resp);
    assert!(
        text.contains("exit_session"),
        "response should instruct to call exit_session, got: {text}"
    );
    assert!(
        text.contains("done"),
        "response should mention done action, got: {text}"
    );

    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "wrap_up(done) must not mark the task Done yet — exit_session does"
    );

    // Closing the session is what actually marks it Done.
    let token = latest_token(&state, task_id).await;
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Done,
        "task should be Done after exit_session"
    );
}

// ---------------------------------------------------------------------------
// Epic recalc via handler paths (service layer boundary tests)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_done_recalculates_epic_status() {
    // wrap_up(done) on an epic's only running subtask must NOT advance the
    // epic yet (status is deferred to exit_session); the closing call is
    // what auto-advances the epic to Done.
    let (state, db) = wrap_up_state_with_db().await;

    let epic = db.create_epic("E", "", None).await.unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).await.unwrap();
    db.patch_task(
        task_id,
        &store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/1-t"))
            .tmux_window(Some(&test_tmux_window("task-1"))),
    )
    .await
    .unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "done" } })),
    )
    .await;
    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    let epic_after_wrap_up = db.get_epic(epic.id).await.unwrap();
    assert_ne!(
        epic_after_wrap_up.status,
        TaskStatus::Done,
        "epic must not advance until exit_session closes"
    );

    let token = latest_token(&state, task_id).await;
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token, "action": "done" }
        })),
    )
    .await;

    let epic_after_close = db.get_epic(epic.id).await.unwrap();
    assert_eq!(
        epic_after_close.status,
        TaskStatus::Done,
        "epic must auto-advance to Done once its only subtask is marked Done via exit_session"
    );
}

#[tokio::test]
async fn wrap_up_pr_recalculates_epic_status() {
    // wrap_up(pr) no longer moves the task; exit_session(action="pr") does,
    // and the epic recalc should run from that closing mutation.
    let state = wrap_up_state_with_db().await.0;
    let epic = state.db_write().create_epic("E", "", None).await.unwrap();
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    state
        .db_write()
        .set_task_epic_id(task_id, Some(epic.id))
        .await
        .unwrap();
    state
        .db_write()
        .patch_task(
            task_id,
            &store::TaskPatch::new()
                .host(Some(crate::store::MEMORY_HOST_ID))
                .worktree(Some("/repo/.worktrees/1-t"))
                .tmux_window(Some(&test_tmux_window("task-1"))),
        )
        .await
        .unwrap();
    state
        .db_write()
        .recalculate_epic_status(epic.id)
        .await
        .unwrap();
    let epic_before = state.db.get_epic(epic.id).await.unwrap();
    assert_ne!(epic_before.status, TaskStatus::Done, "precondition");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "pr" } })),
    )
    .await;
    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "wrap_up(pr) must not move the task to review yet"
    );

    let token = latest_token(&state, task_id).await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": {
                "task_id": task_id.0,
                "token": token,
                "action": "pr",
                "pr_url": "https://github.com/owner/repo/pull/99"
            }
        })),
    )
    .await;
    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    // Epic should still not be Done (task is Review, not Done)
    // but the recalc ran without panicking and the task is now Review.
    let task = state.db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
}

#[tokio::test]
async fn dispatch_task_recalculates_epic_status() {
    // dispatch_task on an epic's backlog task must trigger epic recalc.
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    // The worktree dir is pre-created below, so this is the reused-worktree
    // shape — see `src/dispatch/mock_sequence.rs`.
    let runner: Arc<dyn ProcessRunner> =
        crate::dispatch::mock_sequence::DispatchScript::dispatch().shared_runner();
    let (mut state, db) = test_state_with_overrides(runner, None, None).await;
    // The request is the claim, and the recalc follows it at once; nothing
    // here serves the request, so do not wait for it to be provisioned.
    Arc::get_mut(&mut state)
        .expect("the fixture's state is not shared yet")
        .dispatch_task_wait_cap = std::time::Duration::ZERO;

    let epic = db.create_epic("E", "", None).await.unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            description: "d",
            plan: Some("docs/plan.md"),
            ..CreateTaskRequest::fixture("Dispatch Me", &repo_path)
        })
        .await
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).await.unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();

    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-dispatch-me", task_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    // Task should now be Running and epic recalculated (still not Done — task just started)
    let task = db.get_task(task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    // Epic recalc ran — epic should still be in backlog/non-done (task is running, not done)
    let epic_after = db.get_epic(epic.id).await.unwrap();
    assert_ne!(
        epic_after.status,
        TaskStatus::Done,
        "epic with a running task should not be Done"
    );
}

#[tokio::test]
async fn wrap_up_tool_schema_includes_done_action() {
    let state = test_state().await;
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
    let wrap_up = tools
        .iter()
        .find(|t| t["name"] == "wrap_up")
        .expect("wrap_up not in tool list");
    let action_enum = wrap_up["inputSchema"]["properties"]["action"]["enum"]
        .as_array()
        .expect("action should have an enum");
    let values: Vec<&str> = action_enum.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        values.contains(&"done"),
        "wrap_up action enum should include 'done', got: {values:?}"
    );
    assert!(
        values.contains(&"rebase"),
        "wrap_up action enum should include 'rebase', got: {values:?}"
    );
    assert!(
        values.contains(&"pr"),
        "wrap_up action enum should include 'pr', got: {values:?}"
    );
}

#[tokio::test]
async fn wrap_up_tool_schema_no_longer_has_pr_url() {
    // pr_url moved from wrap_up to exit_session.
    let state = test_state().await;
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
    let wrap_up = tools
        .iter()
        .find(|t| t["name"] == "wrap_up")
        .expect("wrap_up not in tool list");
    let properties = wrap_up["inputSchema"]["properties"]
        .as_object()
        .expect("wrap_up should have properties");
    assert!(
        !properties.contains_key("pr_url"),
        "wrap_up schema should no longer have pr_url, got: {properties:?}"
    );
}

#[tokio::test]
async fn exit_session_tool_schema_requires_action_and_has_pr_url() {
    let state = test_state().await;
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
    let exit_session = tools
        .iter()
        .find(|t| t["name"] == "exit_session")
        .expect("exit_session not in tool list");
    let required: Vec<&str> = exit_session["inputSchema"]["required"]
        .as_array()
        .expect("exit_session should have a required list")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        required.contains(&"action"),
        "exit_session schema should require 'action', got: {required:?}"
    );
    let action_enum = exit_session["inputSchema"]["properties"]["action"]["enum"]
        .as_array()
        .expect("action should have an enum");
    let values: Vec<&str> = action_enum.iter().filter_map(|v| v.as_str()).collect();
    assert!(values.contains(&"rebase"));
    assert!(values.contains(&"done"));
    assert!(values.contains(&"pr"));
    assert!(
        exit_session["inputSchema"]["properties"]
            .as_object()
            .expect("exit_session should have properties")
            .contains_key("pr_url"),
        "exit_session schema should have a pr_url property"
    );
}

// ---------------------------------------------------------------------------
// wrap_up refuses unfinished work (WrapUpRefusesUnfinishedWork,
// docs/specs/mcp-task-tools.allium)
// ---------------------------------------------------------------------------

const WORKTREE: &str = "/repo/.worktrees/1-t";

/// A runner scripted for the checks `wrap_up` makes before it acts, then
/// whatever `after` answers. `status` is what `git status --porcelain` prints;
/// `verify` is the reply to the verify command, or `None` when the repo has no
/// verify command and so no call is made. With a verify command the commit
/// check is followed by the `git rev-parse HEAD` that keys the outcome memory
/// (`HEAD_A`).
fn checked_runner(
    status: &str,
    verify: Option<anyhow::Result<std::process::Output>>,
    after: Vec<(
        Option<std::time::Duration>,
        anyhow::Result<std::process::Output>,
    )>,
) -> Arc<MockProcessRunner> {
    let mut responses = vec![(None, MockProcessRunner::ok_with_stdout(status.as_bytes()))];
    if let Some(v) = verify {
        responses.push((None, MockProcessRunner::ok_with_stdout(HEAD_A.as_bytes())));
        responses.push((None, v));
    }
    responses.extend(after);
    Arc::new(MockProcessRunner::new_with_delays(responses))
}

/// The commit the worktree is at, as `git rev-parse HEAD` prints it.
const HEAD_A: &str = "aaaaaaa\n";
const HEAD_B: &str = "bbbbbbb\n";

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

#[tokio::test]
async fn wrap_up_refuses_a_worktree_with_uncommitted_changes_for_every_action() {
    for action in ["rebase", "pr", "done"] {
        let runner = checked_runner(" M src/lib.rs\n?? notes.md\n", None, vec![]);
        let (state, task_id) = wrappable_task_with_verify(runner.clone(), None).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(is_error(&resp), "{action}: expected a refusal");
        let msg = error_message(&resp);
        assert!(
            msg.contains("src/lib.rs") && msg.contains("notes.md"),
            "{action}: the refusal must name the uncommitted paths; got: {msg}"
        );
        assert!(
            msg.contains("commit"),
            "{action}: the refusal must say to commit; got: {msg}"
        );
        assert_no_token(&state, task_id).await;
        assert_eq!(
            runner.flattened_calls(),
            vec![format!("git -C {WORKTREE} status --porcelain")],
            "{action}: nothing may run after a dirty tree is found"
        );
    }
}

#[tokio::test]
async fn wrap_up_refuses_when_the_verify_command_fails_and_never_rebases() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::fail_with_code(
            101,
            "test result: FAILED. 1 failed",
        )),
        vec![],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let resp = wrap_up_call(&state, task_id, "rebase").await;

    assert!(is_error(&resp), "expected a refusal");
    let msg = error_message(&resp);
    assert!(
        msg.contains("cargo test"),
        "must name the command; got: {msg}"
    );
    assert!(msg.contains("101"), "must give the exit code; got: {msg}");
    assert!(
        msg.contains("1 failed"),
        "must carry the output tail; got: {msg}"
    );
    assert_no_token(&state, task_id).await;
    assert_eq!(
        runner.recorded_calls().len(),
        3,
        "only the status check, the HEAD read and the verify run may happen; got: {:?}",
        runner.flattened_calls()
    );
}

#[tokio::test]
async fn wrap_up_refuses_pr_and_done_when_the_verify_command_fails() {
    for action in ["pr", "done"] {
        let runner = checked_runner("", Some(MockProcessRunner::fail("boom")), vec![]);
        let (state, task_id) = wrappable_task_with_verify(runner, Some("make check")).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(is_error(&resp), "{action}: expected a refusal");
        assert!(error_message(&resp).contains("make check"));
        assert_no_token(&state, task_id).await;
    }
}

#[tokio::test]
async fn wrap_up_refuses_when_the_verify_command_times_out() {
    let runner = Arc::new(MockProcessRunner::new_with_delays(vec![
        (None, MockProcessRunner::ok()),
        (
            Some(crate::service::WRAP_UP_VERIFY_TIMEOUT),
            MockProcessRunner::ok(),
        ),
    ]));
    let (state, task_id) = wrappable_task_with_verify(runner, Some("cargo test")).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&resp), "expected a refusal");
    assert!(
        error_message(&resp).contains("timed out"),
        "got: {}",
        error_message(&resp)
    );
    assert_no_token(&state, task_id).await;
}

#[tokio::test]
async fn wrap_up_runs_verify_in_the_worktree_after_the_commit_check_and_before_the_rebase() {
    let script = DispatchScript::finish().no_remote();
    let runner = checked_runner("", Some(MockProcessRunner::ok()), script.responses());
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let resp = wrap_up_call(&state, task_id, "rebase").await;

    assert!(!is_error(&resp), "got: {}", error_message(&resp));
    let calls = runner.recorded_calls();
    assert_eq!(
        calls[0].1,
        vec!["-C", WORKTREE, "status", "--porcelain"],
        "the commit check runs first"
    );
    assert_eq!(
        calls[1].1,
        vec!["-C", WORKTREE, "rev-parse", "HEAD"],
        "HEAD is read next, to key the outcome"
    );
    let (program, args) = &calls[2];
    assert_eq!(program, "sh");
    assert!(
        args.iter().any(|a| a == WORKTREE) && args.iter().any(|a| a == "cargo test"),
        "verify must run the stored command in the worktree; got: {args:?}"
    );
    assert!(
        calls[3..]
            .iter()
            .any(|(_, a)| a.contains(&"rebase".to_string())),
        "the rebase comes after verify; got: {:?}",
        runner.flattened_calls()
    );
}

#[tokio::test]
async fn wrap_up_verified_success_says_so_and_has_no_reverify_instruction() {
    for action in ["rebase", "pr", "done"] {
        let after = if action == "rebase" {
            DispatchScript::finish().no_remote().responses()
        } else {
            vec![]
        };
        let runner = checked_runner("", Some(MockProcessRunner::ok()), after);
        let (state, task_id) = wrappable_task_with_verify(runner, Some("cargo test")).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(!is_error(&resp), "{action}: got: {}", error_message(&resp));
        let text = extract_response_text(&resp);
        assert!(
            !text.contains("Verify before"),
            "{action}: the tool ran verify, so it must not ask the agent to: {text}"
        );
        assert!(
            text.contains("`cargo test` passed"),
            "{action}: the response must report the verification: {text}"
        );
        assert!(text.contains("exit_session"), "{action}: {text}");
    }
}

#[tokio::test]
async fn wrap_up_without_a_verify_command_only_checks_the_worktree() {
    let runner = checked_runner("", None, vec![]);
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), None).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&resp), "got: {}", error_message(&resp));
    assert_eq!(runner.recorded_calls().len(), 1);
    assert!(!extract_response_text(&resp).contains("passed"));
}

#[tokio::test]
async fn wrap_up_refusal_names_at_most_ten_paths() {
    let status: String = (0..13).map(|i| format!(" M f{i:02}.rs\n")).collect();
    let runner = checked_runner(&status, None, vec![]);
    let (state, task_id) = wrappable_task_with_verify(runner, None).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&resp));
    let msg = error_message(&resp);
    assert!(
        msg.contains("f09.rs") && !msg.contains("f10.rs"),
        "got: {msg}"
    );
    assert!(msg.contains("and 3 more"), "got: {msg}");
}

/// Answers every call with success, except the verify run (`sh`), which
/// reports that it started and then blocks until the test releases it.
/// `git rev-parse HEAD` answers `head`, which a test moves to model a commit.
struct GatedVerifyRunner {
    started: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    verify_runs: std::sync::atomic::AtomicUsize,
    head: std::sync::Mutex<String>,
    sccache: bool,
    last_script: std::sync::Mutex<String>,
}

impl ProcessRunner for GatedVerifyRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
        if program == "sh" {
            self.verify_runs
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.last_script.lock().unwrap() = args.get(1).unwrap_or(&"").to_string();
            let _ = self.started.lock().unwrap().send(());
            let _ = self.release.lock().unwrap().recv();
        }
        if args.contains(&"rev-parse") {
            return MockProcessRunner::ok_with_stdout(self.head.lock().unwrap().as_bytes());
        }
        MockProcessRunner::ok()
    }

    fn sccache_available(&self) -> bool {
        self.sccache
    }
}

struct Gated {
    runner: Arc<GatedVerifyRunner>,
    state: Arc<McpState>,
    task_id: crate::models::TaskId,
    started: std::sync::mpsc::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
}

impl Gated {
    fn runs(&self) -> usize {
        self.runner
            .verify_runs
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Block until a verify run has started.
    async fn wait_started(self) -> Self {
        tokio::task::spawn_blocking(move || {
            self.started.recv().unwrap();
            self
        })
        .await
        .unwrap()
    }
}

/// A board whose verify run blocks until released, and whose `wrap_up` waits
/// only 50ms for it instead of `wrap_up_verify_wait`.
async fn gated_board(sccache: bool) -> Gated {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let runner = Arc::new(GatedVerifyRunner {
        started: std::sync::Mutex::new(started_tx),
        release: std::sync::Mutex::new(release_rx),
        verify_runs: std::sync::atomic::AtomicUsize::new(0),
        head: std::sync::Mutex::new(HEAD_A.to_string()),
        sccache,
        last_script: std::sync::Mutex::new(String::new()),
    });
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let svc = crate::service::TaskService::new(db.clone(), runner.clone())
        .with_verify_wait(std::time::Duration::from_millis(50));
    let (state, db) = state_over(db, runner.clone(), None, Some(Arc::new(svc)), None);
    let task_id = create_wrappable_task(&db).await;
    db.save_repo_path("/repo").await.unwrap();
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    Gated {
        runner,
        state,
        task_id,
        started: started_rx,
        release: release_tx,
    }
}

/// mcp-task-tools.allium: WrapUpRefusesUnfinishedWork, "Verify outcome
/// memory". A call that gives up on a slow verify is refused with "still
/// running"; the run carries on, and the retry reuses it rather than starting
/// another.
#[tokio::test]
async fn a_retry_after_a_slow_verify_reuses_that_run_instead_of_starting_another() {
    let g = gated_board(false).await;
    let state = g.state.clone();
    let task_id = g.task_id;
    let first = tokio::spawn({
        let state = state.clone();
        async move { wrap_up_call(&state, task_id, "done").await }
    });
    let g = g.wait_started().await;

    let first = first.await.unwrap();
    assert!(is_error(&first), "the slow verify must not hold the call");
    assert!(
        error_message(&first).contains("still running"),
        "got: {}",
        error_message(&first)
    );
    assert_no_token(&state, task_id).await;

    // The retry joins the run still going; let it finish while the retry waits.
    g.release.send(()).unwrap();
    let mut second = wrap_up_call(&state, task_id, "done").await;
    // The 50ms wait may end just before the run does; the outcome is recorded
    // either way, so one more call reads it.
    if is_error(&second) {
        second = wrap_up_call(&state, task_id, "done").await;
    }

    assert!(!is_error(&second), "got: {}", error_message(&second));
    assert!(extract_response_text(&second).contains("`cargo test` passed"));
    assert_eq!(g.runs(), 1, "the retry must reuse the run, not repeat it");
}

/// A commit since the run started is a different key: the retry is neither
/// joined to the old run nor allowed to start a second beside it.
#[tokio::test]
async fn a_commit_since_the_running_verify_started_is_not_joined_and_starts_no_second_run() {
    let g = gated_board(false).await;
    let state = g.state.clone();
    let task_id = g.task_id;
    let first = tokio::spawn({
        let state = state.clone();
        async move { wrap_up_call(&state, task_id, "done").await }
    });
    let g = g.wait_started().await;
    let _ = first.await.unwrap();

    *g.runner.head.lock().unwrap() = HEAD_B.to_string();
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&second));
    assert!(
        error_message(&second).contains("still running"),
        "got: {}",
        error_message(&second)
    );
    assert_eq!(g.runs(), 1, "no second run beside the one going");
    assert_no_token(&state, task_id).await;

    // Once the old run ends, the new commit is verified on its own.
    g.release.send(()).unwrap();
    g.release.send(()).unwrap();
    let mut third = wrap_up_call(&state, task_id, "done").await;
    for _ in 0..50 {
        if !is_error(&third) {
            break;
        }
        third = wrap_up_call(&state, task_id, "done").await;
    }
    assert!(!is_error(&third), "got: {}", error_message(&third));
    assert_eq!(g.runs(), 2, "the new commit gets its own run");
}

#[tokio::test]
async fn a_recorded_pass_is_reused_on_the_same_commit_without_running_verify_again() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::ok()),
        vec![
            // The second call: commit check, then HEAD; no verify run.
            (None, MockProcessRunner::ok_with_stdout(b"")),
            (None, MockProcessRunner::ok_with_stdout(HEAD_A.as_bytes())),
        ],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let first = wrap_up_call(&state, task_id, "done").await;
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&first), "got: {}", error_message(&first));
    assert!(!is_error(&second), "got: {}", error_message(&second));
    assert!(extract_response_text(&second).contains("`cargo test` passed"));
    let sh = runner
        .recorded_calls()
        .iter()
        .filter(|(p, _)| p == "sh")
        .count();
    assert_eq!(sh, 1, "calls: {:?}", runner.flattened_calls());
}

#[tokio::test]
async fn a_recorded_failure_is_returned_again_without_running_verify_again() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::fail_with_code(101, "1 failed")),
        vec![
            (None, MockProcessRunner::ok_with_stdout(b"")),
            (None, MockProcessRunner::ok_with_stdout(HEAD_A.as_bytes())),
        ],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let _first = wrap_up_call(&state, task_id, "done").await;
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&second));
    let msg = error_message(&second);
    assert!(
        msg.contains("101") && msg.contains("1 failed") && msg.contains("cargo test"),
        "the refusal must repeat the original; got: {msg}"
    );
    assert_no_token(&state, task_id).await;
    let sh = runner
        .recorded_calls()
        .iter()
        .filter(|(p, _)| p == "sh")
        .count();
    assert_eq!(sh, 1, "calls: {:?}", runner.flattened_calls());
}

#[tokio::test]
async fn a_new_commit_is_verified_afresh_even_after_a_recorded_pass() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::ok()),
        vec![
            (None, MockProcessRunner::ok_with_stdout(b"")),
            (None, MockProcessRunner::ok_with_stdout(HEAD_B.as_bytes())),
            (None, MockProcessRunner::ok()),
        ],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let _first = wrap_up_call(&state, task_id, "done").await;
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&second), "got: {}", error_message(&second));
    let sh = runner
        .recorded_calls()
        .iter()
        .filter(|(p, _)| p == "sh")
        .count();
    assert_eq!(sh, 2, "a different commit must not reuse the pass");
}

/// mcp-task-tools.allium: the verify run gets `RUSTC_WRAPPER=sccache` when
/// sccache is on the board's PATH, and a value already set is kept.
#[tokio::test]
async fn the_verify_run_gets_sccache_when_it_is_available_and_not_otherwise() {
    for (available, expected) in [(true, true), (false, false)] {
        let g = gated_board(available).await;
        g.release.send(()).unwrap();
        let resp = wrap_up_call(&g.state, g.task_id, "done").await;
        assert!(!is_error(&resp), "got: {}", error_message(&resp));
        let script = g.runner.last_script.lock().unwrap().clone();
        assert_eq!(
            script.contains("RUSTC_WRAPPER"),
            expected,
            "sccache available={available}; script: {script}"
        );
        if expected {
            assert!(
                script.contains(":=sccache"),
                "an existing value must be kept; script: {script}"
            );
        }
    }
}
