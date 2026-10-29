//! The session seam: wrap-up, exit-session and close request rows (store-split
//! M5, task #53258; the close row M7, task #53260).
//!
//! Two halves of one flow, and the only two ways the board takes part in a
//! wrap-up or a close:
//!
//! - **Asking** — [`TaskService::request_wrap_up`],
//!   [`TaskService::request_exit_session`] and [`TaskService::close_task`]
//!   (`store-split.allium: RequestWrapUp`, `RequestExitSession`,
//!   `RequestCloseTask`). Thin reducer calls. The module checks them, sets the
//!   Conflict sub_status, applies the close (terminal write, epic rollup,
//!   phoenix respawn), runs the epic chain and records the exit token as the
//!   row; nothing is rebased or killed here.
//! - **Serving** — [`TaskService::serve_session_request`], this host's reaction
//!   to a `requested` row naming it (`HostClaimsSessionRequest` ->
//!   `pr-workflow.allium: WrapUpRebase` / `ExitSessionTeardown`, which an
//!   exit_session and a close row are both served by): claim the row,
//!   do the host work under the row's deadline, report done or failed. The host
//!   writes no task field.

use std::sync::Arc;

use crate::dispatch::{self, DeadlineRunner, DEADLINE_PASSED};
use crate::models::{
    SessionRequest, SessionRequestId, SessionRequestKind, TaskId, TaskStatus, WrapUpMode,
};
use crate::service::ServiceError;
use crate::store::SessionRequestAnswer;

use super::crud::TaskService;

/// How serving one row ended.
#[derive(Debug)]
pub enum SessionOutcome {
    /// The module refused the claim — another board, an earlier delivery or the
    /// deadline got there first. Nothing was done and nothing is owed
    /// (`OnlyTheClaimedSessionRequestIsServed`).
    ClaimRefused(String),
    /// The claim got no answer (a transport error): the row may still be
    /// `requested`, and is this board's to claim again on the next reload.
    /// Nothing was done.
    ClaimNotSent(String),
    /// The host work was done and the module accepted the report.
    Done,
    /// The row ended failed: this host's work failed or stopped at the deadline
    /// (and was reported), or the row failed under it. The string is the reason
    /// the row records.
    Failed(String),
}

/// What [`TaskService::serve_session_request`] made of a row.
#[derive(Debug)]
pub struct ServedSession {
    pub outcome: SessionOutcome,
}

impl ServedSession {
    fn plain(outcome: SessionOutcome) -> Self {
        Self { outcome }
    }
}

impl TaskService {
    /// `RequestWrapUp`: write a wrap_up row for `task_id` naming THIS host. The
    /// host-local checks (committed, verified, branch) have already passed in
    /// the caller. `done` and `pr` come back with a row already done, whose id is
    /// the exit token; a `rebase` row is `requested` for this host's board to
    /// serve. Provisions nothing.
    ///
    /// A refusal is an ordinary [`SessionRequestAnswer::Refused`], not an error:
    /// it carries the module's reason and wrote nothing.
    pub async fn request_wrap_up(
        &self,
        task_id: TaskId,
        action: WrapUpMode,
    ) -> Result<SessionRequestAnswer, ServiceError> {
        Ok(self.db.request_wrap_up(task_id, action).await?)
    }

    /// `RequestExitSession`: the token check, the close, the epic chain and the
    /// exit_session row, in one module transaction. The close is whole or
    /// absent: a refusal changes nothing and leaves `token` valid. Tears
    /// nothing down; this host's board serves the exit_session row.
    pub async fn request_exit_session(
        &self,
        task_id: TaskId,
        token: SessionRequestId,
        action: WrapUpMode,
        pr_url: Option<String>,
    ) -> Result<SessionRequestAnswer, ServiceError> {
        Ok(self
            .db
            .request_exit_session(task_id, token, action, pr_url.as_deref())
            .await?)
    }

