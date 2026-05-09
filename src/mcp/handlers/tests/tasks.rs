#![allow(clippy::unwrap_used, clippy::expect_used)]
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

#[tokio::test]
async fn get_task_found() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "My Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

#[tokio::test]
async fn unknown_tool() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "bogus_tool", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("Unknown tool"));
}

#[tokio::test]
async fn unknown_method() {
    let state = test_state();
    let resp = call(&state, "bogus/method", None).await;
    assert!(resp.error.is_some());
    assert!(resp.error.unwrap().message.contains("Method not found"));
}

#[tokio::test]
async fn create_task_minimal() {
    let state = test_state();
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let other = state.db.create_project("Other", 1).await.unwrap();
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

// -- String task_id coercion (Claude Code sends integers as strings) ------

#[tokio::test]
async fn update_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "My Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Old",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Old",
            description: "old desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/old/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "Desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: Some("/existing.md"),
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "PR test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "desc a",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "desc b",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Backlog Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Running Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Backlog Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Running Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Review Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Claimable",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Running",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Other Repo",
            description: "desc",
            repo_path: "/other-repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// -- claim_task tests -------------------------------------------------------

#[tokio::test]
async fn claim_task_accepts_string_task_id() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Claimable",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// =======================================================================
// Epic tool tests
// =======================================================================

#[tokio::test]
async fn create_epic_minimal() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": { "title": "My Epic", "repo_path": "/repo" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Epic"));
    assert!(text.contains("created"));

    let epics = state.db.list_epics().unwrap();
    assert_eq!(epics.len(), 1);
    assert_eq!(epics[0].title, "My Epic");
    assert_eq!(epics[0].repo_path, "/repo");
}

#[tokio::test]
async fn create_epic_with_all_fields() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "Full Epic",
                "repo_path": "/repo",
                "description": "Epic desc"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let epics = state.db.list_epics().unwrap();
    assert_eq!(epics[0].description, "Epic desc");
}

#[tokio::test]
async fn create_epic_missing_title() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": { "repo_path": "/repo" }
        })),
    )
    .await;
    assert_error(&resp, "Invalid arguments");
}

#[tokio::test]
async fn create_epic_missing_repo_path() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": { "title": "No Repo" }
        })),
    )
    .await;
    assert_error(&resp, "Invalid arguments");
}

#[tokio::test]
async fn get_epic_found() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Get Me", "desc", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Get Me"));
    assert!(text.contains("desc"));
    assert!(text.contains("/repo"));
}

#[tokio::test]
async fn get_epic_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn get_epic_shows_subtask_summary() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("With Tasks", "", "/repo", None, ProjectId(1))
        .unwrap();
    let t1 = state
        .db
        .create_task(CreateTaskRequest {
            title: "Sub 1",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let t2 = state
        .db
        .create_task(CreateTaskRequest {
            title: "Sub 2",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();
    state.db.set_task_epic_id(t2, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("1/2 done"),
        "expected subtask summary, got: {text}"
    );
}

#[tokio::test]
async fn get_epic_accepts_string_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("String ID", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0.to_string() }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(text.contains("String ID"));
}

#[tokio::test]
async fn list_epics_empty() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No epics found"));
}

#[tokio::test]
async fn list_epics_with_items() {
    let state = test_state();
    state
        .db
        .create_epic("Epic A", "desc a", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .create_epic("Epic B", "desc b", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Epic A"));
    assert!(text.contains("Epic B"));
}

#[tokio::test]
async fn list_epics_shows_subtask_counts() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Tracked", "", "/repo", None, ProjectId(1))
        .unwrap();
    let t1 = state
        .db
        .create_task(CreateTaskRequest {
            title: "Done",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let t2 = state
        .db
        .create_task(CreateTaskRequest {
            title: "Pending",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state.db.set_task_epic_id(t1, Some(epic.id)).unwrap();
    state.db.set_task_epic_id(t2, Some(epic.id)).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("1/2 done"),
        "expected subtask counts, got: {text}"
    );
}

#[tokio::test]
async fn update_epic_title() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Old Title", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "title": "New Title" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("updated"));
    assert!(text.contains("title"));

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.title, "New Title");
}

#[tokio::test]
async fn update_epic_mark_done() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("To Finish", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "status": "done" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("status"),
        "response should mention status field: {text}"
    );

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.status, crate::models::TaskStatus::Done);
}

#[tokio::test]
async fn update_epic_multiple_fields() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Old", "old desc", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": {
                "epic_id": epic.id.0,
                "title": "New",
                "description": "new desc"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.title, "New");
    assert_eq!(updated.description, "new desc");
}

#[tokio::test]
async fn update_epic_accepts_string_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Str Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0.to_string(), "title": "Updated" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );
}

#[tokio::test]
async fn update_epic_plan() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Planned Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "plan_path": "docs/plans/epic-plan.md" }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("plan"),
        "response should mention plan: {text}"
    );

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(
        updated.plan_path.as_deref(),
        Some("docs/plans/epic-plan.md")
    );
}

// =======================================================================
// Additional edge case tests
// =======================================================================

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
    assert_error(&resp, "Unknown status: bogus");
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
    assert_error(&resp, "expected a status string");
}

#[tokio::test]
async fn claim_task_rejects_done_task() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Done",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Review",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Direct",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
async fn create_task_with_epic_id() {
    let state = test_state();
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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
async fn update_epic_no_fields_errors() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Test", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    assert_error(&resp, "At least one");
}

#[tokio::test]
async fn update_epic_feed_command_set() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Feed Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "feed_command": "echo []" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.feed_command.as_deref(), Some("echo []"));
}

#[tokio::test]
async fn update_epic_feed_command_clear() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Feed Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .patch_epic(
            epic.id,
            &crate::db::EpicPatch::default().feed_command(Some("old cmd")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "feed_command": null }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert!(
        updated.feed_command.is_none(),
        "feed_command should be cleared"
    );
}

