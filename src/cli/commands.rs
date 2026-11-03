//! The bodies of the `dispatch` subcommands that read or write shared rows:
//! `repo`, `prune-repo-paths` and `plan`.
//!
//! Here rather than in `src/main.rs` so they can be run against a handle the
//! caller supplies. The binary hands them a store-backed one
//! (`store_connection::open_cli_store` — the store is mandatory, task #4916); the tests
//! hand them an in-memory store (`Store::open_in_memory`). Output goes to the
//! writers passed in, which the binary points at stdout and stderr.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::models::{expand_tilde, TaskId};
use crate::store::{RepoConfigRead, RepoConfigStore, Store};

/// `dispatch repo set-verify <path> <command>`.
pub async fn set_verify(
    database: &Store,
    path: &str,
    command: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let path = expand_tilde(path);
    // Checked before any write: the module refuses these too, but by then the
    // path row below would already exist.
    if command.contains('\n') || command.contains('\r') {
        anyhow::bail!("verify command must be a single line; chain steps with && or ;");
    }
    if command.trim().is_empty() {
        anyhow::bail!("verify command is empty; use `repo clear-verify` to remove it");
    }
    // Creates the saved path when it does not exist yet (`cli.allium`): the
    // store's own `set_verify_command` only updates a path that is saved. A
    // path that is already saved is left alone, so a row another host saved
    // keeps that host (saving would re-stamp it as this one's).
    if !database.list_repo_paths().await.contains(&path) {
        database.save_repo_path(&path).await?;
    }
    database.set_verify_command(&path, Some(command)).await?;
    writeln!(out, "verify_command set for {path}")?;
    Ok(())
}

/// `dispatch repo clear-verify <path>`. A path that is not saved has nothing
/// to clear and is ignored.
pub async fn clear_verify(database: &Store, path: &str, out: &mut dyn Write) -> Result<()> {
    let path = expand_tilde(path);
    if database.list_repo_paths().await.contains(&path) {
        database.set_verify_command(&path, None).await?;
    }
    writeln!(out, "verify_command cleared for {path}")?;
    Ok(())
}

/// `dispatch repo list`.
pub async fn list_repos(database: &Store, out: &mut dyn Write) -> Result<()> {
    let paths = database.list_repo_paths().await;
    if paths.is_empty() {
        writeln!(out, "No repo paths configured.")?;
        return Ok(());
    }
    for p in paths {
        match database.get_verify_command(&p).await {
            Some(cmd) => writeln!(out, "{p}\tverify: {cmd}")?,
            None => writeln!(out, "{p}")?,
        }
    }
    Ok(())
}

/// Whether `path` is on this machine's disk. `Err` when the filesystem cannot
/// say (a permission error, a detached disk): that is not "missing", and the
/// row stays.
fn path_on_disk(path: &str) -> std::io::Result<bool> {
    Path::new(&expand_tilde(path)).try_exists()
}

/// `dispatch prune-repo-paths` — forget every repo path THIS host saved that no
/// longer exists on its disk.
///
/// The table is shared, but a path is a place on one machine: a row another
/// host saved says nothing about this disk and is left alone, verify command
/// and all. A row saved before rows carried a host belongs to no one: this
/// host adopts it when the path exists here and removes it when it does not.
/// `kept` counts the whole store.
pub async fn prune_repo_paths(database: &Store, out: &mut dyn Write) -> Result<()> {
    let total = database.list_repo_paths().await.len();
    let mut removed = 0;
    for p in &database.list_repo_paths_saved_by_this_host().await {
        if matches!(path_on_disk(p), Ok(false)) {
            database.delete_repo_path(p).await?;
            writeln!(out, "removed: {p}")?;
            removed += 1;
        }
    }
    // Rows saved before hosts were stamped belong to no one. Present on this
    // disk, this host adopts the row (keeping its verify command). Missing
    // here, the row is left alone: the path says nothing about the host that
    // will adopt it.
    for p in &database.list_unowned_repo_paths().await {
        if matches!(path_on_disk(p), Ok(true)) {
            database.adopt_repo_path(p).await?;
            writeln!(out, "adopted: {p}")?;
        }
    }
    writeln!(out, "{removed} path(s) removed, {} kept.", total - removed)?;
    Ok(())
}

/// `dispatch plan <id> <path>` — attach an existing plan file to a task.
///
/// The file is checked before `database` is asked anything, so a mistyped
/// path fails without a store round trip; the caller opens the store only
/// once [`resolve_plan_path`] has answered.
pub async fn attach_plan(
    database: Arc<Store>,
    id: TaskId,
    plan_path: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let svc = crate::service::TaskService::new_with_real_runner(database);
    match svc.attach_plan(id, plan_path).await {
        Ok(()) => writeln!(out, "Plan attached to task #{id}: {plan_path}")?,
        Err(crate::service::ServiceError::NotFound(_)) => {
            anyhow::bail!("Task {id} not found");
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// The canonical path of an existing plan file, or why there is none.
pub fn resolve_plan_path(path: &Path) -> Result<String> {
    if !path.exists() {
        anyhow::bail!("Plan file not found: {}", path.display());
    }
    let plan_path = std::fs::canonicalize(path)
        .map_err(|e| anyhow::anyhow!("Failed to resolve plan path {}: {}", path.display(), e))?;
    Ok(plan_path.to_string_lossy().into_owned())
}
