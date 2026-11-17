//! `task_search`: one read tool with a `task_id` form (the detail view) and a
//! list form (filters), replacing `get_task` and `list_tasks`
//! (mcp-task-tools.allium: GetTaskViaMcp, GetArchivedTaskViaMcp,
//! ListTasksViaMcp, ListArchivedTasksViaMcp, OldToolNameRedirects).

use super::*;
use crate::models::{EpicId, TaskId};

async fn search(state: &Arc<McpState>, args: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "task_search", "arguments": args })),
    )
    .await
}

async fn search_text(state: &Arc<McpState>, args: Value) -> String {
    let resp = search(state, args).await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    text_of(&resp)
}

async fn make(
    state: &Arc<McpState>,
    title: &str,
    description: &str,
    epic: Option<EpicId>,
) -> TaskId {
    state
        .db_write()
        .create_task(CreateTaskRequest {
            description,
            epic_id: epic,
            ..CreateTaskRequest::fixture(title, "/repo")
        })
        .await
        .unwrap()
}

async fn epic(state: &Arc<McpState>, title: &str) -> EpicId {
    state
        .db_write()
        .create_epic(title, "", None)
        .await
        .unwrap()
        .id
}

/// Finish a task and delete it, which archives it.
async fn archive(state: &Arc<McpState>, id: TaskId) {
    state
        .db_write()
        .patch_task(id, &crate::store::TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    state.db_write().delete_task(id).await.unwrap();
}

fn rows(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("- [")).collect()
}

// -- registration and the old names -------------------------------------------

#[tokio::test]
async fn tools_list_has_task_search_and_neither_old_name() {
    let state = test_state().await;
    let resp = call(&state, "tools/list", None).await;
    let names: Vec<String> = resp.result.unwrap()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"task_search".to_string()), "{names:?}");
    assert!(!names.contains(&"get_task".to_string()), "{names:?}");
    assert!(!names.contains(&"list_tasks".to_string()), "{names:?}");
}

/// OldToolNameRedirects: the real dispatcher, not a local table.
#[tokio::test]
async fn calling_get_task_or_list_tasks_names_task_search() {
    let state = test_state().await;
    for old in ["get_task", "list_tasks"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({ "name": old, "arguments": {} })),
        )
        .await;
        assert!(is_error(&resp), "an unknown tool is refused");
        let msg = error_message(&resp);
        assert!(msg.starts_with(&format!("Unknown tool: {old}")), "{msg}");
        assert!(
            msg.contains(&format!("{old} was renamed: call task_search")),
            "{msg}"
        );
    }
}

#[test]
fn list_tasks_hint_says_epic_id_is_now_explicit() {
    let hint = crate::mcp::handlers::dispatch::OLD_TOOL_NAMES
        .iter()
        .find(|(old, _)| *old == "list_tasks")
        .expect("list_tasks is in the redirect table")
        .1;
    assert!(hint.contains("epic_id") && hint.contains("50"), "{hint}");
}

// -- the task_id form ----------------------------------------------------------

#[tokio::test]
async fn task_id_form_returns_the_detail_view_of_a_live_task() {
    let state = test_state().await;
    let id = make(&state, "Detail me", "body text", None).await;
    let text = search_text(&state, json!({ "task_id": id.0 })).await;
    assert!(text.contains(&format!("Task {id}: Detail me")), "{text}");
    assert!(text.contains("Description: body text"), "{text}");
    assert!(text.contains("Base branch:"), "{text}");
}

#[tokio::test]
async fn task_id_form_accepts_the_callers_own_task() {
    let state = test_state().await;
    let id = make(&state, "Mine", "", None).await;
    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "task_search", "arguments": { "task_id": id.0 } })),
        CallerIdentity::Task(id),
    )
    .await;
    assert!(text_of(&resp).contains("Task "), "{resp:?}");
}

#[tokio::test]
async fn task_id_form_reads_an_archived_task() {
    let state = test_state().await;
    let id = make(&state, "Old work", "what it did", None).await;
    archive(&state, id).await;
    let text = search_text(&state, json!({ "task_id": id.0 })).await;
    assert!(text.contains("Status: archived"), "{text}");
}

#[tokio::test]
async fn task_id_form_shows_the_verify_command_line() {
    let state = test_state().await;
    let id = make(&state, "T", "", None).await;
    state.db_write().save_repo_path("/repo").await.unwrap();
    state
        .db_write()
        .set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    let text = search_text(&state, json!({ "task_id": id.0 })).await;
    assert!(text.contains("Verify command: cargo test"), "{text}");
}

#[tokio::test]
async fn task_id_with_any_other_argument_is_refused_naming_it() {
    let state = test_state().await;
    let id = make(&state, "T", "", None).await;
    for (name, value) in [
        ("epic_id", json!(1)),
        ("status", json!("backlog")),
        ("query", json!("x")),
        ("limit", json!(5)),
        ("repo_paths", json!(["/repo"])),
        ("base_branch", json!("main")),
        ("bookmarked", json!(true)),
        ("recursive", json!(true)),
        ("archived", json!(true)),
        ("updated_after", json!("2026-01-01T00:00:00Z")),
        ("completed_before", json!("2026-01-01T00:00:00Z")),
    ] {
        let resp = search(&state, json!({ "task_id": id.0, name: value })).await;
        assert!(is_error(&resp), "{name} must be refused");
        let msg = error_message(&resp);
        assert!(msg.contains(name), "{name}: {msg}");
    }
}

