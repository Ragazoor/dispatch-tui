//! Claude Code MCP server config and tool-permissions: read, merge, and remove.

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{read_json_file, write_json_file};

// ---------------------------------------------------------------------------
// MCP config merging
// ---------------------------------------------------------------------------

pub struct MergeResult {
    pub value: Value,
    pub changed: bool,
}

/// Resolve the `headersHelper` command for the dispatch MCP entry.
///
/// Uses the absolute path of the currently-running dispatch binary so
/// the helper invocation is unambiguous regardless of `$PATH`. Falls
/// back to the bare command name if `current_exe()` is unavailable.
fn caller_headers_helper() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_owned()))
        .unwrap_or_else(|| "dispatch".to_string());
    format!("{exe} caller-headers")
}

pub fn merge_mcp_config(existing: Option<Value>, port: u16) -> MergeResult {
    let server_entry = json!({
        "type": "http",
        "url": format!("http://localhost:{port}/mcp"),
        "headersHelper": caller_headers_helper(),
    });

    let mut root = match existing {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };

    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));

    if let Value::Object(servers_map) = servers {
        if servers_map.get("dispatch") == Some(&server_entry) {
            return MergeResult {
                value: Value::Object(root),
                changed: false,
            };
        }
        servers_map.insert("dispatch".to_string(), server_entry);
    }

    MergeResult {
        value: Value::Object(root),
        changed: true,
    }
}

// ---------------------------------------------------------------------------
// Permissions merging
// ---------------------------------------------------------------------------

fn mcp_permissions() -> Vec<String> {
    crate::mcp::handlers::TOOL_NAMES
        .iter()
        .map(|name| format!("mcp__dispatch__{name}"))
        .collect()
}

pub struct PermissionsMergeResult {
    pub value: Value,
    pub added_count: usize,
}

pub fn merge_permissions(existing: Option<Value>) -> PermissionsMergeResult {
    let mut root = match existing {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };

    let permissions = root.entry("permissions").or_insert_with(|| json!({}));

    let allow = permissions
        .get("allow")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    let mut added_count = 0;
    let mut new_allow = allow;
    for perm in mcp_permissions() {
        let val = Value::String(perm);
        if !new_allow.contains(&val) {
            new_allow.push(val);
            added_count += 1;
        }
    }

    if let Value::Object(ref mut perms_map) = permissions {
        perms_map.insert("allow".to_string(), Value::Array(new_allow));
    }

    PermissionsMergeResult {
        value: Value::Object(root),
        added_count,
    }
}

// ---------------------------------------------------------------------------
// Removal
// ---------------------------------------------------------------------------

pub fn remove_mcp_config(mcp_path: &std::path::Path) -> Result<bool> {
    let existing = match read_json_file(mcp_path)? {
        Some(v) => v,
        None => return Ok(false),
    };

    let mut root = match existing {
        Value::Object(map) => map,
        _ => return Ok(false),
    };

    let had_dispatch = if let Some(Value::Object(servers)) = root.get_mut("mcpServers") {
        servers.remove("dispatch").is_some()
    } else {
        false
    };

    if had_dispatch {
        write_json_file(mcp_path, &Value::Object(root))?;
    }

    Ok(had_dispatch)
}

