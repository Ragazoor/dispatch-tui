use std::time::Duration;

use crate::git::{abort, abort_after_conflict, git_checked, GitFailure};
use crate::models::expand_tilde;
use crate::process::ProcessRunner;

use super::git_output::is_rebase_conflict;
use crate::process::{stderr_str, stdout_str};

/// Errors from the finish (rebase + cleanup) operation.
#[derive(Debug)]
pub enum FinishError {
    NotOnDefaultBranch {
        current: String,
        expected: String,
    },
    /// The primary worktree (repo root) has uncommitted changes. Detected as
    /// a preflight check before any pull/rebase/merge is attempted, so it
    /// never masquerades as a rebase conflict.
    DirtyPrimaryWorktree {
        path: String,
        files: Vec<String>,
    },
    RebaseConflict {
        branch: String,
        files: Vec<String>,
    },
    Other(String),
}

impl std::fmt::Display for FinishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinishError::NotOnDefaultBranch { current, expected } => write!(
                f,
                "Repo root is not on {expected} (currently on {current}) — checkout {expected} first"
            ),
            FinishError::DirtyPrimaryWorktree { path, files } => write!(
                f,
                "Primary worktree at {path} has uncommitted changes ({}) — commit or stash them before wrap_up can rebase",
                files.join(", ")
            ),
            FinishError::RebaseConflict { branch, files } => {
                let location = if files.is_empty() {
                    String::new()
                } else {
                    format!(" in {}", files.join(", "))
                };
                write!(f, "Rebase conflict on {branch}{location} — resolve and try again")
            }
            FinishError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// The git-orchestration inputs for [`finish_task`], grouped to avoid a long
/// list of same-typed positional `&str` arguments that are easy to transpose.
pub struct FinishContext<'a> {
    /// Repo root path (where the base branch is checked out).
    pub repo_path: &'a str,
    /// The task's worktree path (where the task branch is checked out).
    pub worktree: &'a str,
    /// The task branch to rebase and fast-forward onto `base_branch`.
    pub branch: &'a str,
    /// The branch the repo root must be on; rebase/fast-forward target.
    pub base_branch: &'a str,
    /// Bound for every git subprocess `finish_task` issues. Use
    /// [`crate::process::SUBPROCESS_TIMEOUT`] in production; pass a short
    /// duration in tests, mirroring `provision_worktree` in
    /// [`crate::dispatch::worktree`]. Without this seam a test proving the
    /// bound exists would have to wait out the real production bound to go red.
    pub timeout: Duration,
}

/// Rebase the task branch onto `base_branch` and fast-forward it. The git half
/// of a finish and nothing else: no tmux teardown, no task write.
///
/// Killing the session is the caller's job, and deliberately so — the caller
/// gates the teardown on the task's terminal write landing first, so a task
/// whose Done write failed keeps its live window (`ExitSession` in
/// `docs/specs/pr-workflow.allium`). The worktree is preserved
/// — it will be cleaned up when the task is deleted.
pub fn finish_task(
    ctx: &FinishContext,
    runner: &dyn ProcessRunner,
) -> std::result::Result<(), FinishError> {
    finish_task_with_abort_runner(ctx, runner, runner)
}

/// [`finish_task`] with the rebase abort run through `abort_runner` instead of
/// `runner`. A caller that bounds the git steps by a deadline hands the
/// deadline-bound runner as `runner` and the inner one here: the abort runs
/// under its own `ctx.timeout`, so a deadline cannot leave the worktree
/// mid-rebase (`WrapUpRebase` in `docs/specs/pr-workflow.allium`).
pub fn finish_task_with_abort_runner(
    ctx: &FinishContext,
    runner: &dyn ProcessRunner,
    abort_runner: &dyn ProcessRunner,
) -> std::result::Result<(), FinishError> {
    let FinishContext {
        repo_path,
        worktree,
        branch,
        base_branch,
        timeout,
    } = *ctx;
    let repo_path = &expand_tilde(repo_path);
    let worktree = &expand_tilde(worktree);

    check_primary_worktree(repo_path, base_branch, runner)?;
    pull_base(repo_path, base_branch, runner, abort_runner, timeout)?;
    rebase_branch(worktree, branch, base_branch, runner, abort_runner, timeout)?;
    fast_forward_base(repo_path, branch, base_branch, runner, timeout)
}

type FinishResult = std::result::Result<(), FinishError>;

