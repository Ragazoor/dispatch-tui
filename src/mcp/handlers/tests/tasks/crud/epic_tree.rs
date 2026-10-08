use super::*;
use crate::models::{EpicId, TaskId};

// ---------------------------------------------------------------------------
// Walking an epic tree and changing many tasks at once: list_epics
// (parent_epic_id, recursive), list_tasks (recursive, base_branch) and
// update_tasks. See ListEpicsViaMcp in docs/specs/epics.allium and
// ListTasksViaMcp / UpdateTasksViaMcp in docs/specs/mcp-task-tools.allium.
// ---------------------------------------------------------------------------

/// root ─┬─ child ── grandchild
///       └─ sibling
/// plus an unrelated root `other`.
struct Tree {
    root: EpicId,
    child: EpicId,
    grandchild: EpicId,
    sibling: EpicId,
    other: EpicId,
}

async fn make_tree(state: &Arc<McpState>) -> Tree {
    let db = state.db_write();
    let root = db.create_epic("Root", "", None).await.unwrap().id;
    let child = db.create_epic("Child", "", Some(root)).await.unwrap().id;
    let grandchild = db
        .create_epic("Grandchild", "", Some(child))
        .await
        .unwrap()
        .id;
    let sibling = db.create_epic("Sibling", "", Some(root)).await.unwrap().id;
    let other = db.create_epic("Other", "", None).await.unwrap().id;
    Tree {
        root,
        child,
        grandchild,
        sibling,
        other,
    }
}

async fn make_task(
    state: &Arc<McpState>,
    title: &str,
    epic_id: Option<EpicId>,
    status: TaskStatus,
    base_branch: &str,
) -> TaskId {
    state
        .db_write()
        .create_task(CreateTaskRequest {
            title,
            description: "",
            repo_path: "/repo",
            plan: None,
            status,
            base_branch,
            epic_id,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap()
}

async fn tool(state: &Arc<McpState>, name: &str, arguments: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": name, "arguments": arguments })),
    )
    .await
}

fn row(text: &str, id: impl std::fmt::Display) -> Option<&str> {
    let prefix = format!("- [{id}] ");
    text.lines().find(|l| l.starts_with(&prefix))
}

// -- list_epics ---------------------------------------------------------------

#[tokio::test]
async fn list_epics_with_parent_returns_direct_children_only() {
    let state = test_state().await;
    let t = make_tree(&state).await;

    let resp = tool(&state, "list_epics", json!({ "parent_epic_id": t.root.0 })).await;
    let text = extract_response_text(&resp);

    assert!(row(&text, t.child).is_some(), "{text}");
    assert!(row(&text, t.sibling).is_some(), "{text}");
    assert!(row(&text, t.grandchild).is_none(), "{text}");
    assert!(row(&text, t.root).is_none(), "{text}");
    assert!(row(&text, t.other).is_none(), "{text}");
}

#[tokio::test]
async fn list_epics_recursive_returns_every_descendant_but_not_the_parent() {
    let state = test_state().await;
    let t = make_tree(&state).await;

    let resp = tool(
        &state,
        "list_epics",
        json!({ "parent_epic_id": t.root.0, "recursive": true }),
    )
    .await;
    let text = extract_response_text(&resp);

    for e in [t.child, t.grandchild, t.sibling] {
        assert!(row(&text, e).is_some(), "missing {e}: {text}");
    }
    assert!(row(&text, t.root).is_none(), "{text}");
    assert!(row(&text, t.other).is_none(), "{text}");
}

#[tokio::test]
async fn list_epics_rows_name_their_parent() {
    let state = test_state().await;
    let t = make_tree(&state).await;

    let text = extract_response_text(&tool(&state, "list_epics", json!({})).await);

    let grandchild = row(&text, t.grandchild).unwrap();
    assert!(
        grandchild.contains(&format!("(parent:{})", t.child)),
        "{grandchild}"
    );
    let root = row(&text, t.root).unwrap();
    assert!(!root.contains("(parent:"), "{root}");
}

#[tokio::test]
async fn list_epics_recursive_without_parent_is_invalid_params() {
    let state = test_state().await;
    make_tree(&state).await;

    let resp = tool(&state, "list_epics", json!({ "recursive": true })).await;
    assert_error(&resp, "parent_epic_id");
}

#[tokio::test]
async fn list_epics_with_unknown_parent_is_not_found() {
    let state = test_state().await;

    let resp = tool(&state, "list_epics", json!({ "parent_epic_id": 9999 })).await;
    assert!(is_error(&resp), "{:?}", resp.result);
}

#[tokio::test]
async fn list_epics_with_childless_parent_reports_none_found() {
    let state = test_state().await;
    let t = make_tree(&state).await;

    let resp = tool(&state, "list_epics", json!({ "parent_epic_id": t.other.0 })).await;
    assert!(extract_response_text(&resp).contains("No epics found"));
}

// -- list_tasks ---------------------------------------------------------------

#[tokio::test]
async fn list_tasks_recursive_includes_tasks_of_descendant_epics() {
    let state = test_state().await;
    let t = make_tree(&state).await;
    let in_root = make_task(&state, "in root", Some(t.root), TaskStatus::Backlog, "main").await;
    let in_grandchild = make_task(
        &state,
        "in grandchild",
        Some(t.grandchild),
        TaskStatus::Backlog,
        "main",
    )
    .await;
    let in_sibling = make_task(
        &state,
        "in sibling",
        Some(t.sibling),
        TaskStatus::Done,
        "main",
    )
    .await;
    let in_other = make_task(
        &state,
        "in other",
        Some(t.other),
        TaskStatus::Backlog,
        "main",
    )
    .await;

    let resp = tool(
        &state,
        "list_tasks",
        json!({ "epic_id": t.root.0, "recursive": true }),
    )
    .await;
    let text = extract_response_text(&resp);

    for id in [in_root, in_grandchild, in_sibling] {
        assert!(row(&text, id).is_some(), "missing {id}: {text}");
    }
    assert!(row(&text, in_other).is_none(), "{text}");
}

#[tokio::test]
async fn list_tasks_without_recursive_stays_on_the_named_epic() {
    let state = test_state().await;
    let t = make_tree(&state).await;
    let in_root = make_task(&state, "in root", Some(t.root), TaskStatus::Backlog, "main").await;
    let in_child = make_task(
        &state,
        "in child",
        Some(t.child),
        TaskStatus::Backlog,
        "main",
    )
    .await;

    let text =
        extract_response_text(&tool(&state, "list_tasks", json!({ "epic_id": t.root.0 })).await);

    assert!(row(&text, in_root).is_some(), "{text}");
    assert!(row(&text, in_child).is_none(), "{text}");
}

#[tokio::test]
async fn list_tasks_recursive_follows_an_agents_derived_epic_scope() {
    let state = test_state().await;
    let t = make_tree(&state).await;
    let caller = make_task(&state, "caller", Some(t.root), TaskStatus::Running, "main").await;
    let in_child = make_task(
        &state,
        "in child",
        Some(t.child),
        TaskStatus::Backlog,
        "main",
    )
    .await;

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "recursive": true } })),
        CallerIdentity::Task(caller),
    )
    .await;
    let text = extract_response_text(&resp);

    assert!(row(&text, in_child).is_some(), "{text}");
    assert!(row(&text, caller).is_none(), "{text}");
}

