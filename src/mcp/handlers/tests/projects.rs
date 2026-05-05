use super::*;

// ---------------------------------------------------------------------------
// create_task project_id tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_with_project_id_assigns_correctly() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Other", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
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
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
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
    let default_id = db.get_default_project().unwrap().id;
    assert_eq!(epics[0].project_id, default_id);
}

#[tokio::test]
async fn create_epic_with_project_id_assigns_correctly() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let other = db.create_project("Other", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
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
    db.create_project("Dispatch", 1).unwrap();
    db.create_project("wizard_game", 2).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
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
    let other = db.create_project("Dispatch", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let task_id = create_task_fixture(&state);
    let default_id = db.get_default_project().unwrap().id;
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
    let other = db.create_project("Dispatch", 1).unwrap();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let state = Arc::new(McpState {
        db: db.clone(),
        notify_tx: None,
        runner,
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic(
            "Test Epic",
            "",
            "/repo",
            None,
            db.get_default_project().unwrap().id,
        )
        .unwrap();
    let default_id = db.get_default_project().unwrap().id;
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
        exit_session_pending: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    let epic = db
        .create_epic("E", "", "/r", None, db.get_default_project().unwrap().id)
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
