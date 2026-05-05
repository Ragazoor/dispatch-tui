use super::*;

// -- update_task tests -------------------------------------------------------

#[tokio::test]
async fn update_task_valid() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Running);
}

#[tokio::test]
async fn update_task_invalid_status() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "bogus" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

#[tokio::test]
async fn update_task_rejects_done_status() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
    )
    .await;
    assert_error(&resp, "Cannot set status to done or archived via MCP");

    // Verify task status unchanged
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_ne!(task.status, crate::models::TaskStatus::Done);

    // Also verify archived is rejected
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "archived" }
        })),
    )
    .await;
    assert_error(&resp, "Cannot set status to done or archived via MCP");
}

#[tokio::test]
async fn update_task_still_allows_other_statuses() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    for status in &["running", "review", "ready", "backlog"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "status": status }
            })),
        )
        .await;
        assert!(
            resp.error.is_none(),
            "status={status} should be allowed, got: {:?}",
            resp.error
        );
    }
}

#[tokio::test]
async fn update_task_missing_args() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "update_task", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_some());
}

// -- get_task tests ----------------------------------------------------------

#[tokio::test]
async fn get_task_found() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("My Task"));
}

#[tokio::test]
async fn get_task_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("not found"));
}

// -- String task_id coercion (Claude Code sends integers as strings) ------

#[tokio::test]
async fn update_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0.to_string(), "status": "running" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_task should accept string task_id, got: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Running);
}

#[tokio::test]
async fn get_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0.to_string() }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "get_task should accept string task_id, got: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("My Task"));
}

#[tokio::test]
async fn update_task_with_plan() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "ready", "plan_path": "/path/to/plan.md" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
    assert_eq!(task.plan_path.as_deref(), Some("/path/to/plan.md"));
}

#[tokio::test]
async fn update_task_title_only() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Old",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "New Title" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should succeed with title only: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.title, "New Title");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog); // unchanged
}

#[tokio::test]
async fn update_task_status_optional() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "Renamed" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.title, "Renamed");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
}

#[tokio::test]
async fn update_task_title_and_description() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Old",
            "old desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "New", "description": "new desc" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.title, "New");
    assert_eq!(task.description, "new desc");
}

#[tokio::test]
async fn update_task_repo_path() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/old/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "repo_path": "/new/repo" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should succeed with repo_path only: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.repo_path, "/new/repo");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog); // unchanged
}

#[tokio::test]
async fn update_task_no_fields_errors() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            None,
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(
        resp.error.is_some(),
        "should error with no fields to update"
    );
}

#[tokio::test]
async fn patch_task_sets_multiple_fields() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "Desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "status": "ready",
                "title": "Updated Title"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.title, "Updated Title");
}

#[tokio::test]
async fn update_task_without_plan_preserves_existing() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Test",
            "desc",
            "/repo",
            Some("/existing.md"),
            crate::models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "ready" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.plan_path.as_deref(),
        Some("/existing.md"),
        "plan should be preserved when not provided"
    );
}

#[tokio::test]
async fn update_task_sets_pr_fields() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "PR test",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "pr_url": "https://github.com/org/repo/pull/99"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "Expected success, got: {:?}",
        resp.error
    );

    let updated = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        updated.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/99")
    );
}

// -- list_tasks tests -------------------------------------------------------

#[tokio::test]
async fn list_tasks_returns_all_when_no_filter() {
    let state = test_state();
    state
        .db
        .create_task(
            "Task A",
            "desc a",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Task B",
            "desc b",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Task A"));
    assert!(text.contains("Task B"));
}

#[tokio::test]
async fn list_tasks_filters_by_single_status() {
    let state = test_state();
    state
        .db
        .create_task(
            "Backlog Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Running Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "backlog" } })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Backlog Task"));
    assert!(!text.contains("Running Task"));
}

#[tokio::test]
async fn list_tasks_filters_by_multiple_statuses() {
    let state = test_state();
    state
        .db
        .create_task(
            "Backlog Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Running Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Review Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": ["backlog", "running"] } })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Backlog Task"));
    assert!(text.contains("Running Task"));
    assert!(!text.contains("Review Task"));
}

#[tokio::test]
async fn list_tasks_empty_result() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "running" } })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("No tasks found"));
}

// -- claim_task tests -------------------------------------------------------

#[tokio::test]
async fn claim_task_success() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Claimable",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other-task",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "claim should succeed: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(
        task.worktree.as_deref(),
        Some("/repo/.worktrees/5-other-task")
    );
    assert_eq!(task.tmux_window.as_deref(), Some("task-5"));
}

#[tokio::test]
async fn claim_task_rejects_running_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Running",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("already"));
}

#[tokio::test]
async fn claim_task_rejects_different_repo() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Other Repo",
            "desc",
            "/other-repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other-task",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("repo"));
}

#[tokio::test]
async fn claim_task_not_found() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": 9999,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("not found"));
}

// -- report_usage tests -----------------------------------------------------

#[tokio::test]
async fn report_usage_stores_and_accumulates() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    // First session
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "report_usage",
            "arguments": {
                "task_id": task_id.0,
                "input_tokens": 1000,
                "output_tokens": 500
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "first call failed: {:?}", resp.error);

    // Second session — should accumulate
    let resp2 = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "report_usage",
            "arguments": {
                "task_id": task_id.0,
                "input_tokens": 500,
                "output_tokens": 250,
                "cache_read_tokens": 100,
                "cache_write_tokens": 50
            }
        })),
    )
    .await;
    assert!(
        resp2.error.is_none(),
        "second call failed: {:?}",
        resp2.error
    );

    let all = state.db.get_all_usage().unwrap();
    assert_eq!(all.len(), 1);
    let u = &all[0];
    assert_eq!(u.task_id, task_id);
    assert_eq!(u.input_tokens, 1_500);
    assert_eq!(u.output_tokens, 750);
    assert_eq!(u.cache_read_tokens, 100);
    assert_eq!(u.cache_write_tokens, 50);
}

