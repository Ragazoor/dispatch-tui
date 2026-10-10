use super::*;
use crate::models::{TaskId, WorkPackageId, WorkPackageItem};

// =======================================================================
// Work package tool tests (work-packages.allium, "MCP tools")
// =======================================================================

async fn tool(state: &Arc<McpState>, name: &str, arguments: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": name, "arguments": arguments })),
    )
    .await
}

async fn seed_task(state: &McpState, title: &str) -> TaskId {
    state
        .db_write()
        .create_task(store::CreateTaskRequest::fixture(title, "/repo"))
        .await
        .unwrap()
}

async fn seed_epic(state: &McpState, title: &str) -> crate::models::EpicId {
    state
        .db_write()
        .create_epic(title, "", None)
        .await
        .unwrap()
        .id
}

async fn only_package(state: &McpState) -> WorkPackageId {
    let packages = state.db.list_work_packages().await;
    assert_eq!(packages.len(), 1, "{packages:?}");
    packages[0].id
}

async fn links(state: &McpState) -> Vec<WorkPackageItem> {
    state
        .db
        .list_work_package_links()
        .await
        .into_iter()
        .map(|l| l.item)
        .collect()
}

#[tokio::test]
async fn create_work_package_links_the_named_items() {
    let state = test_state().await;
    let t = seed_task(&state, "migrate db-a").await;
    let e = seed_epic(&state, "db-b").await;

    let resp = tool(
        &state,
        "create_work_package",
        json!({"title": "Window 1", "link_task_ids": [t.0], "link_epic_ids": [e.0.to_string()]}),
    )
    .await;

    let text = extract_response_text(&resp);
    let id = only_package(&state).await;
    assert!(
        text.contains(&format!("Work package {id} created")),
        "{text}"
    );
    let l = links(&state).await;
    assert!(l.contains(&WorkPackageItem::Task(t)) && l.contains(&WorkPackageItem::Epic(e)));
}

#[tokio::test]
async fn create_work_package_with_an_unknown_item_creates_nothing() {
    let state = test_state().await;
    let resp = tool(
        &state,
        "create_work_package",
        json!({"title": "W", "link_task_ids": [404]}),
    )
    .await;
    assert_error(&resp, "Task 404 not found");
    assert!(state.db.list_work_packages().await.is_empty());
}

#[tokio::test]
async fn create_work_package_refuses_a_blank_title() {
    let state = test_state().await;
    let resp = tool(&state, "create_work_package", json!({"title": "  "})).await;
    assert_error(&resp, "non-empty title");
}

#[tokio::test]
async fn list_work_packages_shows_each_item_with_status_and_title() {
    let state = test_state().await;
    let t = seed_task(&state, "the task").await;
    tool(
        &state,
        "create_work_package",
        json!({"title": "Window 1", "description": "plan", "link_task_ids": [t.0]}),
    )
    .await;

    let text = extract_response_text(&tool(&state, "list_work_packages", json!({})).await);
    assert!(text.contains("Window 1"), "{text}");
    assert!(text.contains("Description: plan"), "{text}");
    assert!(
        text.contains(&format!("- task {} [backlog] the task", t.0)),
        "{text}"
    );
}

#[tokio::test]
async fn list_work_packages_with_none_says_how_to_make_one() {
    let state = test_state().await;
    let text = extract_response_text(&tool(&state, "list_work_packages", json!({})).await);
    assert!(text.contains("create_work_package"), "{text}");
}

#[tokio::test]
async fn list_work_packages_of_an_unknown_id_is_not_found() {
    let state = test_state().await;
    let resp = tool(&state, "list_work_packages", json!({"work_package_id": 9})).await;
    assert_error(&resp, "Work package 9 not found");
}

