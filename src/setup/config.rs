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
        json!({ crate::mcp::identity::HEADER_TASK_ID: task_id.to_string() }),
    );
    Some(json!({ "mcpServers": { SERVER_NAME: Value::Object(entry) } }))
}

use crate::process::dispatch_program;

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
/// command strings the same Claude Code process runs under the same `PATH`, and
/// both compose their program name from `process::dispatch_program`, so their
/// agreeing is the compiler's business rather than a test's.
///
/// See `startup.allium`'s `TheHelperIsTheBareCommandName`, which also states
/// what this gives up.
pub(crate) const CALLER_HEADERS_COMMAND: &str = concat!(dispatch_program!(), " caller-headers");

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
    // A non-object `mcpServers` cannot hold the entry; replace it so the merge
    // converges instead of reporting a change it never makes.
    if !servers.is_object() {
        *servers = json!({});
    }

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
mod tests;
