#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::json;

/// Parse "count": N from the embedded JSON string in a search_docs response.
fn parse_count(text: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("count")?.as_u64()
}

// --- index_repo ----------------------------------------------------------

#[tokio::test]
async fn index_repo_with_explicit_path_succeeds() {
    let state = test_state().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note\n\nContent.").unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "index_repo",
            "arguments": {
                "task_id": 1,
                "repo_path": dir.path().to_str().unwrap()
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "unexpected error: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);
    assert!(text.contains("files_indexed"), "got: {text}");
}

#[tokio::test]
async fn index_repo_without_repo_path_uses_task_repo() {
    let state = test_state().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note\n\nContent.").unwrap();

    let task_id = create_task_fixture_at(&state, dir.path().to_str().unwrap()).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "index_repo",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "unexpected error: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);
    assert!(text.contains("files_indexed: 1"), "got: {text}");
}

#[tokio::test]
async fn index_repo_missing_task_and_no_repo_path_returns_error() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "index_repo",
            "arguments": { "task_id": 99999 }
        })),
    )
    .await;
    assert!(is_error(&resp));
}

#[tokio::test]
async fn index_repo_empty_directory_succeeds_with_zero_files() {
    let state = test_state().await;
    let dir = tempfile::tempdir().unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "index_repo",
            "arguments": {
                "task_id": 1,
                "repo_path": dir.path().to_str().unwrap()
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "unexpected error: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);
    assert!(text.contains("files_indexed: 0"), "got: {text}");
}

// --- search_docs ---------------------------------------------------------

#[tokio::test]
async fn search_docs_on_unindexed_repo_returns_empty() {
    let state = test_state().await;
    let dir = tempfile::tempdir().unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "search_docs",
            "arguments": {
                "task_id": 1,
                "query": "anything",
                "repo_path": dir.path().to_str().unwrap()
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "unexpected error: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);
    assert_eq!(
        parse_count(&text),
        Some(0),
        "expected 0 results, got: {text}"
    );
}

#[tokio::test]
async fn search_docs_after_indexing_returns_results() {
    let state = test_state().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("note.md"),
        "# Note\n\nContent about escalation patterns.",
    )
    .unwrap();

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "index_repo",
            "arguments": {
                "task_id": 1,
                "repo_path": dir.path().to_str().unwrap()
            }
        })),
    )
    .await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "search_docs",
            "arguments": {
                "task_id": 1,
                "query": "escalation",
                "repo_path": dir.path().to_str().unwrap()
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "unexpected error: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);
    assert!(
        parse_count(&text).is_some_and(|n| n > 0),
        "expected at least one result, got: {text}"
    );
}

#[tokio::test]
async fn search_docs_without_repo_path_uses_task_repo() {
    let state = test_state().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note\n\nSome content.").unwrap();

    let task_id = create_task_fixture_at(&state, dir.path().to_str().unwrap()).await;

    // Index using explicit path so rag.db exists
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "index_repo",
            "arguments": {
                "task_id": task_id.0,
                "repo_path": dir.path().to_str().unwrap()
            }
        })),
    )
    .await;

    // Search using only task_id (no repo_path)
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "search_docs",
            "arguments": {
                "task_id": task_id.0,
                "query": "content"
            }
        })),
    )
    .await;

    assert!(
        !is_error(&resp),
        "unexpected error: {}",
        error_message(&resp)
    );
    let text = extract_response_text(&resp);
    assert!(
        parse_count(&text).is_some_and(|n| n > 0),
        "expected results via task repo_path, got: {text}"
    );
}

#[tokio::test]
async fn search_docs_missing_task_and_no_repo_path_returns_error() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "search_docs",
            "arguments": {
                "task_id": 99999,
                "query": "anything"
            }
        })),
    )
    .await;
    assert!(is_error(&resp));
}
