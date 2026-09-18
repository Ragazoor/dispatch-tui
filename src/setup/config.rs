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

use crate::process::DISPATCH_PROGRAM;

/// Resolve the `headersHelper` command the dispatch MCP entry records, by
/// looking the installed `dispatch` binary up on the operator's `PATH`.
///
/// **Never `current_exe()`.** The recorded command outlives the run that wrote
/// it, and the runs reaching the startup configuration check are not all the
/// operator's installed one — a worktree build reaches it on the ordinary
/// `dispatch tui` path. Recording the running binary let such a run point the
/// operator's live MCP entry inside a worktree that is then removed. Resolving
/// from `PATH` instead makes the value independent of which build ran, which is
/// also what stops the drift check reporting this artefact stale on every
/// development launch. It is stable across builds, not across environments:
/// the answer is this process's own `PATH` order, so two shells that order it
/// differently still disagree. See `startup.allium`'s
/// `TheHelperPathNamesTheInstalledBinary`, which argues why that is accepted.
///
/// An empty `PATH` and a `PATH` holding no installed `dispatch` are the same
/// answer, which is why this takes no `Option`: the bare command name, stable
/// across runs, resolving if and when the operator installs one. What counts as
/// installed is [`is_installed_binary`], which rejects a relative entry as well
/// as an unrunnable file.
///
/// Takes the variable rather than reading it, so the reading happens once —
/// beside the check's other machine-fixed values, in `SetupPaths::under` — and
/// every consumer is handed the result. Deliberately NOT the handed-in shape of
/// observability.allium's `SettingsLocationIsAnExplicitStartupInput`: that
/// exists to keep a run out of the operator's configuration, and this value
/// names no destination. See `startup.allium`'s
/// `TheHelperPathNamesTheInstalledBinary`.
pub(crate) fn caller_headers_command_in(path: &std::ffi::OsStr) -> String {
    let installed = std::env::split_paths(path)
        .map(|dir| dir.join(DISPATCH_PROGRAM))
        .find(|candidate| is_installed_binary(candidate))
        .and_then(|candidate| candidate.to_str().map(str::to_owned))
        .unwrap_or_else(|| DISPATCH_PROGRAM.to_string());

    format!("{installed} caller-headers")
}

/// The same command, resolved from this process's environment. The only reading
/// of `PATH` behind this value — every consumer of the command is handed the
/// result rather than resolving one.
pub(crate) fn caller_headers_command() -> String {
    caller_headers_command_in(&std::env::var_os("PATH").unwrap_or_default())
}