#[tokio::test]
async fn list_tasks_recursive_without_epic_scope_is_invalid_params() {
    let state = test_state().await;

    let resp = tool(&state, "list_tasks", json!({ "recursive": true })).await;
    assert_error(&resp, "epic_id");
}

#[tokio::test]
async fn list_tasks_base_branch_filters_by_exact_match() {
    let state = test_state().await;
    let on_main = make_task(&state, "on main", None, TaskStatus::Backlog, "main").await;
    let on_master = make_task(&state, "on master", None, TaskStatus::Backlog, "master").await;

    let text = extract_response_text(
        &tool(&state, "list_tasks", json!({ "base_branch": "master" })).await,
    );

    assert!(row(&text, on_master).is_some(), "{text}");
    assert!(row(&text, on_main).is_none(), "{text}");
}

#[tokio::test]
async fn list_tasks_rows_show_base_branch() {
    let state = test_state().await;
    let id = make_task(&state, "t", None, TaskStatus::Backlog, "develop").await;

    let text = extract_response_text(&tool(&state, "list_tasks", json!({})).await);

    let line = row(&text, id).unwrap();
    assert!(line.contains(" | Base: develop"), "{line}");
}

// -- update_tasks -------------------------------------------------------------

#[tokio::test]
async fn update_tasks_updates_every_named_task_whatever_its_status() {
    let state = test_state().await;
    let backlog = make_task(&state, "b", None, TaskStatus::Backlog, "main").await;
    let running = make_task(&state, "r", None, TaskStatus::Running, "main").await;
    let done = make_task(&state, "d", None, TaskStatus::Done, "main").await;

    let resp = tool(
        &state,
        "update_tasks",
        json!({ "task_ids": [backlog.0, running.0, done.0], "base_branch": "master" }),
    )
    .await;
    let text = extract_response_text(&resp);

    for id in [backlog, running, done] {
        let task = state.db.get_task(id).await.unwrap().unwrap();
        assert_eq!(task.base_branch, "master", "task {id}");
        assert!(
            text.contains(&format!("Task {id} updated (base_branch)")),
            "{text}"
        );
    }
    assert!(text.ends_with("Updated 3 of 3 tasks."), "{text}");
}

