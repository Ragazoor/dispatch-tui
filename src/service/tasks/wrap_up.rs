//! Wrap-up business logic: the rebase half of `WrapUpRebase`
//! (`docs/specs/pr-workflow.allium`) and the session-window teardown
//! `ExitSession` owes.
//!
//! Both used to live inside the MCP JSON-RPC handler — including a direct
//! `crate::tmux::kill_window` call, the only one any handler made. There is no
//! board-initiated finish path today, so that was defensible; the moment one is
//! added it would have become a second copy of the same drift problem the
//! dispatch seam next door exists to prevent. The handler now owns request
//! parsing and response shaping only.

use std::time::Duration;

use crate::dispatch;
use crate::models::{SubStatus, Task, TmuxWindow};
use crate::service::{ServiceError, UpdateTaskParams};

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

/// What a [`TaskService::wrap_up_rebase`] attempt made of the task.
///
/// The two "cannot even start" arms are separate variants rather than one
/// error string because the handler answers them with different JSON-RPC
/// codes: a missing worktree is a broken internal contract, an underivable
/// branch is a bad request.
#[derive(Debug)]
pub enum WrapUpRebaseOutcome {
    /// The branch was rebased onto its base. The task's `Conflict` sub_status,
    /// if it had one, is cleared.
    Rebased,
    /// `validate_wrap_up` returned a task with no worktree. Defence in depth:
    /// `is_wrappable` guarantees `Some` today, but a future change to the
    /// validator could silently break that contract, and this must not panic.
    MissingWorktree,
    /// The worktree path does not name a branch.
    UnderivableBranch { worktree: String },
    /// The rebase ran and failed. On a rebase conflict specifically, the task
    /// also now carries [`SubStatus::Conflict`] — that is a side effect
    /// observed on the row, deliberately not reported here: every caller
    /// answers both failures identically, and `message` already names the
    /// conflict.
    Failed { message: String },
    /// The blocking rebase worker died (panic or cancellation). Nothing is
    /// known about what the rebase did, so no sub_status is written.
    WorkerDied(String),
}

impl TaskService {
    /// Rebase `task`'s branch onto its base branch, maintaining the
    /// `Conflict` sub_status either side of it.
    ///
    /// The sub_status is cleared *optimistically before* the rebase so the card
    /// is not left visually flagged while the rebase runs, and set again only
    /// on a conflict — `WrapUpRebase` in `docs/specs/pr-workflow.allium`.
    ///
    /// Never returns `Err`: every failure is a [`WrapUpRebaseOutcome`] the
    /// caller shapes into its own response.
    pub async fn wrap_up_rebase(&self, task: Task) -> WrapUpRebaseOutcome {
        let Some(worktree) = task.worktree.clone() else {
            return WrapUpRebaseOutcome::MissingWorktree;
        };
        let Some(branch) = dispatch::branch_from_worktree(&worktree) else {
            return WrapUpRebaseOutcome::UnderivableBranch { worktree };
        };

        let task_id = task.id;
        let repo_path = task.repo_path.clone();
        let base_branch = task.base_branch.clone();
        let runner = self.runner.clone();

        self.clear_conflict_sub_status_if_set(&task).await;

        let joined = tokio::task::spawn_blocking(move || {
            tracing::info!(task_id = task_id.0, %branch, "wrap_up rebase starting");
            dispatch::finish_task(
                &dispatch::FinishContext {
                    repo_path: &repo_path,
                    worktree: &worktree,
                    branch: &branch,
                    base_branch: &base_branch,
                    timeout: crate::process::SUBPROCESS_TIMEOUT,
                },
                &*runner,
            )
        })
        .await;

        let rebase_result = match joined {
            Ok(r) => r,
            Err(e) => return WrapUpRebaseOutcome::WorkerDied(format!("{e}")),
        };

        match rebase_result {
            Ok(()) => WrapUpRebaseOutcome::Rebased,
            Err(e) => {
                if matches!(e, dispatch::FinishError::RebaseConflict { .. }) {
                    let patch = UpdateTaskParams::for_task(task_id).sub_status(SubStatus::Conflict);
                    if let Err(e) = self.update_task(patch).await {
                        tracing::warn!(
                            task_id = task_id.0,
                            "wrap_up: failed to set conflict sub_status: {e}"
                        );
                    }
                }
                WrapUpRebaseOutcome::Failed {
                    message: format!("{e}"),
                }
            }
        }
    }

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
        let verify = dispatch::fetch_verify_command(&*self.db, &task.repo_path).await;
        let runner = self.runner.clone();

        let joined = tokio::task::spawn_blocking(move || {
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

    /// Kill the tmux window a closed session left behind.
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

    /// Optimistically clear a `Conflict` sub_status before rebasing, so the
    /// task is no longer visually flagged while the rebase runs.
    async fn clear_conflict_sub_status_if_set(&self, task: &Task) {
        if task.sub_status != SubStatus::Conflict {
            return;
        }
        let clear =
            UpdateTaskParams::for_task(task.id).sub_status(SubStatus::default_for(task.status));
        if let Err(e) = self.update_task(clear).await {
            tracing::warn!(
                task_id = task.id.0,
                "wrap_up: failed to clear conflict sub_status: {e}"
            );
        }
    }
}