/// Whether `path` names an installed binary this helper may be recorded as.
///
/// Absolute, because the recorded command is run by Claude Code from ITS own
/// working directory. A relative `PATH` entry — `.`, or the `./target/debug` a
/// development shell may carry — would resolve somewhere else there, or
/// nowhere: the same vanishing-path hazard this resolution exists for, in a
/// form that also moves with the caller's directory.
///
/// A regular file, and executable: a file named `dispatch` that cannot be run
/// is not an installed binary either, and recording it would produce a helper
/// Claude Code fails to invoke.
fn is_installed_binary(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// `caller_headers_command` is the `headers_helper` every caller must hand in;
/// see `caller_headers_command_in` for why it is never derived here.
///
/// `port` sits beside it under the OPPOSITE rule, deliberately: the entry's
/// address names the board this invocation is starting, which is what a launch
/// on a different port is asking for, while the helper identifies a session to
/// whichever board it reaches. Making `port` handed-in too would undo a
/// decision rather than extend one — see the closing paragraph of
/// `startup.allium`'s `TheHelperPathNamesTheInstalledBinary`.
pub fn merge_mcp_config(existing: Option<Value>, port: u16, headers_helper: &str) -> MergeResult {
    let server_entry = json!({
        "type": "http",
        "url": format!("http://localhost:{port}/mcp"),
        // Serves non-dispatched sessions only. A dispatched agent's launch
        // overrides this entry with one that drops this key and carries a fixed
        // caller-task header instead — see `dispatch_entry_identifying` below,
        // and `dispatch::caller_identity` for why the helper cannot answer for
        // an agent. Removing the strip without removing this would put both
        // identity headers on the wire, which is rejected outright.
        "headersHelper": headers_helper,
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

    /// Stands in for the installed binary's command, so a merge test asserts
    /// about the entry rather than about this machine's PATH.
    const INSTALLED: &str = "/usr/local/bin/dispatch caller-headers";

    // -- The installed caller-headers command --

    /// Make `dir/dispatch` exist and be executable, and yield its path.
    fn install_fake_dispatch(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("dispatch");
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn path_var<P: AsRef<std::path::Path>>(dirs: &[P]) -> std::ffi::OsString {
        std::env::join_paths(dirs.iter().map(|d| d.as_ref())).unwrap()
    }

    /// `dir` spelled relative to the test's working directory. Nothing changes
    /// the process's working directory to arrange this: that is global, and the
    /// suite runs in parallel.
    fn relative_to_working_dir(dir: &std::path::Path) -> std::path::PathBuf {
        let relative = dir
            .strip_prefix(std::env::current_dir().unwrap())
            .expect("the temp dir must sit under the working directory")
            .to_path_buf();
        assert!(
            relative.is_relative(),
            "the entry under test must be relative"
        );
        relative
    }

    /// The defect this rule exists for: a build running out of a worktree must
    /// record the SAME command the installed binary would, never its own path.
    /// `current_exe()` under `cargo test` is exactly such a build, so the test
    /// binary standing in for the installed one is the regression.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn caller_headers_command_names_the_binary_found_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = install_fake_dispatch(dir.path());
        let running = std::env::current_exe().unwrap();

        let command = caller_headers_command_in(&path_var(&[dir.path()]));

        assert_eq!(command, format!("{} caller-headers", bin.display()));
        assert!(
            !command.contains(running.to_str().unwrap()),
            "the running binary must not reach the recorded command, got {command}"
        );
    }

    /// No installed binary to name, so the bare command name is recorded — and
    /// still not the running one. It resolves if the operator installs one.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn caller_headers_command_falls_back_to_the_bare_name_when_nothing_is_installed() {
        let empty = tempfile::tempdir().unwrap();

        let command = caller_headers_command_in(&path_var(&[empty.path()]));

        assert_eq!(command, "dispatch caller-headers");
    }

    /// A file named `dispatch` that cannot be run is not an installed binary.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn caller_headers_command_skips_a_non_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dispatch"), b"not a program").unwrap();

        let command = caller_headers_command_in(&path_var(&[dir.path()]));

        assert_eq!(command, "dispatch caller-headers");
    }

    /// A relative PATH entry — `.`, or the `./target/debug` a development shell
    /// may carry — resolves against the CALLER's working directory, and Claude
    /// Code runs this helper from its own. Recording one is the same hazard
    /// this rule exists for, in a form that also moves with the cwd, so a
    /// relative candidate is skipped and the bare name recorded instead.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn caller_headers_command_skips_a_relative_path_entry() {
        // Under the test's own working directory, so the entry below names a
        // directory that really does hold a runnable `dispatch` — the skip has
        // to be the reason it is not recorded, not a failed lookup.
        let dir = tempfile::tempdir_in(".").unwrap();
        install_fake_dispatch(dir.path());
        let relative = relative_to_working_dir(dir.path());

        let command = caller_headers_command_in(&path_var(&[&relative]));

        assert_eq!(command, "dispatch caller-headers");
    }

    /// Skipping a relative entry must not abandon the search: an absolute entry
    /// behind it is still found. The relative entry really does hold a runnable
    /// `dispatch`, so this is distinct from merely stepping over an empty
    /// directory.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn caller_headers_command_looks_past_a_relative_entry() {
        let skipped = tempfile::tempdir_in(".").unwrap();
        install_fake_dispatch(skipped.path());
        let relative = relative_to_working_dir(skipped.path());
        let installed = tempfile::tempdir().unwrap();
        let bin = install_fake_dispatch(installed.path());

        let command =
            caller_headers_command_in(&path_var(&[&relative, &installed.path().to_path_buf()]));

        assert_eq!(command, format!("{} caller-headers", bin.display()));
    }

    /// PATH order decides, as it does for the operator's own shell.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn caller_headers_command_takes_the_first_path_entry_holding_it() {
        let empty = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let wanted = install_fake_dispatch(first.path());
        install_fake_dispatch(second.path());

        let command =
            caller_headers_command_in(&path_var(&[empty.path(), first.path(), second.path()]));

        assert_eq!(command, format!("{} caller-headers", wanted.display()));
    }

    /// The merge records the command it is handed, verbatim. Handing it in
    /// rather than looking it up is the whole of the fix — there is no
    /// remaining path by which the running binary could reach the entry.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
    #[test]
    fn merge_mcp_config_records_the_command_it_is_given() {
        let result = merge_mcp_config(None, DEFAULT_PORT, "/opt/dispatch caller-headers");

        assert_eq!(
            result.value["mcpServers"]["dispatch"]["headersHelper"],
            "/opt/dispatch caller-headers"
        );
    }

    /// A recorded command that no longer matches the installed one IS drift,
    /// and the merge repairs it — which is how an operator whose entry already
    /// points into a removed worktree gets it back.
    ///
    /// docs/specs/startup.allium: `TheHelperPathNamesTheInstalledBinary`.
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

        let result = merge_mcp_config(
            existing,
            DEFAULT_PORT,
            "/usr/local/bin/dispatch caller-headers",
        );

        assert!(result.changed);
        assert_eq!(
            result.value["mcpServers"]["dispatch"]["headersHelper"],
            "/usr/local/bin/dispatch caller-headers"
        );
    }

    // -- MCP config merging --

    #[test]
    fn merge_mcp_config_into_empty() {
        let result = merge_mcp_config(None, DEFAULT_PORT, INSTALLED);
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
        let result = merge_mcp_config(None, DEFAULT_PORT, INSTALLED);
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
        let result = merge_mcp_config(existing, DEFAULT_PORT, INSTALLED);
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
        let first = merge_mcp_config(None, DEFAULT_PORT, INSTALLED);
        let result = merge_mcp_config(Some(first.value.clone()), DEFAULT_PORT, INSTALLED);
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
        let result = merge_mcp_config(existing, DEFAULT_PORT, INSTALLED);
        assert!(result.changed);
        assert!(result.value["mcpServers"]["dispatch"]["headersHelper"].is_string());
    }

    #[test]
    fn merge_mcp_config_custom_port() {
        let result = merge_mcp_config(None, 4000, INSTALLED);
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