    /// `RequestCloseTask`: `update_tasks`' done path. One module transaction
    /// applies the Done close (terminal write, epic rollup, phoenix respawn,
    /// the task's open session request failed "task closed") and, when the task
    /// had a window or a pr-typed url on this host, writes a `close` row that
    /// [`serve_session_request`](Self::serve_session_request) tears down. The
    /// close is whole or absent: `Err` means it is not known to have landed,
    /// and nothing is torn down here either way.
    pub async fn close_task(&self, task_id: TaskId) -> Result<(), ServiceError> {
        self.db.close_task(task_id).await?;
        Ok(())
    }

    /// `HostRestartFailsInterruptedSessionRequests`: fail the rows this host
    /// left `running` when its previous board stopped. Run once, on the first
    /// connection, before any `requested` row is served (W8).
    pub async fn fail_interrupted_session_requests(&self) -> Result<(), ServiceError> {
        self.db.fail_interrupted_session_requests().await?;
        Ok(())
    }

    /// `HostClaimsSessionRequest`, then the row's host work, for one delivered
    /// row: a rebase (`WrapUpRebase`) or a window teardown
    /// (`ExitSessionTeardown`), under the ROW's deadline ([`DeadlineRunner`]).
    ///
    /// Never `Err`: every outcome is a [`ServedSession`]. Only this host's rows
    /// reach here ([`dispatch::SessionRequestObserver`] hands out no other, and
    /// the module refuses a claim from any other host).
    pub async fn serve_session_request(&self, request: SessionRequest) -> ServedSession {
        let id = request.id;
        match self.db.claim_session_request(id).await {
            Ok(true) => {}
            Ok(false) => {
                return ServedSession::plain(SessionOutcome::ClaimRefused(format!(
                    "session request {} is no longer requested",
                    id.0
                )))
            }
            Err(e) => return ServedSession::plain(SessionOutcome::ClaimNotSent(format!("{e:#}"))),
        }
        match request.kind {
            SessionRequestKind::WrapUp => self.serve_rebase(&request).await,
            SessionRequestKind::ExitSession | SessionRequestKind::Close => {
                self.serve_teardown(&request).await
            }
        }
    }

    /// `KillWindowOfFailedExitSession`: a close whose teardown failed — or whose
    /// row was failed by the backstop, a restart or the task's deletion before
    /// it ran — still owes its window a kill, because the close already cleared
    /// the window from the task and nothing else would ever kill it. Best
    /// effort, nothing reported: the row is already finished.
    pub async fn tear_down_failed_exit_window(&self, request: &SessionRequest) {
        if !request.kind.is_teardown() {
            return;
        }
        if let Some(window) = request.tmux_window.clone() {
            self.kill_session_window(window).await;
        }
    }

    // -- WrapUpRebase -----------------------------------------------------------

    /// The row's deadline, or one a fresh row of its kind would have if the row
    /// carries none (a host-work row always does).
    fn session_deadline(&self, request: &SessionRequest) -> chrono::DateTime<chrono::Utc> {
        request.deadline.unwrap_or_else(|| {
            let budget = match request.kind {
                SessionRequestKind::WrapUp => dispatch::WRAP_UP_REBASE_TIMEOUT,
                SessionRequestKind::ExitSession | SessionRequestKind::Close => {
                    dispatch::EXIT_SESSION_TEARDOWN_TIMEOUT
                }
            };
            request.requested_at
                + chrono::Duration::from_std(budget).unwrap_or_else(|_| chrono::Duration::zero())
        })
    }

    fn session_deadline_passed(&self, request: &SessionRequest) -> bool {
        self.clock.now() >= self.session_deadline(request)
    }

