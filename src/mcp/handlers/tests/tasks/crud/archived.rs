//! Soft delete over MCP: `task_search` and `task_search(archived: true)` read
//! archived records, and the write tools refuse an archived id
//! (mcp-task-tools.allium: GetArchivedTaskViaMcp, ListArchivedTasksViaMcp,
//! ArchivedTaskIsReadOnly).

use super::*;
use crate::models::TaskId;

/// Create a task in `repo`, finish it and delete it. Returns its id.
async fn archived_task(state: &Arc<McpState>, title: &str, repo: &str) -> TaskId {
    let id = state
        .db_write()
        .create_task(CreateTaskRequest {
            description: "what it did",
            ..CreateTaskRequest::fixture(title, repo)
        })
        .await
        .unwrap();
    state
        .db_write()
        .patch_task(id, &crate::store::TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    state.db_write().delete_task(id).await.unwrap();
    id
}

async fn tool(state: &Arc<McpState>, name: &str, args: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": name, "arguments": args })),
    )
    .await
}

#[tokio::test]
async fn get_task_reads_an_archived_task() {
    let state = test_state().await;
    let id = archived_task(&state, "Old work", "/a").await;

    let resp = tool(&state, "task_search", json!({ "task_id": id.0 })).await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(text.contains(&format!("Task {}: Old work", id.0)), "{text}");
    assert!(text.contains("Status: archived"), "{text}");
    assert!(text.contains("Archived: "), "{text}");
    assert!(text.contains("what it did"), "{text}");
}

#[tokio::test]
async fn get_task_on_an_id_never_used_is_still_not_found() {
    let state = test_state().await;
    let resp = tool(&state, "task_search", json!({ "task_id": 9999 })).await;
    assert!(is_error(&resp), "{resp:?}");
}

#[tokio::test]
async fn list_tasks_archived_lists_only_archived_records() {
    let state = test_state().await;
    let gone = archived_task(&state, "Gone task", "/a").await;
    state
        .db_write()
        .create_task(CreateTaskRequest::fixture("Live task", "/a"))
        .await
        .unwrap();

    let resp = tool(&state, "task_search", json!({ "archived": true })).await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(text.contains(&format!("#{}: Gone task", gone.0)), "{text}");
    assert!(text.contains("archived "), "{text}");
    assert!(!text.contains("Live task"), "{text}");

    let live = extract_response_text(&tool(&state, "task_search", json!({})).await);
    assert!(
        !live.contains("Gone task"),
        "the default lists live tasks only: {live}"
    );
}

#[tokio::test]
async fn list_tasks_archived_applies_repo_and_query_filters_newest_first() {
    let state = test_state().await;
    let first = archived_task(&state, "Login fix", "/a").await;
    archived_task(&state, "Other repo", "/b").await;
    let second = archived_task(&state, "Login polish", "/a").await;

    let text = extract_response_text(
        &tool(
            &state,
            "task_search",
            json!({ "archived": true, "repo_paths": ["/a"], "query": "login" }),
        )
        .await,
    );
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].starts_with(&format!("#{}:", second.0)),
        "newest first: {text}"
    );
    assert!(lines[1].starts_with(&format!("#{}:", first.0)), "{text}");
}

#[tokio::test]
async fn list_tasks_archived_with_nothing_says_so() {
    let state = test_state().await;
    let text =
        extract_response_text(&tool(&state, "task_search", json!({ "archived": true })).await);
    assert_eq!(text, "No archived tasks match (no filters)");
}

#[tokio::test]
async fn list_tasks_archived_rejects_live_only_filters() {
    let state = test_state().await;
    for extra in [
        json!({ "status": "done" }),
        json!({ "bookmarked": true }),
        json!({ "recursive": true, "epic_id": 1 }),
    ] {
        let mut args = extra.clone();
        args["archived"] = json!(true);
        let resp = tool(&state, "task_search", args).await;
        assert_error(&resp, "archived");
    }
}

#[tokio::test]
async fn write_tools_refuse_an_archived_task_as_archived() {
    let state = test_state().await;
    let id = archived_task(&state, "Done and gone", "/a").await;

    for (name, args, verb) in [
        ("resume_task", json!({ "task_id": id.0 }), "resumed"),
        ("dispatch_task", json!({ "task_id": id.0 }), "dispatched"),
        (
            "update_tasks",
            json!({ "task_ids": [id.0], "title": "x" }),
            "changed",
        ),
    ] {
        let resp = tool(&state, name, args).await;
        let text = if is_error(&resp) {
            error_message(&resp)
        } else {
            extract_response_text(&resp)
        };
        assert!(
            text.contains(&format!("task {} is archived and cannot be {verb}", id.0)),
            "{name}: {text}"
        );
    }
}
