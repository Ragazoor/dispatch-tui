use super::*;

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------

fn default_project_id(state: &Arc<McpState>) -> ProjectId {
    state.db.get_default_project().unwrap().id
}

fn create_task_in_repo(state: &Arc<McpState>, repo: &str) -> crate::models::TaskId {
    let pid = default_project_id(state);
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
        .create_learning(
            crate::models::LearningKind::Convention,
            summary,
            None,
            scope,
            scope_ref,
            &tag_strings,
            None,
        )
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
    let task_id = create_task_in_repo(&state, "/repo/foo");

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
    let task_id = create_task_in_repo(&state, "/repo/bar");

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
    let pid = default_project_id(&state);
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
                "kind": "episodic",
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
    let task_id = create_task_in_repo(&state, "/repo/baz");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "episodic",
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
    let task_id = create_task_in_repo(&state, "/repo/foo");

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
    let task_id = create_task_in_repo(&state, "/repo");

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

// --- record_learning: similar-entries echo -----------------------------------

#[tokio::test]
async fn record_learning_echoes_similar_approved_entries() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo");

    // Pre-seed an approved learning with same (kind=convention, scope=repo, scope_ref=/repo/foo)
    let existing_id = create_approved_learning(
        &state,
        "Always use cargo fmt before committing",
        crate::models::LearningScope::Repo,
        Some("/repo/foo"),
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "Prefer rustfmt over manual formatting",
                "scope": "repo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let text = extract_response_text(&resp);
    assert!(
        text.contains(&existing_id.0.to_string()),
        "expected existing learning id {} in response: {text}",
        existing_id.0
    );
    assert!(
        text.contains("confirm_learning"),
        "expected confirm_learning suggestion in response: {text}"
    );
}

#[tokio::test]
async fn record_learning_no_echo_when_different_kind() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo");

    // Pre-seed an approved convention learning; we will submit a pitfall learning.
    create_approved_learning(
        &state,
        "Watch out for integer overflow",
        crate::models::LearningScope::Repo,
        Some("/repo/foo"),
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "pitfall",
                "summary": "Use checked_add to avoid overflow",
                "scope": "repo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("confirm_learning"),
        "should not suggest confirm_learning when existing entry has a different kind: {text}"
    );
}

#[tokio::test]
async fn record_learning_still_creates_when_similar_exists() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo");

    create_approved_learning(
        &state,
        "Existing approved entry",
        crate::models::LearningScope::Repo,
        Some("/repo/foo"),
        &[],
    );

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "New entry despite similar existing",
                "scope": "repo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let all = state
        .db
        .list_learnings(crate::db::LearningFilter::default())
        .unwrap();
    assert_eq!(
        all.len(),
        2,
        "expected pre-existing + newly created learning"
    );
    let new_one = all
        .iter()
        .find(|l| l.summary == "New entry despite similar existing");
    assert!(new_one.is_some(), "newly created learning must exist");
}

#[tokio::test]
async fn record_learning_does_not_echo_itself() {
    // When no pre-existing similar entry exists, the newly created entry must
    // not be echoed as a "similar" entry (it should exclude itself).
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/foo");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "record_learning",
            "arguments": {
                "task_id": task_id.0,
                "kind": "convention",
                "summary": "A brand new learning with no prior similar entries",
                "scope": "repo"
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("confirm_learning"),
        "should not suggest confirm_learning when no pre-existing similar entries: {text}"
    );
}

// --- query_learnings ---------------------------------------------------------

#[tokio::test]
async fn query_learnings_returns_approved_for_task() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo/myproject");
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
    let task_id = create_task_in_repo(&state, "/repo/tagged");
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
    let task_id = create_task_in_repo(&state, "/repo/limited");
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

// --- confirm_learning --------------------------------------------------------

#[tokio::test]
async fn confirm_learning_increments_count() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo");
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
            "name": "confirm_learning",
            "arguments": { "learning_id": learning_id, "task_id": task_id.0 }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);

    let learning = state.db.get_learning(learning_id).unwrap().unwrap();
    assert_eq!(learning.confirmed_count, 1);
}

#[tokio::test]
async fn confirm_learning_unknown_learning_fails() {
    let state = test_state();
    let task_id = create_task_in_repo(&state, "/repo");

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "confirm_learning",
            "arguments": { "learning_id": 9999, "task_id": task_id.0 }
        })),
    )
    .await;
    assert_error(&resp, "9999");
}
