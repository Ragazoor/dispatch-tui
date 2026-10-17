use super::*;
use crate::DEFAULT_PORT;
use serde_json::json;

/// The merge composes the helper from the constant and from nothing else.
/// There is no input by which the running binary, the operator's PATH or
/// the working directory could reach the entry.
///
/// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
#[test]
fn merge_mcp_config_records_the_bare_helper_command() {
    let result = merge_mcp_config(None, DEFAULT_PORT);

    assert_eq!(
        result.value["mcpServers"]["dispatch"]["headersHelper"],
        "dispatch caller-headers"
    );
}

/// Both abandoned derivations leave an absolute helper behind, and the
/// merge migrates each of them: the worktree path a pre-fix build wrote,
/// which stopped resolving the moment the worktree was removed, and the
/// perfectly valid installed path the PATH-resolving version wrote.
///
/// Each is rewritten ONCE. The value it becomes is the same on every
/// machine and in every shell, so no later run finds it stale again —
/// which is what `OneDefinitionOfOutOfDate` and `ApplyIsIdempotent` need
/// from this artefact.
///
/// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
#[test]
fn merge_mcp_config_migrates_an_absolute_helper_once() {
    for stale in [
        "/home/o/repo/.worktrees/42-x/target/debug/dispatch caller-headers",
        "/usr/local/bin/dispatch caller-headers",
    ] {
        let existing = Some(json!({
            "mcpServers": {
                "dispatch": {
                    "type": "http",
                    "url": format!("http://localhost:{DEFAULT_PORT}/mcp"),
                    "headersHelper": stale,
                }
            }
        }));

        let migrated = merge_mcp_config(existing, DEFAULT_PORT);
        assert!(migrated.changed, "{stale} must be rewritten");
        assert_eq!(
            migrated.value["mcpServers"]["dispatch"]["headersHelper"],
            "dispatch caller-headers"
        );

        let settled = merge_mcp_config(Some(migrated.value), DEFAULT_PORT);
        assert!(
            !settled.changed,
            "the entry rewritten over {stale} must be current, not rewritten again"
        );
    }
}

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
    assert!(
        !result.changed,
        "second merge with identical input must be a no-op"
    );
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

#[test]
fn remove_mcp_config_noop_when_root_is_not_object() {
    // File exists but top-level JSON is an array, not an object.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".mcp.json");
    // Write raw bytes since write_json_file expects an object Value
    std::fs::write(&path, "[1,2,3]\n").unwrap();

    let removed = remove_mcp_config(&path).unwrap();
    assert!(
        !removed,
        "non-object root JSON must be treated as no-op (no dispatch to remove)"
    );
}

#[test]
fn remove_mcp_config_noop_when_mcp_servers_missing() {
    // Valid JSON object but no "mcpServers" key at all.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".mcp.json");
    write_json_file(&path, &json!({"otherKey": "value"})).unwrap();

    let removed = remove_mcp_config(&path).unwrap();
    assert!(!removed, "missing mcpServers must be treated as no-op");
}

#[test]
fn merge_mcp_config_replaces_a_non_object_mcp_servers() {
    let result = merge_mcp_config(Some(json!({"mcpServers": "oops"})), DEFAULT_PORT);
    assert!(result.changed);
    assert!(result.value["mcpServers"]["dispatch"].is_object());
    // A second pass is a no-op, so the user is not prompted every launch.
    assert!(!merge_mcp_config(Some(result.value), DEFAULT_PORT).changed);
}
