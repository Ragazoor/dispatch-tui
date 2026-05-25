#![allow(clippy::unwrap_used, clippy::expect_used)]
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
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "My Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
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

    let map = state.exit_tokens.read().unwrap();
    let et = map
        .get(&task_id)
        .expect("token should be stored after successful rebase");
    assert!(
        text.contains(&et.token),
        "response must include the exit token; got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_task_no_worktree() {
    let state = test_state().await;
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
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
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "My Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-my-task")),
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"),
        MockProcessRunner::fail(""),
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));
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

    let map = state.exit_tokens.read().unwrap();
    let et = map
        .get(&task_id)
        .expect("token should be in exit_tokens after wrap_up rebase");
    assert!(!et.token.is_empty(), "token must be non-empty");
    assert!(!et.reflected, "reflected must start false");
    assert!(
        text.contains(&et.token),
        "response text must contain the token; got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_done_returns_exit_token() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-t"))
            .tmux_window(Some("task-1")),
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

    let map = state.exit_tokens.read().unwrap();
    let et = map
        .get(&task_id)
        .expect("token should be in exit_tokens after wrap_up done");
    assert!(!et.token.is_empty(), "token must be non-empty");
    assert!(
        text.contains(&et.token),
        "response text must contain the token; got: {text}"
    );
}

// ---------------------------------------------------------------------------
// wrap_up PR action tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_pr_sets_review_and_pr_url() {
    let state = test_state().await;
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "PR Task",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-pr-task")),
        )
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": {
                "task_id": task_id.0,
                "action": "pr",
                "pr_url": "https://github.com/owner/repo/pull/42"
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review, "task should be in review");
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/owner/repo/pull/42"),
        "pr_url should be set"
    );
}

#[tokio::test]
async fn wrap_up_pr_without_pr_url_errors() {
    let state = test_state().await;
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
        )
        .await
        .unwrap();

    // No pr_url provided
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "pr" } })),
    )
    .await;
    assert_error(&resp, "pr_url");

    // Empty pr_url
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "pr", "pr_url": "" }
        })),
    )
    .await;
    assert_error(&resp, "pr_url");
}

#[tokio::test]
async fn wrap_up_pr_response_contains_no_token() {
    let state = test_state().await;
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
        )
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": {
                "task_id": task_id.0,
                "action": "pr",
                "pr_url": "https://github.com/owner/repo/pull/1"
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "expected success, got: {}",
        error_message(&resp)
    );

    // No token in exit_tokens map
    let map = state.exit_tokens.read().unwrap();
    assert!(
        map.get(&task_id).is_none(),
        "wrap_up(pr) must not insert an exit token"
    );

    // Response must tell agent not to call exit_session
    let text = extract_response_text(&resp);
    assert!(
        text.contains("do not call exit_session") || text.contains("do not call `exit_session`"),
        "response should tell agent not to call exit_session; got: {text}"
    );
}

// ---------------------------------------------------------------------------
// wrap_up + learning_verdicts tests
// ---------------------------------------------------------------------------

async fn make_state_with_runner(
    runner: Arc<dyn ProcessRunner>,
) -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));
    (state, db)
}

fn rebase_ok_runner() -> Arc<dyn ProcessRunner> {
    Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
        MockProcessRunner::fail(""),                  // remote get-url
        MockProcessRunner::ok(),                      // git rebase
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]))
}

async fn create_wrappable_task(db: &Arc<dyn db::TaskStore>) -> crate::models::TaskId {
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
    )
    .await
    .unwrap();
    task_id
}