    /// Run `finish_task` for a claimed wrap_up row and report it.
    async fn serve_rebase(&self, request: &SessionRequest) -> ServedSession {
        if self.session_deadline_passed(request) {
            return self
                .report_session_failure(request, DEADLINE_PASSED, false)
                .await;
        }
        let Some(task) = self.db.get_task(request.task_id).await else {
            return self
                .report_session_failure(request, "task deleted", false)
                .await;
        };
        let Some(worktree) = task.worktree.clone() else {
            return self
                .report_session_failure(request, "the task has no worktree to rebase", false)
                .await;
        };
        let Some(branch) = dispatch::branch_from_worktree(&worktree) else {
            return self
                .report_session_failure(
                    request,
                    &format!("Cannot derive branch from worktree: {worktree}"),
                    false,
                )
                .await;
        };

        let runner = DeadlineRunner::new(
            Arc::clone(&self.runner),
            Arc::clone(&self.clock),
            self.session_deadline(request),
        );
        // The rebase abort is exempt from the deadline: it runs through the
        // inner runner under its own SUBPROCESS_TIMEOUT.
        let abort_runner = Arc::clone(&self.runner);
        let repo_path = task.repo_path.clone();
        let base_branch = task.base_branch.clone();
        let task_id = task.id;
        tracing::info!(task_id = task_id.0, %branch, request = request.id.0, "wrap_up rebase starting");
        let rebase_repo = repo_path.clone();
        let joined = tokio::task::spawn_blocking(move || {
            dispatch::finish_task_with_abort_runner(
                &dispatch::FinishContext {
                    repo_path: &rebase_repo,
                    worktree: &worktree,
                    branch: &branch,
                    base_branch: &base_branch,
                    timeout: crate::process::SUBPROCESS_TIMEOUT,
                },
                &runner,
                &*abort_runner,
            )
        })
        .await;

        match joined {
            Err(e) => {
                self.report_session_failure(request, &format!("wrap_up worker died: {e}"), false)
                    .await
            }
            Ok(Err(e)) => {
                // `conflict` is true only for a genuine textual rebase
                // conflict; the module then sets the Conflict sub_status.
                let conflict = matches!(e, dispatch::FinishError::RebaseConflict { .. });
                let reason = if !conflict && self.session_deadline_passed(request) {
                    DEADLINE_PASSED.to_string()
                } else {
                    e.to_string()
                };
                self.report_session_failure(request, &reason, conflict)
                    .await
            }
            Ok(Ok(())) => self.report_rebase_done(request).await,
        }
    }

    /// Report a finished rebase. A report that got no answer is sent once more:
    /// the module accepts a repeat of a report that already landed and changes
    /// nothing (`HostRepeatsSessionRequestDone`), so an accepted retry means
    /// the token exists.
    async fn report_rebase_done(&self, request: &SessionRequest) -> ServedSession {
        let id = request.id;
        let report = match self.db.report_session_request_done(id).await {
            Err(first) => {
                tracing::warn!(
                    request = id.0,
                    "the done report got no answer, sending it again: {first:#}"
                );
                self.db.report_session_request_done(id).await
            }
            answered => answered,
        };
        let reason = match report {
            Ok(true) => return ServedSession::plain(SessionOutcome::Done),
            // Refused: the row failed under this host, or its deadline passed
            // before the report landed — the module's clock decides. The rebase
            // stays in git and no token was issued.
            Ok(false) if self.session_deadline_passed(request) => DEADLINE_PASSED.to_string(),
            Ok(false) => "the done report was refused".to_string(),
            Err(e) => format!("could not report the rebase: {e:#}"),
        };
        self.report_session_failure(request, &reason, false).await
    }

    // -- ExitSessionTeardown ----------------------------------------------------

    /// Kill the window the claimed exit_session or close row names and report.
    /// A window that is already gone counts as torn down. A failed teardown
    /// does NOT revert the close: it is recorded on the row and logged.
    ///
    /// A `close` row may name no window (a task carrying only a PR url): then
    /// there is nothing to kill and only `gh pr close` is left. An exit_session
    /// row always has one.
    async fn serve_teardown(&self, request: &SessionRequest) -> ServedSession {
        let failure = match request.tmux_window.clone() {
            Some(window) => self.kill_row_window(request, window).await,
            None if request.kind == SessionRequestKind::Close => None,
            None => {
                return self
                    .report_session_failure(request, "the row names no window to tear down", false)
                    .await
            }
        };

        // ClosePrOnDone, on this route: the module closed the task into Done and
        // only a host can run `gh pr close`. Alongside the kill, best effort.
        self.close_pr_of_exit(request).await;

        match failure {
            None => self.report_session_done(request).await,
            Some(reason) => self.report_session_failure(request, &reason, false).await,
        }
    }