#[tokio::test]
async fn report_usage_unknown_task_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "report_usage",
            "arguments": {
                "task_id": 9999,
                "input_tokens": 1000,
                "output_tokens": 500
            }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn claim_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Claimable",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0.to_string(),
                "worktree": "/repo/.worktrees/5-other-task",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string task_id: {:?}",
        resp.error
    );
}

// -- list_tasks edge case tests ----------------------------------------------

#[tokio::test]
async fn list_tasks_invalid_status_string() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "bogus" } })),
    )
    .await;
    assert_error(&resp, "Unknown status");
}

#[tokio::test]
async fn list_tasks_invalid_status_in_array() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": ["backlog", "bogus"] } })),
    )
    .await;
    assert_error(&resp, "Invalid status in array");
}

#[tokio::test]
async fn list_tasks_status_as_number_errors() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": 42 } })),
    )
    .await;
    assert_error(&resp, "string or array");
}

// -- claim_task additional edge cases ----------------------------------------

#[tokio::test]
async fn claim_task_rejects_done_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Done",
            "desc",
            "/repo",
            None,
            TaskStatus::Done,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert_error(&resp, "already");
}

#[tokio::test]
async fn claim_task_rejects_review_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Review",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/5-other",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert_error(&resp, "already");
}

#[tokio::test]
async fn claim_task_worktree_without_worktrees_dir() {
    let state = test_state();
    // Task repo is "/repo", worktree path has no /.worktrees/ segment
    // so the full path is used as the repo — should match when equal
    let task_id = state
        .db
        .create_task(
            "Direct",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo",
                "tmux_window": "task-5"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should match when worktree equals repo: {:?}",
        resp.error
    );
}

#[tokio::test]
async fn claim_task_updates_status_to_running() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Claim",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "claim_task",
            "arguments": {
                "task_id": task_id.0,
                "worktree": "/repo/.worktrees/1-claim",
                "tmux_window": "task-1"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state
        .db
        .get_task(crate::models::TaskId(task_id.0))
        .unwrap()
        .unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-claim"));
    assert_eq!(task.tmux_window.as_deref(), Some("task-1"));
}

// -- create_task tests -------------------------------------------------------

#[tokio::test]
async fn create_task_minimal() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "New Task",
                "repo_path": "/my/repo",
                "project_id": default_id,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("created"));

    // Verify task was created in DB
    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "New Task");
    assert_eq!(tasks[0].status, TaskStatus::Backlog);
    assert!(tasks[0].plan_path.is_none());
}

#[tokio::test]
async fn create_task_with_plan_stays_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let plan_file = dir.path().join("plan.md");
    std::fs::write(&plan_file, "# Plan").unwrap();

    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Planned Task",
                "repo_path": "/my/repo",
                "plan_path": plan_file.to_string_lossy(),
                "project_id": default_id,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Backlog);
    let stored = tasks[0].plan_path.as_deref().unwrap();
    assert!(
        std::path::Path::new(stored).is_absolute(),
        "plan path should be absolute, got: {stored}"
    );
    assert_eq!(
        stored,
        std::fs::canonicalize(&plan_file).unwrap().to_string_lossy()
    );
}

#[tokio::test]
async fn create_task_with_description() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Described Task",
                "repo_path": "/repo",
                "description": "Some details",
                "project_id": default_id,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks[0].description, "Some details");
}

#[tokio::test]
async fn create_task_missing_title() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "repo_path": "/repo", "project_id": default_id }
        })),
    )
    .await;
    assert!(resp.error.is_some());
}

#[tokio::test]
async fn create_task_missing_project_id_is_invalid_params() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "No project", "repo_path": "/repo" }
        })),
    )
    .await;
    assert_error(&resp, "project_id");
    assert_eq!(resp.error.as_ref().unwrap().code, -32602);
    let tasks = state.db.list_all().unwrap();
    assert!(tasks.is_empty(), "no task should be created");
}

#[tokio::test]
async fn create_task_with_unknown_project_id_is_invalid_params() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Bogus project",
                "repo_path": "/repo",
                "project_id": 999_999,
            }
        })),
    )
    .await;
    assert_error(&resp, "project");
    assert_eq!(resp.error.as_ref().unwrap().code, -32602);
    let tasks = state.db.list_all().unwrap();
    assert!(tasks.is_empty(), "no task should be created");
}

#[tokio::test]
async fn create_task_assigns_to_provided_project() {
    let state = test_state();
    let other = state.db.create_project("Other", 1).unwrap();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Project task",
                "repo_path": "/repo",
                "project_id": other.id,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "got error: {:?}", resp.error);
    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].project_id, other.id);
}

#[tokio::test]
async fn create_task_with_epic_id() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let epic = state
        .db
        .create_epic("Parent Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Epic Child",
                "repo_path": "/repo",
                "epic_id": epic.id.0,
                "project_id": default_id,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let subtasks = state.db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].title, "Epic Child");
}

#[tokio::test]
async fn create_task_with_string_epic_id() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let epic = state
        .db
        .create_epic("Parent", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "String Epic Child",
                "repo_path": "/repo",
                "epic_id": epic.id.0.to_string(),
                "project_id": default_id,
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );

    let subtasks = state.db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(subtasks.len(), 1);
}

#[tokio::test]
async fn create_task_invalid_tag() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Tagged", "repo_path": "/repo", "tag": "bogus", "project_id": default_id }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

#[tokio::test]
async fn create_task_valid_tag() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Bug Task", "repo_path": "/repo", "tag": "bug", "project_id": default_id }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::Bug));
}

#[tokio::test]
async fn create_task_with_sort_order() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Ordered Task", "repo_path": "/repo", "sort_order": 99, "project_id": default_id }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let tasks = state.db.list_all().unwrap();
    assert_eq!(tasks[0].sort_order, Some(99));
}

#[tokio::test]
async fn create_task_with_nonexistent_epic() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Orphan", "repo_path": "/repo", "epic_id": 9999, "project_id": default_id }
        })),
    )
    .await;
    // Should fail because the epic FK doesn't exist
    assert!(resp.error.is_some(), "should error with invalid epic_id");
}

// -- update_task additional validation --------------------------------------

#[tokio::test]
async fn update_task_nonexistent_task_returns_error() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": 9999, "status": "running" }
        })),
    )
    .await;
    assert_error(&resp, "Database error");
}

