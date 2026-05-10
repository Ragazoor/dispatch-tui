//! First-run setup: MCP config merging, plugin installation (hooks, skills, commands).
//!
//! Split into submodules:
//! - `config` — Claude Code MCP config + permissions read/write/merge
//! - `plugins` — embedded plugin install (skills, slash commands, hooks, example feed script)
//! - `hooks` — tests for the embedded hook scripts (the install path lives in `plugins`)

mod config;
mod hooks;
mod plugins;

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::db::Database;
use crate::process::RealProcessRunner;
use crate::tmux;

pub use config::{
    merge_mcp_config, merge_permissions, remove_mcp_config, remove_permissions, MergeResult,
    PermissionsMergeResult,
};
pub use plugins::{install_example_script, install_plugin, remove_plugin, seed_feed_epics};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn claude_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("$HOME is not set")?;
    Ok(PathBuf::from(home).join(".claude"))
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
    fs::write(path, content + "\n").with_context(|| format!("Failed to write {}", path.display()))
}

fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [Y/n] ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}

/// Like [`confirm`] but defaults to **No** — the user must explicitly type "y".
fn confirm_dangerous(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}

fn count_tasks(db_path: &std::path::Path) -> Result<i64> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
    Ok(count)
}

pub fn remove_database(db_path: &std::path::Path) -> Result<bool> {
    if !db_path.exists() {
        return Ok(false);
    }

    let parent = db_path
        .parent()
        .context("database path has no parent directory")?;

    for name in ["tasks.db", "tasks.db-wal", "tasks.db-shm", "app.log"] {
        let path = parent.join(name);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        }
    }

    if parent.exists() && parent.read_dir()?.next().is_none() {
        fs::remove_dir(parent).with_context(|| format!("Failed to remove {}", parent.display()))?;
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// run_setup — top-level orchestrator
// ---------------------------------------------------------------------------

pub async fn run_setup(port: u16, yes: bool, db_path: &Path) -> Result<()> {
    let db = Database::open(db_path).await?;
    let data_dir = db_path
        .parent()
        .context("database path has no parent directory")?;
    seed_feed_epics(&db, data_dir).await?;
    let claude_dir = claude_dir()?;
    fs::create_dir_all(&claude_dir)
        .with_context(|| format!("Failed to create {}", claude_dir.display()))?;

    let mut any_changes = false;

    // 1. MCP config
    let mcp_path = claude_dir.join(".mcp.json");
    let existing_mcp = read_json_file(&mcp_path)?;
    let mcp_result = merge_mcp_config(existing_mcp, port);
    if mcp_result.changed {
        if yes
            || confirm(&format!(
                "Add dispatch MCP server (localhost:{port}) to ~/.claude/.mcp.json?"
            ))?
        {
            write_json_file(&mcp_path, &mcp_result.value)?;
            println!("MCP config: added dispatch to ~/.claude/.mcp.json (port {port})");
            any_changes = true;
        } else {
            println!("MCP config: skipped");
        }
    } else {
        println!("MCP config: dispatch already configured in ~/.claude/.mcp.json");
    }

    // 2. Permissions
    let settings_path = claude_dir.join("settings.json");
    let existing_settings = read_json_file(&settings_path)?;
    let perms_result = merge_permissions(existing_settings);
    if perms_result.added_count > 0 {
        if yes
            || confirm(&format!(
                "Add {} dispatch tool permission(s) to ~/.claude/settings.json?",
                perms_result.added_count
            ))?
        {
            write_json_file(&settings_path, &perms_result.value)?;
            println!(
                "Permissions: added {} MCP tool permission(s) to ~/.claude/settings.json",
                perms_result.added_count
            );
            any_changes = true;
        } else {
            println!("Permissions: skipped");
        }
    } else {
        println!(
            "Permissions: all MCP tool permissions already present in ~/.claude/settings.json"
        );
    }

    // 3. Plugin (hooks, skills, commands)
    if plugins::plugin_needs_update()? {
        if yes || confirm("Install dispatch plugin (skills, hooks, commands) to ~/.claude/plugins/local/dispatch/?")? {
            install_plugin()?;
            println!("Plugin: installed dispatch plugin to ~/.claude/plugins/local/dispatch/");
            let skills: Vec<String> = plugins::PLUGIN_DIR
                .get_dir("skills")
                .map(|d| {
                    let mut names: Vec<String> = d
                        .dirs()
                        .filter_map(|sd| {
                            sd.path().file_name()?.to_str().map(|n| format!("/{n}"))
                        })
                        .collect();
                    names.sort();
                    names
                })
                .unwrap_or_default();
            println!("  → Skills: {}", skills.join(", "));
            let commands: Vec<String> = plugins::PLUGIN_DIR
                .get_dir("commands")
                .map(|d| {
                    let mut names: Vec<String> = d
                        .files()
                        .filter_map(|f| {
                            f.path()
                                .file_stem()?
                                .to_str()
                                .map(|n| format!("/{n}"))
                        })
                        .collect();
                    names.sort();
                    names
                })
                .unwrap_or_default();
            println!("  → Commands: {}", commands.join(", "));
            println!("  → Hooks: task-status, task-usage");
            any_changes = true;
        } else {
            println!("Plugin: skipped");
        }
    } else {
        println!("Plugin: dispatch plugin already up to date");
    }

    // 4. Tmux focus-events
    let runner = RealProcessRunner;
    if !tmux::focus_events_enabled(&runner) {
        if yes || confirm("Enable tmux focus-events? (will run `tmux set-option -g focus-events on` and add `set -g focus-events on` to ~/.tmux.conf)")? {
            tmux::set_focus_events(&runner)?;
            tmux::write_focus_events_to_tmux_conf()?;
            println!("Tmux: enabled focus-events (set for current server and added to ~/.tmux.conf)");
            any_changes = true;
        } else {
            println!("Tmux: focus-events skipped");
        }
    } else {
        tmux::write_focus_events_to_tmux_conf()?;
        println!("Tmux: focus-events already enabled (ensuring ~/.tmux.conf is up to date)");
    }

    if any_changes {
        println!("Setup complete.");
    } else {
        println!("Already configured, nothing to do.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// run_uninstall — reverse of run_setup
// ---------------------------------------------------------------------------

pub fn run_uninstall(yes: bool, purge: bool) -> Result<()> {
    let claude_dir = claude_dir()?;
    let mcp_path = claude_dir.join(".mcp.json");
    let settings_path = claude_dir.join("settings.json");
    let plugin_path = plugins::plugin_dir()?;
    let db_path = crate::default_db_path();

    // Show what will be removed
    eprintln!("This will remove:");
    eprintln!("  Plugin:      {}", plugin_path.display());
    eprintln!(
        "  MCP config:  mcpServers.dispatch from {}",
        mcp_path.display()
    );
    eprintln!(
        "  Permissions: mcp__dispatch__* from {}",
        settings_path.display()
    );
    if purge {
        eprintln!("  Database:    {}", db_path.display());
    }

    if !yes && !confirm("\nContinue?")? {
        println!("Aborted.");
        return Ok(());
    }

    let mut any_removed = false;

    match remove_plugin(&plugin_path) {
        Ok(true) => {
            println!("Removed plugin directory");
            any_removed = true;
        }
        Ok(false) => println!("Plugin directory not found, skipping"),
        Err(e) => eprintln!("Warning: failed to remove plugin: {e}"),
    }

    match remove_mcp_config(&mcp_path) {
        Ok(true) => {
            println!("Removed dispatch from MCP config");
            any_removed = true;
        }
        Ok(false) => println!("No dispatch entry in MCP config, skipping"),
        Err(e) => eprintln!("Warning: failed to update MCP config: {e}"),
    }

    match remove_permissions(&settings_path) {
        Ok(true) => {
            println!("Removed dispatch permissions");
            any_removed = true;
        }
        Ok(false) => println!("No dispatch permissions found, skipping"),
        Err(e) => eprintln!("Warning: failed to update permissions: {e}"),
    }

    if purge {
        if db_path.exists() {
            let task_count = count_tasks(&db_path).unwrap_or(0);
            eprintln!("\n  Database contains {task_count} task(s). This cannot be undone.");
            if confirm_dangerous("Delete database?")? {
                match remove_database(&db_path) {
                    Ok(true) => {
                        println!("Removed database");
                        any_removed = true;
                    }
                    Ok(false) => println!("Database not found, skipping"),
                    Err(e) => eprintln!("Warning: failed to remove database: {e}"),
                }
            } else {
                println!("Kept database.");
            }
        } else {
            println!("Database not found, skipping");
        }
    }

    if any_removed {
        println!("Uninstall complete.");
    } else {
        println!("Nothing to remove.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests for shared helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use serde_json::json;

    // -- File I/O --

    #[test]
    fn read_json_file_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result = read_json_file(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_json_file_invalid_json_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "not json").unwrap();
        let result = read_json_file(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid JSON"),);
    }

    #[test]
    fn write_and_read_json_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        let value = json!({"key": "value"});
        write_json_file(&path, &value).unwrap();
        let read_back = read_json_file(&path).unwrap().unwrap();
        assert_eq!(read_back, value);
    }

    // -- Database removal --

    #[test]
    fn remove_database_deletes_db_and_related_files() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("dispatch");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("tasks.db"), "db").unwrap();
        fs::write(data_dir.join("tasks.db-wal"), "wal").unwrap();
        fs::write(data_dir.join("tasks.db-shm"), "shm").unwrap();
        fs::write(data_dir.join("app.log"), "log").unwrap();

        let db_path = data_dir.join("tasks.db");
        let removed = remove_database(&db_path).unwrap();
        assert!(removed);
        assert!(!data_dir.join("tasks.db").exists());
        assert!(!data_dir.join("tasks.db-wal").exists());
        assert!(!data_dir.join("tasks.db-shm").exists());
        assert!(!data_dir.join("app.log").exists());
        assert!(!data_dir.exists());
    }

    #[test]
    fn remove_database_keeps_parent_if_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("dispatch");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("tasks.db"), "db").unwrap();
        fs::write(data_dir.join("other.txt"), "keep").unwrap();

        let db_path = data_dir.join("tasks.db");
        let removed = remove_database(&db_path).unwrap();
        assert!(removed);
        assert!(!data_dir.join("tasks.db").exists());
        assert!(data_dir.exists());
        assert!(data_dir.join("other.txt").exists());
    }

    #[test]
    fn remove_database_noop_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dispatch").join("tasks.db");

        let removed = remove_database(&db_path).unwrap();
        assert!(!removed);
    }
}
