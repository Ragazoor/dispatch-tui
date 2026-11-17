//! `keybinding_search`: the read-only catalogue of the keybinding table
//! (`ListKeybindingsViaMcp` in `docs/specs/keybindings.allium`).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::keybindings::{bindings_in, namespaces_matching, KeyNamespace, KEY_FAMILIES};
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
    _state: &McpState,
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

    // The whole table is about 44 KB: with no filter, answer with an index.
    if args.namespace.is_none() && query.is_none() {
        let index: Vec<Value> = KeyNamespace::ALL
            .iter()
            .map(|ns| {
                json!({
                    "namespace": ns.name(),
                    "receiver": ns.receiver().as_str(),
                    "bindings": bindings_in(*ns).count(),
                })
            })
            .collect();
        let text = format!(
            "{}\nThis is an index of the namespaces, not the keys. Pass namespace for one namespace's keys (or a family such as board.confirm), or query to find rows across every namespace.",
            serde_json::to_string(&index).unwrap_or_else(|_| "[]".to_string())
        );
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
            let bindings: Vec<Value> = bindings_in(ns)
                .filter(|b| {
                    query.as_deref().is_none_or(|q| {
                        b.keys.iter().any(|k| k.to_lowercase().contains(q))
                            || b.action.to_lowercase().contains(q)
                            || b.description.to_lowercase().contains(q)
                    })
                })
                .map(|b| {
                    let mut row = json!({
                        "keys": b.keys,
                        "action": b.action,
                        "description": b.description,
                    });
                    if let Some(c) = b.context {
                        row["context"] = json!(c.words());
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
    let text = serde_json::to_string(&groups).unwrap_or_else(|_| "[]".to_string());
    JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }))
}