/// Steps 1–2: the repo root must be on `base_branch` and clean before anything
/// touches it.
fn check_primary_worktree(
    repo_path: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> FinishResult {
    // 1. Verify we're on the base branch
    let current_branch =
        crate::git::current_branch(repo_path, runner).map_err(FinishError::Other)?;
    if current_branch != base_branch {
        return Err(FinishError::NotOnDefaultBranch {
            current: current_branch,
            expected: base_branch.to_string(),
        });
    }

    // 2. Check the primary worktree (repo root) is clean before touching it.
    let dirty_files = crate::git::dirty_files(repo_path, runner).map_err(FinishError::Other)?;
    if !dirty_files.is_empty() {
        return Err(FinishError::DirtyPrimaryWorktree {
            path: repo_path.to_string(),
            files: dirty_files,
        });
    }
    Ok(())
}

/// Step 3: pull the latest base branch (skip if no remote configured). A probe
/// that could not be run at all is *not* "no remote": it means git could not be
/// spawned, which is a failure worth naming rather than a licence to skip the
/// pull and rebase anyway. A failed pull is aborted, through `abort_runner`.
fn pull_base(
    repo_path: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
    abort_runner: &dyn ProcessRunner,
    timeout: Duration,
) -> FinishResult {
    let has_remote =
        crate::git::has_origin_remote(repo_path, runner).map_err(FinishError::Other)?;
    if !has_remote {
        return Ok(());
    }
    match git_checked(
        runner,
        repo_path,
        &["pull", "--no-rebase", "origin", base_branch],
        timeout,
    ) {
        Ok(_) => Ok(()),
        // A pull that stopped on a conflict, or was killed at the bound, leaves
        // the primary checkout mid-merge; abort it as the rebase path does.
        Err(failure) => {
            let message = match &failure {
                GitFailure::Spawn(e) => format!("Failed to pull: {e}"),
                GitFailure::Exit(output) => {
                    format!("Failed to pull {base_branch}: {}", stderr_str(output))
                }
            };
            Err(FinishError::Other(with_abort_failure(
                message,
                crate::git::abort_merge_if_in_progress(abort_runner, repo_path, timeout),
            )))
        }
    }
}

/// `message`, plus the `git merge --abort` failure when there was one: the
/// checkout is then still mid-merge, which the operator must be told.
fn with_abort_failure(message: String, abort_failure: Option<String>) -> String {
    match abort_failure {
        Some(failure) => format!("{message}; `git merge --abort` also failed: {failure}"),
        None => message,
    }
}

/// Step 4: rebase `branch` onto `base_branch` (from the worktree, where the
/// branch is checked out). A failed rebase is aborted, through `abort_runner`;
/// so is one that could not finish (killed at a timeout), which may have left
/// the worktree mid-rebase.
fn rebase_branch(
    worktree: &str,
    branch: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
    abort_runner: &dyn ProcessRunner,
    timeout: Duration,
) -> FinishResult {
    let output = match git_checked(runner, worktree, &["rebase", base_branch], timeout) {
        Ok(_) => return Ok(()),
        Err(GitFailure::Spawn(e)) => {
            abort(abort_runner, worktree, "rebase", timeout);
            return Err(FinishError::Other(format!("Failed to run git rebase: {e}")));
        }
        Err(GitFailure::Exit(output)) => output,
    };
    let stderr = stderr_str(&output);
    let stdout = stdout_str(&output);
    let is_conflict = is_rebase_conflict(&stdout, &stderr);

    // A conflict's files are read before the abort clears them
    // (`ConflictFilesCapturedBeforeAbort`); any other failure just aborts.
    if !is_conflict {
        abort(abort_runner, worktree, "rebase", timeout);
        return Err(FinishError::Other(format!("Rebase failed: {}", stderr)));
    }
    Err(FinishError::RebaseConflict {
        branch: branch.to_string(),
        files: abort_after_conflict(abort_runner, worktree, "rebase", timeout),
    })
}

/// Step 5: fast-forward the base branch to the rebased branch.
fn fast_forward_base(
    repo_path: &str,
    branch: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
    timeout: Duration,
) -> FinishResult {
    match git_checked(runner, repo_path, &["merge", "--ff-only", branch], timeout) {
        Ok(_) => Ok(()),
        Err(GitFailure::Spawn(e)) => Err(FinishError::Other(format!(
            "Failed to fast-forward {base_branch}: {e}"
        ))),
        Err(GitFailure::Exit(output)) => Err(FinishError::Other(format!(
            "Fast-forward failed after rebase: {}",
            stderr_str(&output)
        ))),
    }
}

#[cfg(test)]
mod tests;
