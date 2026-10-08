//! Uninstall flow: remove installed files and optionally purge data.

use super::*;

/// Filesystem locations the uninstall flow removes. Grouped so tests can point
/// the whole flow at temp directories instead of the real `$HOME`.
pub(super) struct UninstallPaths {
    pub mcp_path: PathBuf,
    pub legacy_mcp_path: PathBuf,
    pub plugin_path: PathBuf,
    pub data_dir: PathBuf,
    pub statusline_path: PathBuf,
}

impl UninstallPaths {
    /// Resolve the real `$HOME`-derived locations used in production.
    ///
    /// `data_dir` is the operator's `--db` / `DISPATCH_DATA_DIR`: it names the data
    /// directory the purge works in, and is never opened.
    fn resolve(data_dir: &Path) -> Result<Self> {
        let claude_dir = claude_dir()?;
        Ok(Self {
            mcp_path: user_global_config_path()?,
            legacy_mcp_path: claude_dir.join(".mcp.json"),
            plugin_path: plugins::plugin_dir()?,
            data_dir: data_dir.to_path_buf(),
            statusline_path: statusline::settings_path(&claude_dir),
        })
    }
}

/// `data_dir` is the global `--db` (default: the XDG data directory's
/// `tasks.db`); `--purge` works in its parent directory.
pub fn run_uninstall(yes: bool, purge: bool, data_dir: &Path) -> Result<()> {
    let paths = UninstallPaths::resolve(data_dir)?;
    run_uninstall_in(&paths, &StdinConfirmer, yes, purge)
}

/// Injectable core of [`run_uninstall`]. Takes the target filesystem locations
/// and a confirmer so the removal decision matrix can be exercised
/// deterministically in tests.
pub(super) fn run_uninstall_in(
    paths: &UninstallPaths,
    confirmer: &dyn Confirmer,
    yes: bool,
    purge: bool,
) -> Result<()> {
    print_uninstall_plan(paths, purge);

    if !yes && !confirmer.confirm("\nContinue?")? {
        println!("Aborted.");
        return Ok(());
    }

    let mut any_removed = remove_installed_files(paths);

    // Note: ~/.claude/settings.json is intentionally not touched. Dispatch no
    // longer manages permissions in that file — it is user-owned config. Users
    // who ran an older `dispatch setup` may have stale mcp__dispatch__* entries
    // in settings.json; those are inert once the MCP server is removed and can
    // be cleaned up manually.

    if purge {
        any_removed |= purge_data_dir(&paths.data_dir, confirmer)?;
    }

    if any_removed {
        println!("Uninstall complete.");
    } else {
        println!("Nothing to remove.");
    }

    Ok(())
}

/// Tell the user what `run_uninstall_in` is about to remove.
pub(super) fn print_uninstall_plan(paths: &UninstallPaths, purge: bool) {
    let UninstallPaths {
        mcp_path,
        legacy_mcp_path,
        plugin_path,
        data_dir,
        statusline_path,
    } = paths;

    // Show what will be removed
    eprintln!("This will remove:");
    eprintln!("  Plugin:      {}", plugin_path.display());
    eprintln!(
        "  MCP config:  mcpServers.dispatch from {}",
        mcp_path.display()
    );
    eprintln!(
        "  Legacy MCP:  mcpServers.dispatch from {} (if present)",
        legacy_mcp_path.display()
    );
    eprintln!("  Status line: {} (if present)", statusline_path.display());
    if purge {
        eprintln!(
            "  Identity:    host.json and app.log in {}",
            data_dir.display()
        );
    }
}

/// Remove the plugin, the MCP entries and the status line file. Each is
/// best-effort: a failure warns and the rest still run. Returns whether
/// anything was removed.
pub(super) fn remove_installed_files(paths: &UninstallPaths) -> bool {
    let UninstallPaths {
        mcp_path,
        legacy_mcp_path,
        plugin_path,
        statusline_path,
        data_dir: _,
    } = paths;
    let mut any_removed = false;

    match remove_plugin(plugin_path) {
        Ok(true) => {
            println!("Removed plugin directory");
            any_removed = true;
        }
        Ok(false) => println!("Plugin directory not found, skipping"),
        Err(e) => eprintln!("Warning: failed to remove plugin: {e}"),
    }

    match remove_mcp_config(mcp_path) {
        Ok(true) => {
            println!("Removed dispatch from MCP config");
            any_removed = true;
        }
        Ok(false) => println!("No dispatch entry in MCP config, skipping"),
        Err(e) => eprintln!("Warning: failed to update MCP config: {e}"),
    }

    // Legacy cleanup: remove any stale entry from ~/.claude/.mcp.json that
    // earlier dispatch versions mistakenly wrote there.
    match remove_mcp_config(legacy_mcp_path) {
        Ok(true) => {
            println!(
                "Removed stale dispatch entry from {}",
                legacy_mcp_path.display()
            );
            any_removed = true;
        }
        Ok(false) => {}
        Err(e) => eprintln!("Warning: failed to clean up legacy MCP config: {e}"),
    }

    // Status line settings file — dispatch-owned, written by the startup
    // configuration check
    // (see src/setup/statusline.rs). Best-effort: a missing file is a no-op.
    match fs::remove_file(statusline_path) {
        Ok(()) => {
            println!("Removed status line settings file");
            any_removed = true;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Status line settings file not found, skipping")
        }
        Err(e) => eprintln!("Warning: failed to remove status line settings file: {e}"),
    }

    any_removed
}

/// The `--purge` step: forget this machine's identity (`host.json`) and
/// remove the log, behind the dangerous prompt, and the data directory too
/// once that leaves it empty. Returns whether anything was removed.
///
/// Tasks live in the store, which a purge does not touch, so no count is
/// stated (counting would mean opening a database). A leftover `tasks.db` and
/// its companions are never opened, read, moved or deleted
/// (`storage.allium: CodeNeverTouchesLegacyDatabase`): they are the operator's
/// to delete by hand.
pub(super) fn purge_data_dir(data_dir: &Path, confirmer: &dyn Confirmer) -> Result<bool> {
    let host_file = crate::host_file::host_file_path(data_dir);
    let log = data_dir.join("app.log");
    let present: Vec<&PathBuf> = [&host_file, &log]
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    if present.is_empty() {
        println!("No host file or log in {}, skipping", data_dir.display());
        return Ok(false);
    }
    eprintln!(
        "\n  This forgets this machine's identity: a later launch mints a new one, and tasks \
         stamped with the old host id stop being this machine's. This cannot be undone."
    );
    if !confirmer.confirm_dangerous("Remove this machine's host file and log?")? {
        println!("Kept host file and log.");
        return Ok(false);
    }
    let mut removed = false;
    for path in present {
        match fs::remove_file(path) {
            Ok(()) => {
                println!("Removed {}", path.display());
                removed = true;
            }
            Err(e) => eprintln!("Warning: failed to remove {}: {e}", path.display()),
        }
    }
    if data_dir
        .read_dir()
        .is_ok_and(|mut entries| entries.next().is_none())
    {
        match fs::remove_dir(data_dir) {
            Ok(()) => println!("Removed {}", data_dir.display()),
            Err(e) => eprintln!("Warning: failed to remove {}: {e}", data_dir.display()),
        }
    } else {
        eprintln!(
            "Note: {} still holds other files. A leftover tasks.db (and -wal/-shm) there is no \
             longer used by dispatch and can be deleted by hand.",
            data_dir.display()
        );
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Tests for shared helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test seam
// ---------------------------------------------------------------------------
