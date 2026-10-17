use super::*;

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
    db.give_agent(task_id);
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
    state.db_write().give_agent(task_id);
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
