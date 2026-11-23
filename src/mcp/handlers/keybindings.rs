//! `keybinding_search`: the read-only catalogue of the keybinding table
//! (`ListKeybindingsViaMcp` in `docs/specs/keybindings.allium`). It reads
//! the table the board loaded, so it shows the user's keys where an override
//! was accepted (`CatalogueIsTheDispatchTable`), and beside them each
//! overridden row's default keys and any change waiting for a restart
//! (`OverridesAreVisible`).
//!
//! `keybinding_set`: its write twin, which changes the override file
//! (`SetKeyOverridesViaMcp`).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::keybindings::{
    check_overrides, namespaces_matching, read_override_entries, set_key_overrides, KeyBinding,
    KeyChange, KeyNamespace, KEY_BINDINGS, KEY_FAMILIES,
};
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::{parse_args, JsonRpcResponse, INVALID_PARAMS};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeybindingSearchArgs {
    #[serde(default)]
    pub(super) namespace: Option<String>,
    #[serde(default)]
    pub(super) query: Option<String>,
}

pub(crate) async fn handle_keybinding_search(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let args: KeybindingSearchArgs = match parse_args(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let query = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);

    let table = state.key_table.table;
    let next = next_keys(state);

    // The whole table is about 44 KB: with no filter, answer with an index.
    if args.namespace.is_none() && query.is_none() {
        let index: Vec<Value> = KeyNamespace::ALL
            .iter()
            .map(|ns| {
                let rows = rows_in(table, *ns);
                json!({
                    "namespace": ns.name(),
                    "receiver": ns.receiver().as_str(),
                    "bindings": rows.clone().count(),
                    "overridden": rows.filter(|(i, b)| b.keys != KEY_BINDINGS[*i].keys).count(),
                })
            })
            .collect();
        let mut text = serde_json::to_string(&index).unwrap_or_else(|_| "[]".to_string());
        let errors = &state.key_table.errors;
        if errors.is_empty() {
            text.push_str("\nThe board loaded the key override file with no errors.");
        } else {
            let listed: Vec<Value> = errors
                .iter()
                .map(|e| {
                    json!({
                        "problem": e.problem.as_str(),
                        "namespace": e.namespace,
                        "action": e.action,
                        "context": e.context,
                        "message": e.message,
                    })
                })
                .collect();
            text.push_str(&format!(
                "\nThe board refused these entries of {}; their rows keep their default keys: {}",
                state.key_table.file.display(),
                serde_json::to_string(&listed).unwrap_or_else(|_| "[]".to_string())
            ));
        }
        let pending = pending_rows(table, &next).count();
        if pending > 0 {
            text.push_str(&format!(
                "\n{pending} rows have keys in the override file that apply when the board next starts."
            ));
        }
        text.push_str("\nThis is an index of the namespaces, not the keys. Pass namespace for one namespace's keys (or a family such as board.confirm), or query to find rows across every namespace.");
        return JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }));
    }

    let namespaces = match args.namespace.as_deref() {
        None => KeyNamespace::ALL.to_vec(),
        Some(name) => match namespaces_matching(name) {
            Some(ns) => ns,
            None => {
                let valid: Vec<&str> = KeyNamespace::ALL
                    .iter()
                    .map(|n| n.name())
                    .chain(KEY_FAMILIES)
                    .collect();
                return JsonRpcResponse::err(
                    id,
                    INVALID_PARAMS,
                    format!(
                        "unknown namespace: {name}. Valid namespaces and families: {}",
                        valid.join(", ")
                    ),
                );
            }
        },
    };

    let groups: Vec<Value> = namespaces
        .into_iter()
        .map(|ns| {
            let bindings: Vec<Value> = rows_in(table, ns)
                .filter(|(_, b)| {
                    query.as_deref().is_none_or(|q| {
                        b.keys.iter().any(|k| k.to_lowercase().contains(q))
                            || b.action.to_lowercase().contains(q)
                            || b.description.to_lowercase().contains(q)
                    })
                })
                .map(|(i, b)| {
                    let mut row = json!({
                        "keys": b.keys,
                        "action": b.action,
                        "description": b.description,
                    });
                    if b.keys != KEY_BINDINGS[i].keys {
                        row["default_keys"] = json!(KEY_BINDINGS[i].keys);
                    }
                    if let Some(pending) = next.get(i).filter(|k| !same_keys(b, k)) {
                        row["pending_keys"] = json!(pending);
                    }
                    if let Some(c) = b.context {
                        row["context"] = json!(c.words());
                        row["context_literal"] = json!(c.literal());
                    }
                    if let Some(n) = b.note {
                        row["note"] = json!(n);
                    }
                    row
                })
                .collect();
            json!({
                "namespace": ns.name(),
                "receiver": ns.receiver().as_str(),
                "bindings": bindings,
            })
        })
        .filter(|g| query.is_none() || !g["bindings"].as_array().is_none_or(Vec::is_empty))
        .collect();

    if groups.is_empty() {
        let text = format!(
            "No keybindings matched. Applied namespace: {}; query: {}.",
            args.namespace.as_deref().unwrap_or("(all)"),
            args.query.as_deref().unwrap_or("(none)")
        );
        return JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }));
    }
    let mut text = serde_json::to_string(&groups).unwrap_or_else(|_| "[]".to_string());
    let pending_shown = groups.iter().any(|g| {
        g["bindings"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|r| r.get("pending_keys").is_some()))
    });
    if pending_shown {
        text.push_str(&format!("\n{PENDING_NOTE}"));
    }
    JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }))
}