#[tokio::test]
async fn update_task_invalid_tag() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "tag": "invalid_tag" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `invalid_tag`");
}

#[tokio::test]
async fn update_task_valid_tag() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    for tag in &["bug", "feature", "chore"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "tag": tag }
            })),
        )
        .await;
        assert!(
            resp.error.is_none(),
            "tag={tag} should be valid, got: {:?}",
            resp.error
        );
    }

    // Verify last tag persisted
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.tag, Some(crate::models::TaskTag::Chore));
}

#[tokio::test]
async fn update_task_rejects_epic_tag() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "tag": "epic" }
        })),
    )
    .await;
    assert!(
        resp.error.is_some(),
        "tag=epic should be rejected; the variant was removed"
    );
}

#[tokio::test]
async fn update_task_sets_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "epic_id": epic.id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(
        text.contains("epic_id"),
        "response should list epic_id: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));
}

#[tokio::test]
async fn update_task_sort_order() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sort_order": 42 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.sort_order, Some(42));
}

// -- sub_status tests --------------------------------------------------------

#[tokio::test]
async fn update_task_sets_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sub_status": "needs_input" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(
        text.contains("sub_status"),
        "response should mention sub_status: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.sub_status, crate::models::SubStatus::NeedsInput);
}

#[tokio::test]
async fn update_task_rejects_invalid_sub_status_for_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sub_status": "approved" }
        })),
    )
    .await;
    assert_error(&resp, "not valid for status");
}

#[tokio::test]
async fn update_task_rejects_bogus_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "sub_status": "bogus" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

#[tokio::test]
async fn update_task_sub_status_with_status_change() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Change status to review and set sub_status to approved in one call
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "review", "sub_status": "approved" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, crate::models::SubStatus::Approved);
}

#[tokio::test]
async fn update_task_status_running_with_needs_input() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Set status=running and sub_status=needs_input in one call.
    // Before the fix, status() auto-reset sub_status to Active, which could
    // overwrite the explicit needs_input depending on builder call order.
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running", "sub_status": "needs_input" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "expected success: {:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sub_status, crate::models::SubStatus::NeedsInput);
}

#[tokio::test]
async fn update_task_sub_status_invalid_for_new_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Change status to review but set sub_status to active (valid for running, not review)
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "review", "sub_status": "active" }
        })),
    )
    .await;
    assert_error(&resp, "not valid for status");
}

#[tokio::test]
async fn list_tasks_shows_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Listed Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().sub_status(crate::models::SubStatus::NeedsInput),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("running/needs_input"),
        "expected running/needs_input in list output, got: {text}"
    );
}

#[tokio::test]
async fn get_task_shows_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Detail Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().sub_status(crate::models::SubStatus::ChangesRequested),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Sub-status: changes_requested"),
        "expected sub-status in detail, got: {text}"
    );
}

