//! First-run setup: MCP config merging, plugin installation (hooks, skills, commands).
//!
//! Split into submodules:
//! - `config` — Claude Code MCP config read/write/merge
//! - `plugins` — embedded plugin install (skills, slash commands, hooks, example feed script)
//! - `hooks` — tests for the embedded hook scripts (the install path lives in `plugins`)

mod config;
mod config_update;
mod confirm;
mod hooks;
mod plugins;
#[cfg(test)]
mod purge_tests;
pub(crate) mod statusline;
mod uninstall;

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::tmux;

pub(crate) use config::dispatch_entry_identifying;
pub use config::{has_dispatch_entry, merge_mcp_config, remove_mcp_config, MergeResult};
pub use config_update::*;
pub use confirm::*;
pub(crate) use plugins::built_in_skills_dir;
pub use plugins::remove_plugin;
pub use uninstall::*;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Path to Claude Code's user-global configuration directory (`~/.claude`).
///
/// Every caller resolves this once and then passes the result around — see
/// `SetupPaths::resolve`, [`UninstallPaths::resolve`] and
/// `runtime::StartupPaths::resolve`. Nothing re-derives it mid-flow, which is
/// what lets a test point a whole flow at a temp directory.
///
/// The directory's name comes from `crate::claude_paths`, the one place it is
/// written down — the spawn constant expands the same token. `$HOME` is the
/// other half of the agreement: the spawn constant names this directory as
/// `~/…`, which the launching shell resolves under the home directory, so the
/// two halves agree only while this lookup reads nothing but `$HOME`. Adding a
/// second input here (an operator-settable configuration directory, say) parts
/// them silently for whoever sets it, and the spawn constant would have to stop
/// naming the location outright in the same change. See
/// docs/specs/dispatch.allium:
/// `SpawnSitesAndStartupNameTheSameConfigurationDirectory`.
pub(super) fn claude_dir() -> Result<PathBuf> {
    Ok(claude_dir_in(&home_dir()?))
}

/// [`claude_dir`] under a given home directory.
pub(super) fn claude_dir_in(home: &std::path::Path) -> PathBuf {
    home.join(crate::claude_paths::claude_dir_name!())
}

/// Path to Claude Code's user-global config file (`~/.claude.json`).
///
/// This is where Claude Code reads user-level MCP servers from — *not*
/// `~/.claude/.mcp.json`, which Claude Code does not consume. It is also
/// Claude Code's trust store, which is what the trust-gated dispatch arms read
/// and write through `TuiRuntime::claude_json_path`; the two consumers share
/// this one resolution rather than each deriving the location. See
/// docs/specs/dispatch.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
pub(super) fn user_global_config_path() -> Result<PathBuf> {
    Ok(user_global_config_path_in(&home_dir()?))
}

/// [`user_global_config_path`] under a given home directory.
///
/// Beside the configuration directory rather than inside it. Its name is
/// stated here and not among the `crate::claude_paths` tokens because nothing
/// launches a session from it: those tokens exist so the spawn-side `~/`
/// literals and the writer-side joins cannot drift apart, and this file has no
/// spawn-side half to agree with.
pub(super) fn user_global_config_path_in(home: &std::path::Path) -> PathBuf {
    home.join(".claude.json")
}

/// The operator's home directory, from a `$HOME` *value* — `None` when the
/// variable is unset.
///
/// An empty value is unavailable, not the root: `export HOME=` is a shell's
/// other spelling of "unset", and `PathBuf::from("").join(..)` yields a path
/// relative to the process's working directory, which is not the operator's
/// anything. Taking the value as a parameter is what makes that testable —
/// `std::env::set_var` is `unsafe` in edition 2024 and races the test
/// harness's threads regardless (see `src/editor.rs::resolve_editor`, the same
/// shape for the same reason).
pub(super) fn home_dir_from_value(home: Option<&str>) -> Result<PathBuf> {
    home.filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .context("$HOME is not set")
}

/// [`home_dir_from_value`] against the real process environment.
///
/// The one reader of `$HOME` behind every location `SetupPaths`,
/// `UninstallPaths` and `runtime::StartupPaths` are built from, so an absent
/// home directory is reported the same way whichever of them a run asked for.
/// Nothing downstream can name the cause — see docs/specs/dispatch.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
///
/// Not every `$HOME` reader in the crate: `crate::models::expand_tilde` and
/// `crate::default_data_dir` resolve their own, and neither fails — they are
/// string-in/string-out and fall back to a default respectively, so they have
/// no `Result` to report an absence through.
pub(super) fn home_dir() -> Result<PathBuf> {
    home_dir_from_value(std::env::var("HOME").ok().as_deref())
}