async fn create_approved_user_learning(
    db: &Arc<dyn db::TaskStore>,
    summary: &str,
) -> crate::models::LearningId {
    let id = db
        .create_learning(CreateLearningRow {
            kind: crate::models::LearningKind::Convention,
            summary,
            detail: None,
            scope: crate::models::LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    db.patch_learning(
        id,
        &crate::db::LearningPatch::new().status(crate::models::LearningStatus::Approved),
    )
    .await
    .unwrap();
    id
}

async fn setup_state_after_dispatch_with_two_retrieved_approved_learnings() -> (
    Arc<McpState>,
    Arc<dyn db::TaskStore>,
    crate::models::TaskId,
    crate::models::LearningId,
    crate::models::LearningId,
) {
    let (state, db) = make_state_with_runner(rebase_ok_runner()).await;
    let task_id = create_wrappable_task(&db).await;
    let l1 = create_approved_user_learning(&db, "first learning").await;
    let l2 = create_approved_user_learning(&db, "second learning").await;
    db.record_retrieval(task_id, l1, crate::models::RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.record_retrieval(task_id, l2, crate::models::RetrievalSource::PromptInjection)
        .await
        .unwrap();
    (state, db, task_id, l1, l2)
}

#[tokio::test]
async fn wrap_up_with_verdicts_applies_them() {
    let (state, db, task_id, l1_id, l2_id) =
        setup_state_after_dispatch_with_two_retrieved_approved_learnings().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": {
                "task_id": task_id.0,
                "action": "rebase",
                "learning_verdicts": [
                    {"learning_id": l1_id.0, "verdict": "helped"},
                    {"learning_id": l2_id.0, "verdict": "wrong"}
                ]
            }
        })),
    )
    .await;
    // Verdicts are applied before the rebase, so they persist regardless of
    // rebase outcome. Assert on DB state.
    let l1 = db.get_learning(l1_id).await.unwrap().unwrap();
    let l2 = db.get_learning(l2_id).await.unwrap().unwrap();
    assert_eq!(l1.upvote_count, 1);
    assert_eq!(l2.status, crate::models::LearningStatus::NeedsReview);
    let _ = resp;
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

#[tokio::test]
async fn wrap_up_rejects_verdict_for_unretrieved_learning() {
    let (state, _db, task_id, _l1, _l2) =
        setup_state_after_dispatch_with_two_retrieved_approved_learnings().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": {
                "task_id": task_id.0,
                "action": "rebase",
                "learning_verdicts": [{"learning_id": 9999, "verdict": "helped"}]
            }
        })),
    )
    .await;
    assert!(is_error(&resp));
}

#[tokio::test]
async fn wrap_up_rejects_unknown_verdict_string() {
    let (state, _db, task_id, l1_id, _l2) =
        setup_state_after_dispatch_with_two_retrieved_approved_learnings().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": {
                "task_id": task_id.0,
                "action": "rebase",
                "learning_verdicts": [{"learning_id": l1_id.0, "verdict": "bogus"}]
            }
        })),
    )
    .await;
    assert!(is_error(&resp));
}

// ---------------------------------------------------------------------------
// wrap_up verify reminder tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wrap_up_success_includes_verify_reminder_when_configured() {
    let (state, db) = make_state_with_runner(rebase_ok_runner()).await;
    let task_id = create_wrappable_task(&db).await;
    db.set_verify_command("/repo", Some("cargo test"))
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
        text.contains("Verify before exiting"),
        "Expected 'Verify before exiting' in response, got: {text}"
    );
    assert!(
        text.contains("cargo test"),
        "Expected 'cargo test' in response, got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "Expected 'exit_session' in response, got: {text}"
    );
}

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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::fail("CONFLICT (content): Merge conflict in foo.rs"), // git rebase
        MockProcessRunner::ok(),                      // git rebase --abort
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Conflict Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-conflict-task")),
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
    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_not_on_main_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail(""), // git rev-parse (empty stdout → treated as non-main)
        MockProcessRunner::ok_with_stdout(b"feature\n"), // unused
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Wrong Branch",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-wrong-branch")),
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
    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on error"
    );
}

#[tokio::test]
async fn update_task_status_recalculates_epic_status() {
    let state = test_state().await;
    let epic = state.db.create_epic("E", "", None).await.unwrap();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    state
        .db
        .set_task_epic_id(task_id, Some(epic.id))
        .await
        .unwrap();

    // Move subtask to Running
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_task should succeed: {:?}",
        resp.error
    );

    // Epic stays in backlog (running tasks do not auto-advance)
    let epic = state.db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}
// =======================================================================
// Notification flow tests
// =======================================================================

