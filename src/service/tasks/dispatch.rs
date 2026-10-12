//! The dispatch seam: request rows (store-split M4, task #53257).
//!
//! Two halves of one flow, and the only two ways the board takes part in a
//! dispatch:
//!
//! - **Asking** — [`TaskService::request_dispatch`], the one call every launch
//!   path makes (`store-split.allium: RequestDispatch`, D6): the TUI keys and
//!   retry-fresh, quick dispatch, MCP `dispatch_task` and the epic chain. The
//!   module claims the task and writes a `requested` row naming this host in
//!   one transaction. Nothing is provisioned here, so `DispatchClaimExclusive`
//!   (`docs/specs/dispatch.allium`) is the module's: a second request for a
//!   claimed task is refused, whichever entry point sends it.
//! - **Provisioning** — [`TaskService::serve_dispatch_request`], this host's
//!   reaction to a `requested` row naming it (`store-split.allium:
//!   HostClaimsDispatch`, `dispatch.allium: ProvisionDispatch`): claim the row,
//!   provision with the row's mode under the row's deadline, and report
//!   dispatched or failed. The module writes the task's worktree, window and
//!   host, and releases the claim on any failure; the host releases nothing.

use std::sync::Arc;

use crate::dispatch::{self, DeadlineRunner, DEADLINE_PASSED};
use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchResult, TaskId,
};
use crate::service::ServiceError;
use crate::store::RequestDispatchAnswer;

use super::crud::TaskService;

/// What [`TaskService::serve_dispatch_request`] made of a row.
#[derive(Debug)]
pub enum ServedDispatch {
    /// The row names another machine; it is not this board's to claim
    /// (`HostClaimsDispatch`'s `requires: request.host = local_host()`).
    NotThisHost,
    /// The claim was refused — another board or an earlier delivery took the
    /// row, or it already finished — or could not be sent. Nothing was
    /// provisioned and nothing is owed (`OnlyTheClaimedRequestProvisions`).
    ClaimRefused(String),
    /// The agent is running and the module recorded the worktree and window.
    Dispatched(DispatchResult),
    /// The row ended failed: this host's provisioning failed or stopped at the
    /// deadline (rolled back and reported), or the row failed under it (torn
    /// down). The string is the reason the row records.
    Failed(String),
}

impl TaskService {
    /// `RequestDispatch`: claim `task_id` and write a `requested` row naming
    /// THIS host. `mode` `None` lets the module derive it (D3). Provisions
    /// nothing.
    ///
    /// A refusal is an ordinary [`RequestDispatchAnswer::Refused`], not an
    /// error: it is how a caller learns another entry point got there first.
    pub async fn request_dispatch(
        &self,
        task_id: TaskId,
        origin: DispatchOrigin,
        mode: Option<DispatchMode>,
    ) -> Result<RequestDispatchAnswer, ServiceError> {
        // The module rolls up the claimed task's epic chain in the same
        // transaction (`RequestDispatch`), so nothing is recalculated here.
        Ok(self.db.request_dispatch(task_id, origin, mode).await?)
    }

    /// `HostRestartFailsInterrupted`: fail the rows this host left
    /// `provisioning` when its previous board stopped, releasing their claims.
    /// Run once, on the first connection, before any `requested` row is served
    /// (D5).
    ///
    /// The module rolls up each released task's epic chain itself
    /// (`ReleaseClaimOfFailedRequest`), so nothing is recalculated here.
    pub async fn fail_interrupted_dispatch_requests(&self) -> Result<(), ServiceError> {
        self.db.fail_interrupted_dispatch_requests().await?;
        Ok(())
    }

    /// Whether the worktree a request this board provisioned was reused rather
    /// than created — what `dispatch_task`'s success text names. Only
    /// provisioning can tell (the row has no column for it), so it is kept
    /// here, in memory, for the requests this board served. `None` for any
    /// other request.
    pub async fn dispatched_worktree_reused(&self, id: DispatchRequestId) -> Option<bool> {
        self.reused_worktrees
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .copied()
    }