#[tokio::test]
async fn update_epic_feed_command_absent_preserves_existing() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Feed Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .patch_epic(
            epic.id,
            &crate::db::EpicPatch::default().feed_command(Some("keep me")),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "title": "Updated Title" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.feed_command.as_deref(), Some("keep me"));
}

#[tokio::test]
async fn update_epic_feed_interval_secs_set() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Feed Epic", "", "/repo", None, ProjectId(1))
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "feed_interval_secs": 60 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.feed_interval_secs, Some(60));
}

#[tokio::test]
async fn update_epic_feed_interval_secs_clear() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Feed Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .patch_epic(
            epic.id,
            &crate::db::EpicPatch::default().feed_interval_secs(Some(120)),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "feed_interval_secs": null }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let updated = state.db.get_epic(epic.id).unwrap().unwrap();
    assert!(
        updated.feed_interval_secs.is_none(),
        "feed_interval_secs should be cleared"
    );
}

#[tokio::test]
async fn get_epic_shows_feed_command() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Feed Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .patch_epic(
            epic.id,
            &crate::db::EpicPatch::default()
                .feed_command(Some("./scripts/feed.sh"))
                .feed_interval_secs(Some(300)),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_epic",
            "arguments": { "epic_id": epic.id.0 }
        })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("./scripts/feed.sh"),
        "get_epic should show feed_command: {text}"
    );
    assert!(
        text.contains("300"),
        "get_epic should show feed_interval_secs: {text}"
    );
}

#[tokio::test]
async fn claim_task_updates_status_to_running() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Claim",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// wrap_up tests
// ---------------------------------------------------------------------------

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
            project_id: ProjectId(1),
        })
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
    });

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
            project_id: ProjectId(1),
        })
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
    });

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
            project_id: ProjectId(1),
        })
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
async fn wrap_up_rebase_response_demands_exit_session_imperatively() {
    // The wrap_up rebase response is the agent's primary cue to call
    // exit_session. It must:
    //   - name exit_session as the next call,
    //   - be imperative (not advisory like "when ready"),
    //   - say the session is not yet closed so the agent does not stop.
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
    });

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
            project_id: ProjectId(1),
        })
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
}

#[tokio::test]
async fn wrap_up_task_no_worktree() {
    let state = test_state();
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
            project_id: ProjectId(1),
        })
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
            project_id: ProjectId(1),
        })
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
    });

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
            project_id: ProjectId(1),
        })
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
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// wrap_up + learning_verdicts tests
// ---------------------------------------------------------------------------

fn make_state_with_runner(
    runner: Arc<dyn ProcessRunner>,
) -> (Arc<McpState>, Arc<dyn db::TaskStore>) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });
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

fn create_wrappable_task(db: &Arc<dyn db::TaskStore>) -> crate::models::TaskId {
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
            project_id: ProjectId(1),
        })
        .unwrap();
    db.patch_task(
        task_id,
        &db::TaskPatch::new().worktree(Some("/repo/.worktrees/1-t")),
    )
    .unwrap();
    task_id
}

fn create_approved_user_learning(
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
        })
        .unwrap();
    db.patch_learning(
        id,
        &crate::db::LearningPatch::new().status(crate::models::LearningStatus::Approved),
    )
    .unwrap();
    id
}

fn setup_state_after_dispatch_with_two_retrieved_approved_learnings() -> (
    Arc<McpState>,
    Arc<dyn db::TaskStore>,
    crate::models::TaskId,
    crate::models::LearningId,
    crate::models::LearningId,
) {
    let (state, db) = make_state_with_runner(rebase_ok_runner());
    let task_id = create_wrappable_task(&db);
    let l1 = create_approved_user_learning(&db, "first learning");
    let l2 = create_approved_user_learning(&db, "second learning");
    db.record_retrieval(task_id, l1, crate::models::RetrievalSource::PromptInjection)
        .unwrap();
    db.record_retrieval(task_id, l2, crate::models::RetrievalSource::PromptInjection)
        .unwrap();
    (state, db, task_id, l1, l2)
}

#[tokio::test]
async fn wrap_up_with_verdicts_applies_them() {
    let (state, db, task_id, l1_id, l2_id) =
        setup_state_after_dispatch_with_two_retrieved_approved_learnings();
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
    let l1 = db.get_learning(l1_id).unwrap().unwrap();
    let l2 = db.get_learning(l2_id).unwrap().unwrap();
    assert_eq!(l1.confirmed_count, 1);
    assert_eq!(l2.status, crate::models::LearningStatus::NeedsReview);
    let _ = resp;
}

#[tokio::test]
async fn wrap_up_without_verdicts_still_succeeds() {
    let (state, db) = make_state_with_runner(rebase_ok_runner());
    let task_id = create_wrappable_task(&db);
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
        setup_state_after_dispatch_with_two_retrieved_approved_learnings();
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
    assert_eq!(resp.error.expect("expected error").code, -32602);
}

#[tokio::test]
async fn wrap_up_rejects_unknown_verdict_string() {
    let (state, _db, task_id, l1_id, _l2) =
        setup_state_after_dispatch_with_two_retrieved_approved_learnings();
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
    assert_eq!(resp.error.expect("expected error").code, -32602);
}

