use super::*;
use crate::models::{TaskId, WorkPackageId, WorkPackageItem};

// =======================================================================
// Work package tool tests (work-packages.allium, "MCP tools")
// =======================================================================

async fn tool(state: &Arc<McpState>, name: &str, mut arguments: Value) -> JsonRpcResponse {
    // The test store's default workspace, which a package is always created in.
    if name == "work-package_create" && arguments.get("workspace_id").is_none() {
        arguments["workspace_id"] = json!(1);
    }
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
        "work-package_create",
        json!({"title": "Window 1", "link_task_ids": [t.0], "link_epic_ids": [e.0.to_string()]}),
    )
    .await;

    let text = extract_response_text(&resp);
    let id = only_package(&state).await;
    assert!(
        text.contains(&format!("Work package {id} (\"Window 1\") created")),
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
        "work-package_create",
        json!({"title": "W", "link_task_ids": [404]}),
    )
    .await;
    assert_error(&resp, "no task 404");
    assert!(state.db.list_work_packages().await.is_empty());
}

#[tokio::test]
async fn create_work_package_refuses_a_blank_title() {
    let state = test_state().await;
    let resp = tool(&state, "work-package_create", json!({"title": "  "})).await;
    assert_error(&resp, "non-empty title");
}

#[tokio::test]
async fn list_work_packages_shows_each_item_with_status_and_title() {
    let state = test_state().await;
    let t = seed_task(&state, "the task").await;
    tool(
        &state,
        "work-package_create",
        json!({"title": "Window 1", "description": "plan", "link_task_ids": [t.0]}),
    )
    .await;

    let text = extract_response_text(&tool(&state, "work-package_search", json!({})).await);
    assert!(text.contains("Window 1"), "{text}");
    assert!(text.contains("Description: plan"), "{text}");
    assert!(
        text.contains(&format!(
            "- task {} [backlog] {}: the task",
            t.0,
            state.db.get_task(t).await.unwrap().agent_name.unwrap()
        )),
        "{text}"
    );
}

#[tokio::test]
async fn list_work_packages_with_none_says_how_to_make_one() {
    let state = test_state().await;
    let text = extract_response_text(&tool(&state, "work-package_search", json!({})).await);
    assert!(text.contains("work-package_create"), "{text}");
}

#[tokio::test]
async fn list_work_packages_of_an_unknown_id_is_not_found() {
    let state = test_state().await;
    let resp = tool(&state, "work-package_search", json!({"work_package_id": 9})).await;
    assert_error(&resp, "Work package 9 not found");
}

#[tokio::test]
async fn update_work_package_renames_and_swaps_links() {
    let state = test_state().await;
    let a = seed_task(&state, "a").await;
    let b = seed_task(&state, "b").await;
    tool(
        &state,
        "work-package_create",
        json!({"title": "W", "link_task_ids": [a.0]}),
    )
    .await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "work-package_update",
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
async fn update_work_package_names_the_unlinks_that_touched_nothing() {
    let state = test_state().await;
    let a = seed_task(&state, "a").await;
    let b = seed_task(&state, "b").await;
    tool(
        &state,
        "work-package_create",
        json!({"title": "W", "link_task_ids": [a.0]}),
    )
    .await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "work-package_update",
        json!({"work_package_id": id.0, "unlink_task_ids": [a.0, b.0]}),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("updated"), "{text}");
    assert!(
        text.contains(&format!("not linked: {}", task_ref_of(&state, b).await)),
        "{text}"
    );
    assert!(text.starts_with("Work package "), "{text}");
    assert!(text.contains("(\"W\") updated"), "{text}");
    assert!(!text.contains(&task_ref_of(&state, a).await), "{text}");
    assert!(links(&state).await.is_empty());
}

#[tokio::test]
async fn update_work_package_refuses_an_item_in_both_lists() {
    let state = test_state().await;
    let a = seed_task(&state, "a").await;
    tool(&state, "work-package_create", json!({"title": "W"})).await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "work-package_update",
        json!({"work_package_id": id.0, "link_task_ids": [a.0], "unlink_task_ids": [a.0]}),
    )
    .await;
    assert_error(&resp, "both a link and an unlink list");
    assert!(links(&state).await.is_empty());
}

