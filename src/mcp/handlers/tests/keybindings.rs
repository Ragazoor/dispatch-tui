//! `keybinding_search` (`ListKeybindingsViaMcp` in
//! `docs/specs/keybindings.allium`).
#![allow(clippy::unwrap_used)]
use super::*;

async fn list(state: &Arc<McpState>, args: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "keybinding_search", "arguments": args })),
    )
    .await
}

/// The groups of a reply: its first line. A line after it, when there is
/// one, is prose (the restart note).
fn groups(resp: &JsonRpcResponse) -> Vec<Value> {
    let text = text_of(resp);
    let first = text.lines().next().unwrap_or_default();
    serde_json::from_str::<Value>(first)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn text_of(resp: &JsonRpcResponse) -> String {
    resp.result.as_ref().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

/// keybindings.allium: ListKeybindingsViaMcp. With neither namespace nor query
/// the reply is an index, not the 44 KB table: one entry per namespace in
/// table order with its receiver and row count, then a line saying how to get
/// the keys.
#[tokio::test]
async fn without_a_filter_an_index_of_namespaces_is_returned_in_table_order() {
    let state = test_state().await;
    let resp = list(&state, json!({})).await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let text = text_of(&resp);
    assert!(text.len() < 8_000, "the index is {} bytes", text.len());
    let (first, hint) = text
        .split_once('\n')
        .expect("a hint line follows the index");
    assert!(
        hint.contains("namespace") && hint.contains("query"),
        "{hint}"
    );
    let index: Vec<Value> = serde_json::from_str(first).unwrap();
    let names: Vec<String> = index
        .iter()
        .map(|g| g["namespace"].as_str().unwrap().to_string())
        .collect();
    let expected: Vec<String> = crate::keybindings::KeyNamespace::ALL
        .iter()
        .map(|n| n.name().to_string())
        .collect();
    assert_eq!(names, expected);
    let normal = index
        .iter()
        .find(|g| g["namespace"] == "board.normal")
        .unwrap();
    assert_eq!(
        normal["bindings"],
        crate::keybindings::default_bindings_in(crate::keybindings::KeyNamespace::BoardNormal)
            .count()
    );
    assert!(index.iter().all(|g| g["bindings"].is_number()));
}

/// keybindings.allium: ListKeybindingsViaMcp. query keeps the rows whose keys,
/// action id or description contain it, ignoring case, across every namespace,
/// and drops a group left with none.
#[tokio::test]
async fn a_query_returns_only_the_matching_rows_across_namespaces() {
    let state = test_state().await;
    let resp = list(&state, json!({"query": "NAVIGATE_ROW_LAST"})).await;
    let gs = groups(&resp);
    assert!(!gs.is_empty());
    for g in &gs {
        let rows = g["bindings"].as_array().unwrap();
        assert!(!rows.is_empty(), "{} kept with no rows", g["namespace"]);
        for r in rows {
            let hay = format!("{} {} {}", r["keys"], r["action"], r["description"]).to_lowercase();
            assert!(hay.contains("navigate_row_last"), "{r}");
        }
    }
    assert!(gs.iter().any(|g| g["namespace"] == "board.normal"));
}

#[tokio::test]
async fn a_query_narrows_the_namespace_it_is_combined_with() {
    let state = test_state().await;
    let resp = list(
        &state,
        json!({"namespace": "board.normal", "query": "navigate_row_last"}),
    )
    .await;
    let gs = groups(&resp);
    assert_eq!(gs.len(), 1);
    assert_eq!(gs[0]["namespace"], "board.normal");
    assert!(gs[0]["bindings"].as_array().unwrap().len() < 5);
}

/// A query that matches nothing is not an error: the reply names the namespace
/// and query it applied.
#[tokio::test]
async fn a_query_with_no_match_names_what_it_applied() {
    let state = test_state().await;
    let resp = list(
        &state,
        json!({"namespace": "board.normal", "query": "zzzqqq"}),
    )
    .await;
    assert!(resp.error.is_none() && !is_error(&resp), "{resp:?}");
    let text = text_of(&resp);
    assert!(
        text.contains("zzzqqq") && text.contains("board.normal"),
        "{text}"
    );
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
        crate::keybindings::default_bindings_in(crate::keybindings::KeyNamespace::BoardNormal)
            .count()
    );
}

#[tokio::test]
async fn a_family_name_returns_every_group_of_the_family() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.confirm"})).await;
    let gs = groups(&resp);
    assert_eq!(gs.len(), 15);
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
    assert_eq!(q["context"], "inside an epic or work package view");
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
    // same way it does for usage_read's invalid category.
    assert!(is_error(&resp));
    let message = error_message(&resp);
    assert!(message.contains("board.nope"), "{message}");
    assert!(message.contains("board.normal"), "{message}");
    assert!(message.contains("board.confirm"), "{message}");
    assert!(message.contains("tmux.global"), "{message}");
}