// ---------------------------------------------------------------------------
// sub_status tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_sets_sub_status() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Listed Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Detail Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

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
            project_id: ProjectId(1),
        })
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
    });

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
            project_id: ProjectId(1),
        })
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
async fn update_task_status_recalculates_epic_status() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
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
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// send_message tests
// ---------------------------------------------------------------------------

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
    });

    // Create sender and receiver tasks
    let sender_id = db
        .create_task(CreateTaskRequest {
            title: "Fix auth bug",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let receiver_id = db
        .create_task(CreateTaskRequest {
            title: "Review PR",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Sender",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Sender",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let receiver_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Receiver",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Sender",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let receiver_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Receiver",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// =======================================================================
// Notification flow tests
// =======================================================================

/// Helper: creates a test state with a real notification channel.
fn test_state_with_notify() -> (
    Arc<McpState>,
    tokio::sync::mpsc::UnboundedReceiver<crate::mcp::McpEvent>,
) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let state = Arc::new(McpState {
        db,
        notify_tx: Some(tx),
        runner,
    });
    (state, rx)
}

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
    let (state, mut rx) = test_state_with_notify();
    let default_id = state.db.get_default_project().await.unwrap().id;

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
    assert!(
        matches!(event, crate::mcp::McpEvent::TaskChanged(_)),
        "expected TaskChanged, got {event:?}"
    );
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
    assert!(
        matches!(event, crate::mcp::McpEvent::TaskChanged(t) if t == task_id),
        "expected TaskChanged({task_id:?}), got {event:?}"
    );
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

// =======================================================================
// update_task: additional validation and edge cases
// =======================================================================

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

// =======================================================================
// create_task: additional validation and edge cases
// =======================================================================

#[tokio::test]
async fn create_task_invalid_tag() {
    let state = test_state();
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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
    let default_id = state.db.get_default_project().await.unwrap().id;
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

// =======================================================================
// list_tasks: filtering edge cases
// =======================================================================

#[tokio::test]
async fn list_tasks_filters_by_epic_id() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("My Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    let t1 = state
        .db
        .create_task(CreateTaskRequest {
            title: "Epic Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Standalone Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Backlog Epic",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let t2 = state
        .db
        .create_task(CreateTaskRequest {
            title: "Running Epic",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "No Epic",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Done Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Backlog Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// =======================================================================
// wrap_up: verify DB state after successful operations
// =======================================================================

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
    });

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Rebase Done",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    let text = extract_response_text(&resp);
    assert!(text.contains("wrap_up complete"));

    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "wrap_up must not change status — exit_session owns the Done transition"
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
    });

    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Only Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let task_id = db
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
            project_id: ProjectId(1),
        })
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

// =======================================================================
// get_task: additional formatting checks
// =======================================================================

#[tokio::test]
async fn get_task_shows_all_fields() {
    let state = test_state();
    let epic = state
        .db
        .create_epic("Parent Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Full Task",
            description: "detailed desc",
            repo_path: "/repo",
            plan: Some("/plan.md"),
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Solo Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// =======================================================================
// list_tasks: format verification
// =======================================================================

#[tokio::test]
async fn list_tasks_shows_tag_and_plan_indicators() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "Tagged Planned",
            description: "desc",
            repo_path: "/repo",
            plan: Some("/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Epic Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Long Desc",
            description: &long_desc,
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

#[tokio::test]
async fn list_tasks_excludes_archived_by_default() {
    let state = test_state();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Active Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Archived Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Archived,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

#[tokio::test]
async fn list_epics_excludes_archived() {
    let state = test_state();
    state
        .db
        .create_epic("Active Epic", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    let archived_epic = state
        .db
        .create_epic("Archived Epic", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    state
        .db
        .patch_epic(
            archived_epic.id,
            &db::EpicPatch::new().status(TaskStatus::Archived),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_epics", "arguments": {} })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Active Epic"), "should show active epic");
    assert!(
        !text.contains("Archived Epic"),
        "should not show archived epic: {text}"
    );
}

// -- dispatch_next tests ------------------------------------------------------

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
        .create_task(CreateTaskRequest {
            title: "Running Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, ProjectId(1))
        .unwrap();
    let task1_id = db
        .create_task(CreateTaskRequest {
            title: "Task 1",
            description: "first",
            repo_path: &repo_path,
            plan: Some("docs/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task2_id = db
        .create_task(CreateTaskRequest {
            title: "Task 2",
            description: "second",
            repo_path: &repo_path,
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, ProjectId(1))
        .unwrap();

    // task1 has higher ID but lower sort_order — should be picked second
    let task1_id = db
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "first by id",
            repo_path: &repo_path,
            plan: Some("docs/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    let task2_id = db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "second by id",
            repo_path: &repo_path,
            plan: Some("docs/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let epic = db
        .create_epic("Test Epic", "desc", &repo_path, None, ProjectId(1))
        .unwrap();

    // Create a feature-tagged task with no plan — should use Plan mode
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Feature Task",
            description: "a feature",
            repo_path: &repo_path,
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// update_review_status tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_review_status_updates_pr() {
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let state = test_state();
    let pr = ReviewPr {
        number: 42,
        title: "Test PR".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(crate::db::PrKind::Review, &[pr]).unwrap();
    state
        .db
        .set_pr_agent(
            crate::db::PrKind::Review,
            "acme/app",
            42,
            "dispatch:review-42",
            "/tmp/wt",
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/app", "number": 42, "status": "findings_ready" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let status = state
        .db
        .pr_agent_status("review_prs", "acme/app", 42)
        .unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::FindingsReady));
}

#[tokio::test]
async fn update_review_status_no_match_errors() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/unknown", "number": 999, "status": "idle" }
        })),
    )
    .await;
    assert!(resp.error.is_some());
}

#[tokio::test]
async fn update_review_status_invalid_status_errors() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/app", "number": 1, "status": "bogus" }
        })),
    )
    .await;
    assert!(resp.error.is_some());
}

#[tokio::test]
async fn update_review_status_findings_ready_sets_action_required() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr, WorkflowItemKind};
    use chrono::Utc;

    let state = test_state();

    // Insert a PR and set an active review agent so update_agent_status succeeds
    let pr = ReviewPr {
        number: 42,
        title: "Test PR".to_string(),
        author: "alice".to_string(),
        repo: "org/repo".to_string(),
        url: "https://github.com/org/repo/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(crate::db::PrKind::Review, &[pr]).unwrap();
    state
        .db
        .set_pr_agent(
            crate::db::PrKind::Review,
            "org/repo",
            42,
            "dispatch:review-42",
            "/tmp/wt",
        )
        .unwrap();

    // Pre-insert a workflow row in Ongoing/Reviewing
    state
        .db
        .insert_pr_workflow_if_absent("org/repo", 42, WorkflowItemKind::ReviewerPr)
        .unwrap();
    state
        .db
        .upsert_pr_workflow(
            "org/repo",
            42,
            WorkflowItemKind::ReviewerPr,
            "ongoing",
            Some("reviewing"),
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "org/repo", "number": 42, "status": "findings_ready" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let row = state
        .db
        .get_pr_workflow("org/repo", 42, WorkflowItemKind::ReviewerPr)
        .unwrap()
        .unwrap();
    assert_eq!(row.state, "action_required");
    assert_eq!(row.sub_state.as_deref(), Some("findings_ready"));
}

#[tokio::test]
async fn update_review_status_findings_ready_without_workflow_row() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr, WorkflowItemKind};
    use chrono::Utc;

    let state = test_state();

    // Insert a PR and set an active review agent so update_agent_status succeeds
    let pr = ReviewPr {
        number: 88,
        title: "Test PR No Workflow".to_string(),
        author: "bob".to_string(),
        repo: "acme/product".to_string(),
        url: "https://github.com/acme/product/pull/88".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(crate::db::PrKind::Review, &[pr]).unwrap();
    state
        .db
        .set_pr_agent(
            crate::db::PrKind::Review,
            "acme/product",
            88,
            "dispatch:review-88",
            "/tmp/wt",
        )
        .unwrap();

    // NOTE: NO workflow row is inserted — find_pr_workflow_kind will return None

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_review_status",
            "arguments": { "repo": "acme/product", "number": 88, "status": "findings_ready" }
        })),
    )
    .await;
    // Should succeed even though there's no workflow row
    // (find_workflow_kind_for returns None, so upsert_pr_workflow is skipped)
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    // Agent status should be updated
    let status = state
        .db
        .pr_agent_status("review_prs", "acme/product", 88)
        .unwrap();
    assert_eq!(status.map(|s| s.as_db_str()), Some("findings_ready"));

    // No workflow row should exist since find_pr_workflow_kind found none
    let no_workflow = state
        .db
        .get_pr_workflow("acme/product", 88, WorkflowItemKind::ReviewerPr)
        .unwrap();
    assert!(no_workflow.is_none());
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
    });

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Rebase Preserve Window",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Conflict Sub",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Stale Conflict",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// base_branch: create_task and update_task MCP schema tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_with_base_branch_stores_it() {
    let state = test_state();
    let default_id = state.db.get_default_project().await.unwrap().id;

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
    let default_id = state.db.get_default_project().await.unwrap().id;

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
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Sub",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// Step 6: MCP sub-epic creation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mcp_create_sub_epic() {
    let state = test_state();

    // Create parent epic first
    let parent = state
        .db
        .create_epic("Parent Epic", "desc", "/tmp", None, ProjectId(1))
        .unwrap();

    // Create sub-epic via MCP with parent_epic_id
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "Sub Epic",
                "repo_path": "/tmp",
                "description": "child",
                "parent_epic_id": parent.id.0
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got: {:?}",
        resp.error
    );

    // Verify the sub-epic has the correct parent
    let epics = state.db.list_epics().unwrap();
    let sub = epics
        .iter()
        .find(|e| e.title == "Sub Epic")
        .expect("sub epic should be created");
    assert_eq!(
        sub.parent_epic_id,
        Some(parent.id),
        "sub epic should have parent_epic_id set"
    );
}

