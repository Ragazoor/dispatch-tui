use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use rusqlite;
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crate::process::RealProcessRunner;
use crate::tmux;

// The entire plugin/ directory is embedded at compile time. Any file added to
// plugin/ is automatically picked up — no manual registration required.
static PLUGIN_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/plugin");

// ---------------------------------------------------------------------------
// MCP config merging
// ---------------------------------------------------------------------------

pub struct MergeResult {
    pub value: Value,
    pub changed: bool,
}

pub fn merge_mcp_config(existing: Option<Value>, port: u16) -> MergeResult {
    let server_entry = json!({
        "type": "http",
        "url": format!("http://localhost:{port}/mcp")
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
// Plugin installation
// ---------------------------------------------------------------------------

fn plugin_dir() -> Result<PathBuf> {
    let claude_dir = claude_dir()?;
    Ok(claude_dir.join("plugins").join("local").join("dispatch"))
}

fn is_executable(path: &std::path::Path) -> bool {
    path.starts_with("hooks/scripts")
}

pub fn install_plugin() -> Result<bool> {
    let plugin_dir = plugin_dir()?;
    let mut changed = false;
    install_dir_recursive(&PLUGIN_DIR, &plugin_dir, &mut changed)?;
    Ok(changed)
}

fn install_dir_recursive(dir: &Dir, base: &std::path::Path, changed: &mut bool) -> Result<()> {
    for file in dir.files() {
        let path = base.join(file.path());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        let content = file
            .contents_utf8()
            .with_context(|| format!("Non-UTF-8 plugin file: {}", file.path().display()))?;
        *changed |= write_file_if_changed(&path, content, is_executable(file.path()))?;
    }
    for subdir in dir.dirs() {
        install_dir_recursive(subdir, base, changed)?;
    }
    Ok(())
}

fn write_file_if_changed(path: &std::path::Path, content: &str, executable: bool) -> Result<bool> {
    if path.exists() {
        let existing = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if existing == content {
            return Ok(false);
        }
    }
    fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    if executable {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// File I/O helpers
// ---------------------------------------------------------------------------

fn claude_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("$HOME is not set")?;
    Ok(PathBuf::from(home).join(".claude"))
}

fn read_json_file(path: &std::path::Path) -> Result<Option<Value>> {
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

fn write_json_file(path: &std::path::Path, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;
    fs::write(path, content + "\n").with_context(|| format!("Failed to write {}", path.display()))
}

// ---------------------------------------------------------------------------
// Interactive confirmation
// ---------------------------------------------------------------------------

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

fn plugin_needs_update() -> Result<bool> {
    plugin_needs_update_in(&plugin_dir()?)
}

fn plugin_needs_update_in(base: &std::path::Path) -> Result<bool> {
    needs_update_recursive(&PLUGIN_DIR, base)
}

fn needs_update_recursive(dir: &Dir, base: &std::path::Path) -> Result<bool> {
    for file in dir.files() {
        let path = base.join(file.path());
        let content = file.contents_utf8().unwrap_or("");
        match fs::read_to_string(&path) {
            Ok(existing) if existing == content => continue,
            _ => return Ok(true),
        }
    }
    for subdir in dir.dirs() {
        if needs_update_recursive(subdir, base)? {
            return Ok(true);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Removal functions
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

pub fn remove_plugin(plugin_path: &std::path::Path) -> Result<bool> {
    if !plugin_path.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(plugin_path)
        .with_context(|| format!("Failed to remove {}", plugin_path.display()))?;
    Ok(true)
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

pub fn run_setup(port: u16, yes: bool) -> Result<()> {
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
    if plugin_needs_update()? {
        if yes || confirm("Install dispatch plugin (skills, hooks, commands) to ~/.claude/plugins/local/dispatch/?")? {
            install_plugin()?;
            println!("Plugin: installed dispatch plugin to ~/.claude/plugins/local/dispatch/");
            let skills: Vec<String> = PLUGIN_DIR
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
            let commands: Vec<String> = PLUGIN_DIR
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
    let plugin_path = plugin_dir()?;
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_PORT;
    use serde_json::json;

    // -- MCP config merging --

    #[test]
    fn merge_mcp_config_into_empty() {
        let existing = None;
        let result = merge_mcp_config(existing, DEFAULT_PORT);
        let expected = json!({
            "mcpServers": {
                "dispatch": {
                    "type": "http",
                    "url": format!("http://localhost:{DEFAULT_PORT}/mcp")
                }
            }
        });
        assert_eq!(result.value, expected);
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
    }

    #[test]
    fn merge_mcp_config_already_configured() {
        let existing = Some(json!({
            "mcpServers": {
                "dispatch": {
                    "type": "http",
                    "url": format!("http://localhost:{DEFAULT_PORT}/mcp")
                }
            }
        }));
        let result = merge_mcp_config(existing, DEFAULT_PORT);
        assert!(!result.changed);
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

    fn hook_script() -> &'static str {
        PLUGIN_DIR
            .get_file("hooks/scripts/task-status-hook")
            .expect("task-status-hook must be embedded")
            .contents_utf8()
            .expect("task-status-hook must be UTF-8")
    }

    fn usage_hook_script() -> &'static str {
        PLUGIN_DIR
            .get_file("hooks/scripts/task-usage-hook")
            .expect("task-usage-hook must be embedded")
            .contents_utf8()
            .expect("task-usage-hook must be UTF-8")
    }

    // -- Hook script --

    #[test]
    fn hook_script_is_valid_bash() {
        assert!(hook_script().starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn usage_hook_script_is_valid_bash() {
        assert!(usage_hook_script().starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn hook_script_handles_all_events() {
        let s = hook_script();
        assert!(s.contains("PreToolUse)"));
        assert!(s.contains("Stop)"));
        assert!(s.contains("Notification)"));
    }

    #[test]
    fn hook_script_skips_dispatch_mcp_in_pretooluse() {
        // The PreToolUse handler must read tool_name from the JSON input
        // and skip dispatch MCP tool calls to avoid clobbering review status
        // during wrap-up (get_task and wrap_up would otherwise set running).
        let s = hook_script();
        assert!(
            s.contains("tool_name"),
            "hook must extract tool_name from PreToolUse input"
        );
        assert!(
            s.contains("mcp__dispatch__"),
            "hook must skip mcp__dispatch__ tools in PreToolUse"
        );
    }

    #[test]
    fn hook_script_notification_uses_sub_status_needs_input() {
        // Notification must NOT change status to review — it keeps running and
        // sets sub_status=needs_input so the task stays in the Blocked visual column.
        let s = hook_script();
        assert!(
            s.contains("--sub-status needs_input"),
            "Notification handler must use --sub-status needs_input, not --needs-input"
        );
        assert!(
            !s.contains("--needs-input"),
            "Deprecated --needs-input flag must not appear in the hook script"
        );
    }

    // -- Plugin --

    #[test]
    fn plugin_json_is_valid() {
        let content = PLUGIN_DIR
            .get_file(".claude-plugin/plugin.json")
            .expect("plugin.json must be embedded")
            .contents_utf8()
            .expect("plugin.json must be UTF-8");
        let value: Value = serde_json::from_str(content).expect("plugin.json is invalid JSON");
        assert_eq!(value["name"], "dispatch");
    }

    #[test]
    fn hooks_json_is_valid() {
        let content = PLUGIN_DIR
            .get_file("hooks/hooks.json")
            .expect("hooks.json must be embedded")
            .contents_utf8()
            .expect("hooks.json must be UTF-8");
        let value: Value = serde_json::from_str(content).expect("hooks.json is invalid JSON");
        assert!(value["PreToolUse"].is_array(), "missing PreToolUse");
        assert!(value["Stop"].is_array(), "missing Stop");
        assert!(value["Notification"].is_array(), "missing Notification");
    }

    #[test]
    fn plugin_embeds_required_files() {
        let required = [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/scripts/task-status-hook",
            "hooks/scripts/task-usage-hook",
            "skills/wrap-up/SKILL.md",
            "skills/decompose-review/SKILL.md",
            "skills/decompose-review/references/plan-template.md",
            "skills/alert-monitor/SKILL.md",
            "commands/queue-plan.md",
        ];
        for path in required {
            assert!(
                PLUGIN_DIR.get_file(path).is_some(),
                "{path} must be embedded in PLUGIN_DIR"
            );
        }
    }

    #[test]
    fn plugin_hook_scripts_are_executable() {
        let hooks_scripts = PLUGIN_DIR
            .get_dir("hooks/scripts")
            .expect("hooks/scripts dir must exist");
        for file in hooks_scripts.files() {
            assert!(
                is_executable(file.path()),
                "{} should be marked executable",
                file.path().display()
            );
        }
    }

    #[test]
    fn write_file_if_changed_creates_new() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let changed = write_file_if_changed(&path, "hello", false).unwrap();
        assert!(changed);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn write_file_if_changed_skips_identical() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same.txt");
        fs::write(&path, "hello").unwrap();
        let changed = write_file_if_changed(&path, "hello", false).unwrap();
        assert!(!changed);
    }

    #[test]
    fn write_file_if_changed_updates_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stale.txt");
        fs::write(&path, "old").unwrap();
        let changed = write_file_if_changed(&path, "new", false).unwrap();
        assert!(changed);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn write_file_if_changed_sets_executable_permission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        write_file_if_changed(&path, "#!/bin/bash", true).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode();
        assert_eq!(mode & 0o755, 0o755, "should have executable permissions");
    }

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

    // -- Plugin removal --

    #[test]
    fn remove_plugin_deletes_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("dispatch");
        fs::create_dir_all(plugin.join("hooks/scripts")).unwrap();
        fs::write(plugin.join("hooks/hooks.json"), "{}").unwrap();

        let removed = remove_plugin(&plugin).unwrap();
        assert!(removed);
        assert!(!plugin.exists());
    }

    #[test]
    fn remove_plugin_noop_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("dispatch");

        let removed = remove_plugin(&plugin).unwrap();
        assert!(!removed);
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

    // -- plugin_needs_update --

    #[test]
    fn plugin_needs_update_true_when_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(plugin_needs_update_in(dir.path()).unwrap());
    }

    fn write_all_plugin_files(base: &std::path::Path) {
        fn write_dir(dir: &Dir, base: &std::path::Path) {
            for file in dir.files() {
                let path = base.join(file.path());
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, file.contents_utf8().unwrap_or("")).unwrap();
            }
            for subdir in dir.dirs() {
                write_dir(subdir, base);
            }
        }
        write_dir(&PLUGIN_DIR, base);
    }

    #[test]
    fn plugin_needs_update_false_when_all_match() {
        let dir = tempfile::tempdir().unwrap();
        write_all_plugin_files(dir.path());
        assert!(!plugin_needs_update_in(dir.path()).unwrap());
    }

    #[test]
    fn plugin_needs_update_true_when_one_file_differs() {
        let dir = tempfile::tempdir().unwrap();
        write_all_plugin_files(dir.path());
        // Corrupt one file
        fs::write(dir.path().join(".claude-plugin/plugin.json"), "corrupted").unwrap();
        assert!(plugin_needs_update_in(dir.path()).unwrap());
    }
}