/// Non-vacuity of the family filter: every board.confirm.* dialog has rows,
/// so the family returns fifteen groups none of which is empty.
#[tokio::test]
async fn the_confirm_family_returns_fifteen_non_empty_groups() {
    let state = test_state().await;
    let resp = list(&state, json!({"namespace": "board.confirm"})).await;
    let gs = groups(&resp);
    assert_eq!(gs.len(), 15);
    for g in &gs {
        assert!(
            !g["bindings"].as_array().unwrap().is_empty(),
            "{} has no rows",
            g["namespace"]
        );
        assert_eq!(g["receiver"], "dispatch");
    }
}

/// The picker family is every board.picker.* namespace — ten of them
/// (repo_path, base_branch, tag, wrap_up_mode, model, quick_dispatch,
/// move_to_epic, reparent_epic, work_package, placement) — and each has rows.
#[tokio::test]
async fn the_picker_family_returns_ten_non_empty_groups() {
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
            "board.picker.model",
            "board.picker.quick_dispatch",
            "board.picker.move_to_epic",
            "board.picker.reparent_epic",
            "board.picker.work_package",
            "board.picker.placement",
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
    assert_eq!(
        esc["context"],
        "inside an epic or work package view, no search active"
    );
}

/// CatalogueIsTheDispatchTable for the merged table (LoadKeyTable): with an
/// override file in the data directory, keybinding_search answers from the
/// table the board loaded, so a rebound row shows the user's keys and its
/// default keys, and a query for the user's key finds it.
#[tokio::test]
async fn the_catalogue_shows_the_users_keys_from_the_override_file() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        r#"[{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]}]"#,
    );
    let state = board_started_in(dir.path()).await;

    let row = row_of(&state, "board.normal", "delete_task").await;
    assert_eq!(row["keys"], json!(["O"]));
    assert_eq!(row["default_keys"], json!(default_keys("delete_task")));
    assert!(row.get("pending_keys").is_none(), "{row}");

    // A query matches on the user's key, not the default.
    let resp = list(
        &state,
        json!({"namespace": "board.normal", "query": "delete_task"}),
    )
    .await;
    assert_eq!(groups(&resp)[0]["bindings"][0]["keys"], json!(["O"]));
}

/// ReadersFollowTheRemap for the catalogue: under the remapped table every
/// namespace's rows come back with the keys they have there, in table order.
#[tokio::test]
async fn the_catalogue_follows_the_remapped_table() {
    let table = crate::keybindings::remapped_table();
    let dir = tempfile::tempdir().unwrap();
    let mut load = crate::keybindings::KeyTableLoad::defaults(dir.path());
    load.table = table;
    let state = state_with_key_table(load).await;
    for ns in crate::keybindings::KeyNamespace::ALL {
        let resp = list(&state, json!({"namespace": ns.name()})).await;
        let got: Vec<Value> = groups(&resp)[0]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["keys"].clone())
            .collect();
        let want: Vec<Value> = crate::keybindings::bindings_in(table, ns)
            .map(|b| json!(b.keys))
            .collect();
        assert_eq!(got, want, "{}", ns.name());
    }
}