#[tokio::test]
async fn update_work_package_renames_and_swaps_links() {
    let state = test_state().await;
    let a = seed_task(&state, "a").await;
    let b = seed_task(&state, "b").await;
    tool(
        &state,
        "create_work_package",
        json!({"title": "W", "link_task_ids": [a.0]}),
    )
    .await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": id.0, "title": "Window 2", "unlink_task_ids": [a.0], "link_task_ids": [b.0]}),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("title") && text.contains("link_task_ids"),
        "{text}"
    );
    assert_eq!(state.db.list_work_packages().await[0].title, "Window 2");
    assert_eq!(links(&state).await, vec![WorkPackageItem::Task(b)]);
}

#[tokio::test]
async fn update_work_package_refuses_an_item_in_both_lists() {
    let state = test_state().await;
    let a = seed_task(&state, "a").await;
    tool(&state, "create_work_package", json!({"title": "W"})).await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": id.0, "link_task_ids": [a.0], "unlink_task_ids": [a.0]}),
    )
    .await;
    assert_error(&resp, "both a link and an unlink list");
    assert!(links(&state).await.is_empty());
}

#[tokio::test]
async fn update_work_package_with_no_field_is_refused() {
    let state = test_state().await;
    tool(&state, "create_work_package", json!({"title": "W"})).await;
    let id = only_package(&state).await;
    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": id.0}),
    )
    .await;
    assert_error(&resp, "at least one");
}

#[tokio::test]
async fn delete_work_package_keeps_the_items() {
    let state = test_state().await;
    let t = seed_task(&state, "kept").await;
    tool(
        &state,
        "create_work_package",
        json!({"title": "W", "link_task_ids": [t.0]}),
    )
    .await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": id.0, "delete": true}),
    )
    .await;

    assert!(extract_response_text(&resp).contains("unchanged"));
    assert!(state.db.list_work_packages().await.is_empty());
    assert!(links(&state).await.is_empty());
    assert!(state.db.get_task(t).await.is_some());
}

#[tokio::test]
async fn delete_work_package_of_an_unknown_id_is_not_found() {
    let state = test_state().await;
    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": 9, "delete": true}),
    )
    .await;
    assert_error(&resp, "Work package 9 not found");
}

/// work-packages.allium: DeleteWorkPackageViaMcp. A delete is a call of its
/// own: mixed with a rename or a relink it is refused, and nothing is written.
#[tokio::test]
async fn delete_with_another_field_is_refused_and_writes_nothing() {
    let state = test_state().await;
    let t = seed_task(&state, "kept").await;
    tool(&state, "create_work_package", json!({"title": "W"})).await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": id.0, "delete": true, "link_task_ids": [t.0]}),
    )
    .await;

    assert_error(&resp, "delete");
    assert_eq!(state.db.list_work_packages().await.len(), 1);
    assert!(links(&state).await.is_empty());
}

#[tokio::test]
async fn delete_false_is_the_same_as_leaving_it_out() {
    let state = test_state().await;
    tool(&state, "create_work_package", json!({"title": "W"})).await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "update_work_package",
        json!({"work_package_id": id.0, "delete": false, "title": "Renamed"}),
    )
    .await;

    assert!(!is_error(&resp), "{resp:?}");
    let packages = state.db.list_work_packages().await;
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].title, "Renamed");
}

#[test]
fn work_package_tools_carry_their_access_class() {
    let defs = tool_definitions();
    let hints = |name: &str| tool_def_from(&defs, name)["annotations"].clone();
    assert_eq!(hints("list_work_packages")["readOnlyHint"], json!(true));
    assert_eq!(hints("update_work_package")["destructiveHint"], json!(true));
    assert_eq!(hints("create_work_package")["idempotentHint"], json!(false));
}

/// work-packages.allium: a delete is a field of update_work_package, so
/// there is no delete_work_package tool.
#[test]
fn there_is_no_delete_work_package_tool() {
    let defs = tool_definitions();
    assert!(!defs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "delete_work_package"));
}

fn tool_def_from(defs: &Value, name: &str) -> Value {
    defs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no tool {name}"))
}
