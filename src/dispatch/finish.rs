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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::process::MockProcessRunner;
    use std::process::Output;

    fn exit_fail() -> std::process::ExitStatus {
        // UNIX only, but tests only run on Linux/macOS anyway.
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(1)
    }

    // The has_window path when tmux itself can't be executed (runner returns
    // Err).  finish_task should warn and still return Ok(()) — a missing tmux
    // window is not a fatal error at this stage.
    #[test]
    fn finish_task_has_window_runner_error_warns_and_succeeds() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
            MockProcessRunner::fail(""),                  // remote get-url (no remote)
            MockProcessRunner::ok(),                      // git rebase main
            MockProcessRunner::ok(),                      // git merge --ff-only
            Err(anyhow::anyhow!("tmux: command not found")), // tmux list-windows (has_window Err)
        ]);

        finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            Some("task-42"),
            &mock,
        )
        .expect("should succeed despite has_window runner error");
    }

    // Pull runner returns Err (process could not be spawned) rather than a
    // non-zero exit — maps to FinishError::Other via map_err.
    #[test]
    fn finish_task_pull_runner_error_returns_other() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url
            Err(anyhow::anyhow!("git: command not found")), // git pull
        ]);

        let err = finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        )
        .unwrap_err();

        assert!(
            matches!(err, FinishError::Other(ref m) if m.contains("Failed to pull")),
            "pull runner error should map to FinishError::Other, got: {err}"
        );
    }

    // FF-only runner returns Err (process could not be spawned) — maps to
    // FinishError::Other via map_err with "Failed to fast-forward" prefix.
    #[test]
    fn finish_task_ff_only_runner_error_returns_other() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
            MockProcessRunner::fail(""),                  // remote get-url (no remote)
            MockProcessRunner::ok(),                      // git rebase
            Err(anyhow::anyhow!("git: command not found")), // git merge --ff-only
        ]);

        let err = finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        )
        .unwrap_err();

        assert!(
            matches!(err, FinishError::Other(ref m) if m.contains("Failed to fast-forward")),
            "ff-only runner error should map to FinishError::Other, got: {err}"
        );
    }

    // Rebase detects conflict via stdout CONFLICT marker (stderr is empty).
    #[test]
    fn finish_task_rebase_conflict_in_stdout_returns_rebase_conflict() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
            MockProcessRunner::fail(""),                  // remote get-url (no remote)
            Ok(Output {
                status: exit_fail(),
                stdout: b"CONFLICT (content): Merge conflict in lib.rs\n".to_vec(),
                stderr: vec![],
            }),
            MockProcessRunner::ok(), // git rebase --abort
        ]);

        let err = finish_task(
            "/repo",
            "/repo/.worktrees/42-fix-bug",
            "42-fix-bug",
            "main",
            None,
            &mock,
        )
        .unwrap_err();

        assert!(
            matches!(err, FinishError::RebaseConflict(_)),
            "CONFLICT in stdout should still map to RebaseConflict, got: {err}"
        );
    }
}