#[tokio::test]
async fn update_work_package_sets_sort_order_and_list_shows_it() {
    let state = test_state().await;
    tool(&state, "work-package_create", json!({"title": "W"})).await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "work-package_update",
        json!({"work_package_id": id.0, "sort_order": 42}),
    )
    .await;

    assert!(extract_response_text(&resp).contains("sort_order"));
    assert_eq!(state.db.list_work_packages().await[0].sort_order, Some(42));
    let listed = tool(&state, "work-package_search", json!({})).await;
    assert!(extract_response_text(&listed).contains("Sort order: 42"));
}

/// work-packages.allium: DeleteWorkPackageViaMcp. work-package_delete takes the
/// package id and nothing else; any other argument is an unknown field and
/// nothing is written.
#[tokio::test]
async fn work_package_delete_refuses_any_other_argument() {
    let state = test_state().await;
    let t = seed_task(&state, "kept").await;
    tool(&state, "work-package_create", json!({"title": "W"})).await;
    let id = only_package(&state).await;
    for extra in [
        json!({"sort_order": 1}),
        json!({"link_task_ids": [t.0]}),
        json!({"title": "Renamed"}),
    ] {
        let mut args = json!({"work_package_id": id.0});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let resp = tool(&state, "work-package_delete", args).await;
        assert!(is_error(&resp) || resp.error.is_some(), "{resp:?}");
    }
    assert_eq!(state.db.list_work_packages().await.len(), 1);
    assert!(links(&state).await.is_empty());
}

#[tokio::test]
async fn update_work_package_with_no_field_is_refused() {
    let state = test_state().await;
    tool(&state, "work-package_create", json!({"title": "W"})).await;
    let id = only_package(&state).await;
    let resp = tool(
        &state,
        "work-package_update",
        json!({"work_package_id": id.0}),
    )
    .await;
    assert_error(&resp, "at least one");
}

#[tokio::test]
async fn work_package_delete_keeps_the_items() {
    let state = test_state().await;
    let t = seed_task(&state, "kept").await;
    tool(
        &state,
        "work-package_create",
        json!({"title": "W", "link_task_ids": [t.0]}),
    )
    .await;
    let id = only_package(&state).await;

    let resp = tool(
        &state,
        "work-package_delete",
        json!({"work_package_id": id.0}),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("unchanged"));
    assert!(
        text.starts_with("Work package ") && text.contains("(\"W\") deleted"),
        "{text}"
    );
    assert!(state.db.list_work_packages().await.is_empty());
    assert!(links(&state).await.is_empty());
    assert!(state.db.get_task(t).await.is_some());
}

#[tokio::test]
async fn work_package_delete_of_an_unknown_id_is_not_found() {
    let state = test_state().await;
    let resp = tool(&state, "work-package_delete", json!({"work_package_id": 9})).await;
    assert_error(&resp, "Work package 9 not found");
}

/// work-packages.allium: UpdateWorkPackageViaMcp. The `delete` flag is gone from
/// work-package_update: it is an unknown field, and nothing is written.
#[tokio::test]
async fn work_package_update_refuses_the_old_delete_flag() {
    let state = test_state().await;
    tool(&state, "work-package_create", json!({"title": "W"})).await;
    let id = only_package(&state).await;
    for delete in [true, false] {
        let resp = tool(
            &state,
            "work-package_update",
            json!({"work_package_id": id.0, "delete": delete, "title": "Renamed"}),
        )
        .await;
        assert!(is_error(&resp) || resp.error.is_some(), "{resp:?}");
    }
    let packages = state.db.list_work_packages().await;
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].title, "W");
}