#[tokio::test]
async fn update_task_status_recalculates_epic_status() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = state
        .db
        .create_task(
            "T",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

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

    // Epic should auto-advance to Running
    let epic = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

// -- send_message tests ------------------------------------------------------

#[tokio::test]
async fn send_message_writes_file_and_sends_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let worktree_path = tmp.path().to_str().unwrap().to_string();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux send-keys -l (notification text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    // Create sender and receiver tasks
    let sender_id = db
        .create_task(
            "Fix auth bug",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    let receiver_id = db
        .create_task(
            "Review PR",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        receiver_id,
        &db::TaskPatch::new()
            .worktree(Some(&worktree_path))
            .tmux_window(Some("task-2")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": receiver_id.0,
                "body": "Can you review path/to/file.rs?"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("Message sent to task"),
        "Expected success message, got: {text}"
    );

    // Verify message file was written in .claude-messages/ directory
    let messages_dir = tmp.path().join(".claude-messages");
    assert!(
        messages_dir.is_dir(),
        ".claude-messages directory should exist"
    );
    let entries: Vec<_> = std::fs::read_dir(&messages_dir).unwrap().collect();
    assert_eq!(entries.len(), 1, "Should have exactly one message file");
    let message_path = entries[0].as_ref().unwrap().path();
    let file_name = message_path.file_name().unwrap().to_str().unwrap();
    assert!(
        file_name.starts_with(&format!("{}-", sender_id.0)),
        "Filename should start with sender task id"
    );
    assert!(file_name.ends_with(".md"), "Filename should end with .md");
    let content = std::fs::read_to_string(&message_path).unwrap();
    assert!(
        content.contains("Fix auth bug"),
        "Message should contain sender title"
    );
    assert!(
        content.contains("Can you review path/to/file.rs?"),
        "Message should contain body"
    );
}

#[tokio::test]
async fn send_message_target_not_found() {
    let state = test_state();

    let sender_id = state
        .db
        .create_task(
            "Sender",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": 9999,
                "body": "hello"
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_some(),
        "Should return error for missing target"
    );
    let err = resp.error.unwrap();
    assert!(
        err.message.contains("not found"),
        "Error should mention not found: {}",
        err.message
    );
}

#[tokio::test]
async fn send_message_target_no_worktree() {
    let state = test_state();

    let sender_id = state
        .db
        .create_task(
            "Sender",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    let receiver_id = state
        .db
        .create_task(
            "Receiver",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": receiver_id.0,
                "body": "hello"
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_some(),
        "Should return error for target without worktree"
    );
    let err = resp.error.unwrap();
    assert!(
        err.message.contains("no worktree"),
        "Error should mention no worktree: {}",
        err.message
    );
}

#[tokio::test]
async fn send_message_target_no_tmux_window() {
    let state = test_state();

    let sender_id = state
        .db
        .create_task(
            "Sender",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    let receiver_id = state
        .db
        .create_task(
            "Receiver",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            receiver_id,
            &db::TaskPatch::new().worktree(Some("/some/worktree")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "send_message",
            "arguments": {
                "from_task_id": sender_id.0,
                "to_task_id": receiver_id.0,
                "body": "hello"
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_some(),
        "Should return error for target without tmux window"
    );
    let err = resp.error.unwrap();
    assert!(
        err.message.contains("no tmux window"),
        "Error should mention no tmux window: {}",
        err.message
    );
}

// -- Notification flow tests -------------------------------------------------

#[tokio::test]
async fn update_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_task_fixture(&state);

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

    // Should have received a Refresh event
    let event = rx
        .try_recv()
        .expect("expected notification after update_task");
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn create_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify();
    let default_id = state.db.get_default_project().unwrap().id;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "Notified Task", "repo_path": "/repo", "project_id": default_id }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let event = rx
        .try_recv()
        .expect("expected notification after create_task");
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn claim_task_sends_refresh_notification() {
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_task_fixture(&state);

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
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn failed_update_does_not_send_notification() {
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_task_fixture(&state);

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
    assert!(resp.error.is_some());

    assert!(
        rx.try_recv().is_err(),
        "no notification should be sent on validation error"
    );
}

// -- list_tasks filtering edge cases ----------------------------------------

#[tokio::test]
async fn list_tasks_filters_by_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("My Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    let t1 = state
        .db
        .create_task(
            "Epic Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Standalone Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "epic_id": epic.id.0 } })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Epic Task"),
        "should include task linked to epic"
    );
    assert!(
        !text.contains("Standalone Task"),
        "should exclude task not linked to epic"
    );
}

#[tokio::test]
async fn list_tasks_filters_by_status_and_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Combined Filter", "", "/repo", None, ProjectId(1))
        .unwrap();
    let t1 = state
        .db
        .create_task(
            "Backlog Epic",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    let t2 = state
        .db
        .create_task(
            "Running Epic",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();
    state.db.set_task_epic_id(t2, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "status": "backlog", "epic_id": epic.id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Backlog Epic"),
        "should include backlog task in epic"
    );
    assert!(
        !text.contains("Running Epic"),
        "should exclude running task when filtering by backlog"
    );
}

#[tokio::test]
async fn list_tasks_epic_filter_no_match() {
    let state = test_state();
    state
        .db
        .create_task(
            "No Epic",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "epic_id": 9999 } })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("No tasks found"));
}

#[tokio::test]
async fn list_tasks_done_status_filter() {
    let state = test_state();
    state
        .db
        .create_task(
            "Done Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Done,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Backlog Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": "done" } })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("Done Task"));
    assert!(!text.contains("Backlog Task"));
}

#[tokio::test]
async fn list_tasks_excludes_archived_by_default() {
    let state = test_state();
    state
        .db
        .create_task(
            "Active Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Archived Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Archived,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("Active Task"), "should show active task");
    assert!(
        !text.contains("Archived Task"),
        "should not show archived task: {text}"
    );
}

// -- get_task additional formatting checks -----------------------------------

#[tokio::test]
async fn get_task_shows_all_fields() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = state
        .db
        .create_task(
            "Full Task",
            "detailed desc",
            "/repo",
            Some("/plan.md"),
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new()
                .worktree(Some("/repo/.worktrees/1-full"))
                .tmux_window(Some("task-1"))
                .pr_url(Some("https://github.com/org/repo/pull/5"))
                .tag(Some(crate::models::TaskTag::Feature))
                .sort_order(Some(10)),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Full Task"), "should show title");
    assert!(text.contains("detailed desc"), "should show description");
    assert!(text.contains("/repo"), "should show repo path");
    assert!(text.contains("/plan.md"), "should show plan");
    assert!(text.contains("Parent Epic"), "should show epic title");
    assert!(
        text.contains("/repo/.worktrees/1-full"),
        "should show worktree"
    );
    assert!(text.contains("task-1"), "should show tmux window");
    assert!(text.contains("pull/5"), "should show PR URL");
    assert!(text.contains("feature"), "should show tag");
    assert!(text.contains("Sort order: 10"), "should show sort order");
}

#[tokio::test]
async fn get_task_without_epic_omits_epic_line() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Solo Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        !text.contains("Epic:"),
        "should not show Epic line for task without epic: {text}"
    );
}

// -- list_tasks format verification -----------------------------------------

#[tokio::test]
async fn list_tasks_shows_tag_and_plan_indicators() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Tagged Planned",
            "desc",
            "/repo",
            Some("/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().tag(Some(crate::models::TaskTag::Bug)),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    // [plan] indicator replaced by | Goal: <goal text> when plan is readable;
    // when the plan file doesn't exist the description is shown as fallback.
    assert!(
        !text.contains("[plan]"),
        "old [plan] badge should no longer appear: {text}"
    );
    assert!(text.contains("[bug]"), "should show tag indicator: {text}");
}

#[tokio::test]
async fn list_tasks_shows_epic_indicator() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Sprint 1", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = state
        .db
        .create_task(
            "Epic Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Sprint 1"),
        "should show epic title in list: {text}"
    );
}

#[tokio::test]
async fn list_tasks_truncates_long_descriptions() {
    let state = test_state();
    let long_desc = "x".repeat(300);
    state
        .db
        .create_task(
            "Long Desc",
            &long_desc,
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("..."),
        "should truncate long description: {text}"
    );
    assert!(
        text.len() < long_desc.len() + 100,
        "truncated output should be shorter than full description"
    );
}

// -- dispatch_next tests -----------------------------------------------------

#[tokio::test]
async fn dispatch_next_epic_not_found_returns_error() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_next_no_backlog_returns_success_noop() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Test Epic", "desc", "/repo", None, ProjectId(1))
        .unwrap();

    // Add a task that's already Running (not Backlog)
    let task_id = state
        .db
        .create_task(
            "Running Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("no backlog tasks"),
        "Expected noop message, got: {text}"
    );
}

