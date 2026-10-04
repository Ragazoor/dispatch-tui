use super::*;

// ---------------------------------------------------------------------------
// Mock service injection — demonstrates the TaskServiceApi seam
// ---------------------------------------------------------------------------

/// A minimal mock that satisfies `TaskServiceApi` without a database.
///
/// Only `list_tasks` is mocked; every other method inherits the panicking
/// default from `TaskServiceApiStub`, so an unexpected call fails loudly and
/// adding a method to the seam does not touch this file. See the emitter docs
/// in `src/service/api.rs`.
struct MockTaskService {
    tasks: Vec<crate::models::Task>,
}

#[async_trait::async_trait]
impl crate::service::TaskServiceApiStub for MockTaskService {
    async fn list_tasks(
        &self,
        _filter: crate::service::ListTasksFilter,
    ) -> Result<Vec<crate::models::Task>, crate::service::ServiceError> {
        Ok(self.tasks.clone())
    }
}

crate::task_service_api!(service_api_stub_bridge, MockTaskService);

fn mock_task(id: i64, title: &str) -> crate::models::Task {
    crate::models::Task {
        id: crate::models::TaskId(id),
        title: title.to_string(),
        description: "mock description".to_string(),
        repo_path: "/mock/repo".to_string(),
        ..Default::default()
    }
}

/// Constructs McpState with `task_svc` injected — the mock-service seam.
async fn state_with_mock_task_svc(
    task_svc: Arc<dyn crate::service::TaskServiceApi>,
) -> Arc<McpState> {
    test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        None,
        Some(task_svc),
    )
    .await
    .0
}

/// `list_tasks` returns whatever the service layer provides, independently of
/// what is stored in the DB. This test proves the handler calls `task_svc`,
/// not a raw DB query — and that the seam is injectable in unit tests.
#[tokio::test]
async fn list_tasks_uses_service_not_db_directly() {
    let mock_svc = Arc::new(MockTaskService {
        tasks: vec![mock_task(101, "Alpha task"), mock_task(102, "Beta task")],
    });
    let state = state_with_mock_task_svc(mock_svc).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let text = extract_response_text(&resp);
    // Both mock tasks appear in the response; neither was in the database.
    assert!(text.contains("Alpha task"), "expected mock task in: {text}");
    assert!(text.contains("Beta task"), "expected mock task in: {text}");
}

#[tokio::test]
async fn get_task_accepts_string_task_id() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
    assert_eq!(task.plan_path.as_deref(), Some("/path/to/plan.md"));
}

#[tokio::test]
async fn update_task_title_only() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.title, "New Title");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog); // unchanged
}

#[tokio::test]
async fn update_task_status_optional() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.title, "Renamed");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
}

#[tokio::test]
async fn update_task_title_and_description() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.title, "New");
    assert_eq!(task.description, "new desc");
}

#[tokio::test]
async fn update_task_repo_path() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.repo_path, "/new/repo");
    assert_eq!(task.status, crate::models::TaskStatus::Backlog); // unchanged
}

#[tokio::test]
async fn update_task_no_fields_errors() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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
    assert!(is_error(&resp), "should error with no fields to update");
}

#[tokio::test]
async fn patch_task_sets_multiple_fields() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.title, "Updated Title");
}

#[tokio::test]
async fn update_task_without_plan_preserves_existing() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.plan_path.as_deref(),
        Some("/existing.md"),
        "plan should be preserved when not provided"
    );
}

#[tokio::test]
async fn update_task_sets_pr_fields() {
    let state = test_state().await;
    let task_id = state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://github.com/org/repo/pull/99",
                "url_type": "pr"
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "Expected success, got: {:?}",
        resp.error
    );

    let updated = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        updated.url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/99")
    );
    assert_eq!(
        updated.url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Pr)
    );
}

#[tokio::test]
async fn update_task_rejects_unknown_url_type() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://x/y",
                "url_type": "bogus"
            }
        })),
    )
    .await;
    assert_error(&resp, "url_type");
}

/// `url_type` is parsed into its enum at the JSON-RPC boundary, like `status`,
/// `tag` and `sub_status` — so a bad literal is rejected on its own, not only
/// when a `url` happens to accompany it. Previously it was carried inward as a
/// `String` and only validated on the url-setting path, where a url-less call
/// with a typo'd `url_type` succeeded silently.
#[tokio::test]
async fn update_task_rejects_unknown_url_type_even_without_a_url() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "title": "t",
                "url_type": "bogus"
            }
        })),
    )
    .await;
    assert_error(&resp, "url_type");
}