/// A test state whose board loaded `load`.
async fn state_with_key_table(load: crate::keybindings::KeyTableLoad) -> Arc<McpState> {
    let Ok(mut state) = Arc::try_unwrap(test_state().await) else {
        panic!("the test state is shared");
    };
    state.key_table = load;
    Arc::new(state)
}

// ---------------------------------------------------------------------------
// keybinding_set (SetKeyOverridesViaMcp) and what keybinding_search shows of it
// (OverridesAreVisible).
// ---------------------------------------------------------------------------

use crate::keybindings::{KeyNamespace, KEY_BINDINGS, KEY_OVERRIDE_FILE_NAME};

/// The MCP state of a board that started in `dir`: it loaded its table from
/// the override file there, as the board does at start (LoadKeyTable).
/// Calling it again on the same directory is a board restart.
async fn board_started_in(dir: &std::path::Path) -> Arc<McpState> {
    let load = crate::keybindings::load_key_table(dir, crate::keybindings::KeyTableReader::Board);
    state_with_key_table(load).await
}

/// The board's load must have refused nothing, so a test cannot pass on a
/// table that quietly kept its defaults.
async fn board_restarted_cleanly_in(dir: &std::path::Path) -> Arc<McpState> {
    let load = crate::keybindings::load_key_table(dir, crate::keybindings::KeyTableReader::Board);
    assert!(load.errors.is_empty(), "{:?}", load.errors);
    state_with_key_table(load).await
}

fn write_file(dir: &std::path::Path, text: &str) {
    std::fs::write(dir.join(KEY_OVERRIDE_FILE_NAME), text).unwrap();
}

fn file_text(dir: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(dir.join(KEY_OVERRIDE_FILE_NAME)).ok()
}

async fn set(state: &Arc<McpState>, overrides: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": "keybinding_set", "arguments": { "overrides": overrides } })),
    )
    .await
}

/// The board.normal row of `action` with no context.
fn default_keys(action: &str) -> Vec<&'static str> {
    KEY_BINDINGS
        .iter()
        .find(|b| {
            b.namespace == KeyNamespace::BoardNormal && b.action == action && b.context.is_none()
        })
        .unwrap()
        .keys
        .to_vec()
}

/// The context-free row of `action` in `ns`, as keybinding_search returns it.
async fn row_of(state: &Arc<McpState>, ns: &str, action: &str) -> Value {
    let resp = list(state, json!({"namespace": ns})).await;
    assert!(!is_error(&resp), "{resp:?}");
    groups(&resp)[0]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["action"] == action && b.get("context").is_none())
        .unwrap_or_else(|| panic!("no {ns} {action} row"))
        .clone()
}

/// Modified keys no board.normal row lists.
fn free_normal_keys() -> Vec<&'static str> {
    ["Ctrl+T", "Ctrl+Y", "Ctrl+O", "F5", "F6", "F7"]
        .into_iter()
        .filter(|k| {
            !KEY_BINDINGS
                .iter()
                .any(|b| b.namespace == KeyNamespace::BoardNormal && b.keys.contains(k))
        })
        .collect()
}

