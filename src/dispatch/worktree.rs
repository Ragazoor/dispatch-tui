use anyhow::{Context, Result};
use std::fs;

use crate::models::{expand_tilde, slugify, Task};
use crate::process::ProcessRunner;
use crate::tmux;

use super::prompts::build_tmux_window_name;
use super::stderr_str;

pub(super) struct ProvisionResult {
    pub(super) worktree_path: String,
    pub(super) tmux_window: String,
}

/// Create a git worktree and open a tmux window.
/// Shared by both `dispatch_agent` and `brainstorm_agent`.
pub(super) fn provision_worktree(
    task: &Task,
    runner: &dyn ProcessRunner,
    base_branch: Option<&str>,
) -> Result<ProvisionResult> {
    let repo_path = expand_tilde(&task.repo_path);
    let slug = slugify(&task.title);
    let worktree_name = format!("{}-{slug}", task.id);
    let worktree_path = format!("{repo_path}/.worktrees/{worktree_name}");
    let tmux_window = build_tmux_window_name(task.id);

    tracing::info!(task_id = task.id.0, %worktree_path, ?base_branch, "provisioning worktree");

    fs::create_dir_all(format!("{repo_path}/.worktrees"))
        .context("failed to create .worktrees directory")?;

    if std::path::Path::new(&worktree_path).exists() {
        tracing::info!(task_id = task.id.0, %worktree_path, "worktree already exists, reusing");
    } else {
        // Fetch origin/<base_branch> so the new branch starts from the latest
        // remote state rather than a potentially stale local branch.
        // Soft-fail: if fetch is unavailable (no origin, no network), fall
        // back to the local branch and continue — dispatch is not blocked.
        let start_point: Option<String> = if let Some(base) = base_branch {
            let fetch_ok = runner
                .run("git", &["-C", &repo_path, "fetch", "origin", base])
                .map(|o| o.status.success())
                .unwrap_or(false);
            if fetch_ok {
                Some(format!("origin/{base}"))
            } else {
                tracing::warn!(
                    base,
                    "git fetch origin failed, falling back to local branch"
                );
                Some(base.to_string())
            }
        } else {
            None
        };

        let mut args = vec![
            "-C",
            &repo_path,
            "worktree",
            "add",
            &*worktree_path,
            "-B",
            &*worktree_name,
        ];
        if let Some(ref sp) = start_point {
            args.push(sp.as_str());
        }
        let output = runner
            .run("git", &args)
            .context("failed to run git worktree add")?;
        anyhow::ensure!(
            output.status.success(),
            "git worktree add failed: {}",
            stderr_str(&output)
        );
    }

    tmux::new_window(&tmux_window, &worktree_path, runner)
        .context("failed to create tmux window")?;

    tmux::set_window_dispatch_dir(&tmux_window, &worktree_path, runner)
        .context("failed to set tmux window dispatch dir")?;
    tmux::ensure_split_hook(runner).context("failed to ensure tmux split hook")?;

    Ok(ProvisionResult {
        worktree_path,
        tmux_window,
    })
}

/// Remove the tmux window (if it still exists) and the git worktree.
///
/// Errors are logged but not propagated for the tmux step so that the
/// worktree removal is always attempted.
pub fn cleanup_task(
    repo_path: &str,
    worktree_path: &str,
    tmux_window: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    tracing::info!(worktree_path, "cleaning up task");

    if let Some(window) = tmux_window {
        match tmux::has_window(window, runner) {
            Ok(true) => {
                tmux::kill_window(window, runner)
                    .context("failed to kill tmux window during cleanup")?;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("could not check tmux window during cleanup: {e}");
            }
        }
    }

    let repo = expand_tilde(repo_path);
    let output = runner
        .run(
            "git",
            &["-C", &repo, "worktree", "remove", "--force", worktree_path],
        )
        .context("failed to run git worktree remove")?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        // If the worktree is already gone (manually removed or pruned), treat as success.
        if stderr.contains("is not a working tree") {
            tracing::info!(worktree_path, "worktree already removed, skipping");
        } else {
            anyhow::bail!(
                "git worktree remove failed for path {worktree_path}: {}",
                stderr
            );
        }
    }

    if let Some(branch) = std::path::Path::new(worktree_path)
        .file_name()
        .and_then(|n| n.to_str())
    {
        // Best-effort: ignore errors (branch may not exist).
        let _ = runner.run("git", &["-C", &repo, "branch", "-D", branch]);
    }

    Ok(())
}

/// Extract the branch name from a worktree path (its last path component).
pub fn branch_from_worktree(worktree: &str) -> Option<String> {
    std::path::Path::new(worktree)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Validate that a repo path points to an existing directory.
///
/// Returns the expanded path on success, or an error message on failure.
pub fn validate_repo_path(path: &str) -> Result<String, String> {
    let expanded = expand_tilde(path);
    let p = std::path::Path::new(&expanded);
    if !p.exists() {
        return Err(format!("Directory does not exist: {expanded}"));
    }
    if !p.is_dir() {
        return Err(format!("Not a directory: {expanded}"));
    }
    Ok(expanded)
}