/// Helper: creates a test state with a real notification channel.
async fn test_state_with_notify() -> (
    Arc<McpState>,
    tokio::sync::mpsc::UnboundedReceiver<crate::mcp::McpEvent>,
) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let state = Arc::new(McpState::new(
        db,
        Some(tx),
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));
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
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    // Should have received a targeted TaskChanged(task_id) event
    let event = rx
        .try_recv()
        .expect("expected notification after update_task");
    assert!(
        matches!(event, crate::mcp::McpEvent::TaskChanged(t) if t == task_id),
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
            "arguments": { "title": "Notified Task", "repo_path": "/repo" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let event = rx
        .try_recv()
        .expect("expected notification after create_task");
    assert!(
        matches!(event, crate::mcp::McpEvent::TaskChanged(_)),
        "expected TaskChanged, got {event:?}"
    );
}

#[tokio::test]
async fn claim_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/1-test",
                "tmux_window": "task-1"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let event = rx
        .try_recv()
        .expect("expected notification after claim_task");
    assert!(
        matches!(event, crate::mcp::McpEvent::TaskChanged(t) if t == task_id),
        "expected TaskChanged({task_id:?}), got {event:?}"
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
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "bogus" }
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

async fn make_rebase_state() -> (Arc<dyn db::TaskStore>, Arc<McpState>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));
    (db, state)
}

async fn seed_task_with_worktree(
    db: &Arc<dyn db::TaskStore>,
    suffix: &str,
) -> crate::models::TaskId {
    let task_id = db
        .create_task(CreateTaskRequest {
            title: &format!("Task {suffix}"),
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some(&format!(
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

#[tokio::test]
async fn exit_session_first_call_returns_reflection_prompt() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "tok1".to_string(),
            reflected: false,
        },
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "tok1" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("pitfall") || text.contains("convention") || text.contains("record_learning"),
        "first call should prompt for reflection; got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "first call should tell agent to call exit_session again; got: {text}"
    );

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert!(
        task.tmux_window.is_some(),
        "window should still be set after first call"
    );
    assert_ne!(
        task.status,
        TaskStatus::Done,
        "task should not be Done after first call"
    );

    let map = state.exit_tokens.read().unwrap();
    assert!(
        map.get(&task_id).map(|et| et.reflected).unwrap_or(false),
        "reflected should be true after first call"
    );
}

#[tokio::test]
async fn exit_session_second_call_closes_session() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "tok2".to_string(),
            reflected: false,
        },
    );

    // First call — reflect
    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok2" } })),
    )
    .await;

    // Second call — close
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok2" } })),
    )
    .await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert!(task.tmux_window.is_none());
    assert_eq!(task.status, TaskStatus::Done);
}

#[tokio::test]
async fn exit_session_closes_after_reflection_and_record_learning() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "tok3".to_string(),
            reflected: false,
        },
    );

    // First call — reflect
    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok3" } })),
    )
    .await;

    // (Agent would call record_learning here — separate tool, not simulated)

    // Second call — close
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok3" } })),
    )
    .await;

    assert_eq!(extract_response_text(&resp), "Session closed.");
    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert!(task.tmux_window.is_none());
    assert_eq!(task.status, TaskStatus::Done);
}

#[tokio::test]
async fn exit_session_after_close_token_is_gone() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "tok4".to_string(),
            reflected: false,
        },
    );

    // First call
    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok4" } })),
    )
    .await;

    // Second call — closes session
    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok4" } })),
    )
    .await;

    // Third call — token is gone (task also has no window now)
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": "tok4" } })),
    )
    .await;
    assert_error(&resp, "wrap_up first");
}

#[tokio::test]
async fn exit_session_full_flow_rebase() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"),
        MockProcessRunner::fail(""),
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));
    let task_id = create_wrappable_task(&db).await;
    db.patch_task(task_id, &db::TaskPatch::new().tmux_window(Some("task-1")))
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

    let token = state
        .exit_tokens
        .read()
        .unwrap()
        .get(&task_id)
        .unwrap()
        .token
        .clone();

    // First exit_session — reflection prompt
    let reflect_resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": token } })),
    )
    .await;
    assert!(!is_error(&reflect_resp));
    let reflect_text = extract_response_text(&reflect_resp);
    assert!(
        reflect_text.contains("record_learning") || reflect_text.contains("pitfall"),
        "first call should be reflection prompt; got: {reflect_text}"
    );

    // Second exit_session — close
    let close_resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": token } })),
    )
    .await;
    assert_eq!(extract_response_text(&close_resp), "Session closed.");

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn wrap_up_second_call_overwrites_token() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // First rebase
        MockProcessRunner::ok_with_stdout(b"main\n"),
        MockProcessRunner::fail(""),
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
        // Second rebase
        MockProcessRunner::ok_with_stdout(b"main\n"),
        MockProcessRunner::fail(""),
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));
    let task_id = create_wrappable_task(&db).await;
    db.patch_task(task_id, &db::TaskPatch::new().tmux_window(Some("task-1")))
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
    let first_token = state
        .exit_tokens
        .read()
        .unwrap()
        .get(&task_id)
        .unwrap()
        .token
        .clone();

    call(
        &state,
        "tools/call",
        Some(
            json!({ "name": "wrap_up", "arguments": { "task_id": task_id.0, "action": "rebase" } }),
        ),
    )
    .await;
    let second_token = state
        .exit_tokens
        .read()
        .unwrap()
        .get(&task_id)
        .unwrap()
        .token
        .clone();

    assert_ne!(
        first_token, second_token,
        "second wrap_up should generate a new token"
    );

    // Old token rejected
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0, "token": first_token } })),
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

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn exit_session_task_without_window_returns_error() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await; // Backlog task, no tmux_window

    // Insert token so we get past the token check
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "tok-no-win".to_string(),
            reflected: false,
        },
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "tok-no-win" }
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
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0 } })),
    )
    .await;

    assert_error(&resp, "wrap_up first");
}