#[tokio::test]
async fn task_id_with_recursive_or_archived_false_is_not_a_conflict() {
    let state = test_state().await;
    let id = make(&state, "T", "", None).await;
    let text = search_text(
        &state,
        json!({ "task_id": id.0, "recursive": false, "archived": false }),
    )
    .await;
    assert!(text.contains("Task "), "{text}");
}

#[tokio::test]
async fn task_id_accepts_an_id_given_as_a_string() {
    let state = test_state().await;
    let id = make(&state, "T", "", None).await;
    let text = search_text(&state, json!({ "task_id": id.0.to_string() })).await;
    assert!(text.contains(&format!("Task {id}:")), "{text}");
}

// -- no hidden scope -------------------------------------------------------------

#[tokio::test]
async fn a_dispatched_caller_with_no_arguments_sees_other_epics_and_its_own_task() {
    let state = test_state().await;
    let mine = epic(&state, "Mine").await;
    let other = epic(&state, "Other").await;
    let me = make(&state, "caller task", "", Some(mine)).await;
    make(&state, "sibling", "", Some(mine)).await;
    make(&state, "foreign", "", Some(other)).await;
    make(&state, "loose", "", None).await;
    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "task_search", "arguments": {} })),
        CallerIdentity::Task(me),
    )
    .await;
    let text = text_of(&resp);
    for title in ["caller task", "sibling", "foreign", "loose"] {
        assert!(text.contains(title), "{title} missing: {text}");
    }
}

#[tokio::test]
async fn an_explicit_epic_id_scopes_for_a_dispatched_caller_and_keeps_its_own_task() {
    let state = test_state().await;
    let mine = epic(&state, "Mine").await;
    let other = epic(&state, "Other").await;
    let me = make(&state, "caller task", "", Some(mine)).await;
    make(&state, "foreign", "", Some(other)).await;
    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "task_search", "arguments": { "epic_id": mine.0 } })),
        CallerIdentity::Task(me),
    )
    .await;
    let text = text_of(&resp);
    assert!(
        text.contains("caller task") && !text.contains("foreign"),
        "{text}"
    );
}

#[tokio::test]
async fn recursive_without_epic_id_is_refused_even_for_a_dispatched_caller() {
    let state = test_state().await;
    let mine = epic(&state, "Mine").await;
    let me = make(&state, "caller task", "", Some(mine)).await;
    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "task_search", "arguments": { "recursive": true } })),
        CallerIdentity::Task(me),
    )
    .await;
    assert!(is_error(&resp), "recursive without epic_id is refused");
    assert!(error_message(&resp).contains("epic_id"));
}

// -- limit, order and rows ---------------------------------------------------------

#[tokio::test]
async fn the_list_form_returns_50_rows_by_default_and_says_how_to_narrow() {
    let state = test_state().await;
    for i in 0..51 {
        make(&state, &format!("task {i}"), "", None).await;
    }
    let text = search_text(&state, json!({})).await;
    assert_eq!(rows(&text).len(), 50, "{text}");
    let cut = text.lines().last().unwrap();
    assert!(cut.starts_with("Showing 50 of 51"), "{cut}");
    assert!(cut.contains("most recently updated first"), "{cut}");
    assert!(cut.contains("epic_id") && cut.contains("limit"), "{cut}");
}

#[tokio::test]
async fn a_raised_limit_returns_every_match_with_no_cut_line() {
    let state = test_state().await;
    for i in 0..51 {
        make(&state, &format!("task {i}"), "", None).await;
    }
    let text = search_text(&state, json!({ "limit": 100 })).await;
    assert_eq!(rows(&text).len(), 51, "{text}");
    assert!(!text.contains("Showing"), "{text}");
}

#[tokio::test]
async fn the_archived_list_has_the_same_default_limit() {
    let state = test_state().await;
    for i in 0..51 {
        let id = make(&state, &format!("gone {i}"), "", None).await;
        archive(&state, id).await;
    }
    let text = search_text(&state, json!({ "archived": true })).await;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 51, "{text}");
    assert!(lines[50].starts_with("Showing 50 of 51"), "{}", lines[50]);
}

/// A state over an in-memory store whose clock only moves when told to, so
/// two updates are provably a second apart.
fn state_with_clock() -> (Arc<McpState>, crate::clock::FixedClock) {
    use chrono::TimeZone;
    const HOST: &str = "search-host";
    let rows = Arc::new(crate::sync::SharedRows::new());
    let clock =
        crate::clock::FixedClock::new(chrono::Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap());
    let caller = Arc::new(
        crate::sync::MemoryReducerCaller::new(rows.clone(), Arc::new(clock.clone()))
            .with_sender("test-user")
            .with_registered_host(HOST, "test-user"),
    );
    let identity = Arc::new(crate::sync::SettledIdentity::default());
    identity.settle("test-user");
    let host_dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::new(
        rows,
        caller,
        identity,
        Arc::new(clock.clone()),
        HOST.to_string(),
        host_dir.path(),
    ));
    let (state, _db) = state_over(
        db,
        Arc::new(MockProcessRunner::new(vec![])),
        None,
        None,
        None,
    );
    (state, clock)
}