#[test]
fn work_package_tools_carry_their_access_class() {
    let defs = tool_definitions();
    let hints = |name: &str| tool_def_from(&defs, name)["annotations"].clone();
    assert_eq!(hints("work-package_search")["readOnlyHint"], json!(true));
    assert_eq!(hints("work-package_create")["idempotentHint"], json!(false));
    // Decision 4 of docs/adr/0002: the update tool has no delete form, so it is
    // idempotent and not destructive; the delete is a tool of its own.
    let update = hints("work-package_update");
    assert_eq!(update["destructiveHint"], json!(false));
    assert_eq!(update["idempotentHint"], json!(true));
    let delete = hints("work-package_delete");
    assert_eq!(delete["destructiveHint"], json!(true));
    assert_eq!(delete["readOnlyHint"], json!(false));
}

/// work-packages.allium: DeleteWorkPackageViaMcp. The delete is
/// work-package_delete, takes only the id, and the update schema carries no
/// `delete` property.
#[test]
fn work_package_delete_is_its_own_tool() {
    let defs = tool_definitions();
    let delete = tool_def_from(&defs, "work-package_delete");
    let props = delete["inputSchema"]["properties"].as_object().unwrap();
    assert_eq!(props.keys().collect::<Vec<_>>(), ["work_package_id"]);
    let update = tool_def_from(&defs, "work-package_update");
    assert!(update["inputSchema"]["properties"].get("delete").is_none());
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

// -----------------------------------------------------------------------
// People and display names (workspaces.allium: SetDisplayNameViaMcp,
// CreatorIsShownByName)
// -----------------------------------------------------------------------

#[test]
fn display_name_set_is_a_write_and_idempotent() {
    let defs = tool_definitions();
    let hints = tool_def_from(&defs, "display-name_set")["annotations"].clone();
    assert_eq!(hints["readOnlyHint"], json!(false));
    assert_eq!(hints["destructiveHint"], json!(false));
    assert_eq!(hints["idempotentHint"], json!(true));
}

#[tokio::test]
async fn display_name_set_saves_the_trimmed_name() {
    let state = test_state().await;
    let resp = tool(&state, "display-name_set", json!({"name": "  Alice  "})).await;
    assert!(!is_error(&resp), "{resp:?}");
    assert!(
        format!("{resp:?}").contains("Display name set to Alice"),
        "{resp:?}"
    );
}

#[tokio::test]
async fn display_name_set_refuses_a_blank_name() {
    let state = test_state().await;
    let resp = tool(&state, "display-name_set", json!({"name": "   "})).await;
    assert!(is_error(&resp), "{resp:?}");
}

#[tokio::test]
async fn display_name_set_refuses_a_name_over_the_limit() {
    let state = test_state().await;
    let resp = tool(&state, "display-name_set", json!({"name": "x".repeat(65)})).await;
    assert!(is_error(&resp), "{resp:?}");
}

/// `CreatorIsShownByName`: a package's creator is shown by prefix until they
/// set a name, then by name.
#[tokio::test]
async fn list_work_packages_names_the_creator_by_display_name_or_prefix() {
    let state = test_state().await;
    let resp = tool(&state, "work-package_create", json!({"title": "Window"})).await;
    assert!(!is_error(&resp), "{resp:?}");

    let listed = format!("{:?}", tool(&state, "work-package_search", json!({})).await);
    assert!(
        listed.contains("Created by: test-use"),
        "no name set: the 8-character prefix is shown: {listed}"
    );

    let resp = tool(&state, "display-name_set", json!({"name": "Alice"})).await;
    assert!(!is_error(&resp), "{resp:?}");
    let listed = format!("{:?}", tool(&state, "work-package_search", json!({})).await);
    assert!(listed.contains("Created by: Alice"), "{listed}");
}

// =======================================================================
// Workspace and visibility (work-packages.allium: CreateWorkPackageViaMcp, UpdateWorkPackageViaMcp)
// =======================================================================

fn is_error(resp: &JsonRpcResponse) -> bool {
    resp.result
        .as_ref()
        .and_then(|r| r.get("isError"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[tokio::test]
async fn create_work_package_without_a_workspace_is_refused_and_lists_yours() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "work-package_create", "arguments": {"title": "W"}})),
    )
    .await;
    assert!(is_error(&resp));
    let text = extract_response_text(&resp);
    assert!(text.contains("needs a workspace_id"), "{text}");
    assert!(text.contains("1 (Default)"), "{text}");
    assert!(state.db.list_work_packages().await.is_empty());
}