    /// `HostClaimsDispatch` then `ProvisionDispatch` for one delivered row.
    ///
    /// Claims the row (exactly one board's claim is accepted), provisions with
    /// the ROW's mode — never re-derived — under the ROW's deadline
    /// ([`DeadlineRunner`]), and reports. Every failure is rolled back by the
    /// provisioning itself and reported; the module releases the claim. A row
    /// that failed under this host — the backstop, or the task's deletion — is
    /// learnt from the refused report and torn down here, so no agent keeps
    /// running for a task that is back in backlog or gone.
    ///
    /// Never `Err`: every outcome is a [`ServedDispatch`].
    pub async fn serve_dispatch_request(
        &self,
        request: DispatchRequest,
        emb_svc: Arc<crate::embeddings::EmbeddingService>,
    ) -> ServedDispatch {
        if request.host != self.db.host_id() {
            return ServedDispatch::NotThisHost;
        }
        let id = request.id;
        match self.db.claim_dispatch_request(id).await {
            Ok(true) => {}
            Ok(false) => {
                return ServedDispatch::ClaimRefused(format!(
                    "dispatch request {} is no longer requested",
                    id.0
                ))
            }
            Err(e) => return ServedDispatch::ClaimRefused(format!("{e:#}")),
        }

        if self.deadline_passed(&request) {
            return self.report_failure(&request, DEADLINE_PASSED).await;
        }
        let Some(task) = self.db.get_task(request.task_id).await else {
            return self.report_failure(&request, "task deleted").await;
        };

        // The prologue runs a local embedding inference and several writes, so
        // it happens before the task is handed to the blocking pool.
        let inputs = dispatch::prepare_inputs(&*self.db, &task, &emb_svc).await;
        let runner = DeadlineRunner::new(
            Arc::clone(&self.runner),
            Arc::clone(&self.clock),
            request.deadline,
        );
        let mode = request.mode;
        tracing::info!(
            task_id = task.id.0,
            request = id.0,
            label = mode.label(),
            "provisioning dispatch request"
        );
        let repo_path = task.repo_path.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            dispatch::run_agent_for_mode(&task, mode, &runner, inputs)
        })
        .await;

        let provisioned = match outcome {
            Ok(Ok(dr)) => dr,
            Ok(Err(e)) => {
                // The provisioning rolled back what it made before returning.
                let reason = if self.deadline_passed(&request) {
                    DEADLINE_PASSED.to_string()
                } else {
                    format!("{e:#}")
                };
                return self.report_failure(&request, &reason).await;
            }
            Err(e) => {
                return self
                    .report_failure(&request, &format!("dispatch worker died: {e}"))
                    .await
            }
        };

        // Checked once more after the last step: an attempt that finished past
        // its deadline is not reported dispatched (ProvisioningStopsAtTheDeadline).
        if self.deadline_passed(&request) {
            self.tear_down(&repo_path, &provisioned).await;
            return self.report_failure(&request, DEADLINE_PASSED).await;
        }

        match self
            .db
            .report_dispatch_provisioned(id, &provisioned.worktree_path, &provisioned.tmux_window)
            .await
        {
            Ok(true) => {
                self.reused_worktrees
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, provisioned.reused_worktree);
                ServedDispatch::Dispatched(provisioned)
            }
            Ok(false) => {
                // The row failed under this host and the report was refused:
                // nothing was recorded on the task, so what this attempt made
                // must go.
                self.tear_down(&repo_path, &provisioned).await;
                let reason = self.row_reason(id).await.unwrap_or_else(|| {
                    "the dispatch request failed while it was being provisioned".to_string()
                });
                ServedDispatch::Failed(reason)
            }
            Err(e) => {
                // Unknown whether the store recorded it. Tear down rather than
                // leave an agent running against a task whose request the
                // backstop will fail and release.
                self.tear_down(&repo_path, &provisioned).await;
                self.report_failure(&request, &format!("could not report the dispatch: {e:#}"))
                    .await
            }
        }
    }

    fn deadline_passed(&self, request: &DispatchRequest) -> bool {
        self.clock.now() >= request.deadline
    }

    /// Report `reason` and answer with the reason the row ends up recording —
    /// the module's own when the row had already failed under this host.
    async fn report_failure(&self, request: &DispatchRequest, reason: &str) -> ServedDispatch {
        let id = request.id;
        tracing::warn!(
            request = id.0,
            task_id = request.task_id.0,
            "dispatch failed: {reason}"
        );
        let recorded = match self.db.report_dispatch_failed(id, reason).await {
            Ok(true) => reason.to_string(),
            Ok(false) => self
                .row_reason(id)
                .await
                .unwrap_or_else(|| reason.to_string()),
            Err(e) => {
                tracing::warn!(request = id.0, "could not report the failure: {e:#}");
                reason.to_string()
            }
        };
        // The module rolled up the task's epic chain with the release
        // (`ReleaseClaimOfFailedRequest`); nothing to recalculate here.
        ServedDispatch::Failed(recorded)
    }

    async fn row_reason(&self, id: DispatchRequestId) -> Option<String> {
        self.db.dispatch_request(id).await.and_then(|r| r.reason)
    }

    /// The provisioning-failure rollback, after the fact: the window this
    /// attempt opened and, only if this attempt created it, the worktree and
    /// its branch. A reused worktree predates the attempt and is left alone.
    ///
    /// The window is killed without a presence check first: this attempt
    /// opened it moments ago, and `kill_window` resolves it by its exact name,
    /// so a window that is already gone fails the kill rather than reaching a
    /// prefix-matched sibling. Then the worktree half of `TaskTeardown`.
    async fn tear_down(&self, repo_path: &str, provisioned: &DispatchResult) {
        let runner = Arc::clone(&self.runner);
        let repo_path = crate::models::expand_tilde(repo_path);
        let worktree = (!provisioned.reused_worktree).then(|| provisioned.worktree_path.clone());
        let window = provisioned.tmux_window.clone();
        let torn = tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::tmux::kill_window(&window, &*runner) {
                tracing::warn!(%window, "could not kill the window of a failed dispatch: {e:#}");
            }
            dispatch::teardown_task(&repo_path, worktree.as_deref(), None, &*runner)
                .map_err(|e| e.to_string())
        })
        .await;
        match torn {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(
                worktree = %provisioned.worktree_path,
                "failed to tear down a dispatch whose request failed: {e}"
            ),
            Err(e) => tracing::warn!("the teardown of a failed dispatch died: {e}"),
        }
    }
}
