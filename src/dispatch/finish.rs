use crate::models::expand_tilde;
use crate::process::ProcessRunner;
use crate::tmux;

use super::{stderr_str, stdout_str};

/// Errors from the finish (rebase + cleanup) operation.
#[derive(Debug)]
pub enum FinishError {
    NotOnDefaultBranch { current: String, expected: String },
    RebaseConflict(String),
    Other(String),
}

impl std::fmt::Display for FinishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinishError::NotOnDefaultBranch { current, expected } => write!(
                f,
                "Repo root is not on {expected} (currently on {current}) — checkout {expected} first"
            ),
            FinishError::RebaseConflict(branch) => {
                write!(f, "Rebase conflict on {branch} — resolve and try again")
            }
            FinishError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// Rebase the task branch onto `base_branch` and fast-forward it, then kill the tmux window.
/// The worktree is preserved — it will be cleaned up when the task is archived.
pub fn finish_task(
    repo_path: &str,
    worktree: &str,
    branch: &str,
    base_branch: &str,
    tmux_window: Option<&str>,
    runner: &dyn ProcessRunner,
) -> std::result::Result<(), FinishError> {
    let repo_path = &expand_tilde(repo_path);
    let worktree = &expand_tilde(worktree);

    // 1. Verify we're on the base branch
    let output = runner
        .run(
            "git",
            &["-C", repo_path, "rev-parse", "--abbrev-ref", "HEAD"],
        )
        .map_err(|e| FinishError::Other(format!("Failed to check current branch: {e}")))?;
    let current_branch = stdout_str(&output);
    if current_branch != base_branch {
        return Err(FinishError::NotOnDefaultBranch {
            current: current_branch,
            expected: base_branch.to_string(),
        });
    }

    // 2. Pull latest base branch (skip if no remote configured)
    let has_remote = runner
        .run("git", &["-C", repo_path, "remote", "get-url", "origin"])
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_remote {
        let output = runner
            .run("git", &["-C", repo_path, "pull", "origin", base_branch])
            .map_err(|e| FinishError::Other(format!("Failed to pull: {e}")))?;
        if !output.status.success() {
            return Err(FinishError::Other(format!(
                "Failed to pull {base_branch}: {}",
                stderr_str(&output)
            )));
        }
    }

    // 3. Rebase branch onto base branch (from worktree, where branch is checked out)
    let output = runner
        .run("git", &["-C", worktree, "rebase", base_branch])
        .map_err(|e| FinishError::Other(format!("Failed to run git rebase: {e}")))?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        let stdout = stdout_str(&output);
        let is_conflict = stderr.contains("CONFLICT")
            || stdout.contains("CONFLICT")
            || stderr.contains("could not apply")
            || stderr.contains("Merge conflict");

        let _ = runner.run("git", &["-C", worktree, "rebase", "--abort"]);

        if is_conflict {
            return Err(FinishError::RebaseConflict(branch.to_string()));
        }
        return Err(FinishError::Other(format!("Rebase failed: {}", stderr)));
    }

    // 4. Fast-forward base branch to the rebased branch
    let output = runner
        .run("git", &["-C", repo_path, "merge", "--ff-only", branch])
        .map_err(|e| FinishError::Other(format!("Failed to fast-forward {base_branch}: {e}")))?;
    if !output.status.success() {
        return Err(FinishError::Other(format!(
            "Fast-forward failed after rebase: {}",
            stderr_str(&output)
        )));
    }

    // 5. Kill tmux window (worktree is preserved for later archival)
    if let Some(window) = tmux_window {
        match tmux::has_window(window, runner) {
            Ok(true) => {
                tmux::kill_window(window, runner)
                    .map_err(|e| FinishError::Other(format!("Failed to kill tmux window: {e}")))?;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("could not check tmux window during finish: {e}");
            }
        }
    }

    Ok(())
}
