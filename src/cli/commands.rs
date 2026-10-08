//! The bodies of the `dispatch` subcommands that read or write shared rows:
//! `repo`, `prune-repo-paths` and `plan`.
//!
//! Here rather than in `src/main.rs` so they can be run against a handle the
//! caller supplies. The binary hands them a store-backed one
//! (`runtime::open_cli_store` — the store is mandatory, task #4916); the tests
//! hand them SQLite, the stand-in until Phase 12b (#4975). Output goes to the
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
    // Creates the saved path when it does not exist yet (`cli.allium`): the
    // store's own `set_verify_command` only updates a path that is saved.
    // Saving a path that is already saved only touches it.
    database.save_repo_path(&path).await?;
    database.set_verify_command(&path, Some(command)).await?;
    writeln!(out, "verify_command set for {path}")?;
    Ok(())
}

/// `dispatch repo clear-verify <path>`.
pub async fn clear_verify(database: &Store, path: &str, out: &mut dyn Write) -> Result<()> {
    let path = expand_tilde(path);
    database.set_verify_command(&path, None).await?;
    writeln!(out, "verify_command cleared for {path}")?;
    Ok(())
}

/// `dispatch repo list`.
pub async fn list_repos(database: &Store, out: &mut dyn Write) -> Result<()> {
    let paths = database.list_repo_paths().await?;
    if paths.is_empty() {
        writeln!(out, "No repo paths configured.")?;
        return Ok(());
    }
    for p in paths {
        match database.get_verify_command(&p).await? {
            Some(cmd) => writeln!(out, "{p}\tverify: {cmd}")?,
            None => writeln!(out, "{p}")?,
        }
    }
    Ok(())
}

/// `dispatch repo status [--no-fetch]` — one row per saved repo path.
///
/// Fetches before measuring unless suppressed, so the counts are current. A
/// repository that could not be measured shows no ahead/behind figures at all
/// (`UnmeasuredIsNeverPresentedAsClean`) and, when the fetch was the cause, its
/// fetch error instead.
pub async fn repo_status(database: &Store, no_fetch: bool, out: &mut dyn Write) -> Result<()> {
    let paths = database.list_repo_paths().await?;
    if paths.is_empty() {
        writeln!(out, "No repo paths configured.")?;
        return Ok(());
    }
    // Every repo is measured concurrently: with a fetch this is a network
    // round-trip each, so N repos sequentially would cost N latencies for work
    // that has no ordering between repositories. Mirrors the board's startup
    // fan-out (`exec_refresh_all_repo_sync`). Handles are spawned up front and
    // awaited in `paths` order, so the table stays deterministic regardless of
    // which repository answers first.
    let handles: Vec<_> = paths
        .iter()
        .map(|path| {
            let expanded = expand_tilde(path);
            tokio::task::spawn_blocking(move || {
                let runner = crate::process::RealProcessRunner::default();
                crate::repo_sync::measure_repo(&expanded, !no_fetch, &runner)
            })
        })
        .collect();

    let mut cache = crate::repo_sync::RepoSyncCache::default();
    for (path, handle) in paths.iter().zip(handles) {
        let expanded = expand_tilde(path);
        cache.apply(handle.await?);
        // `measure_repo` keys the state by the path it was handed.
        let Some(state) = cache.get(&expanded) else {
            continue;
        };
        match state.counts {
            Some(counts) => writeln!(
                out,
                "{}\t{}\t\u{2191}{} \u{2193}{}",
                state.repo_path, state.base_branch, counts.ahead, counts.behind
            )?,
            None => match &state.last_fetch_error {
                Some(err) => writeln!(
                    out,
                    "{}\t{}\tunknown\t{err}",
                    state.repo_path, state.base_branch
                )?,
                None => writeln!(out, "{}\t{}\tunknown", state.repo_path, state.base_branch)?,
            },
        }
    }
    Ok(())
}

/// `dispatch repo sync [<path>]` — sync one saved repo path or every one.
///
/// Every target is attempted; one failure does not abandon the rest. The exit
/// code is non-zero when any target failed, so the command is usable from a
/// script. Needs a multi-thread runtime (`block_in_place`).
pub async fn repo_sync(
    database: &Store,
    path: Option<String>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let saved = database.list_repo_paths().await?;
    let targets: Vec<String> = match &path {
        Some(p) => {
            let expanded = expand_tilde(p);
            saved
                .into_iter()
                .filter(|s| expand_tilde(s) == expanded)
                .collect()
        }
        None => saved,
    };
    if targets.is_empty() {
        match path {
            Some(p) => anyhow::bail!("{p} is not a saved repo path"),
            None => anyhow::bail!("No repo paths configured."),
        }
    }

    let runner = crate::process::RealProcessRunner::default();
    let mut failed = 0;
    for target in &targets {
        let expanded = expand_tilde(target);
        let base =
            tokio::task::block_in_place(|| crate::git::detect_default_branch(&expanded, &runner));
        let result =
            tokio::task::block_in_place(|| crate::repo_sync::sync_repo(&expanded, &base, &runner));
        match result {
            Ok(crate::repo_sync::SyncOutcome::AlreadyInSync) => {
                writeln!(out, "{expanded}\t{base}\tnothing to do")?;
            }
            Ok(crate::repo_sync::SyncOutcome::Synced { pulled, pushed }) => {
                writeln!(out, "{expanded}\t{base}\tpulled {pulled}, pushed {pushed}")?;
            }
            Err(e) => {
                failed += 1;
                writeln!(err, "{expanded}\t{base}\tfailed: {e}")?;
            }
        }
    }
    if failed > 0 {
        anyhow::bail!("{failed} of {} repo(s) failed to sync", targets.len());
    }
    Ok(())
}

/// `dispatch prune-repo-paths` — forget every saved repo path that no longer
/// exists on disk.
pub async fn prune_repo_paths(database: &Store, out: &mut dyn Write) -> Result<()> {
    let paths = database.list_repo_paths().await?;
    let total = paths.len();
    let mut removed = 0;
    for p in &paths {
        let expanded = expand_tilde(p);
        if !Path::new(&expanded).exists() {
            database.delete_repo_path(p).await?;
            writeln!(out, "removed: {p}")?;
            removed += 1;
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
    id: i64,
    plan_path: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let svc = crate::service::TaskService::new_with_real_runner(database);
    match svc.attach_plan(TaskId(id), plan_path).await {
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