#[tokio::test]
async fn update_task_url_without_type_is_rejected() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://x/y"
            }
        })),
    )
    .await;
    assert_error(&resp, "url_type");
}

// -- wrap_up_mode tests -----------------------------------------------------

#[tokio::test]
async fn update_task_sets_wrap_up_mode() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "wrap_up_mode": "rebase" }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "got error: {:?}", resp.error);

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.wrap_up_mode, Some(crate::models::WrapUpMode::Rebase));
}

#[tokio::test]
async fn update_task_wrap_up_mode_all_variants() {
    use crate::models::WrapUpMode;
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    for (input, expected) in [
        ("rebase", WrapUpMode::Rebase),
        ("pr", WrapUpMode::Pr),
        ("done", WrapUpMode::Done),
    ] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "wrap_up_mode": input }
            })),
        )
        .await;
        assert!(
            resp.error.is_none(),
            "wrap_up_mode={input} should succeed, got: {:?}",
            resp.error
        );
        let task = state.db.get_task(task_id).await.unwrap().unwrap();
        assert_eq!(
            task.wrap_up_mode,
            Some(expected),
            "wrap_up_mode should be {expected:?} after setting to {input}"
        );
    }
}

#[tokio::test]
async fn update_task_clears_wrap_up_mode_with_null() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    // First set a mode
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "wrap_up_mode": "pr" }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    // Now clear it with null
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "wrap_up_mode": null }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "clearing wrap_up_mode with null should succeed: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert!(
        task.wrap_up_mode.is_none(),
        "wrap_up_mode should be cleared after null"
    );
}

#[tokio::test]
async fn update_task_rejects_invalid_wrap_up_mode() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "wrap_up_mode": "teleport" }
        })),
    )
    .await;
    assert!(is_error(&resp), "invalid wrap_up_mode should error");
}

#[tokio::test]
async fn create_task_with_wrap_up_mode() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Task with mode",
                "repo_path": "/repo",
                "epic_id": null,
                "wrap_up_mode": "pr"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "got error: {:?}", resp.error);

    let task_id = extract_created_task_id(&resp);
    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.wrap_up_mode, Some(crate::models::WrapUpMode::Pr));
}

#[tokio::test]
async fn create_task_with_auto_run_plan_true() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "T",
                "repo_path": "/r",
                "epic_id": null,
                "auto_run_plan": true
            }
        })),
    )
    .await;
    assert!(!is_error(&resp));

    let tasks = state.db.list_all().await.unwrap();
    let task = tasks
        .iter()
        .find(|t| t.title == "T")
        .expect("task should exist");
    assert!(task.auto_run_plan);
}

#[tokio::test]
async fn update_task_sets_auto_run_plan() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "auto_run_plan": true }
        })),
    )
    .await;
    assert!(!is_error(&resp));

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert!(task.auto_run_plan);
}

#[tokio::test]
async fn get_task_shows_wrap_up_mode() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    // Set wrap_up_mode
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "wrap_up_mode": "rebase" }
        })),
    )
    .await;

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
    assert!(
        text.contains("rebase"),
        "get_task should show wrap_up_mode: {text}"
    );
}

// -- list_tasks tests -------------------------------------------------------

#[tokio::test]
async fn list_tasks_returns_all_when_no_filter() {
    let state = test_state().await;
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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
    let state = test_state().await;
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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
    let state = test_state().await;
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    state
        .db_write()
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
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
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
    let state = test_state().await;

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

// =======================================================================
// Additional edge case tests
// =======================================================================

#[tokio::test]
async fn list_tasks_invalid_status_string() {
    let state = test_state().await;
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
    let state = test_state().await;
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
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "status": 42 } })),
    )
    .await;
    assert_error(&resp, "expected a status string");
}

#[tokio::test]
async fn create_task_with_epic_id() {
    let state = test_state().await;
    let epic = state
        .db_write()
        .create_epic("Parent Epic", "", None)
        .await
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
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    let subtasks = state.db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].title, "Epic Child");
}

#[tokio::test]
async fn create_task_with_string_epic_id() {
    let state = test_state().await;
    let epic = state
        .db_write()
        .create_epic("Parent", "", None)
        .await
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
            }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "should accept string epic_id: {:?}",
        resp.error
    );

    let subtasks = state.db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(subtasks.len(), 1);
}