#[tokio::test]
async fn create_epic_tool_schema_includes_parent_epic_id() {
    let state = test_state();
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.as_ref().unwrap()["tools"].as_array().unwrap();
    let create_epic = tools
        .iter()
        .find(|t| t["name"] == "create_epic")
        .expect("create_epic not in tool list");
    let props = &create_epic["inputSchema"]["properties"];
    assert!(
        props.get("parent_epic_id").is_some(),
        "create_epic schema is missing parent_epic_id property"
    );
}

// ---------------------------------------------------------------------------
// Fixtures for review/security tests
// ---------------------------------------------------------------------------

fn insert_my_pr_fixture(state: &Arc<McpState>, number: i64, repo: &str) {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    let pr = ReviewPr {
        number,
        title: format!("My PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 5,
        deletions: 1,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    let mut existing = state.db.load_prs(PrKind::My).unwrap_or_default();
    existing.retain(|p| !(p.repo == repo && p.number == number));
    existing.push(pr);
    state.db.save_prs(PrKind::My, &existing).unwrap();
}

fn insert_review_pr_fixture(state: &Arc<McpState>, number: i64, repo: &str) {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    let pr = ReviewPr {
        number,
        title: format!("PR #{number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    // Load existing PRs and append to avoid batch-replace deleting prior inserts.
    let mut existing = state.db.load_prs(PrKind::Review).unwrap_or_default();
    existing.retain(|p| !(p.repo == repo && p.number == number));
    existing.push(pr);
    state.db.save_prs(PrKind::Review, &existing).unwrap();
}

fn insert_security_alert_fixture(
    state: &Arc<McpState>,
    number: i64,
    repo: &str,
    kind: crate::models::AlertKind,
) {
    use crate::models::{AlertSeverity, SecurityAlert};
    let alert = SecurityAlert {
        number,
        repo: repo.to_string(),
        severity: AlertSeverity::High,
        kind,
        title: format!("Alert #{number}"),
        package: Some("some-pkg".to_string()),
        vulnerable_range: Some("< 1.0".to_string()),
        fixed_version: Some("1.0.0".to_string()),
        cvss_score: Some(7.5),
        url: format!("https://github.com/{repo}/security/dependabot/{number}"),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "A vulnerability".to_string(),
    };
    // Load existing alerts and append to avoid batch-replace deleting prior inserts.
    let mut existing = state.db.load_security_alerts().unwrap_or_default();
    existing.retain(|a| !(a.repo == repo && a.number == number && a.kind == kind));
    existing.push(alert);
    state.db.save_security_alerts(&existing).unwrap();
}

// ---------------------------------------------------------------------------
// list_review_prs tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_review_prs_empty() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No PRs found"));
}

#[tokio::test]
async fn list_review_prs_returns_stored_prs() {
    let state = test_state();
    insert_review_pr_fixture(&state, 42, "acme/app");
    insert_review_pr_fixture(&state, 99, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "reviewer"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("42"));
    assert!(text.contains("99"));
}

#[tokio::test]
async fn list_review_prs_filters_by_repo() {
    let state = test_state();
    insert_review_pr_fixture(&state, 1, "acme/app");
    insert_review_pr_fixture(&state, 2, "acme/other");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"repo": "acme/app"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(!text.contains("acme/other"));
}

// ---------------------------------------------------------------------------
// get_review_pr tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_review_pr_found() {
    let state = test_state();
    insert_review_pr_fixture(&state, 42, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 42}})),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(text.contains("42"));
}