/// keybindings.allium: SetKeyOverridesViaMcp, ListKeybindingsViaMcp. The
/// done-when of the tool: an agent remaps a key, sees it in keybinding_search
/// (pending until the board restarts, then the user's key beside the
/// default), and restores the default with an empty key list.
#[tokio::test]
async fn an_agent_remaps_a_key_sees_it_and_restores_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let state = board_started_in(dir.path()).await;

    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]}]),
    )
    .await;
    assert!(!is_error(&resp), "{resp:?}");
    let answer = text_of(&resp);
    assert!(
        answer.contains("delete_task") && answer.contains("\"O\""),
        "{answer}"
    );
    assert!(answer.contains("when the board next starts"), "{answer}");

    // The running board still answers to the default; the change is pending.
    let row = row_of(&state, "board.normal", "delete_task").await;
    assert_eq!(row["keys"], json!(default_keys("delete_task")));
    assert_eq!(row["pending_keys"], json!(["O"]));
    let reply = text_of(&list(&state, json!({"namespace": "board.normal"})).await);
    assert!(reply.contains("when the board next starts"), "{reply}");

    // After a restart the board answers to the user's key.
    let state = board_restarted_cleanly_in(dir.path()).await;
    let row = row_of(&state, "board.normal", "delete_task").await;
    assert_eq!(row["keys"], json!(["O"]));
    assert_eq!(row["default_keys"], json!(default_keys("delete_task")));
    assert!(row.get("pending_keys").is_none(), "{row}");

    // An empty list restores the default: pending now, applied at restart.
    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "delete_task", "keys": []}]),
    )
    .await;
    assert!(!is_error(&resp), "{resp:?}");
    assert!(text_of(&resp).contains("default"), "{}", text_of(&resp));
    let row = row_of(&state, "board.normal", "delete_task").await;
    assert_eq!(row["pending_keys"], json!(default_keys("delete_task")));

    let state = board_restarted_cleanly_in(dir.path()).await;
    let row = row_of(&state, "board.normal", "delete_task").await;
    assert_eq!(row["keys"], json!(default_keys("delete_task")));
    assert!(row.get("default_keys").is_none(), "{row}");
    assert!(row.get("pending_keys").is_none(), "{row}");
    let left: Value = serde_json::from_str(&file_text(dir.path()).unwrap()).unwrap();
    assert_eq!(
        left,
        json!([]),
        "removing the last entry leaves an empty list"
    );
}

/// ARefusedChangeWritesNothing, WritersShareTheLoadersChecks: a change the
/// load would refuse is rejected with the loader's message, and the file is
/// left byte for byte as it was.
#[tokio::test]
async fn a_change_the_load_would_refuse_writes_nothing_and_says_what_to_fix() {
    let dir = tempfile::tempdir().unwrap();
    let before = r#"[{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]}]"#;
    write_file(dir.path(), before);
    let state = board_started_in(dir.path()).await;

    let cases = [
        (
            json!({"namespace": "board.normal", "action": "navigate_row_last", "keys": ["ctrl+d"]}),
            "Ctrl+D",
        ),
        (
            json!({"namespace": "board.normal", "action": "no_such_action", "keys": ["O"]}),
            "create_task",
        ),
        (
            json!({"namespace": "board.nope", "action": "delete_task", "keys": ["O"]}),
            "board.normal",
        ),
        (
            json!({"namespace": "board.normal", "action": "select_all", "context": "on_nothing", "keys": ["F5"]}),
            "on_column_select_all",
        ),
        (
            json!({"namespace": "board.text", "action": "submit_input", "keys": ["x"]}),
            "typed",
        ),
        (
            json!({"namespace": "board.error", "action": "dismiss_error", "keys": ["x"]}),
            "cannot be rebound",
        ),
        (
            json!({"namespace": "board.normal", "action": "no_such_action", "keys": []}),
            "create_task",
        ),
    ];
    for (change, hint) in cases {
        let resp = set(&state, json!([change.clone()])).await;
        assert!(is_error(&resp), "{change} was accepted");
        let message = error_message(&resp);
        assert!(message.contains(hint), "{change}: {message}");
        assert_eq!(file_text(dir.path()).as_deref(), Some(before), "{change}");
    }
}

