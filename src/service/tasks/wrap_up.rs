//! Wrap-up business logic the board still owns: the host-local pre-checks
//! `WrapUpRefusesUnfinishedWork` makes before `wrap_up` writes a request, and
//! the best-effort window kill a failed teardown row still owes.
//!
//! Since store-split M5 the rebase and the exit_session teardown are request
//! rows served by this host (`session.rs`), and the Conflict sub_status is the
//! module's. Since M7 so is `update_tasks`' close (a `close` row).

use std::time::Duration;

use crate::dispatch;
use crate::models::{Task, TmuxWindow};
use crate::service::ServiceError;

use super::crud::TaskService;

/// How long `wrap_up` lets the repo's verify command run before it counts as
/// failed. Generous: a cold `cargo test` in a fresh worktree is minutes, and a
/// verify that outlasts this would also outlast the agent's own tool timeout.
pub const WRAP_UP_VERIFY_TIMEOUT: Duration = Duration::from_secs(900);

/// Most uncommitted paths a refusal names before it only counts the rest.
const MAX_NAMED_PATHS: usize = 10;
/// Most lines of verify output a refusal carries, taken from the end.
const VERIFY_TAIL_LINES: usize = 25;

/// What the pre-action checks established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapUpChecks {
    /// The verify command that ran and passed, or `None` when the repo has none.
    pub verified: Option<String>,
}

/// The last [`VERIFY_TAIL_LINES`] lines of a command's combined output.
fn output_tail(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(VERIFY_TAIL_LINES)..].join("\n")
}

/// Marks a task's verify run as in flight; dropping it clears the mark.
/// Moved into the blocking worker, so it outlives a caller that gave up.
struct VerifyGuard {
    set: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<crate::models::TaskId>>>,
    id: crate::models::TaskId,
}

impl Drop for VerifyGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.set.lock() {
            set.remove(&self.id);
        }
    }
}

impl TaskService {
    /// Refuse a wrap-up whose worktree has uncommitted changes or whose repo's
    /// verify command does not pass — `WrapUpRefusesUnfinishedWork` in
    /// `docs/specs/mcp-task-tools.allium`.
    ///
    /// The commit check runs first: a dirty tree makes a verify result
    /// meaningless, and it is cheap. Neither check writes anything, so a
    /// refusal leaves the task as it was.
    pub async fn check_wrap_up_ready(&self, task: Task) -> Result<WrapUpChecks, ServiceError> {
        let Some(worktree) = task.worktree.clone() else {
            return Err(ServiceError::Internal(anyhow::anyhow!(
                "check_wrap_up_ready: task has no worktree"
            )));
        };
        // One verify run per task: a retry of a timed-out call must not stack
        // another run beside the one still going.
        let guard = {
            let mut set = self.verifying.lock().unwrap_or_else(|e| e.into_inner());
            if !set.insert(task.id) {
                return Err(ServiceError::Validation(
                    "A verify run for this task is still in progress. Wait for it to finish, \
                     then call wrap_up again."
                        .to_string(),
                ));
            }
            VerifyGuard {
                set: self.verifying.clone(),
                id: task.id,
            }
        };
        let verify = dispatch::fetch_verify_command(&*self.db, &task.repo_path).await;
        let runner = self.runner.clone();

        let joined = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let status = crate::git::git_in(
                &*runner,
                &worktree,
                &["status", "--porcelain"],
                crate::process::SUBPROCESS_TIMEOUT,
            )
            .map_err(|e| format!("Could not check the worktree for uncommitted changes: {e}"))?;
            if !status.status.success() {
                return Err(format!(
                    "Could not check the worktree for uncommitted changes: git status exited {}",
                    status
                        .status
                        .code()
                        .map_or("abnormally".to_string(), |c| c.to_string())
                ));
            }
            let dirty = crate::git::parse_porcelain_files(&status);
            if !dirty.is_empty() {
                let shown = dirty
                    .iter()
                    .take(MAX_NAMED_PATHS)
                    .cloned()
                    .collect::<Vec<_>>();
                let more = dirty.len().saturating_sub(MAX_NAMED_PATHS);
                let more = if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                };
                return Err(format!(
                    "The worktree has uncommitted changes: {}{more}. Commit them, then call \
                     wrap_up again.",
                    shown.join(", ")
                ));
            }

            let Some(command) = verify else {
                return Ok(WrapUpChecks { verified: None });
            };
            // `sh -c` takes the worktree as $0 and the command as $1, so
            // neither needs quoting into the script text.
            let output = runner
                .run_with_timeout(
                    "sh",
                    &["-c", "cd \"$0\" && eval \"$1\"", &worktree, &command],
                    WRAP_UP_VERIFY_TIMEOUT,
                )
                .map_err(|e| {
                    format!(
                        "Verify command `{command}` did not finish: {e}. Fix the cause, commit, \
                         then call wrap_up again."
                    )
                })?;
            if output.status.success() {
                return Ok(WrapUpChecks {
                    verified: Some(command),
                });
            }
            let code = output
                .status
                .code()
                .map_or("a signal".to_string(), |c| c.to_string());
            Err(format!(
                "Verify command `{command}` failed (exit {code}); nothing was rebased or \
                 closed. Fix the cause, commit, then call wrap_up again. Output tail:\n{}",
                output_tail(&output)
            ))
        })
        .await;

        match joined {
            Ok(Ok(checks)) => Ok(checks),
            Ok(Err(message)) => Err(ServiceError::Validation(message)),
            Err(e) => Err(ServiceError::Internal(anyhow::anyhow!(
                "wrap_up checks worker died: {e}"
            ))),
        }
    }

    /// Kill the tmux window a failed teardown row left behind
    /// (`KillWindowOfFailedExitSession`).
    ///
    /// Blocking (it shells out to tmux), so it runs on the blocking pool. A
    /// failed kill is swallowed: the close already persisted, and there is no
    /// recovery a caller could take — the window is either gone or visibly
    /// still there.
    pub async fn kill_session_window(&self, window: TmuxWindow) {
        let runner = self.runner.clone();
        let joined =
            tokio::task::spawn_blocking(move || crate::tmux::kill_window(&window, &*runner)).await;
        match joined {
            Ok(Ok(())) => {}
            // By the time a session closes its window is usually already gone,
            // which is the state this call was asking for — debug, not warn.
            // A kill that was attempted and failed still warns.
            Ok(Err(e)) if crate::tmux::is_window_absent_error(&e) => {
                tracing::debug!("kill_session_window: tmux window was already gone");
            }
            Ok(Err(e)) => tracing::warn!("kill_session_window: tmux kill-window failed: {e:#}"),
            Err(e) => tracing::warn!("kill_session_window: worker died: {e}"),
        }
    }
}