#[tokio::test]
async fn get_review_pr_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 999}})),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn get_review_pr_found_in_my_prs() {
    let state = test_state();
    insert_my_pr_fixture(&state, 55, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_review_pr", "arguments": {"repo": "acme/app", "number": 55}})),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/app"));
    assert!(text.contains("55"));
}

// ---------------------------------------------------------------------------
// list_security_alerts tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_security_alerts_empty() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("No alerts found"));
}

#[tokio::test]
async fn list_security_alerts_returns_stored_alerts() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot);
    insert_security_alert_fixture(&state, 2, "acme/api", AlertKind::CodeScanning);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Alert #1"));
    assert!(text.contains("Alert #2"));
}

#[tokio::test]
async fn list_security_alerts_filters_by_kind() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot);
    insert_security_alert_fixture(&state, 2, "acme/api", AlertKind::CodeScanning);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"kind": "dependabot"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("Alert #1"));
    assert!(!text.contains("Alert #2"));
}

// ---------------------------------------------------------------------------
// get_security_alert tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_security_alert_found() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 7, "acme/api", AlertKind::Dependabot);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_security_alert",
            "arguments": {"repo": "acme/api", "number": 7, "kind": "dependabot"}
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/api"));
    assert!(text.contains("Alert #7"));
}

#[tokio::test]
async fn get_security_alert_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_security_alert",
            "arguments": {"repo": "acme/api", "number": 999, "kind": "dependabot"}
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

// ---------------------------------------------------------------------------
// dispatch_review_agent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_review_agent_pr_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 999, "local_repo": "/tmp/repo"}
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_review_agent_already_reviewing() {
    use crate::db::PrKind;
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    let state = test_state();
    let pr = ReviewPr {
        number: 42,
        title: "PR #42".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 2,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature/branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
    };
    state.db.save_prs(PrKind::Review, &[pr]).unwrap();
    // Persist the agent tracking fields (save_prs does not write these).
    state
        .db
        .set_pr_agent(
            PrKind::Review,
            "acme/app",
            42,
            "review-42",
            "/repo/.worktrees/review-42",
        )
        .unwrap();
    let _ = ReviewAgentStatus::Reviewing; // confirm variant exists

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 42, "local_repo": "/tmp/repo"}
        })),
    )
    .await;
    assert_error(&resp, "already has an active review agent");
}

// ---------------------------------------------------------------------------
// dispatch_fix_agent tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_fix_agent_alert_not_found() {
    let state = test_state();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 999,
                "kind": "dependabot", "local_repo": "/tmp/repo"
            }
        })),
    )
    .await;
    assert_error(&resp, "not found");
}

#[tokio::test]
async fn dispatch_fix_agent_already_reviewing() {
    use crate::models::{AlertKind, AlertSeverity, ReviewAgentStatus, SecurityAlert};
    let state = test_state();
    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-9999".to_string(),
        package: Some("pkg".to_string()),
        vulnerable_range: None,
        fixed_version: Some("1.0.0".to_string()),
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "A vuln".to_string(),
    };
    state.db.save_security_alerts(&[alert]).unwrap();
    // Persist the agent tracking fields (save_security_alerts does not write these).
    state
        .db
        .set_alert_agent(
            "acme/api",
            7,
            AlertKind::Dependabot,
            "fix-7",
            "/repo/.worktrees/fix-vuln-7",
        )
        .unwrap();
    let _ = ReviewAgentStatus::Reviewing; // confirm variant exists

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 7,
                "kind": "dependabot", "local_repo": "/tmp/repo"
            }
        })),
    )
    .await;
    assert_error(&resp, "already has an active fix agent");
}

#[tokio::test]
async fn list_review_prs_mode_author() {
    let state = test_state();
    insert_my_pr_fixture(&state, 55, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "author"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("55"), "PR #55 should appear in author mode");
}

#[tokio::test]
async fn list_review_prs_mode_all() {
    let state = test_state();
    insert_review_pr_fixture(&state, 10, "acme/app");
    insert_my_pr_fixture(&state, 20, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_review_prs", "arguments": {"mode": "all"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("10"),
        "reviewer PR #10 should appear in all mode"
    );
    assert!(
        text.contains("20"),
        "author PR #20 should appear in all mode"
    );
}

#[tokio::test]
async fn list_security_alerts_filters_by_severity() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    let state = test_state();

    let high_alert = SecurityAlert {
        number: 1,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "High Alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/1".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    let critical_alert = SecurityAlert {
        number: 2,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::Critical,
        kind: AlertKind::Dependabot,
        title: "Critical Alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/2".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    state
        .db
        .save_security_alerts(&[high_alert, critical_alert])
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"severity": "high"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("High Alert"), "High alert should appear");
    assert!(
        !text.contains("Critical Alert"),
        "Critical alert should not appear"
    );
}

