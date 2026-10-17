use super::*;

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
    db.give_agent(task_id);
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
    db.give_agent(task_id);
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

    // Set up worktree + tmux_window so wrap_up_block passes.
    let window = test_tmux_window("task-rebase-window");
    let patch = crate::store::TaskPatch::new()
        .host(Some(crate::store::MEMORY_HOST_ID))
        .worktree(Some("/repo/.worktrees/task-rebase"))
        .tmux_window(Some(&window));
    state.db_write().give_agent(task_id);
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
    // the epic rollup.
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
    db.give_agent(task_id);
    db.patch_task(
        task_id,
        &crate::store::TaskPatch::new()
            .host(Some(crate::store::MEMORY_HOST_ID))
            .worktree(Some("/repo/.worktrees/e2e"))
            .tmux_window(Some(&test_tmux_window("e2e-window"))),
    )
    .await
    .unwrap();
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
    db.give_agent(task_id);
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
