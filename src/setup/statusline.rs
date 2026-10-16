//! Generates the dispatch-owned statusLine settings file that is injected into
//! every dispatch-spawned Claude session via `--settings`.
//!
//! The file lives at a path fixed at compile time
//! (`~/.claude/dispatch-statusline.json`), so the spawn constant in
//! `src/dispatch/prompts.rs` stays a `const` whose flags cannot go missing.
//! Runtime paths live inside this file instead, where they are quoted properly.
//! The name itself is not written here — it expands from `crate::claude_paths`,
//! the same token the spawn constant uses.
//!
//! Note it is NOT placed under the plugin dir: `remove_stale_files` deletes any
//! non-embedded file there. And it is NOT `~/.claude/settings.json`, which
//! `src/setup/mod.rs` deliberately never writes.

use anyhow::{Context, Result};
use serde_json::json;
use std::path::Path;

use crate::process::dispatch_program;

/// The fixed file name, under the resolved `~/.claude` directory.
///
/// `pub(crate)`: also read by the startup configuration check, which reports
/// this file as drift when its content differs from what the current build
/// would write — absence included — and rewrites it once the operator agrees.
/// See `docs/specs/startup.allium`.
pub(crate) const SETTINGS_FILE_NAME: &str = crate::claude_paths::statusline_settings_name!();

/// The settings file's full path under a resolved `~/.claude` directory.
///
/// One helper rather than an open-coded `join` at each site, so every caller
/// agrees on the layout: setup, uninstall, TUI startup and their tests.
pub(crate) fn settings_path(claude_dir: &Path) -> std::path::PathBuf {
    claude_dir.join(SETTINGS_FILE_NAME)
}

/// The fixed snapshot file name, beside the *default* database — never beside
/// whichever database the current process has open.
///
/// Read from exactly one place, [`crate::budget_snapshot_path`], which both the
/// publisher (the `--snapshot` argument baked into the settings file) and the
/// reader (the TUI's budget indicator) go through. Routing both through one
/// function is what makes them agree: if they ever disagreed the badge would
/// silently go stale — indistinguishable from "no subscription data". See
/// `docs/specs/observability.allium`:
/// `SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase`.
///
/// Tests deliberately keep their own literal instead of importing this: an
/// expectation derived from the same constant as the code under test asserts
/// nothing.
pub(crate) const RATE_LIMITS_FILE_NAME: &str = "rate-limits.json";

/// POSIX single-quoting: wrap in `'…'` and replace each embedded `'` with
/// `'\''`. The generated string is run through `sh -c`, so an unquoted path
/// containing a space would split into two arguments.
pub(super) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The `<program> statusline` prefix, composed from the one name this crate's
/// binary answers to.
///
/// Both the command this module writes and the recursion guard in
/// [`discover_chain`] go through it, so a rename of the binary cannot leave the
/// guard matching a command dispatch no longer writes. It is also what keeps
/// this artefact spelling the same bare program name the MCP entry's helper
/// does — see `startup.allium`'s `TheHelperIsTheBareCommandName`, which treats
/// their agreement as deliberate.
const STATUSLINE_INVOCATION: &str = concat!(dispatch_program!(), " statusline");

/// Build the statusLine command string.
///
/// A BARE command name, resolved on `PATH` by Claude Code when it runs the
/// line. Never an absolute path: see `startup.allium`'s
/// `TheHelperIsTheBareCommandName` for why the MCP entry's helper now agrees.
pub(crate) fn build_command(snapshot_path: &Path, chain: Option<&str>) -> String {
    let mut cmd = format!(
        "{STATUSLINE_INVOCATION} --snapshot {}",
        shell_quote(&snapshot_path.display().to_string())
    );
    if let Some(chain) = chain {
        cmd.push_str(&format!(" --chain {}", shell_quote(chain)));
    }
    cmd
}

/// Read the user's current `statusLine.command` so the decorator can chain to
/// it. Read-only — this never writes `settings.json`.
///
/// Returns `None` when there is nothing to chain, including the
/// **recursion-guard** case where the user's command is already a
/// `dispatch statusline` invocation. Chaining to ourselves would loop; the
/// honest outcome is an empty status line, with the reporter still running.
pub(crate) fn discover_chain(claude_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(claude_dir.join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let command = value
        .get("statusLine")?
        .get("command")?
        .as_str()?
        .trim()
        .to_string();
    if command.is_empty() || command.contains(STATUSLINE_INVOCATION) {
        return None;
    }
    Some(command)
}

/// Write the settings file. Returns whether the on-disk content changed, so
/// setup can report accurately and stay idempotent.
///
/// The write-if-changed contract itself lives in [`super::write_file_if_changed`],
/// shared with the plugin installer — including the parent-directory creation
/// this path needs and the exact-bytes comparison both now use.
pub(crate) fn write_settings_file(
    path: &Path,
    snapshot_path: &Path,
    chain: Option<&str>,
    port: u16,
) -> Result<bool> {
    super::write_file_if_changed(path, &settings_content(snapshot_path, chain, port)?, false)
}

/// The exact bytes [`write_settings_file`] would write.
///
/// Extracted so the drift check and the writer cannot disagree about what the
/// file should contain — `startup.allium`'s `OneDefinitionOfOutOfDate`. A
/// second copy of this literal is precisely the drift the invariant forbids.
pub(crate) fn settings_content(
    snapshot_path: &Path,
    chain: Option<&str>,
    port: u16,
) -> Result<String> {
    serde_json::to_string_pretty(&json!({
        "statusLine": {
            "type": "command",
            "command": build_command(snapshot_path, chain),
        },
        "sandbox": {
            "enabled": false,
        },
        // Which board the session belongs to. The Claude Code hooks running
        // inside it read this to find one (`src/hooks/`), the same way the
        // MCP entry carries the port in its url — without it a board started
        // on a non-default port serves agents whose every hook event is
        // dropped. See `HookDelivery` in `docs/specs/agent-health.allium`.
        "env": {
            "DISPATCH_PORT": port.to_string(),
        }
    }))
    .context("failed to serialize statusline settings")
}

/// Whether the settings file already holds what this build would write.
/// Reads only — nothing here creates the file or its parent.
pub(crate) fn settings_up_to_date(
    path: &Path,
    snapshot_path: &Path,
    chain: Option<&str>,
    port: u16,
) -> bool {
    settings_content(snapshot_path, chain, port)
        .is_ok_and(|content| super::file_is_up_to_date(path, &content))
}

#[cfg(test)]
mod tests;