#[tokio::test]
async fn list_security_alerts_filters_by_repo() {
    use crate::models::AlertKind;
    let state = test_state();
    insert_security_alert_fixture(&state, 1, "acme/api", AlertKind::Dependabot);
    insert_security_alert_fixture(&state, 2, "acme/web", AlertKind::Dependabot);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_security_alerts", "arguments": {"repo": "acme/api"}})),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("acme/api"), "acme/api alert should appear");
    assert!(
        !text.contains("acme/web"),
        "acme/web alert should not appear"
    );
}

#[tokio::test]
async fn dispatch_review_agent_success() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    // Pre-create worktree dir so git worktree add is skipped.
    std::fs::create_dir_all(dir.path().join(".worktrees").join("review-42")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window → false, empty stdout)
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git fetch origin feature/branch
        // git worktree add skipped (dir pre-exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l (claude cmd)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    insert_review_pr_fixture(&state, 42, "acme/app");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_review_agent",
            "arguments": {"repo": "acme/app", "number": 42, "local_repo": repo_path}
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Review agent dispatched"),
        "expected dispatch confirmation: {text}"
    );

    let status = db.pr_agent_status("review_prs", "acme/app", 42).unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be reviewing after dispatch"
    );
}

#[tokio::test]
async fn dispatch_fix_agent_success() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let dir = tempfile::TempDir::new().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    // Pre-create worktree dir so git worktree add is skipped.
    std::fs::create_dir_all(dir.path().join(".worktrees").join("fix-vuln-7")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window)
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"), // git symbolic-ref (detect default branch)
        MockProcessRunner::ok(),                                          // git fetch origin main
        // git worktree add skipped (dir pre-exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l (claude cmd)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-0001".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.21".to_string()),
        cvss_score: None,
        url: "https://example.com/7".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
    };
    db.save_security_alerts(&[alert]).unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "dispatch_fix_agent",
            "arguments": {
                "repo": "acme/api", "number": 7,
                "kind": "dependabot", "local_repo": repo_path
            }
        })),
    )
    .await;

    assert!(
        resp.error.is_none(),
        "expected success, got error: {:?}",
        resp.error
    );
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Fix agent dispatched"),
        "expected dispatch confirmation: {text}"
    );

    let status = db
        .alert_agent_status("acme/api", 7, AlertKind::Dependabot)
        .unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be reviewing after dispatch"
    );
}

// ---------------------------------------------------------------------------
// dispatch_task tests
// ---------------------------------------------------------------------------

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
    });

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "My Backlog Task",
            description: "do the thing",
            repo_path: &repo_path,
            plan: Some("docs/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Running Task",
            description: "already running",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    // Feature-tagged task with no plan → should route to Plan mode
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Feature Task",
            description: "a new feature",
            repo_path: &repo_path,
            plan: None,
            status: // no plan
            TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
    });

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Backlog Task",
            description: "will fail to dispatch",
            repo_path: &repo_path,
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// ---------------------------------------------------------------------------
// create_task project_id tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_with_project_id_assigns_correctly() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Other", 1).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "T",
                "description": "",
                "repo_path": "/r",
                "project_id": other.id
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let tasks = db.list_all().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].project_id, other.id);
}

// ---------------------------------------------------------------------------
// create_epic project_id tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_epic_without_project_id_assigns_to_default() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "E",
                "repo_path": "/r"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let epics = db.list_epics().unwrap();
    assert_eq!(epics.len(), 1);
    let default_id = db.get_default_project().await.unwrap().id;
    assert_eq!(epics[0].project_id, default_id);
}

#[tokio::test]
async fn create_epic_with_project_id_assigns_correctly() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Other", 1).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_epic",
            "arguments": {
                "title": "E",
                "repo_path": "/r",
                "project_id": other.id
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let epics = db.list_epics().unwrap();
    assert_eq!(epics.len(), 1);
    assert_eq!(epics[0].project_id, other.id);
}

// ---------------------------------------------------------------------------
// list_projects
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_projects_returns_all_projects() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    db.create_project("Dispatch", 1).await.unwrap();
    db.create_project("wizard_game", 2).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_projects", "arguments": {} })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let text = extract_response_text(&resp);
    assert!(text.contains("Default"), "expected Default project in list");
    assert!(
        text.contains("Dispatch"),
        "expected Dispatch project in list"
    );
    assert!(
        text.contains("wizard_game"),
        "expected wizard_game project in list"
    );
}

// ---------------------------------------------------------------------------
// update_task project_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_task_project_id_moves_task() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Dispatch", 1).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let task_id = create_task_fixture(&state);
    let default_id = db.get_default_project().await.unwrap().id;
    let task_before = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task_before.project_id, default_id);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "project_id": other.id }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let task_after = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task_after.project_id, other.id);
}

#[tokio::test]
async fn update_task_invalid_project_id_returns_error() {
    let state = test_state();
    let task_id = create_task_fixture(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "project_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "project");
    assert_eq!(resp.error.as_ref().unwrap().code, -32602);
}

// ---------------------------------------------------------------------------
// update_epic project_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_epic_project_id_moves_epic() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Dispatch", 1).await.unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic(
            "Test Epic",
            "",
            "/repo",
            None,
            db.get_default_project().await.unwrap().id,
        )
        .unwrap();
    let default_id = db.get_default_project().await.unwrap().id;
    assert_eq!(epic.project_id, default_id);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "project_id": other.id }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let epics = db.list_epics().unwrap();
    let updated = epics.iter().find(|e| e.id == epic.id).unwrap();
    assert_eq!(updated.project_id, other.id);
}

#[tokio::test]
async fn update_epic_invalid_project_id_returns_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
    });

    let epic = db
        .create_epic(
            "E",
            "",
            "/r",
            None,
            db.get_default_project().await.unwrap().id,
        )
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_epic",
            "arguments": { "epic_id": epic.id.0, "project_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "project");
    assert_eq!(resp.error.as_ref().unwrap().code, -32602);
}