#[tokio::test]
async fn dispatch_next_picks_first_backlog_subtask() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, ProjectId(1))
        .unwrap();
    let task1_id = db
        .create_task(
            "Task 1",
            "first",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    let task2_id = db
        .create_task(
            "Task 2",
            "second",
            &repo_path,
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.set_task_epic_id(task1_id, Some(epic.id)).unwrap();
    db.set_task_epic_id(task2_id, Some(epic.id)).unwrap();

    // Pre-create the worktree directory (mocked git won't create it)
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-task-1", task1_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", task1_id.0)),
        "Expected first task ID in response, got: {text}"
    );

    // Wait for spawn_blocking to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify the task was dispatched
    let task1 = db.get_task(task1_id).unwrap().unwrap();
    assert_eq!(task1.status, TaskStatus::Running);
    assert!(task1.worktree.is_some());
    assert!(task1.tmux_window.is_some());

    // task2 should still be Backlog
    let task2 = db.get_task(task2_id).unwrap().unwrap();
    assert_eq!(task2.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn dispatch_next_respects_sort_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, ProjectId(1))
        .unwrap();

    // task1 has higher ID but lower sort_order — should be picked second
    let task1_id = db
        .create_task(
            "Task A",
            "first by id",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    let task2_id = db
        .create_task(
            "Task B",
            "second by id",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.set_task_epic_id(task1_id, Some(epic.id)).unwrap();
    db.set_task_epic_id(task2_id, Some(epic.id)).unwrap();

    // Give task2 a lower sort_order so it should be picked first
    db.patch_task(task2_id, &db::TaskPatch::new().sort_order(Some(1)))
        .unwrap();
    db.patch_task(task1_id, &db::TaskPatch::new().sort_order(Some(2)))
        .unwrap();

    // Pre-create worktree dir for task2 (the one that should be dispatched)
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-task-b", task2_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", task2_id.0)),
        "Expected task2 (lower sort_order) to be dispatched, got: {text}"
    );
}

#[tokio::test]
async fn dispatch_next_respects_tag_routing() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, ProjectId(1))
        .unwrap();

    // Create a feature-tagged task with no plan — should use Plan mode
    let task_id = db
        .create_task(
            "Feature Task",
            "a feature",
            &repo_path,
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().tag(Some(crate::models::TaskTag::Feature)),
    )
    .unwrap();

    // Pre-create worktree dir
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-feature-task", task_id.0)),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("#{}", task_id.0)),
        "Expected feature task to be dispatched, got: {text}"
    );

    // Wait for spawn_blocking
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn dispatch_next_returns_disabled_when_auto_dispatch_off() {
    let state = test_state();

    // Create epic with auto_dispatch = false
    let epic = state
        .db
        .create_epic("E", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .patch_epic(epic.id, &db::EpicPatch::new().auto_dispatch(false))
        .unwrap();

    // Create a backlog subtask linked to the epic
    let task_id = state
        .db
        .create_task(
            "Sub",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_next",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;

    // Should return informational message, not dispatch
    let text = extract_response_text(&resp);
    assert!(
        text.contains("auto dispatch is disabled"),
        "Expected disabled message, got: {text}"
    );

    // Task must still be in backlog — not dispatched
    let task_after = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task_after.status, TaskStatus::Backlog);
}

// -- wrap_up tests -----------------------------------------------------------

#[tokio::test]
async fn wrap_up_task_not_found() {
    let state = test_state();
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
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-my-task"))
            .sub_status(crate::models::SubStatus::NeedsInput),
    )
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
    )
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
async fn wrap_up_task_no_worktree() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
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
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
        )
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        // task.base_branch = "main" is passed explicitly to finish_task; no symbolic-ref call
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "My Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-my-task")),
    )
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
async fn wrap_up_rejects_pr_action() {
    // PR creation is now agent-driven. The /wrap-up skill instructs the
    // agent to author the title/body, run gh pr create itself, and
    // record the result via update_task. wrap_up only handles rebase.
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
        )
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
    // Serde rejects the unknown variant — the message includes "pr"
    // and lists the valid variants. The skill ensures agents no longer
    // pass this argument.
    assert_error(&resp, "unknown variant `pr`");
}

#[tokio::test]
async fn wrap_up_rebase_conflict_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::fail("CONFLICT (content): Merge conflict in foo.rs"), // git rebase
        MockProcessRunner::ok(),                      // git rebase --abort
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Conflict Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-conflict-task")),
    )
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
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_not_on_main_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail(""), // git rev-parse (empty stdout → treated as non-main)
        MockProcessRunner::ok_with_stdout(b"feature\n"), // unused
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Wrong Branch",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-wrong-branch")),
    )
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
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on error"
    );
}

#[tokio::test]
async fn wrap_up_rebase_does_not_change_status() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Rebase Done",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-rebase-done")),
    )
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
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "wrap_up(rebase) must not change status"
    );
}

#[tokio::test]
async fn wrap_up_rebase_does_not_recalculate_epic_status() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = db
        .create_task(
            "Only Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-only-task")),
    )
    .unwrap();
    db.recalculate_epic_status(epic.id).unwrap();
    let epic_status_before = db.get_epic(epic.id).unwrap().unwrap().status;

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

    let epic_after = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(
        epic_after.status, epic_status_before,
        "wrap_up must not recalculate epic status — that runs at exit_session"
    );
}

#[tokio::test]
async fn wrap_up_accepts_string_task_id() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse
        MockProcessRunner::fail(""),                  // git remote get-url
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
    )
    .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0.to_string(), "action": "rebase" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "wrap_up should accept string task_id: {:?}",
        resp.error
    );
}

#[tokio::test]
async fn wrap_up_rebase_preserves_tmux_window() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Rebase Preserve Window",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-rebase-preserve"))
            .tmux_window(Some("task-99")),
    )
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
    assert!(text.contains("wrap_up complete"));
    assert!(
        text.contains("exit_session"),
        "response should instruct agent to call exit_session; got: {text}"
    );

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "wrap_up must not change status — exit_session owns the Done transition"
    );
    assert!(
        task.tmux_window.is_some(),
        "tmux_window must NOT be cleared — exit_session owns the window kill"
    );
}

#[tokio::test]
async fn wrap_up_rebase_conflict_sets_conflict_substatus() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::fail("CONFLICT (content): Merge conflict in foo.rs"), // git rebase
        MockProcessRunner::ok(),                      // git rebase --abort
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Conflict Sub",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-conflict-sub")),
    )
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
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "Task should remain Review on rebase conflict"
    );
    assert_eq!(
        task.sub_status,
        SubStatus::Conflict,
        "sub_status should be Conflict after rebase conflict"
    );
}

