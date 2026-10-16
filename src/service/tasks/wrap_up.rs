//! Wrap-up business logic the board still owns: the host-local pre-checks
//! `WrapUpRefusesUnfinishedWork` makes before `wrap_up` writes a request, and
//! the best-effort window kill a failed teardown row still owes.
//!
//! Since store-split M5 the rebase and the exit_session teardown are request
//! rows served by this host (`session.rs`), and the Conflict sub_status is the
//! module's. Since M7 so is `update_tasks`' close (a `close` row).

use crate::models::{Task, TmuxWindow};
use crate::service::ServiceError;

use super::crud::TaskService;

/// Most uncommitted paths a refusal names before it only counts the rest.
const MAX_NAMED_PATHS: usize = 10;
/// The commit check: a worktree with uncommitted changes is refused, naming
/// the paths.
fn commit_check(runner: &dyn crate::process::ProcessRunner, worktree: &str) -> Result<(), String> {
    let status = crate::git::git_in(
        runner,
        worktree,
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
    if dirty.is_empty() {
        return Ok(());
    }
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
    Err(format!(
        "The worktree has uncommitted changes: {}{more}. Commit them, then call \
         wrap_up again.",
        shown.join(", ")
    ))
}

/// The commit the worktree is at, or `None` when git cannot say.
fn head_of(runner: &dyn crate::process::ProcessRunner, worktree: &str) -> Option<String> {
    let out = crate::git::git_in(
        runner,
        worktree,
        &["rev-parse", "HEAD"],
        crate::process::SUBPROCESS_TIMEOUT,
    )
    .ok()?;
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !head.is_empty()).then_some(head)
}

impl TaskService {
    /// Refuse a wrap-up whose worktree has uncommitted changes —
    /// `WrapUpRefusesUnfinishedWork` in `docs/specs/mcp-task-tools.allium`.
    /// The board does not run the repo's verify command: the agent does
    /// (`AgentsRunTheVerifyCommand`).
    ///
    /// The check writes nothing, so a refusal leaves the task as it was.
    pub async fn check_wrap_up_ready(&self, task: Task) -> Result<(), ServiceError> {
        let Some(worktree) = task.worktree.clone() else {
            return Err(ServiceError::Internal(anyhow::anyhow!(
                "check_wrap_up_ready: task has no worktree"
            )));
        };
        let runner = self.runner.clone();
        let checked = tokio::task::spawn_blocking(move || commit_check(&*runner, &worktree)).await;
        match checked {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(ServiceError::Validation(message)),
            Err(e) => Err(ServiceError::Internal(anyhow::anyhow!(
                "wrap_up checks worker died: {e}"
            ))),
        }
    }

    /// The commit the worktree is at, or `None` when it cannot be read. Used
    /// around a rebase to tell whether it moved the branch
    /// (`AgentsRunTheVerifyCommand`).
    pub async fn worktree_head(&self, worktree: String) -> Option<String> {
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || head_of(&*runner, &worktree))
            .await
            .ok()
            .flatten()
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