#[tokio::test]
async fn create_work_package_takes_a_visibility_and_list_shows_it() {
    let state = test_state().await;
    let resp = tool(
        &state,
        "work-package_create",
        json!({"title": "Mine", "visibility": "private"}),
    )
    .await;
    assert!(!is_error(&resp), "{}", extract_response_text(&resp));
    let list = tool(&state, "work-package_search", json!({})).await;
    let text = extract_response_text(&list);
    assert!(text.contains("Workspace: 1"), "{text}");
    assert!(text.contains("Visibility: private"), "{text}");
}

#[tokio::test]
async fn update_work_package_sets_visibility_alone() {
    let state = test_state().await;
    tool(&state, "work-package_create", json!({"title": "P"})).await;
    let id = only_package(&state).await;
    let resp = tool(
        &state,
        "work-package_update",
        json!({"work_package_id": id.0, "visibility": "private"}),
    )
    .await;
    assert!(!is_error(&resp), "{}", extract_response_text(&resp));
    assert_eq!(
        state.db.list_work_packages().await[0].visibility,
        crate::models::Visibility::Private
    );
}

#[tokio::test]
async fn an_unknown_visibility_is_refused_and_names_the_choices() {
    let state = test_state().await;
    let resp = tool(
        &state,
        "work-package_create",
        json!({"title": "P", "visibility": "secret"}),
    )
    .await;
    assert!(is_error(&resp));
    let text = extract_response_text(&resp);
    assert!(
        text.contains("public") && text.contains("private"),
        "{text}"
    );
    assert!(state.db.list_work_packages().await.is_empty());
}

// -- work-package_search time filters (work-packages.allium: ListWorkPackagesViaMcp)

fn wp_hours_from_now(hours: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::hours(hours))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[tokio::test]
async fn list_work_packages_updated_filters_bound_updated_at() {
    let state = test_state().await;
    tool(&state, "work-package_create", json!({"title": "Window 1"})).await;

    let inside = extract_response_text(
        &tool(
            &state,
            "work-package_search",
            json!({"updated_after": wp_hours_from_now(-1), "updated_before": wp_hours_from_now(1)}),
        )
        .await,
    );
    assert!(inside.contains("Window 1"), "{inside}");

    let outside = extract_response_text(
        &tool(
            &state,
            "work-package_search",
            json!({"updated_before": wp_hours_from_now(-1)}),
        )
        .await,
    );
    assert!(!outside.contains("Window 1"), "{outside}");
}

#[tokio::test]
async fn list_work_packages_rejects_a_bad_time_and_a_bad_range() {
    let state = test_state().await;
    let bad_time = tool(
        &state,
        "work-package_search",
        json!({"updated_after": "yesterday"}),
    )
    .await;
    assert_error(&bad_time, "updated_after");

    let bad_range = tool(
        &state,
        "work-package_search",
        json!({"updated_after": "2026-11-16T00:00:00Z", "updated_before": "2026-11-16T00:00:00Z"}),
    )
    .await;
    assert_error(&bad_range, "updated_before");
}

#[tokio::test]
async fn list_work_packages_has_no_completed_filters() {
    let state = test_state().await;
    let resp = tool(
        &state,
        "work-package_search",
        json!({"completed_after": "2026-11-09T00:00:00Z"}),
    )
    .await;
    assert!(is_error(&resp), "a package has no completed_at");
}

#[tokio::test]
async fn list_work_packages_by_id_rejects_the_time_filters() {
    let state = test_state().await;
    tool(&state, "work-package_create", json!({"title": "Window 1"})).await;
    let id = only_package(&state).await;
    let resp = tool(
        &state,
        "work-package_search",
        json!({"work_package_id": id.0, "updated_after": "2026-11-09T00:00:00Z"}),
    )
    .await;
    assert!(
        is_error(&resp),
        "id plus a time filter is invalid params: {resp:?}"
    );
}