#[tokio::test]
async fn wrap_up_rebase_clears_conflict_substatus_on_non_conflict_error() {
    // When a task has Conflict sub_status from a previous rebase attempt,
    // and a new rebase fails with a non-conflict error (e.g. Other), the
    // stale Conflict sub_status should be cleared — matching TUI behavior.
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail(""), // detect_default_branch (symbolic-ref)
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""), // git remote get-url (no remote)
        MockProcessRunner::fail("fatal: some other git error"), // git rebase (non-conflict failure)
        MockProcessRunner::ok(),     // git rebase --abort
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Stale Conflict",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/1-stale-conflict"))
            .sub_status(SubStatus::Conflict),
    )
    .unwrap();

    // Verify conflict is set before wrap_up
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Conflict);

    let _resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "wrap_up",
            "arguments": { "task_id": task_id.0, "action": "rebase" }
        })),
    )
    .await;

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_ne!(
        task.sub_status,
        SubStatus::Conflict,
        "Stale Conflict sub_status should be cleared even on non-conflict rebase error"
    );
}

// -- base_branch tests -------------------------------------------------------

#[tokio::test]
async fn create_task_with_base_branch_stores_it() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "My Feature",
                "repo_path": "/repo",
                "base_branch": "develop",
                "project_id": default_id,
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let tasks = state.db.list_all().unwrap();
    let task = tasks.iter().find(|t| t.title == "My Feature").unwrap();
    assert_eq!(task.base_branch, "develop");
}

#[tokio::test]
async fn create_task_without_base_branch_defaults_to_main() {
    let state = test_state();
    let default_id = state.db.get_default_project().unwrap().id;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Default Branch Task",
                "repo_path": "/repo",
                "project_id": default_id,
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let tasks = state.db.list_all().unwrap();
    let task = tasks
        .iter()
        .find(|t| t.title == "Default Branch Task")
        .unwrap();
    assert_eq!(task.base_branch, "main");
}

#[tokio::test]
async fn update_task_with_base_branch_updates_it() {
    let state = test_state();

    let task_id = state
        .db
        .create_task(
            "T",
            "d",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "base_branch": "release/2.0"
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.base_branch, "release/2.0");
}

// -- wrap_up: reflection nudge tests ----------------------------------------

fn make_rebase_state() -> (Arc<dyn db::TaskStore>, Arc<McpState>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });
    (db, state)
}

fn seed_task_with_worktree(db: &Arc<dyn db::TaskStore>, suffix: &str) -> crate::models::TaskId {
    let task_id = db
        .create_task(
            &format!("Task {suffix}"),
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some(&format!(
            "/repo/.worktrees/{}-task-{suffix}",
            task_id.0
        ))),
    )
    .unwrap();
    task_id
}

#[tokio::test]
async fn wrap_up_rebase_directs_to_exit_session_not_reflection_nudge() {
    // After the behavioral change, wrap_up(rebase) no longer emits the
    // reflection nudge inline. Instead it tells the agent to call exit_session,
    // which handles the reflection prompt on first call.
    let (db, state) = make_rebase_state();
    let task_id = seed_task_with_worktree(&db, "nudge-default");

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
    let (db, state) = make_rebase_state();
    db.set_setting_bool("learning_reflection_enabled", true)
        .unwrap();
    let task_id = seed_task_with_worktree(&db, "nudge-setting-irrelevant");

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

// -- update_task PR-finalisation nudge tests --------------------------------

#[tokio::test]
async fn update_task_pr_finalisation_appends_reflection_nudge_by_default() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "PR finalise",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "pr_url": "https://github.com/org/repo/pull/7",
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("record_learning"),
        "nudge should appear when finalising a PR via update_task; got: {text}"
    );
}

#[tokio::test]
async fn update_task_pr_finalisation_omits_nudge_when_disabled() {
    let state = test_state();
    state
        .db
        .set_setting_bool("learning_reflection_enabled", false)
        .unwrap();
    let task_id = state
        .db
        .create_task(
            "PR finalise disabled",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "pr_url": "https://github.com/org/repo/pull/7",
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear when reflection disabled; got: {text}"
    );
}

#[tokio::test]
async fn update_task_pr_set_without_status_does_not_nudge() {
    // Agent setting only pr_url (no status transition) is not a wrap-up
    // finalisation — don't nudge. This preserves current update_task UX
    // for non-wrap-up callers tweaking the URL.
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "PR set no status",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "pr_url": "https://github.com/org/repo/pull/7"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear when status is not transitioning; got: {text}"
    );
}

#[tokio::test]
async fn update_task_status_review_without_pr_url_change_does_not_nudge() {
    // Re-confirming a task to review without setting a new pr_url is
    // not a wrap-up finalisation. No nudge.
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Already in review",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear without pr_url transition; got: {text}"
    );
}

#[tokio::test]
async fn update_task_pr_url_already_set_does_not_nudge_again() {
    // The nudge should fire only on the first null->set transition.
    // Subsequent updates to pr_url (e.g. correcting the URL) must not
    // re-nudge.
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "PR already set",
            "desc",
            "/repo",
            None,
            TaskStatus::Review,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .patch_task(
            task_id,
            &db::TaskPatch::new().pr_url(Some("https://github.com/org/repo/pull/1")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "pr_url": "https://github.com/org/repo/pull/2",
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not fire when pr_url was already set; got: {text}"
    );
}

#[tokio::test]
async fn wrap_up_rebase_does_not_kill_window() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Rebase Task",
            "description",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Set up worktree + tmux_window so is_wrappable passes.
    let patch = crate::db::TaskPatch::new()
        .worktree(Some("/repo/.worktrees/task-rebase"))
        .tmux_window(Some("task-rebase-window"));
    state.db.patch_task(task_id, &patch).unwrap();

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

    let task = state.db.get_task(task_id).unwrap().unwrap();
    // tmux_window must NOT be cleared — exit_session owns the window kill.
    assert!(
        task.tmux_window.is_some(),
        "wrap_up(rebase) must not clear tmux_window — exit_session is responsible"
    );
}

// -- exit_session tests -------------------------------------------------------

