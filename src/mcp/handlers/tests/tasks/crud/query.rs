//! `list_tasks` `query`: a text search over title and description
//! (ListTasksViaMcp, docs/specs/mcp-task-tools.allium).

use super::*;

async fn seed(state: &Arc<McpState>) {
    for (title, description, repo) in [
        ("Fix Login Bug", "the form crashes", "/a"),
        ("Add search", "full text login lookup", "/b"),
        ("Unrelated", "nothing here", "/a"),
    ] {
        state
            .db_write()
            .create_task(CreateTaskRequest {
                description,
                ..CreateTaskRequest::fixture(title, repo)
            })
            .await
            .unwrap();
    }
}

async fn list(state: &Arc<McpState>, args: Value) -> String {
    let resp = call(
        state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": args })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    extract_response_text(&resp)
}

#[tokio::test]
async fn query_matches_title_or_description_case_insensitively() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "query": "LOGIN" })).await;
    assert!(text.contains("Fix Login Bug"), "{text}");
    assert!(text.contains("Add search"), "{text}");
    assert!(!text.contains("Unrelated"), "{text}");
}

#[tokio::test]
async fn query_words_must_all_match_in_any_order_and_field() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "query": "crashes login" })).await;
    assert!(text.contains("Fix Login Bug"), "{text}");
    assert!(!text.contains("Add search"), "{text}");
}

#[tokio::test]
async fn query_is_a_substring_match() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "query": "ogi" })).await;
    assert!(text.contains("Fix Login Bug"), "{text}");
}

#[tokio::test]
async fn blank_query_is_the_same_as_none() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "query": "   " })).await;
    assert_eq!(text.lines().count(), 3, "{text}");
}

#[tokio::test]
async fn query_is_anded_with_other_filters() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "query": "login", "repo_paths": ["/b"] })).await;
    assert!(text.contains("Add search"), "{text}");
    assert!(!text.contains("Fix Login Bug"), "{text}");
}

#[tokio::test]
async fn query_with_no_match_says_no_tasks_found() {
    let state = test_state().await;
    seed(&state).await;
    assert_eq!(
        list(&state, json!({ "query": "zzzz" })).await,
        "No tasks found"
    );
}

#[tokio::test]
async fn limit_caps_rows_and_says_how_many_were_cut() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "limit": 2 })).await;
    assert_eq!(text.lines().count(), 3, "{text}");
    assert_eq!(
        text.lines().last().unwrap(),
        "Showing 2 of 3. Narrow your query or filters."
    );
}

#[tokio::test]
async fn limit_not_exceeded_adds_no_hint() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "limit": 3 })).await;
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(!text.contains("Showing"), "{text}");
}

#[tokio::test]
async fn limit_applies_after_the_query() {
    let state = test_state().await;
    seed(&state).await;
    let text = list(&state, json!({ "query": "login", "limit": 1 })).await;
    assert!(
        text.ends_with("Showing 1 of 2. Narrow your query or filters."),
        "{text}"
    );
}

#[tokio::test]
async fn zero_limit_is_invalid_params() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": { "limit": 0 } })),
    )
    .await;
    let rejected = resp.error.is_some()
        || resp
            .result
            .as_ref()
            .is_some_and(|r| r["isError"] == json!(true));
    assert!(rejected, "limit 0 must be rejected: {resp:?}");
}