/// What a row's pending keys mean, wherever they are shown.
const PENDING_NOTE: &str = "pending_keys are the keys the override file gives a row now: they apply when the board next starts, and a companion or diff pane opened before then already uses them.";

/// The rows of `ns` in `table`, each with its index, which is also its index
/// in `KEY_BINDINGS` (a loaded table has the same rows in the same order).
fn rows_in(
    table: &'static [KeyBinding],
    ns: KeyNamespace,
) -> impl Iterator<Item = (usize, &'static KeyBinding)> + Clone {
    table
        .iter()
        .enumerate()
        .filter(move |(_, b)| b.namespace == ns)
}

/// The keys each row will have when the board next starts: what a load of
/// the override file as it is now would produce (`LoadKeyTable`). A file the
/// load would refuse whole gives every row its default keys.
fn next_keys(state: &McpState) -> Vec<Vec<String>> {
    match read_override_entries(&state.key_table.file) {
        Ok(list) => check_overrides(&list).keys,
        Err(_) => KEY_BINDINGS
            .iter()
            .map(|b| b.keys.iter().map(|k| (*k).to_string()).collect())
            .collect(),
    }
}

fn same_keys(b: &KeyBinding, keys: &[String]) -> bool {
    b.keys.iter().copied().eq(keys.iter().map(String::as_str))
}

/// The rows whose keys at the next start differ from those the board loaded.
fn pending_rows<'a>(
    table: &'static [KeyBinding],
    next: &'a [Vec<String>],
) -> impl Iterator<Item = &'static KeyBinding> + 'a {
    table
        .iter()
        .zip(next)
        .filter(|(b, keys)| !same_keys(b, keys))
        .map(|(b, _)| b)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeybindingSetArgs {
    pub(super) overrides: Vec<KeyOverrideArg>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeyOverrideArg {
    pub(super) namespace: String,
    pub(super) action: String,
    #[serde(default)]
    pub(super) context: Option<String>,
    pub(super) keys: Vec<String>,
}

/// `keybinding_set` (`SetKeyOverridesViaMcp`): change the keys of one or
/// more rows in the override file the board loaded, all or nothing. A
/// rejection is `INVALID_PARAMS` with one line per problem.
pub(crate) async fn handle_keybinding_set(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let args: KeybindingSetArgs = match parse_args(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let changes: Vec<KeyChange> = args
        .overrides
        .into_iter()
        .map(|o| KeyChange {
            namespace: o.namespace,
            action: o.action,
            context: o.context,
            keys: o.keys,
        })
        .collect();
    let file = state.key_table.file.clone();
    let outcome = match set_key_overrides(&file, &changes) {
        Ok(outcome) => outcome,
        Err(problems) => {
            return JsonRpcResponse::err(
                id,
                INVALID_PARAMS,
                format!(
                    "Nothing was written. Fix these and call again:\n- {}",
                    problems.join("\n- ")
                ),
            )
        }
    };
    let mut text = if outcome.written {
        format!("Saved to {}:", file.display())
    } else {
        format!("{} already says this; nothing was written:", file.display())
    };
    for row in &outcome.rows {
        let keys = serde_json::to_string(&row.keys).unwrap_or_default();
        if row.restored {
            text.push_str(&format!("\n- {}: default keys {keys}", row.label));
        } else {
            text.push_str(&format!("\n- {}: {keys}", row.label));
        }
    }
    text.push_str("\nThe running board keeps the keys it loaded: these apply when the board next starts, and a companion or diff pane opened before then already uses them.");
    if !outcome.kept_errors.is_empty() {
        text.push_str(
            "\nThese entries already in the file are refused at load and were left as they are:",
        );
        for e in &outcome.kept_errors {
            text.push_str(&format!("\n- {}", e.message));
        }
    }
    JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }))
}