/// A clash with a row the call does not change is refused; the same keys
/// swapped in one call are accepted, because the changes of a call are
/// checked together.
#[tokio::test]
async fn a_swap_is_one_call_and_either_half_alone_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let state = board_started_in(dir.path()).await;
    let down = KEY_BINDINGS
        .iter()
        .find(|b| b.namespace == KeyNamespace::BoardNormal && b.action == "navigate_row_down")
        .unwrap()
        .keys
        .to_vec();
    let up = KEY_BINDINGS
        .iter()
        .find(|b| b.namespace == KeyNamespace::BoardNormal && b.action == "navigate_row_up")
        .unwrap()
        .keys
        .to_vec();

    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "navigate_row_down", "keys": up}]),
    )
    .await;
    assert!(is_error(&resp));
    let message = error_message(&resp);
    assert!(message.contains("navigate_row_up"), "{message}");
    assert!(
        file_text(dir.path()).is_none(),
        "a refused first change creates no file"
    );

    let resp = set(
        &state,
        json!([
            {"namespace": "board.normal", "action": "navigate_row_down", "keys": up},
            {"namespace": "board.normal", "action": "navigate_row_up", "keys": down},
        ]),
    )
    .await;
    assert!(!is_error(&resp), "{resp:?}");
    let state = board_restarted_cleanly_in(dir.path()).await;
    assert_eq!(
        row_of(&state, "board.normal", "navigate_row_down").await["keys"],
        json!(up)
    );
    assert_eq!(
        row_of(&state, "board.normal", "navigate_row_up").await["keys"],
        json!(down)
    );
}

/// A change that would make an entry the file already has, and the load
/// accepts, clash is refused, and the file is left as it was.
#[tokio::test]
async fn a_change_that_breaks_an_accepted_entry_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let before = r#"[{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]}]"#;
    write_file(dir.path(), before);
    let state = board_started_in(dir.path()).await;
    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "navigate_row_last", "keys": ["O"]}]),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("delete_task"),
        "{}",
        error_message(&resp)
    );
    assert_eq!(file_text(dir.path()).as_deref(), Some(before));
}

/// Two changes naming one row in one call are refused: the call would not
/// say which keys the row gets.
#[tokio::test]
async fn a_row_named_twice_in_one_call_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let state = board_started_in(dir.path()).await;
    let resp = set(
        &state,
        json!([
            {"namespace": "board.normal", "action": "delete_task", "keys": ["O"]},
            {"namespace": "board.normal", "action": "delete_task", "keys": []},
        ]),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("delete_task"),
        "{}",
        error_message(&resp)
    );
    assert!(file_text(dir.path()).is_none());
}

/// A file that is not a JSON list is never written over: the rejection says
/// to fix or remove it.
#[tokio::test]
async fn a_file_that_is_not_a_list_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    for before in ["{\"namespace\": \"board.normal\"}", "[{not json"] {
        write_file(dir.path(), before);
        let state = board_started_in(dir.path()).await;
        let resp = set(
            &state,
            json!([{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]}]),
        )
        .await;
        assert!(is_error(&resp), "{before}");
        let message = error_message(&resp);
        assert!(message.contains("remove"), "{message}");
        assert_eq!(file_text(dir.path()).as_deref(), Some(before));
    }
}

/// An entry already in the file that the load refuses is not this call's
/// problem: it is kept as written and named in the answer.
#[tokio::test]
async fn an_entry_the_load_already_refuses_is_kept_and_named() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        r#"[{"namespace": "board.nope", "action": "delete_task", "keys": ["O"]}, 7]"#,
    );
    let state = board_started_in(dir.path()).await;
    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]}]),
    )
    .await;
    assert!(!is_error(&resp), "{resp:?}");
    let answer = text_of(&resp);
    assert!(answer.contains("board.nope"), "{answer}");
    let file: Value = serde_json::from_str(&file_text(dir.path()).unwrap()).unwrap();
    assert_eq!(
        file,
        json!([
            {"namespace": "board.nope", "action": "delete_task", "keys": ["O"]},
            7,
            {"namespace": "board.normal", "action": "delete_task", "keys": ["O"]},
        ])
    );
}

