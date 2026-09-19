//! Claude Code MCP server config: read, merge, and remove.

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::{read_json_file, write_json_file};
use crate::models::TaskId;

// ---------------------------------------------------------------------------
// MCP config merging
// ---------------------------------------------------------------------------

/// The name of the MCP server entry dispatch owns, in Claude Code's config.
///
/// Written here and matched by `dispatch::caller_identity`, whose per-task
/// entry must reuse it in order to OVERRIDE this one rather than sit beside it.
/// Renaming it in only one of the two places drops every agent back to no
/// caller identity, and says so nowhere the compiler can see — which is why
/// both ends read this constant.
pub(crate) const SERVER_NAME: &str = "dispatch";

pub struct MergeResult {
    pub value: Value,
    pub changed: bool,
}

/// The installed `dispatch` entry, restated to identify `task_id` by a fixed
/// header instead of by a helper.
///
/// Read-back of what [`merge_mcp_config`] wrote, and it lives beside it so the
/// two cannot drift: this knows the `mcpServers` key path, the server name, and
/// which key it has to strip.
///
/// Deriving from the installed entry rather than composing a fresh one is what
/// keeps the transport and URL — a non-default port above all — from having to
/// be taught to a second place.
///
/// Dropping `headersHelper` is required, not tidiness: with both present the
/// two would answer the same request, and a request carrying both identity
/// headers is rejected outright (`CallerIdentity::from_headers` → `Conflict`).
///
/// `None` when there is no config to read, or no `dispatch` entry in it.
pub(crate) fn dispatch_entry_identifying(
    claude_json: &std::path::Path,
    task_id: TaskId,
) -> Option<Value> {
    let json = read_json_file(claude_json).ok().flatten()?;
    let mut entry = dispatch_entry(&json)?.as_object().cloned()?;
    entry.remove("headersHelper");
    entry.insert(
        "headers".to_string(),
        json!({ crate::mcp::identity::HEADER_TASK_ID: task_id.0.to_string() }),
    );
    Some(json!({ "mcpServers": { SERVER_NAME: Value::Object(entry) } }))
}

/// The `headersHelper` command the dispatch MCP entry records: the bare command
/// name and its subcommand, naming no directory.
///
/// **Nothing about the run that writes it may reach this value** — not
/// `current_exe()`, not `PATH`, not the working directory. Resolving it to a
/// file is Claude Code's, under Claude Code's own `PATH`, at the moment it
/// invokes the command.
///
/// This is a constant because the recorded entry outlives the run that wrote
/// it, and the runs that reach the startup configuration check are not all the
/// operator's installed one — a worktree build reaches it on the ordinary
/// `dispatch tui` path. Recording `current_exe()` once let such a run point the
/// operator's live entry inside a worktree that was then removed. Looking
/// `dispatch` up on `PATH` at startup closed that, but recorded the *launching
/// shell's* answer, so two shells ordering `PATH` differently disagreed and
/// each found the other's entry stale — a rewrite prompt for a difference that
/// is not drift. A constant has neither axis.
///
/// The status line is the precedent, not a counter-example: `statusline.rs`
/// records a bare `dispatch statusline` invocation and always has. Both are
/// command strings the same Claude Code process runs under the same `PATH`.
///
/// See `startup.allium`'s `TheHelperIsTheBareCommandName`, which also states
/// what this gives up.
pub(crate) const CALLER_HEADERS_COMMAND: &str = "dispatch caller-headers";

/// The MCP entry's `headersHelper` is [`CALLER_HEADERS_COMMAND`], a constant.
///
/// `port` sits beside it under the OPPOSITE rule, deliberately: the entry's
/// address names the board this invocation is starting, which is what a launch
/// on a different port is asking for, while the helper identifies a session to
/// whichever board it reaches. See the closing paragraph of `startup.allium`'s
/// `TheHelperIsTheBareCommandName`.
pub fn merge_mcp_config(existing: Option<Value>, port: u16) -> MergeResult {
    let server_entry = json!({
        "type": "http",
        "url": format!("http://localhost:{port}/mcp"),
        // Serves non-dispatched sessions only. A dispatched agent's launch
        // overrides this entry with one that drops this key and carries a fixed
        // caller-task header instead — see `dispatch_entry_identifying` below,
        // and `dispatch::caller_identity` for why the helper cannot answer for
        // an agent. Removing the strip without removing this would put both
        // identity headers on the wire, which is rejected outright.
        "headersHelper": CALLER_HEADERS_COMMAND,
    });

    let mut root = match existing {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };

    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));

    if let Value::Object(servers_map) = servers {
        if servers_map.get(SERVER_NAME) == Some(&server_entry) {
            return MergeResult {
                value: Value::Object(root),
                changed: false,
            };
        }
        servers_map.insert(SERVER_NAME.to_string(), server_entry);
    }

    MergeResult {
        value: Value::Object(root),
        changed: true,
    }
}

