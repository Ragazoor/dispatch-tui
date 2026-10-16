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

/// The commit the worktree is at. Empty output is an error: a blank key would
/// let one tree's outcome stand for another's.
fn head_of(runner: &dyn crate::process::ProcessRunner, worktree: &str) -> Result<String, String> {
    let out = crate::git::git_in(
        runner,
        worktree,
        &["rev-parse", "HEAD"],
        crate::process::SUBPROCESS_TIMEOUT,
    )
    .map_err(|e| format!("Could not read the worktree's HEAD commit: {e}"))?;
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || head.is_empty() {
        return Err("Could not read the worktree's HEAD commit: git rev-parse HEAD failed".into());
    }
    Ok(head)
}

fn outcome_to_checks(
    outcome: VerifyOutcome,
    command: String,
) -> Result<WrapUpChecks, ServiceError> {
    match outcome {
        Ok(()) => Ok(WrapUpChecks {
            verified: Some(command),
        }),
        Err(message) => Err(ServiceError::Validation(message)),
    }
}

/// How long one `wrap_up` call waits for a verify run before it gives up and
/// tells the agent to call again (`wrap_up_verify_wait` in
/// `docs/specs/mcp-task-tools.allium`). Below the agent's own tool-call
/// timeout of about a minute: the run is not cut short, only the wait.
pub const WRAP_UP_VERIFY_WAIT: Duration = Duration::from_secs(45);

/// What one verify run was run against: the commit the worktree was at and the
/// command text. An outcome applies to this pair and to nothing else.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct VerifyKey {
    head: String,
    command: String,
}

/// A finished verify run: `Ok` for a pass, `Err` carrying the refusal text the
/// agent got, so a recorded failure can be repeated word for word.
type VerifyOutcome = Result<(), String>;

/// The board's memory of verify runs, host-local and in memory only
/// (`Verify outcome memory` in `docs/specs/mcp-task-tools.allium`). One run per
/// task at a time; the last finished outcome per task.
#[derive(Default)]
pub(super) struct VerifyBook {
    in_flight: std::collections::HashMap<
        crate::models::TaskId,
        (
            VerifyKey,
            tokio::sync::watch::Receiver<Option<VerifyOutcome>>,
        ),
    >,
    done: std::collections::HashMap<crate::models::TaskId, (VerifyKey, VerifyOutcome)>,
}

/// What the verify book holds for a (task, HEAD, command) key.
enum VerifyRun {
    Recorded(VerifyOutcome),
    Running(tokio::sync::watch::Receiver<Option<VerifyOutcome>>),
}

type SharedVerifyBook = std::sync::Arc<std::sync::Mutex<VerifyBook>>;

/// Clears a task's in-flight mark if the worker dies before it records an
/// outcome, so a panic cannot leave the task refused forever.
struct InFlightGuard {
    book: SharedVerifyBook,
    id: crate::models::TaskId,
    armed: bool,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut book) = self.book.lock() {
                book.in_flight.remove(&self.id);
            }
        }
    }
}

fn still_running() -> ServiceError {
    ServiceError::Validation(
        "Verify is still running for this task; it carries on in the background. Wait a \
         minute, then call wrap_up again."
            .to_string(),
    )
}

/// The shell script that runs the verify command in the worktree. `sh -c`
/// takes the worktree as $0 and the command as $1, so neither needs quoting
/// into the script text. With sccache available, the run shares the compiler
/// cache that agent windows use, unless the board already names a wrapper.
fn verify_script(sccache: bool) -> &'static str {
    if sccache {
        "cd \"$0\" && : \"${RUSTC_WRAPPER:=sccache}\" && export RUSTC_WRAPPER && eval \"$1\""
    } else {
        "cd \"$0\" && eval \"$1\""
    }
}