#[tokio::test]
async fn exit_session_first_call_returns_reflection_nudge() {
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("record_learning"),
        "expected reflection nudge, got: {text}"
    );
    assert!(
        text.contains("query_learnings"),
        "expected query_learnings mention, got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "expected exit_session call instruction, got: {text}"
    );

    // Window must NOT be killed on first call
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert!(
        task.tmux_window.is_some(),
        "tmux_window should still be set after first call"
    );
}

#[tokio::test]
async fn exit_session_second_call_clears_window_and_returns_closed() {
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    // First call
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    // Second call
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert_eq!(
        text, "Session closed.",
        "expected 'Session closed.' message, got: {text}"
    );

    // tmux_window must be cleared in DB
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert!(
        task.tmux_window.is_none(),
        "tmux_window should be cleared after second call"
    );
}

#[tokio::test]
async fn exit_session_unknown_task_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn exit_session_task_without_window_returns_error() {
    let state = test_state();
    let task_id = create_task_fixture(&state); // Backlog task, no tmux_window

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    assert_error(&resp, "no active session");
}

#[tokio::test]
async fn exit_session_pending_cleared_on_redispatch() {
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    // First exit_session call — inserts into pending set
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    // Verify it's in the set right now
    {
        let pending = state.exit_session_pending.lock().unwrap();
        assert!(pending.contains(&task_id), "should be pending before clear");
    }

    // Patch task back to backlog so dispatch_task accepts it
    let patch = crate::db::TaskPatch::new().status(crate::models::TaskStatus::Backlog);
    state.db.patch_task(task_id, &patch).unwrap();

    // Call dispatch_task — this should clear the pending state
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    {
        let pending = state.exit_session_pending.lock().unwrap();
        assert!(
            !pending.contains(&task_id),
            "pending should be cleared after dispatch"
        );
    }
}

#[tokio::test]
async fn exit_session_second_call_marks_task_done() {
    // No-epic branch: the closing call must mark the task Done even when
    // there is no epic to recalculate. Pins the `is_some()` guard around
    // recalculate_epic_status.
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Done));
    assert!(task.tmux_window.is_none());
    assert!(task.epic_id.is_none(), "fixture should have no epic");
}

#[tokio::test]
async fn exit_session_first_call_does_not_change_status() {
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).unwrap().unwrap();
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
    let state = test_state();
    let task_id = create_running_task_with_window(&state);
    state
        .db
        .patch_task(
            task_id,
            &crate::db::TaskPatch::new().status(TaskStatus::Done),
        )
        .unwrap();

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn exit_session_recalculates_epic_status() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = create_running_task_with_window(&state);
    state.db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    state.db.recalculate_epic_status(epic.id).unwrap();
    let epic_before = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_ne!(
        epic_before.status,
        TaskStatus::Done,
        "precondition: epic should be in-progress before exit_session"
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let epic_after = state.db.get_epic(epic.id).unwrap().unwrap();
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
    let state = test_state();
    let task_id = create_running_task_with_window(&state);
    state
        .db
        .patch_task(
            task_id,
            &crate::db::TaskPatch::new().sub_status(SubStatus::Stale),
        )
        .unwrap();

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    let task = state.db.get_task(task_id).unwrap().unwrap();
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
    let (state, mut rx) = test_state_with_notify();
    let task_id = create_running_task_with_window(&state);

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    while rx.try_recv().is_ok() {}

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    // DB must already be Done by the time the Refresh fires.
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);

    let event = rx
        .try_recv()
        .expect("expected Refresh after closing exit_session");
    assert!(matches!(event, crate::mcp::McpEvent::Refresh));
}

#[tokio::test]
async fn wrap_up_then_exit_session_end_to_end() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // git rev-parse --abbrev-ref HEAD
        MockProcessRunner::fail(""),                  // git remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main
        MockProcessRunner::ok(),                      // git merge --ff-only
        // exit_session second call kills the tmux window:
        MockProcessRunner::ok(), // tmux has-session
        MockProcessRunner::ok(), // tmux kill-window
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic("E2E Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = db
        .create_task(
            "E2E Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    db.patch_task(
        task_id,
        &crate::db::TaskPatch::new()
            .worktree(Some("/repo/.worktrees/e2e"))
            .tmux_window(Some("e2e-window")),
    )
    .unwrap();
    db.recalculate_epic_status(epic.id).unwrap();
    let epic_before = db.get_epic(epic.id).unwrap().unwrap();
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

    let after_wrap_up = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        after_wrap_up.status,
        TaskStatus::Running,
        "after wrap_up: status must still be Running"
    );
    assert!(
        after_wrap_up.tmux_window.is_some(),
        "after wrap_up: tmux_window must be preserved"
    );
    let epic_after_wrap_up = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(
        epic_after_wrap_up.status, epic_before.status,
        "after wrap_up: epic status must not change"
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let close_resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(close_resp.error.is_none(), "{:?}", close_resp.error);

    let final_task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(final_task.status, TaskStatus::Done);
    assert_eq!(
        final_task.sub_status,
        SubStatus::default_for(TaskStatus::Done)
    );
    assert!(final_task.tmux_window.is_none());

    let final_epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(
        final_epic.status,
        TaskStatus::Done,
        "epic auto-advances once its only subtask completes via exit_session"
    );
}

// -- list_tasks caller_task_id / scope derivation tests ---------------------