pub fn remove_permissions(settings_path: &std::path::Path) -> Result<bool> {
    let existing = match read_json_file(settings_path)? {
        Some(v) => v,
        None => return Ok(false),
    };

    let mut root = match existing {
        Value::Object(map) => map,
        _ => return Ok(false),
    };

    let removed_any = if let Some(Value::Object(perms)) = root.get_mut("permissions") {
        if let Some(Value::Array(allow)) = perms.get_mut("allow") {
            let before = allow.len();
            allow.retain(|v| v.as_str().is_none_or(|s| !s.starts_with("mcp__dispatch__")));
            before != allow.len()
        } else {
            false
        }
    } else {
        false
    };

    if removed_any {
        write_json_file(settings_path, &Value::Object(root))?;
    }

    Ok(removed_any)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::DEFAULT_PORT;
    use serde_json::json;

    // -- MCP config merging --

    #[test]
    fn merge_mcp_config_into_empty() {
        let result = merge_mcp_config(None, DEFAULT_PORT);
        let dispatch = &result.value["mcpServers"]["dispatch"];
        assert_eq!(dispatch["type"], "http");
        assert_eq!(
            dispatch["url"],
            format!("http://localhost:{DEFAULT_PORT}/mcp")
        );
        assert!(dispatch["headersHelper"].is_string());
        assert!(result.changed);
    }

    #[test]
    fn merge_mcp_config_emits_headers_helper_pointing_at_caller_headers() {
        let result = merge_mcp_config(None, DEFAULT_PORT);
        let helper = result.value["mcpServers"]["dispatch"]["headersHelper"]
            .as_str()
            .unwrap();
        assert!(
            helper.ends_with("caller-headers"),
            "expected helper to end with 'caller-headers', got {helper}"
        );
    }

    #[test]
    fn merge_mcp_config_preserves_other_servers() {
        let existing = Some(json!({
            "mcpServers": {
                "github": {
                    "type": "http",
                    "url": "http://localhost:9999/mcp"
                }
            }
        }));
        let result = merge_mcp_config(existing, DEFAULT_PORT);
        assert!(result.changed);
        assert!(result.value["mcpServers"]["github"].is_object());
        assert_eq!(
            result.value["mcpServers"]["dispatch"]["url"],
            format!("http://localhost:{DEFAULT_PORT}/mcp")
        );
        assert!(result.value["mcpServers"]["dispatch"]["headersHelper"].is_string());
    }

    #[test]
    fn merge_mcp_config_already_configured() {
        // Pre-seed the existing entry with the same shape merge_mcp_config would write,
        // so the idempotency short-circuit fires.
        let first = merge_mcp_config(None, DEFAULT_PORT);
        let result = merge_mcp_config(Some(first.value.clone()), DEFAULT_PORT);
        assert!(!result.changed, "second merge with identical input must be a no-op");
    }

    #[test]
    fn merge_mcp_config_rewrites_when_helper_missing() {
        // A user upgrading from a pre-headersHelper install: dispatch entry exists
        // with URL only. Merging must rewrite it to include the helper.
        let existing = Some(json!({
            "mcpServers": {
                "dispatch": {
                    "type": "http",
                    "url": format!("http://localhost:{DEFAULT_PORT}/mcp")
                }
            }
        }));
        let result = merge_mcp_config(existing, DEFAULT_PORT);
        assert!(result.changed);
        assert!(result.value["mcpServers"]["dispatch"]["headersHelper"].is_string());
    }

    #[test]
    fn merge_mcp_config_custom_port() {
        let result = merge_mcp_config(None, 4000);
        assert_eq!(
            result.value["mcpServers"]["dispatch"]["url"],
            "http://localhost:4000/mcp"
        );
    }

    // -- Permissions merging --

    #[test]
    fn merge_permissions_into_empty() {
        let existing = None;
        let result = merge_permissions(existing);
        let allow = result.value["permissions"]["allow"].as_array().unwrap();
        for name in crate::mcp::handlers::TOOL_NAMES {
            assert!(
                allow.contains(&json!(format!("mcp__dispatch__{name}"))),
                "missing permission for {name}"
            );
        }
        assert_eq!(result.added_count, crate::mcp::handlers::TOOL_NAMES.len());
    }

    #[test]
    fn merge_permissions_preserves_existing() {
        let existing = Some(json!({
            "permissions": {
                "allow": ["Bash(git:*)", "Read"],
                "defaultMode": "acceptEdits"
            },
            "hooks": {"Stop": []}
        }));
        let result = merge_permissions(existing);
        let allow = result.value["permissions"]["allow"].as_array().unwrap();
        assert!(allow.contains(&json!("Bash(git:*)")));
        assert!(allow.contains(&json!("Read")));
        for name in crate::mcp::handlers::TOOL_NAMES {
            assert!(
                allow.contains(&json!(format!("mcp__dispatch__{name}"))),
                "missing permission for {name}"
            );
        }
        assert_eq!(result.added_count, crate::mcp::handlers::TOOL_NAMES.len());
        assert_eq!(result.value["permissions"]["defaultMode"], "acceptEdits");
        assert!(result.value["hooks"]["Stop"].is_array());
    }

    #[test]
    fn merge_permissions_already_present() {
        let existing = Some(json!({
            "permissions": {
                "allow": crate::mcp::handlers::TOOL_NAMES
                    .iter()
                    .map(|name| format!("mcp__dispatch__{name}"))
                    .collect::<Vec<_>>()
            }
        }));
        let result = merge_permissions(existing);
        assert_eq!(result.added_count, 0);
    }

    #[test]
    fn mcp_permissions_includes_all_tools() {
        let perms = mcp_permissions();
        for name in crate::mcp::handlers::TOOL_NAMES {
            let expected = format!("mcp__dispatch__{name}");
            assert!(
                perms.contains(&expected),
                "missing permission for tool: {name}"
            );
        }
    }

    // -- MCP config removal --

    #[test]
    fn remove_mcp_config_removes_dispatch_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        let existing = json!({
            "mcpServers": {
                "dispatch": {"type": "http", "url": "http://localhost:3142/mcp"},
                "github": {"type": "http", "url": "http://localhost:9999/mcp"}
            }
        });
        write_json_file(&path, &existing).unwrap();

        let removed = remove_mcp_config(&path).unwrap();
        assert!(removed);

        let result = read_json_file(&path).unwrap().unwrap();
        assert!(result["mcpServers"].get("dispatch").is_none());
        assert!(result["mcpServers"]["github"].is_object());
    }

    #[test]
    fn remove_mcp_config_noop_when_no_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        let existing = json!({
            "mcpServers": {
                "github": {"type": "http", "url": "http://localhost:9999/mcp"}
            }
        });
        write_json_file(&path, &existing).unwrap();

        let removed = remove_mcp_config(&path).unwrap();
        assert!(!removed);
    }

    #[test]
    fn remove_mcp_config_noop_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");

        let removed = remove_mcp_config(&path).unwrap();
        assert!(!removed);
    }

    // -- Permissions removal --

    #[test]
    fn remove_permissions_removes_dispatch_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let existing = json!({
            "permissions": {
                "allow": [
                    "Bash(git:*)",
                    "Read",
                    "mcp__dispatch__update_task",
                    "mcp__dispatch__get_task"
                ],
                "defaultMode": "acceptEdits"
            },
            "hooks": {"Stop": []}
        });
        write_json_file(&path, &existing).unwrap();

        let removed = remove_permissions(&path).unwrap();
        assert!(removed);

        let result = read_json_file(&path).unwrap().unwrap();
        let allow = result["permissions"]["allow"].as_array().unwrap();
        assert_eq!(allow, &[json!("Bash(git:*)"), json!("Read")]);
        assert_eq!(result["permissions"]["defaultMode"], "acceptEdits");
        assert!(result["hooks"]["Stop"].is_array());
    }

    #[test]
    fn remove_permissions_noop_when_no_dispatch_perms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let existing = json!({
            "permissions": {
                "allow": ["Bash(git:*)", "Read"]
            }
        });
        write_json_file(&path, &existing).unwrap();

        let removed = remove_permissions(&path).unwrap();
        assert!(!removed);
    }

    #[test]
    fn remove_permissions_noop_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let removed = remove_permissions(&path).unwrap();
        assert!(!removed);
    }

    #[test]
    fn remove_permissions_handles_empty_allow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let existing = json!({
            "permissions": {
                "allow": []
            }
        });
        write_json_file(&path, &existing).unwrap();

        let removed = remove_permissions(&path).unwrap();
        assert!(!removed);
    }
}