// ---------------------------------------------------------------------------
// Learning tool tests
// ---------------------------------------------------------------------------

async fn default_project_id(state: &Arc<McpState>) -> ProjectId {
    state.db.get_default_project().await.unwrap().id
}

async fn create_task_in_repo(state: &Arc<McpState>, repo: &str) -> crate::models::TaskId {
    let pid = default_project_id(state).await;
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Test task",
            description: "",
            repo_path: repo,
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: pid,
        })
        .unwrap()
}

fn create_approved_learning(
    state: &Arc<McpState>,
    summary: &str,
    scope: crate::models::LearningScope,
    scope_ref: Option<&str>,
    tags: &[&str],
) -> crate::models::LearningId {
    let tag_strings: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    let id = state
        .db
        .create_learning(CreateLearningRow {
            kind: crate::models::LearningKind::Convention,
            summary,
            detail: None,
            scope,
            scope_ref,
            tags: &tag_strings,
            source_task_id: None,
        })
        .unwrap();
    state
        .db
        .patch_learning(
            id,
            &crate::db::LearningPatch::new().status(crate::models::LearningStatus::Approved),
        )
        .unwrap();
    id
}

// --- record_learning ---------------------------------------------------------

#[tokio::test]
async fn record_learning_creates_proposed_entry() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "Always use cargo fmt before committing",
                "scope": "repo",
                "scope_ref": "/repo/foo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(text.contains("active"), "expected 'active' in: {text}");

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Approved),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(
        learnings[0].summary,
        "Always use cargo fmt before committing"
    );
    assert_eq!(learnings[0].scope, crate::models::LearningScope::Repo);
    assert_eq!(learnings[0].source_task_id, Some(task_id));
}

#[tokio::test]
async fn record_learning_derives_scope_ref_for_repo() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/bar").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "pitfall",
                "summary": "Watch out for integer overflow",
                "scope": "repo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Approved),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(learnings[0].scope_ref.as_deref(), Some("/repo/bar"));
}

#[tokio::test]
async fn record_learning_derives_scope_ref_for_epic() {
    let state = test_state();
    let pid = default_project_id(&state).await;
    let epic = state.db.create_epic("E", "", "/r", None, pid).unwrap();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "",
            repo_path: "/r",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: pid,
        })
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "Epic-level outcome",
                "scope": "epic"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Approved),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert_eq!(
        learnings[0].scope_ref.as_deref(),
        Some(epic.id.0.to_string().as_str())
    );
}

#[tokio::test]
async fn record_learning_epic_scope_no_epic_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/baz").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "Epic outcome but no epic",
                "scope": "epic"
            }
        })),
    )
    .await;
    assert_error(&resp, "epic");
}

#[tokio::test]
async fn record_learning_user_scope_no_scope_ref() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "preference",
                "summary": "I prefer verbose variable names",
                "scope": "user"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let filter = crate::db::LearningFilter {
        status: Some(crate::models::LearningStatus::Approved),
        ..Default::default()
    };
    let learnings = state.db.list_learnings(filter).unwrap();
    assert_eq!(learnings.len(), 1);
    assert!(learnings[0].scope_ref.is_none());
}

#[tokio::test]
async fn record_learning_empty_summary_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "pitfall",
                "summary": "   ",
                "scope": "user"
            }
        })),
    )
    .await;
    assert_error(&resp, "summary");
}

#[tokio::test]
async fn record_learning_unknown_task_fails() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": 9999,
                "kind": "pitfall",
                "summary": "Some learning",
                "scope": "user"
            }
        })),
    )
    .await;
    assert_error(&resp, "9999");
}

// --- query_learnings ---------------------------------------------------------

#[tokio::test]
async fn query_learnings_returns_approved_for_task() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/myproject").await;
    create_approved_learning(
        &state,
        "Use anyhow for errors",
        crate::models::LearningScope::Repo,
        Some("/repo/myproject"),
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Use anyhow for errors"),
        "expected learning in: {text}"
    );
}

#[tokio::test]
async fn query_learnings_tag_filter_narrows_results() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/tagged").await;
    create_approved_learning(
        &state,
        "Rust tips",
        crate::models::LearningScope::Repo,
        Some("/repo/tagged"),
        &["rust"],
    );
    create_approved_learning(
        &state,
        "Testing tips",
        crate::models::LearningScope::Repo,
        Some("/repo/tagged"),
        &["testing"],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0, "tag_filter": "rust" }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    assert!(text.contains("Rust tips"), "expected rust learning");
    assert!(
        !text.contains("Testing tips"),
        "should not see testing learning"
    );
}

#[tokio::test]
async fn query_learnings_respects_limit() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/limited").await;
    for i in 0..5 {
        create_approved_learning(
            &state,
            &format!("Learning {i}"),
            crate::models::LearningScope::Repo,
            Some("/repo/limited"),
            &[],
        );
    }

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": task_id.0, "limit": 2 }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let text = extract_response_text(&resp);
    // Each entry starts with "[<id>]", count those occurrences
    let count = text.matches('[').count();
    assert_eq!(count, 2, "expected exactly 2 learnings, got text: {text}");
}

#[tokio::test]
async fn query_learnings_unknown_task_fails() {
    let state = test_state();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "query_learnings",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;
    assert_error(&resp, "9999");
}

// --- upvote_learning --------------------------------------------------------