/// The write keeps every entry the call did not name, replaces the named
/// row's entry in place, and repeating the call leaves the same file.
#[tokio::test]
async fn a_change_replaces_its_rows_entry_in_place_and_repeating_it_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let free = free_normal_keys();
    write_file(
        dir.path(),
        &json!([
            {"namespace": "board.normal", "action": "delete_task", "keys": ["O"]},
            {"namespace": "board.normal", "action": "navigate_row_last", "keys": [free[0]]},
        ])
        .to_string(),
    );
    let state = board_started_in(dir.path()).await;
    let change = json!([{"namespace": "board.normal", "action": "delete_task", "keys": [free[1]]}]);
    assert!(!is_error(&set(&state, change.clone()).await));
    let first = file_text(dir.path()).unwrap();
    let file: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(
        file,
        json!([
            {"namespace": "board.normal", "action": "delete_task", "keys": [free[1]]},
            {"namespace": "board.normal", "action": "navigate_row_last", "keys": [free[0]]},
        ])
    );
    assert!(!is_error(&set(&state, change).await));
    assert_eq!(file_text(dir.path()).unwrap(), first);
}

/// Removing an entry the file does not have changes nothing and writes no
/// file.
#[tokio::test]
async fn restoring_a_row_that_has_no_entry_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = board_started_in(dir.path()).await;
    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "delete_task", "keys": []}]),
    )
    .await;
    assert!(!is_error(&resp), "{resp:?}");
    assert!(file_text(dir.path()).is_none());
}

/// A context row is named by its literal, which keybinding_search returns
/// beside the context's words.
#[tokio::test]
async fn a_context_row_is_returned_with_its_literal_and_set_by_it() {
    let dir = tempfile::tempdir().unwrap();
    let state = board_started_in(dir.path()).await;
    let resp = list(
        &state,
        json!({"namespace": "board.normal", "query": "select_all"}),
    )
    .await;
    let rows = groups(&resp)[0]["bindings"].as_array().unwrap().clone();
    let ctx_row = rows
        .iter()
        .find(|r| r["context_literal"] == "on_column_select_all")
        .unwrap_or_else(|| panic!("{rows:?}"));
    assert!(ctx_row["context"].is_string());

    let free = free_normal_keys();
    let resp = set(
        &state,
        json!([{"namespace": "board.normal", "action": "select_all",
                "context": "on_column_select_all", "keys": [free[0]]}]),
    )
    .await;
    assert!(!is_error(&resp), "{resp:?}");
    let state = board_restarted_cleanly_in(dir.path()).await;
    let resp = list(
        &state,
        json!({"namespace": "board.normal", "query": "select_all"}),
    )
    .await;
    let rows = groups(&resp)[0]["bindings"].as_array().unwrap().clone();
    let ctx_row = rows
        .iter()
        .find(|r| r["context_literal"] == "on_column_select_all")
        .unwrap();
    assert_eq!(ctx_row["keys"], json!([free[0]]));
}

/// The index counts each namespace's overridden rows and lists the board's
/// load errors, each with its message; with none it says so in one line.
#[tokio::test]
async fn the_index_counts_overridden_rows_and_lists_the_load_errors() {
    let dir = tempfile::tempdir().unwrap();
    let state = board_started_in(dir.path()).await;
    let text = text_of(&list(&state, json!({})).await);
    assert!(text.contains("no errors"), "{text}");

    write_file(
        dir.path(),
        r#"[{"namespace": "board.normal", "action": "delete_task", "keys": ["O"]},
            {"namespace": "board.nope", "action": "delete_task", "keys": ["O"]}]"#,
    );
    let state = board_started_in(dir.path()).await;
    let text = text_of(&list(&state, json!({})).await);
    let index: Vec<Value> = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let normal = index
        .iter()
        .find(|g| g["namespace"] == "board.normal")
        .unwrap();
    assert_eq!(normal["overridden"], 1);
    let text_ns = index
        .iter()
        .find(|g| g["namespace"] == "board.text")
        .unwrap();
    assert_eq!(text_ns["overridden"], 0);
    assert!(
        text.contains("board.nope") && text.contains("is not a namespace"),
        "{text}"
    );
}