    /// Kill `window` under the row's deadline: `None` when it is gone, the
    /// failure reason otherwise.
    async fn kill_row_window(
        &self,
        request: &SessionRequest,
        window: crate::models::TmuxWindow,
    ) -> Option<String> {
        let runner = DeadlineRunner::new(
            Arc::clone(&self.runner),
            Arc::clone(&self.clock),
            self.session_deadline(request),
        );
        let killed =
            tokio::task::spawn_blocking(move || crate::tmux::kill_window(&window, &runner)).await;
        match killed {
            Ok(Ok(())) => None,
            // By the time a session closes its window is usually already gone,
            // which is the state the kill asks for — debug, not warn.
            Ok(Err(e)) if crate::tmux::is_window_absent_error(&e) => {
                tracing::debug!(
                    request = request.id.0,
                    "the exit_session window was already gone"
                );
                None
            }
            Ok(Err(e)) => Some(if self.session_deadline_passed(request) {
                DEADLINE_PASSED.to_string()
            } else {
                format!("{e:#}")
            }),
            Err(e) => Some(format!("teardown worker died: {e}")),
        }
    }

    /// `pr-workflow.allium: ClosePrOnDone` for a close through exit_session: when
    /// the close took the task into Done (action rebase or done) and the task
    /// carries a pr-typed url, run `gh pr close` on it.
    async fn close_pr_of_exit(&self, request: &SessionRequest) {
        if request.action == WrapUpMode::Pr {
            return;
        }
        let Some(task) = self.db.get_task(request.task_id).await else {
            return;
        };
        if task.status != TaskStatus::Done {
            return;
        }
        if let Some(url) = task.url.as_ref().filter(|u| u.is_pr()) {
            self.spawn_close_attached_pr(task.id, url.url.clone());
        }
    }

    // -- Reports ------------------------------------------------------------------

    /// Report a finished teardown. The close stands whatever the module makes of
    /// a late one, so a refusal only needs logging.
    async fn report_session_done(&self, request: &SessionRequest) -> ServedSession {
        let id = request.id;
        match self.db.report_session_request_done(id).await {
            Ok(true) => {}
            Ok(false) => tracing::info!(request = id.0, "the teardown report was refused"),
            Err(first) => {
                tracing::warn!(
                    request = id.0,
                    "the teardown report got no answer, sending it again: {first:#}"
                );
                if let Err(e) = self.db.report_session_request_done(id).await {
                    tracing::warn!(request = id.0, "could not report the teardown: {e:#}");
                }
            }
        }
        ServedSession::plain(SessionOutcome::Done)
    }

    /// Report `reason` and answer with the reason the row ends up recording —
    /// the module's own when the row had already failed under this host.
    async fn report_session_failure(
        &self,
        request: &SessionRequest,
        reason: &str,
        conflict: bool,
    ) -> ServedSession {
        let id = request.id;
        // A rebase conflict is an expected outcome: it reaches the agent and
        // the card's Conflict sub_status. Only other failures are warnings.
        if conflict {
            tracing::info!(
                request = id.0,
                task_id = request.task_id.0,
                "session request failed: {reason}"
            );
        } else {
            tracing::warn!(
                request = id.0,
                task_id = request.task_id.0,
                "session request failed: {reason}"
            );
        }
        let recorded = match self
            .db
            .report_session_request_failed(id, reason, conflict)
            .await
        {
            Ok(true) => reason.to_string(),
            Ok(false) => self
                .db
                .session_request(id)
                .await
                .and_then(|r| r.reason)
                .unwrap_or_else(|| reason.to_string()),
            Err(e) => {
                tracing::warn!(request = id.0, "could not report the failure: {e:#}");
                reason.to_string()
            }
        };
        ServedSession::plain(SessionOutcome::Failed(recorded))
    }
}