#[tokio::test]
async fn list_tasks_caller_task_id_scopes_to_epic() {
    let state = test_state();

    // Create epics directly via DB
    let epic = state
        .db
        .create_epic("My Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    let epic2 = state
        .db
        .create_epic("Other Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    // Task A (caller) in epic
    let id_a = state
        .db
        .create_task(
            "Task A",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            Some(epic.id),
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Task B (sibling) in the same epic
    state
        .db
        .create_task(
            "Task B",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            Some(epic.id),
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Task C in a different epic (should NOT appear)
    state
        .db
        .create_task(
            "Task C",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            Some(epic2.id),
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": id_a.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Task B"), "should include sibling Task B");
    assert!(!text.contains("Task A"), "should exclude the caller Task A");
    assert!(
        !text.contains("Task C"),
        "should exclude Task C from other epic"
    );
}

#[tokio::test]
async fn list_tasks_caller_task_id_scopes_to_project_when_no_epic() {
    let state = test_state();

    // Task A (caller) in project 1, no epic
    let id_a = state
        .db
        .create_task(
            "Task A",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Task B sibling in project 1
    state
        .db
        .create_task(
            "Task B",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Task C in project 2 (should NOT appear)
    state
        .db
        .create_task(
            "Task C",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(2),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": id_a.0 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("Task B"),
        "should include project sibling Task B"
    );
    assert!(!text.contains("Task A"), "should exclude caller Task A");
    assert!(
        !text.contains("Task C"),
        "should exclude Task C from project 2"
    );
}

#[tokio::test]
async fn list_tasks_explicit_scope_overrides_caller_derived_scope() {
    let state = test_state();

    // Create epic directly via DB
    let epic = state
        .db
        .create_epic("Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    // Caller is in the epic
    let id_a = state
        .db
        .create_task(
            "Task A",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            Some(epic.id),
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Task B also in the epic
    state
        .db
        .create_task(
            "Task B",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            Some(epic.id),
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Task C in project 2, no epic — explicit project_id=2 should match this
    state
        .db
        .create_task(
            "Task C",
            "",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(2),
        )
        .unwrap();

    // Pass caller_task_id (which has epic) BUT also explicit project_id=2 → project wins
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": id_a.0, "project_id": 2 }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("Task C"),
        "explicit project_id=2 should show Task C"
    );
    assert!(
        !text.contains("Task B"),
        "Task B is in epic/project1, should not appear"
    );
    assert!(!text.contains("Task A"), "caller excluded");
}

#[tokio::test]
async fn list_tasks_repo_paths_filter() {
    let state = test_state();

    state
        .db
        .create_task(
            "Repo A task",
            "",
            "/repo/a",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    state
        .db
        .create_task(
            "Repo B task",
            "",
            "/repo/b",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "repo_paths": ["/repo/a"] }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Repo A task"));
    assert!(!text.contains("Repo B task"));
}

#[tokio::test]
async fn list_tasks_unknown_caller_task_id_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "caller_task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "Unknown caller_task_id");
}

#[tokio::test]
async fn list_tasks_includes_pr_url_in_output() {
    let state = test_state();

    let task_id = create_task_fixture(&state);
    state
        .db
        .patch_task(
            task_id,
            &crate::db::TaskPatch::new().pr_url(Some("https://github.com/org/repo/pull/42")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("| PR: https://github.com/org/repo/pull/42"),
        "PR URL should appear in output; got: {text}"
    );
}

#[tokio::test]
async fn list_tasks_includes_plan_goal_in_output() {
    let state = test_state();

    let plan_path = std::env::temp_dir().join("dispatch_test_plan_345.md");
    std::fs::write(
        &plan_path,
        "# My Feature — Implementation Plan\n\n**Goal:** Implement the learning enrichment.\n",
    )
    .unwrap();
    let plan_path_str = plan_path.to_string_lossy().to_string();

    state
        .db
        .create_task(
            "Feature task",
            "desc",
            "/repo",
            Some(&plan_path_str),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("| Goal: Implement the learning enrichment."),
        "Plan goal should appear in output; got: {text}"
    );

    let _ = std::fs::remove_file(&plan_path);
}

#[tokio::test]
async fn list_tasks_falls_back_to_description_when_no_plan() {
    let state = test_state();

    state
        .db
        .create_task(
            "No Plan Task",
            "A task without a plan file",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("A task without a plan file"),
        "Description should appear as fallback; got: {text}"
    );
}

#[tokio::test]
async fn list_tasks_omits_pr_segment_when_no_pr_url() {
    let state = test_state();
    create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("| PR:"),
        "No PR segment should appear when pr_url is null; got: {text}"
    );
}

// -- dispatch_task tests -----------------------------------------------------

#[tokio::test]
async fn dispatch_task_dispatches_backlog_task() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text / write prompt file)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "My Backlog Task",
            "do the thing",
            &repo_path,
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();

    // Pre-create worktree dir (mocked git won't create it)
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-my-backlog-task", task_id.0)),
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

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected 'dispatched' in response, got: {text}"
    );

    // dispatch_task is synchronous — no sleep needed
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(
        task.worktree.is_some(),
        "worktree should be set after dispatch"
    );
    assert!(
        task.tmux_window.is_some(),
        "tmux_window should be set after dispatch"
    );
}

#[tokio::test]
async fn dispatch_task_returns_error_for_non_backlog_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(
            "Running Task",
            "already running",
            "/repo",
            None,
            TaskStatus::Running,
            "main",
            None,
            None,
            None,
            ProjectId(1),
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

    assert_error(&resp, "not in backlog");
}

#[tokio::test]
async fn dispatch_task_unknown_task_id_returns_error() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_task",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;

    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_task_respects_tag_routing() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (literal text / write prompt file)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    // Feature-tagged task with no plan → should route to Plan mode
    let task_id = db
        .create_task(
            "Feature Task",
            "a new feature",
            &repo_path,
            None, // no plan
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
        )
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().tag(Some(crate::models::TaskTag::Feature)),
    )
    .unwrap();

    // Pre-create worktree dir
    std::fs::create_dir_all(
        dir.path()
            .join(".worktrees")
            .join(format!("{}-feature-task", task_id.0)),
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

    let text = extract_response_text(&resp);
    assert!(
        text.contains("dispatched"),
        "Expected dispatch confirmation, got: {text}"
    );

    // Task should be Running — plan mode still dispatches an agent
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn dispatch_task_returns_error_when_dispatch_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    // First mock call fails (tmux new-window fails) → dispatch errors out
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("tmux: no server running"), // tmux new-window fails
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = db
        .create_task(
            "Backlog Task",
            "will fail to dispatch",
            &repo_path,
            None,
            TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            ProjectId(1),
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

    assert!(resp.error.is_some(), "expected error when dispatch fails");

    // Task status must remain Backlog — dispatch failure must not leave it as Running
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Backlog,
        "task should remain Backlog after dispatch failure"
    );
}