#[tokio::test]
async fn update_tasks_reports_a_failure_and_carries_on() {
    let state = test_state().await;
    let first = make_task(&state, "a", None, TaskStatus::Backlog, "main").await;
    let last = make_task(&state, "c", None, TaskStatus::Backlog, "main").await;

    let resp = tool(
        &state,
        "update_tasks",
        json!({ "task_ids": [first.0, 9999, last.0], "base_branch": "master" }),
    )
    .await;
    let text = extract_response_text(&resp);
    let lines: Vec<&str> = text.lines().collect();

    assert!(
        lines[0].starts_with(&format!("Task {first} updated")),
        "{text}"
    );
    assert!(lines[1].starts_with("Task 9999 failed: "), "{text}");
    assert!(
        lines[2].starts_with(&format!("Task {last} updated")),
        "{text}"
    );
    assert_eq!(lines[3], "Updated 2 of 3 tasks.");
    for id in [first, last] {
        assert_eq!(
            state.db.get_task(id).await.unwrap().unwrap().base_branch,
            "master"
        );
    }
}

#[tokio::test]
async fn update_tasks_handles_a_repeated_id_once() {
    let state = test_state().await;
    let id = make_task(&state, "a", None, TaskStatus::Backlog, "main").await;

    let resp = tool(
        &state,
        "update_tasks",
        json!({ "task_ids": [id.0, id.0], "title": "renamed" }),
    )
    .await;
    let text = extract_response_text(&resp);

    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.ends_with("Updated 1 of 1 tasks."), "{text}");
}

#[tokio::test]
async fn update_tasks_applies_update_task_guards_per_task() {
    // status=done must be the only field set — the same guard update_task has.
    let state = test_state().await;
    let id = make_task(&state, "a", None, TaskStatus::Review, "main").await;

    let resp = tool(
        &state,
        "update_tasks",
        json!({ "task_ids": [id.0], "status": "done", "title": "x" }),
    )
    .await;
    let text = extract_response_text(&resp);

    assert!(text.contains(&format!("Task {id} failed: ")), "{text}");
    assert!(text.contains("only field set"), "{text}");
    assert_eq!(
        state.db.get_task(id).await.unwrap().unwrap().status,
        TaskStatus::Review
    );
}

#[tokio::test]
async fn update_tasks_rejects_empty_task_ids() {
    let state = test_state().await;

    let resp = tool(
        &state,
        "update_tasks",
        json!({ "task_ids": [], "base_branch": "master" }),
    )
    .await;
    assert_error(&resp, "task_ids");
}

#[tokio::test]
async fn update_tasks_rejects_a_call_with_no_field() {
    let state = test_state().await;
    let id = make_task(&state, "a", None, TaskStatus::Backlog, "main").await;

    let resp = tool(&state, "update_tasks", json!({ "task_ids": [id.0] })).await;
    assert!(is_error(&resp), "{:?}", resp.result);
}

#[tokio::test]
async fn update_tasks_rejects_task_id() {
    let state = test_state().await;
    let id = make_task(&state, "a", None, TaskStatus::Backlog, "main").await;

    let resp = tool(
        &state,
        "update_tasks",
        json!({ "task_ids": [id.0], "task_id": id.0, "title": "x" }),
    )
    .await;
    assert!(is_error(&resp), "{:?}", resp.result);
    assert_eq!(state.db.get_task(id).await.unwrap().unwrap().title, "a");
}

/// update_tasks advertises exactly update_task's field set, with task_ids in
/// place of task_id, so the two never drift apart.
#[tokio::test]
async fn update_tasks_schema_is_update_tasks_fields_with_task_ids() {
    let state = test_state().await;
    let resp = call(&state, "tools/list", None).await;
    let tools = resp.result.unwrap()["tools"].clone();
    let schema = |name: &str| {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} not listed"))["inputSchema"]
            .clone()
    };
    let keys = |s: &Value| {
        let mut k: Vec<String> = s["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        k.sort();
        k
    };

    let single = schema("update_task");
    let bulk = schema("update_tasks");
    let mut expected: Vec<String> = keys(&single)
        .into_iter()
        .filter(|k| k != "task_id")
        .chain(std::iter::once("task_ids".to_string()))
        .collect();
    expected.sort();

    assert_eq!(keys(&bulk), expected);
    assert_eq!(bulk["required"], json!(["task_ids"]));
}