#[tokio::test]
async fn exit_session_with_wrong_token_errors() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;

    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "correct-token".to_string(),
            reflected: false,
        },
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "wrong-token" }
        })),
    )
    .await;

    assert_error(&resp, "invalid exit token");
}

#[tokio::test]
async fn exit_session_reflection_includes_verify_command_when_set() {
    let state = test_state().await;
    state
        .db
        .set_verify_command("/repo", Some("cargo test --workspace"))
        .await
        .unwrap();
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "test-token".to_string(),
            reflected: false,
        },
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "test-token" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("cargo test --workspace"),
        "reflection should include existing verify command; got: {text}"
    );
}

#[tokio::test]
async fn exit_session_reflection_nudges_to_set_verify_when_not_set() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "test-token".to_string(),
            reflected: false,
        },
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "test-token" }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("set_verify_command"),
        "reflection should nudge to set_verify_command when none is set; got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_does_not_kill_window() {
    let state = test_state().await;
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Rebase Task",
            description: "description",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    // Set up worktree + tmux_window so is_wrappable passes.
    let patch = crate::db::TaskPatch::new()
        .worktree(Some("/repo/.worktrees/task-rebase"))
        .tmux_window(Some("task-rebase-window"));
    state.db.patch_task(task_id, &patch).await.unwrap();

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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    // tmux_window must NOT be cleared — exit_session owns the window kill.
    assert!(
        task.tmux_window.is_some(),
        "wrap_up(rebase) must not clear tmux_window — exit_session is responsible"
    );
}

// -- exit_session: Done transition (added with the wrap_up/exit_session alignment) ---

#[tokio::test]
async fn exit_session_second_call_marks_task_done() {
    // No-epic branch: the closing call must mark the task Done even when
    // there is no epic to recalculate. Pins the `is_some()` guard around
    // recalculate_epic_status.
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "close-tok".to_string(),
            reflected: true,
        },
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "close-tok" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Done));
    assert!(task.tmux_window.is_none());
    assert!(task.epic_id.is_none(), "fixture should have no epic");
}

#[tokio::test]
async fn exit_session_first_call_does_not_change_status() {
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "reflect-tok".to_string(),
            reflected: false,
        },
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "reflect-tok" }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "first exit_session call (reflection nudge) must not change status"
    );
    assert!(
        task.tmux_window.is_some(),
        "first call must not clear tmux_window"
    );
}