pub(super) fn read_json_file(path: &std::path::Path) -> Result<Option<Value>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let value: Value = serde_json::from_str(&content)
                .with_context(|| format!("Invalid JSON in {}", path.display()))?;
            Ok(Some(value))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Failed to read {}", path.display())),
    }
}

pub(super) fn write_json_file(path: &std::path::Path, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;
    crate::atomic_write::write_bytes(
        path,
        crate::atomic_write::Mode::KeepOr { default: 0o600 },
        (content + "\n").as_bytes(),
    )
    .with_context(|| format!("Failed to write {}", path.display()))
}

/// How often [`update_json_file`] re-merges after the file changed under it.
const UPDATE_JSON_ATTEMPTS: usize = 5;

/// Read `path`, let `merge` produce the new value (`None` = nothing to write),
/// and write it back, re-merging if the file changed between our read and the
/// write. Claude Code rewrites `~/.claude.json` often; a plain read-merge-write
/// would drop a write that landed in between. Returns whether it wrote.
pub(super) fn update_json_file(
    path: &std::path::Path,
    merge: impl FnMut(Option<Value>) -> Option<Value>,
) -> Result<bool> {
    update_json_file_with(path, merge, || {})
}

/// [`update_json_file`] with a hook that runs after the merge and before the
/// compare, so a test can land a concurrent write in that window.
fn update_json_file_with(
    path: &std::path::Path,
    mut merge: impl FnMut(Option<Value>) -> Option<Value>,
    mut before_check: impl FnMut(),
) -> Result<bool> {
    let read_raw = |path: &std::path::Path| match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Failed to read {}", path.display())),
    };
    for _ in 0..UPDATE_JSON_ATTEMPTS {
        let raw = read_raw(path)?;
        let existing = match &raw {
            Some(bytes) => Some(
                serde_json::from_slice::<Value>(bytes)
                    .with_context(|| format!("Invalid JSON in {}", path.display()))?,
            ),
            None => None,
        };
        let Some(merged) = merge(existing) else {
            return Ok(false);
        };
        before_check();
        if read_raw(path)? != raw {
            continue;
        }
        write_json_file(path, &merged)?;
        return Ok(true);
    }
    anyhow::bail!(
        "{} kept changing while it was being updated; gave up after {UPDATE_JSON_ATTEMPTS} attempts",
        path.display()
    )
}

/// Whether `path` already holds exactly `content` — the single definition of "this
/// file is already up to date" for everything setup manages.
///
/// Two callers must agree on it or setup contradicts itself:
/// [`write_file_if_changed`], which decides whether to write and what to report,
/// and `plugins::plugin_needs_update_in`, which decides whether to *offer* an
/// update. A file the predicate calls stale and the writer calls unchanged would
/// be reported as needing an update forever.
///
/// **Exact bytes, deliberately.** The statusline settings file previously compared
/// `.trim()`-normalized, which is what made the two disagree. Exact equality also
/// rewrites a file something else edited back to the canonical content, and
/// converges rather than flip-flopping. An unreadable or absent file is not up to
/// date: the writer's own error is a better report than a read error here.
fn file_is_up_to_date(path: &std::path::Path, content: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|existing| existing == content)
}

/// Write `content` at `path`, creating parent directories, and report whether the
/// on-disk bytes actually changed. Every setup-managed file goes through this, so
/// setup can be run repeatedly and only report what it really touched.
///
/// Shared by the plugin installer (`plugins::install_dir_recursive`) and the
/// statusline settings file (`statusline::write_settings_file`) — it lives here
/// rather than in either of them so there is one place to change what writing
/// means. Up-to-dateness is [`file_is_up_to_date`]'s to define.
///
/// Deliberately unmarked rather than `pub(…)`: a private item in this module is
/// already reachable from every child that needs it, and a filesystem write is
/// not something the rest of the crate should be able to name.
fn write_file_if_changed(path: &std::path::Path, content: &str, executable: bool) -> Result<bool> {
    if file_is_up_to_date(path, content) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    // Atomic, with the final mode set at creation: a live agent's hook or
    // `--settings` read never sees a truncated or not-yet-executable file.
    let mode = if executable { 0o755 } else { 0o644 };
    crate::atomic_write::write_bytes(
        path,
        crate::atomic_write::Mode::Fixed(mode),
        content.as_bytes(),
    )
    .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Confirmation seam (mirrors `ProcessRunner` in src/process.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