#[tokio::test]
async fn upvote_learning_increments_count() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo").await;
    let learning_id = create_approved_learning(
        &state,
        "Useful tip",
        crate::models::LearningScope::User,
        None,
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "upvote_learning",
            "arguments": { "learning_id": learning_id, "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let learning = state.db.get_learning(learning_id).unwrap().unwrap();
    assert_eq!(learning.confirmed_count, 1);
}

#[tokio::test]
async fn upvote_learning_unknown_learning_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo").await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "upvote_learning",
            "arguments": { "learning_id": 9999, "task_id": task_id.0 }
        })),
    )
    .await;
    assert_error(&resp, "9999");
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
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Task B (sibling) in the same epic
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Task C in a different epic (should NOT appear)
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task C",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic2.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Task B sibling in project 1
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Task C in project 2 (should NOT appear)
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task C",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(2),
        })
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
        .create_task(CreateTaskRequest {
            title: "Task A",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Task B also in the epic
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task B",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    // Task C in project 2, no epic — explicit project_id=2 should match this
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Task C",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(2),
        })
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
        .create_task(CreateTaskRequest {
            title: "Repo A task",
            description: "",
            repo_path: "/repo/a",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    state
        .db
        .create_task(CreateTaskRequest {
            title: "Repo B task",
            description: "",
            repo_path: "/repo/b",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Feature task",
            description: "desc",
            repo_path: "/repo",
            plan: Some(&plan_path_str),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "No Plan Task",
            description: "A task without a plan file",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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

// =======================================================================
// wrap_up: reflection nudge
// =======================================================================

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
    });
    (db, state)
}

fn seed_task_with_worktree(db: &Arc<dyn db::TaskStore>, suffix: &str) -> crate::models::TaskId {
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
            project_id: ProjectId(1),
        })
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

// -- update_task PR-finalisation nudge tests -------------------------------
//
// When the agent records a freshly-created PR via update_task (per the
// agent-driven /wrap-up flow), the response should append the same
// reflection nudge that the rebase wrap_up emits — i.e. when pr_url
// transitions from null to a value AND status is being set to review.

#[tokio::test]
async fn update_task_pr_finalisation_appends_reflection_nudge_by_default() {
    let state = test_state();
    let task_id = state
        .db
        .create_task(CreateTaskRequest {
            title: "PR finalise",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "PR finalise disabled",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "PR set no status",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "Already in review",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        .create_task(CreateTaskRequest {
            title: "PR already set",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
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
        text.contains("pitfall") || text.contains("convention"),
        "first call should ask about pitfalls/conventions; got: {text}"
    );
    assert!(
        text.contains("has_learnings"),
        "first call should mention has_learnings parameter; got: {text}"
    );
    assert!(
        text.contains("exit_session"),
        "first call should instruct to call exit_session again; got: {text}"
    );

    // Window must NOT be killed on first call
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert!(
        task.tmux_window.is_some(),
        "tmux_window should still be set after first call"
    );
}

#[tokio::test]
async fn exit_session_has_learnings_true_returns_record_prompt() {
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    // First call
    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0 } })),
    )
    .await;

    // Second call: has_learnings=true
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": true }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("record_learning"),
        "has_learnings=true should prompt to call record_learning; got: {text}"
    );
    assert!(
        text.contains("kind"),
        "has_learnings=true should mention kind parameter; got: {text}"
    );

    // Session must NOT be closed yet
    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert!(
        task.tmux_window.is_some(),
        "tmux_window should still be set after has_learnings=true"
    );
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "task should still be Running after has_learnings=true"
    );
}

#[tokio::test]
async fn exit_session_has_learnings_false_closes_session() {
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    // First call
    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0 } })),
    )
    .await;

    // Second call: has_learnings=false — should close immediately
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": false }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert_eq!(
        text, "Session closed.",
        "has_learnings=false should close session; got: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert!(
        task.tmux_window.is_none(),
        "tmux_window should be cleared after has_learnings=false"
    );
    assert_eq!(task.status, TaskStatus::Done);
}

#[tokio::test]
async fn exit_session_after_record_prompt_closes_with_has_learnings_false() {
    // Stateless flow: 1st call asks, 2nd with has_learnings=true returns the
    // record_learning prompt, 3rd with has_learnings=false closes.
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0 } })),
    )
    .await;

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": true }
        })),
    )
    .await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": false }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert_eq!(
        text, "Session closed.",
        "has_learnings=false should close session; got: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert!(task.tmux_window.is_none());
    assert_eq!(task.status, TaskStatus::Done);
}

#[tokio::test]
async fn exit_session_bare_call_after_has_learnings_true_still_asks() {
    // Regression guard for the stateless model: the server keeps no per-task
    // state, so a bare exit_session call always returns the reflection
    // question — even after a previous has_learnings=true.
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": true }
        })),
    )
    .await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "exit_session", "arguments": { "task_id": task_id.0 } })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("has_learnings"),
        "bare call should always ask for has_learnings; got: {text}"
    );

    let task = state.db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(task.tmux_window.is_some());
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
async fn wrap_up_rebase_does_not_kill_window() {
    let state = test_state();
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
            project_id: ProjectId(1),
        })
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

// -- exit_session: Done transition (added with the wrap_up/exit_session alignment) ---

#[tokio::test]
async fn exit_session_second_call_marks_task_done() {
    // No-epic branch: the closing call must mark the task Done even when
    // there is no epic to recalculate. Pins the `is_some()` guard around
    // recalculate_epic_status.
    let state = test_state();
    let task_id = create_running_task_with_window(&state);

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": false }
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
            "arguments": { "task_id": task_id.0, "has_learnings": false }
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
            "arguments": { "task_id": task_id.0, "has_learnings": false }
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
            "arguments": { "task_id": task_id.0, "has_learnings": false }
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
            "arguments": { "task_id": task_id.0, "has_learnings": false }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    // DB must already be Done by the time the Refresh fires.
    let task = state.db.get_task(task_id).unwrap().unwrap();
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
    });

    let epic = db
        .create_epic("E2E Epic", "", "/repo", None, ProjectId(1))
        .unwrap();
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
            project_id: ProjectId(1),
        })
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

    let close_resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "exit_session",
            "arguments": { "task_id": task_id.0, "has_learnings": false }
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
