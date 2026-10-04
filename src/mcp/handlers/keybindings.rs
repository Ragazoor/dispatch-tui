//! `list_keybindings`: the read-only catalogue of the keybinding table
//! (`ListKeybindingsViaMcp` in `docs/specs/keybindings.allium`).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::keybindings::{bindings_in, namespaces_matching, KeyNamespace, KEY_FAMILIES};
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::{parse_args, JsonRpcResponse, INVALID_PARAMS};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListKeybindingsArgs {
    #[serde(default)]
    pub(super) namespace: Option<String>,
}

pub(crate) async fn handle_list_keybindings(
    _state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let args: ListKeybindingsArgs = match parse_args(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

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
        .collect();

    let text = serde_json::to_string_pretty(&groups).unwrap_or_else(|_| "[]".to_string());
    JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }))
}