#[tokio::test]
async fn rows_are_sorted_most_recently_updated_first() {
    let (state, clock) = state_with_clock();
    let a = make(&state, "alpha", "", None).await;
    clock.advance(chrono::Duration::seconds(10));
    let b = make(&state, "bravo", "", None).await;
    clock.advance(chrono::Duration::seconds(10));
    let c = make(&state, "charlie", "", None).await;
    clock.advance(chrono::Duration::seconds(10));
    state
        .db_write()
        .patch_task(a, &crate::store::TaskPatch::new().title("alpha2"))
        .await
        .unwrap();
    let text = search_text(&state, json!({})).await;
    assert_eq!(ids(&text), vec![a, c, b], "{text}");
}

#[tokio::test]
async fn rows_updated_at_the_same_time_sort_by_id_descending() {
    let state = test_state().await;
    let a = make(&state, "alpha", "", None).await;
    let b = make(&state, "bravo", "", None).await;
    let c = make(&state, "charlie", "", None).await;
    let text = search_text(&state, json!({})).await;
    assert_eq!(ids(&text), vec![c, b, a], "{text}");
}

fn ids(text: &str) -> Vec<TaskId> {
    rows(text)
        .iter()
        .map(|r| {
            let id = r.trim_start_matches("- [").split(']').next().unwrap();
            TaskId(id.parse().unwrap())
        })
        .collect()
}

#[tokio::test]
async fn a_row_is_one_line_with_the_goal_collapsed_and_cut_to_80_characters() {
    let state = test_state().await;
    let long = format!("first line\n\nsecond   line\r\n{}", "x".repeat(200));
    make(&state, "multi", &long, None).await;
    let text = search_text(&state, json!({})).await;
    assert_eq!(text.lines().count(), 1, "{text}");
    let goal = text.split("| Goal: ").nth(1).expect("goal part");
    assert!(goal.starts_with("first line second line xxx"), "{goal}");
    assert_eq!(goal.chars().count(), 80 + 3, "{goal}");
    assert!(goal.ends_with("..."), "{goal}");
}

#[tokio::test]
async fn a_short_goal_is_not_cut() {
    let state = test_state().await;
    make(&state, "short", "tiny goal", None).await;
    let text = search_text(&state, json!({})).await;
    assert!(text.ends_with("| Goal: tiny goal"), "{text}");
}

#[tokio::test]
async fn an_archived_row_is_one_line_with_a_cut_goal() {
    let state = test_state().await;
    let id = make(&state, "gone", &format!("a\nb {}", "y".repeat(200)), None).await;
    archive(&state, id).await;
    let text = search_text(&state, json!({ "archived": true })).await;
    assert_eq!(text.lines().count(), 1, "{text}");
    let goal = text.split("| Goal: ").nth(1).unwrap();
    assert!(
        goal.starts_with("a b yyy") && goal.ends_with("..."),
        "{goal}"
    );
    assert_eq!(goal.chars().count(), 83, "{goal}");
}

// -- the empty reply names its filters ----------------------------------------------

#[tokio::test]
async fn an_empty_result_names_the_filters_it_applied() {
    let state = test_state().await;
    let text = search_text(&state, json!({ "epic_id": 319, "status": ["backlog"] })).await;
    assert_eq!(text, "No tasks match epic_id=319, status=[backlog]");
}

#[tokio::test]
async fn an_empty_result_with_no_filters_says_so() {
    let state = test_state().await;
    let text = search_text(&state, json!({})).await;
    assert_eq!(text, "No tasks match (no filters)");
}

#[tokio::test]
async fn an_empty_result_names_query_repo_paths_and_time_filters() {
    let state = test_state().await;
    let text = search_text(
        &state,
        json!({
            "query": "nothing here",
            "repo_paths": ["/a", "/b"],
            "base_branch": "dev",
            "bookmarked": true,
            "updated_after": "2026-01-01T00:00:00Z",
        }),
    )
    .await;
    assert_eq!(
        text,
        "No tasks match repo_paths=[/a, /b], base_branch=dev, query=\"nothing here\", \
         bookmarked=true, updated_after=2026-01-01T00:00:00Z"
    );
}

#[tokio::test]
async fn an_empty_archived_result_names_the_filters_too() {
    let state = test_state().await;
    let text = search_text(&state, json!({ "archived": true, "epic_id": 7 })).await;
    assert_eq!(text, "No archived tasks match epic_id=7");
    let text = search_text(&state, json!({ "archived": true })).await;
    assert_eq!(text, "No archived tasks match (no filters)");
}
