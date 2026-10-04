//! `list_keybindings` (`ListKeybindingsViaMcp` in
//! `docs/specs/keybindings.allium`).
#![allow(clippy::unwrap_used)]
use super::*;

async fn list(state: &Arc<McpState>, args: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "list_keybindings", "arguments": args })),
    )
    .await
}

fn groups(resp: &JsonRpcResponse) -> Vec<Value> {
    let text = resp.result.as_ref().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    serde_json::from_str::<Value>(&text)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn without_a_filter_every_namespace_is_returned_in_table_order() {
    let state = test_state().await;
    let resp = list(&state, json!({})).await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let names: Vec<String> = groups(&resp)
        .iter()
        .map(|g| g["namespace"].as_str().unwrap().to_string())
        .collect();
    let expected: Vec<String> = crate::keybindings::KeyNamespace::ALL
        .iter()
        .map(|n| n.name().to_string())
        .collect();
    assert_eq!(names, expected);
}

#[tokio::test]
async fn a_namespace_name_returns_exactly_that_group() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.normal"})).await;
    let gs = groups(&resp);
    assert_eq!(gs.len(), 1);
    assert_eq!(gs[0]["namespace"], "board.normal");
    let rows = gs[0]["bindings"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        crate::keybindings::bindings_in(crate::keybindings::KeyNamespace::BoardNormal).count()
    );
}

#[tokio::test]
async fn a_family_name_returns_every_group_of_the_family() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.confirm"})).await;
    let gs = groups(&resp);
    assert_eq!(gs.len(), 14);
    assert!(gs.iter().all(|g| g["namespace"]
        .as_str()
        .unwrap()
        .starts_with("board.confirm.")));
}

#[tokio::test]
async fn the_last_row_row_carries_its_note_and_context_is_shown_as_words() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.normal"})).await;
    let gs = groups(&resp);
    let rows = gs[0]["bindings"].as_array().unwrap();
    let g = rows
        .iter()
        .find(|r| r["action"] == "navigate_row_last")
        .unwrap();
    assert!(g["note"]
        .as_str()
        .unwrap()
        .contains("does not enter an epic"));
    assert!(g["keys"].as_array().unwrap().iter().any(|k| k == "G"));

    let q = rows.iter().find(|r| r["action"] == "exit_epic").unwrap();
    assert_eq!(q["context"], "inside an epic view");
    // Absent context and note are omitted, not null.
    let n = rows.iter().find(|r| r["action"] == "create_task").unwrap();
    assert!(n.get("context").is_none());
    assert!(n.get("note").is_none());
}

#[tokio::test]
async fn tmux_global_rows_are_marked_as_handled_by_tmux() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "tmux.global"})).await;
    let gs = groups(&resp);
    assert_eq!(gs[0]["receiver"], "tmux");
    assert!(!gs[0]["bindings"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_namespace_is_rejected_listing_the_valid_names() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.nope"})).await;
    // tools/call re-wraps a tool's INVALID_PARAMS as an `isError` result, the
    // same way it does for query_usage's invalid category.
    assert!(is_error(&resp));
    let message = error_message(&resp);
    assert!(message.contains("board.nope"), "{message}");
    assert!(message.contains("board.normal"), "{message}");
    assert!(message.contains("board.confirm"), "{message}");
    assert!(message.contains("tmux.global"), "{message}");
}

/// Non-vacuity of the family filter: every board.confirm.* dialog has rows,
/// so the family returns fourteen groups none of which is empty.
#[tokio::test]
async fn the_confirm_family_returns_fourteen_non_empty_groups() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.confirm"})).await;
    let gs = groups(&resp);
    assert_eq!(gs.len(), 14);
    for g in &gs {
        assert!(
            !g["bindings"].as_array().unwrap().is_empty(),
            "{} has no rows",
            g["namespace"]
        );
        assert_eq!(g["receiver"], "dispatch");
    }
}

/// The picker family is every board.picker.* namespace — seven of them
/// (repo_path, base_branch, tag, wrap_up_mode, quick_dispatch, move_to_epic,
/// reparent_epic) — and each has rows.
#[tokio::test]
async fn the_picker_family_returns_seven_non_empty_groups() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.picker"})).await;
    let gs = groups(&resp);
    let names: Vec<&str> = gs
        .iter()
        .map(|g| g["namespace"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "board.picker.repo_path",
            "board.picker.base_branch",
            "board.picker.tag",
            "board.picker.wrap_up_mode",
            "board.picker.quick_dispatch",
            "board.picker.move_to_epic",
            "board.picker.reparent_epic",
        ]
    );
    for g in &gs {
        assert!(
            !g["bindings"].as_array().unwrap().is_empty(),
            "{} has no rows",
            g["namespace"]
        );
    }
}

/// The catch-all is listed like any other row, so an agent can see that any
/// key dismisses the error popup.
#[tokio::test]
async fn the_error_popups_catch_all_is_listed() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.error"})).await;
    let gs = groups(&resp);
    let rows = gs[0]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["action"], "dismiss_error");
    assert_eq!(rows[0]["keys"], json!([crate::keybindings::ANY_OTHER_KEY]));
}

/// The Esc exit_epic row's context is shown as the spec's words for
/// epic_view_no_search.
#[tokio::test]
async fn the_esc_exit_epic_row_shows_its_context_words() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.normal"})).await;
    let gs = groups(&resp);
    let rows = gs[0]["bindings"].as_array().unwrap();
    let esc = rows
        .iter()
        .find(|r| r["action"] == "exit_epic" && r["keys"] == json!(["Esc"]))
        .unwrap();
    assert_eq!(esc["context"], "inside an epic view, no search active");
}
