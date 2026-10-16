use super::*;

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
// wrap_up verify line tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_success_omits_verify_line_when_unconfigured() {
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
        !text.contains("Re-run"),
        "Expected no re-run line in unconfigured response, got: {text}"
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