// ---------------------------------------------------------------------------
// Removal
// ---------------------------------------------------------------------------

/// The dispatch entry inside a parsed Claude Code config, or `None` when the
/// shape does not hold one.
///
/// The one traversal of `mcpServers` → `dispatch`. Every reader goes through it
/// rather than re-walking the path by hand, so they cannot disagree about what
/// counts as an entry being present.
fn dispatch_entry(root: &Value) -> Option<&Value> {
    root.get("mcpServers")?.get(SERVER_NAME)
}

/// Whether `mcp_path` still carries a dispatch entry that wants removing.
///
/// The read-only half of [`remove_mcp_config`]: the drift check must be able to
/// ask "is there a stale legacy entry?" without writing, and the two must agree
/// about the answer (`startup.allium`'s `OneDefinitionOfOutOfDate`). An
/// unreadable or absent file carries nothing to remove.
pub fn has_dispatch_entry(mcp_path: &std::path::Path) -> bool {
    read_json_file(mcp_path)
        .ok()
        .flatten()
        .is_some_and(|root| dispatch_entry(&root).is_some())
}

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
        servers.remove(SERVER_NAME).is_some()
    } else {
        false
    };

    if had_dispatch {
        write_json_file(mcp_path, &Value::Object(root))?;
    }

    Ok(had_dispatch)
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

    /// The recorded command must name no directory at all. Asserting the
    /// literal back would assert nothing; what the rule actually forbids is a
    /// value that could have come from this run — a path separator, or any part
    /// of the binary running the test.
    ///
    /// `current_exe()` under `cargo test` stands in for the worktree build that
    /// caused the original defect, so its absence is the regression.
    ///
    /// The constant spells the same program name every other spawn uses. It is
    /// written out rather than composed, so nothing but a test ties the two
    /// together — and a rename of one without the other would otherwise record
    /// a helper Claude Code cannot invoke.
    ///
    /// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
    #[test]
    fn caller_headers_command_invokes_the_dispatch_program() {
        assert_eq!(
            CALLER_HEADERS_COMMAND,
            format!("{} caller-headers", crate::process::DISPATCH_PROGRAM)
        );
    }

    /// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
    #[test]
    fn caller_headers_command_names_no_directory() {
        let running = std::env::current_exe().unwrap();

        assert!(
            !CALLER_HEADERS_COMMAND.contains(std::path::MAIN_SEPARATOR),
            "the helper command must be a bare command name, got {CALLER_HEADERS_COMMAND}"
        );
        assert!(
            !CALLER_HEADERS_COMMAND.contains(running.to_str().unwrap()),
            "the running binary must not reach the recorded command, got \
             {CALLER_HEADERS_COMMAND}"
        );
        assert!(
            CALLER_HEADERS_COMMAND.ends_with(" caller-headers"),
            "the helper command must invoke the caller-headers subcommand, got \
             {CALLER_HEADERS_COMMAND}"
        );
    }

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

    /// An entry pointing into a worktree that has since been removed is drift,
    /// and the merge repairs it — which is how an operator left with a broken
    /// helper by a pre-fix build gets it back.
    ///
    /// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
    #[test]
    fn merge_mcp_config_repairs_a_helper_pointing_at_a_vanished_build() {
        let existing = Some(json!({
            "mcpServers": {
                "dispatch": {
                    "type": "http",
                    "url": format!("http://localhost:{DEFAULT_PORT}/mcp"),
                    "headersHelper": "/home/o/repo/.worktrees/42-x/target/debug/dispatch caller-headers",
                }
            }
        }));

        let result = merge_mcp_config(existing, DEFAULT_PORT);

        assert!(result.changed);
        assert_eq!(
            result.value["mcpServers"]["dispatch"]["headersHelper"],
            "dispatch caller-headers"
        );
    }

    /// The migration the bare form itself requires: an entry left by the
    /// PATH-resolving version is a perfectly valid absolute path, and is still
    /// rewritten. It is rewritten ONCE — the value it becomes is the same on
    /// every machine and every shell, so no later run finds it stale again.
    ///
    /// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
    #[test]
    fn merge_mcp_config_replaces_a_path_resolved_helper_once() {
        let existing = Some(json!({
            "mcpServers": {
                "dispatch": {
                    "type": "http",
                    "url": format!("http://localhost:{DEFAULT_PORT}/mcp"),
                    "headersHelper": "/usr/local/bin/dispatch caller-headers",
                }
            }
        }));

        let migrated = merge_mcp_config(existing, DEFAULT_PORT);
        assert!(migrated.changed, "an absolute helper must be rewritten");
        assert_eq!(
            migrated.value["mcpServers"]["dispatch"]["headersHelper"],
            "dispatch caller-headers"
        );

        let settled = merge_mcp_config(Some(migrated.value), DEFAULT_PORT);
        assert!(
            !settled.changed,
            "the rewritten entry must be current, not rewritten again"
        );
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
}