/// Run the verify command to the end and shape its outcome.
fn run_verify(
    runner: &dyn crate::process::ProcessRunner,
    worktree: &str,
    command: &str,
) -> VerifyOutcome {
    let output = runner
        .run_with_timeout(
            "sh",
            &[
                "-c",
                verify_script(runner.sccache_available()),
                worktree,
                command,
            ],
            WRAP_UP_VERIFY_TIMEOUT,
        )
        .map_err(|e| {
            format!(
                "Verify command `{command}` did not finish: {e}. Fix the cause, commit, \
                 then call wrap_up again."
            )
        })?;
    if output.status.success() {
        return Ok(());
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
}

impl TaskService {
    /// Override how long a `wrap_up` call waits for a verify run. Tests shorten
    /// it; production keeps [`WRAP_UP_VERIFY_WAIT`].
    #[cfg(test)]
    pub fn with_verify_wait(mut self, wait: Duration) -> Self {
        self.verify_wait = wait;
        self
    }

    /// Refuse a wrap-up whose worktree has uncommitted changes or whose repo's
    /// verify command does not pass — `WrapUpRefusesUnfinishedWork` in
    /// `docs/specs/mcp-task-tools.allium`.
    ///
    /// The commit check runs first: a dirty tree makes a verify result
    /// meaningless, and it is cheap. Neither check writes anything, so a
    /// refusal leaves the task as it was.
    ///
    /// Verify outcomes are remembered per (task, HEAD, command). A retry of a
    /// call that gave up on a slow run joins that run, or reads its recorded
    /// outcome, instead of paying for the run again.
    pub async fn check_wrap_up_ready(&self, task: Task) -> Result<WrapUpChecks, ServiceError> {
        let Some(worktree) = task.worktree.clone() else {
            return Err(ServiceError::Internal(anyhow::anyhow!(
                "check_wrap_up_ready: task has no worktree"
            )));
        };
        let verify = dispatch::fetch_verify_command(&*self.db, &task.repo_path).await;

        // The commit check, and HEAD when a verify command makes it the key.
        let head = self
            .commit_check_and_head(&worktree, verify.is_some())
            .await?;
        let Some(command) = verify else {
            return Ok(WrapUpChecks { verified: None });
        };

        let key = VerifyKey {
            head,
            command: command.clone(),
        };
        let mut rx = match self.join_or_start_verify(task.id, key, worktree)? {
            VerifyRun::Recorded(outcome) => return outcome_to_checks(outcome, command),
            VerifyRun::Running(rx) => rx,
        };

        let waited = tokio::time::timeout(self.verify_wait, rx.wait_for(|o| o.is_some())).await;
        let outcome = match waited {
            Ok(Ok(seen)) => (*seen).clone(),
            Ok(Err(_)) => {
                return Err(ServiceError::Internal(anyhow::anyhow!(
                    "wrap_up checks worker died"
                )))
            }
            Err(_) => return Err(still_running()),
        };
        outcome_to_checks(outcome.unwrap_or(Ok(())), command)
    }

    /// Run the commit check on the blocking pool and, when `need_head`, read
    /// HEAD too (an empty string otherwise).
    async fn commit_check_and_head(
        &self,
        worktree: &str,
        need_head: bool,
    ) -> Result<String, ServiceError> {
        let runner = self.runner.clone();
        let worktree = worktree.to_owned();
        let pre = tokio::task::spawn_blocking(move || {
            commit_check(&*runner, &worktree).and_then(|()| {
                if need_head {
                    head_of(&*runner, &worktree)
                } else {
                    Ok(String::new())
                }
            })
        })
        .await;
        match pre {
            Ok(Ok(head)) => Ok(head),
            Ok(Err(message)) => Err(ServiceError::Validation(message)),
            Err(e) => Err(ServiceError::Internal(anyhow::anyhow!(
                "wrap_up checks worker died: {e}"
            ))),
        }
    }

    /// Look `key` up in the verify book: a recorded outcome, the run already in
    /// flight, or a new detached run that records its outcome for the retry.
    fn join_or_start_verify(
        &self,
        task_id: crate::models::TaskId,
        key: VerifyKey,
        worktree: String,
    ) -> Result<VerifyRun, ServiceError> {
        let mut book = self.verifying.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, outcome)) = book.done.get(&task_id) {
            if *k == key {
                return Ok(VerifyRun::Recorded(outcome.clone()));
            }
        }
        match book.in_flight.get(&task_id) {
            Some((k, rx)) if *k == key => return Ok(VerifyRun::Running(rx.clone())),
            // The agent committed (or the command changed) since that run
            // started: its outcome would be for another tree, and a second
            // run beside it is what one-run-per-task forbids.
            Some(_) => return Err(still_running()),
            None => {}
        }
        let (tx, rx) = tokio::sync::watch::channel(None);
        book.in_flight.insert(task_id, (key.clone(), rx.clone()));
        let guard = InFlightGuard {
            book: self.verifying.clone(),
            id: task_id,
            armed: true,
        };
        let book_handle = self.verifying.clone();
        let runner = self.runner.clone();
        // Detached: it outlives a caller that gave up, and records
        // its outcome for the retry.
        drop(tokio::task::spawn_blocking(move || {
            let mut guard = guard;
            let outcome = run_verify(&*runner, &worktree, &key.command);
            {
                let mut book = book_handle.lock().unwrap_or_else(|e| e.into_inner());
                book.in_flight.remove(&task_id);
                book.done.insert(task_id, (key, outcome.clone()));
            }
            guard.armed = false;
            let _ = tx.send(Some(outcome));
        }));
        Ok(VerifyRun::Running(rx))
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