#[tokio::test]
async fn exit_session_already_done_task_stays_done() {
    // Idempotency: a task that is somehow already Done before exit_session
    // closes must remain Done after the closing call.
    let state = test_state().await;
    let task_id = create_running_task_with_window(&state).await;
    state
        .db
        .patch_task(
            task_id,
            &crate::db::TaskPatch::new().status(TaskStatus::Done),
        )
        .await
        .unwrap();
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "close-done-tok".to_string(),
            reflected: true,
        },
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "close-done-tok" }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn exit_session_recalculates_epic_status() {
    let state = test_state().await;
    let epic = state.db.create_epic("E", "", None).await.unwrap();
    let task_id = create_running_task_with_window(&state).await;
    state
        .db
        .set_task_epic_id(task_id, Some(epic.id))
        .await
        .unwrap();
    state.db.recalculate_epic_status(epic.id).await.unwrap();
    let epic_before = state.db.get_epic(epic.id).await.unwrap().unwrap();
    assert_ne!(
        epic_before.status,
        TaskStatus::Done,
        "precondition: epic should be in-progress before exit_session"
    );
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "epic-close-tok".to_string(),
            reflected: true,
        },
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "epic-close-tok" }
        })),
    )
    .await;

    let epic_after = state.db.get_epic(epic.id).await.unwrap().unwrap();
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
        .db
        .patch_task(
            task_id,
            &crate::db::TaskPatch::new().sub_status(SubStatus::Stale),
        )
        .await
        .unwrap();
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "sub-close-tok".to_string(),
            reflected: true,
        },
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "sub-close-tok" }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
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

    // First call — reflect (drain any events)
    state.exit_tokens.write().unwrap().insert(
        task_id,
        crate::mcp::ExitToken {
            token: "notify-tok".to_string(),
            reflected: false,
        },
    );
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "notify-tok" }
        })),
    )
    .await;
    while rx.try_recv().is_ok() {}

    // Second call — close
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": "notify-tok" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    // DB must already be Done by the time the Refresh fires.
    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);

    let event = rx
        .try_recv()
        .expect("expected TaskChanged after closing exit_session");
    assert!(
        matches!(event, crate::mcp::McpEvent::TaskChanged(t) if t == task_id),
        "expected TaskChanged({task_id:?}), got {event:?}"
    );
}

#[tokio::test]
async fn wrap_up_then_exit_session_end_to_end() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
        // exit_session second call kills the tmux window:
        MockProcessRunner::ok(), // tmux has-session
        MockProcessRunner::ok(), // tmux kill-window
    ]));
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let epic = db.create_epic("E2E Epic", "", None).await.unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "E2E Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).await.unwrap();
    db.patch_task(
        task_id,
        &crate::db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/e2e"))
            .tmux_window(Some("e2e-window")),
    )
    .await
    .unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic_before = db.get_epic(epic.id).await.unwrap().unwrap();
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

    let after_wrap_up = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        after_wrap_up.status,
        TaskStatus::Running,
        "after wrap_up: status must still be Running"
    );
    assert!(
        after_wrap_up.tmux_window.is_some(),
        "after wrap_up: tmux_window must be preserved"
    );
    let epic_after_wrap_up = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        epic_after_wrap_up.status, epic_before.status,
        "after wrap_up: epic status must not change"
    );

    // Extract the token that wrap_up placed in exit_tokens.
    let token = {
        let map = state.exit_tokens.read().unwrap();
        map.get(&task_id)
            .expect("wrap_up must have inserted an exit token")
            .token
            .clone()
    };

    // First exit_session call → reflection prompt.
    let reflect_resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token }
        })),
    )
    .await;
    assert!(reflect_resp.error.is_none(), "{:?}", reflect_resp.error);
    let reflect_text = extract_response_text(&reflect_resp);
    assert!(
        reflect_text.contains("Reflect on this session"),
        "first call should return reflection prompt, got: {reflect_text}"
    );

    // Task must still be Running after the first call.
    let mid_task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(mid_task.status, TaskStatus::Running);

    // Second exit_session call → closes session.
    let close_resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "token": token }
        })),
    )
    .await;
    assert!(close_resp.error.is_none(), "{:?}", close_resp.error);

    let final_task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(final_task.status, TaskStatus::Done);
    assert_eq!(
        final_task.sub_status,
        SubStatus::default_for(TaskStatus::Done)
    );
    assert!(final_task.tmux_window.is_none());

    let final_epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        final_epic.status,
        TaskStatus::Done,
        "epic auto-advances once its only subtask completes via exit_session"
    );
}

#[tokio::test]
async fn wrap_up_done_marks_task_done_without_git_ops() {
    use crate::process::MockProcessRunner;
    let runner: Arc<dyn crate::process::ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let state = Arc::new(McpState::new(
        db.clone(),
        None,
        runner,
        EmbeddingService::new_test(),
        std::env::temp_dir(),
    ));

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Done Task",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: Some(crate::models::WrapUpMode::Done),
        })
        .await
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-done-task"))
            .tmux_window(Some("task-1")),
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

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done, "task should be marked Done");
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
