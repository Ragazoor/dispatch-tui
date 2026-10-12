//! Dispatch requests (store-split M4): reads from this host's subscribed rows,
//! writes as the module's request reducers, always naming THIS host.

use anyhow::Result;

use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, TaskId, TmuxWindow,
};
use crate::store::{DispatchRequestRead, DispatchRequestStore, RequestDispatchAnswer};
use crate::sync::ReducerOutcome;

use super::Store;

/// Whether a host-side call applied — and if it did not, WHY, in the log.
///
/// The caller's next move is the same whichever reason it was refused for, but
/// the reasons are not the same (`sync.allium: StoreRejectsAnInvalidMutation`
/// says a rejection carries one): a redelivered claim is ordinary, a report
/// refused because the backstop failed the row is worth reading about.
fn applied(outcome: ReducerOutcome, what: &str, id: DispatchRequestId) -> bool {
    match outcome {
        ReducerOutcome::Applied(_) => true,
        ReducerOutcome::Refused(why) => {
            tracing::info!(request = id.0, "the shared store refused the {what}: {why}");
            false
        }
    }
}

#[async_trait::async_trait]
impl DispatchRequestRead for Store {
    async fn dispatch_request(&self, id: DispatchRequestId) -> Option<DispatchRequest> {
        self.rows.dispatch_request(id)
    }

    async fn dispatch_requests(&self) -> Vec<DispatchRequest> {
        self.rows.dispatch_requests()
    }

    async fn latest_dispatch_request_for_task(&self, task: TaskId) -> Option<DispatchRequest> {
        self.rows.latest_dispatch_request_for_task(task)
    }

    fn dispatch_request_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.rows.changed()
    }
}

#[async_trait::async_trait]
impl DispatchRequestStore for Store {
    /// The requester is this handle's host, always: a request is provisioned
    /// by the machine that made it (D2).
    async fn request_dispatch(
        &self,
        task: TaskId,
        origin: DispatchOrigin,
        mode: Option<DispatchMode>,
    ) -> Result<RequestDispatchAnswer> {
        let mode = mode.map(DispatchMode::as_str).unwrap_or_default();
        match self
            .caller
            .request_dispatch(
                task,
                self.host.clone(),
                origin.as_str().to_string(),
                mode.to_string(),
            )
            .await?
        {
            ReducerOutcome::Applied(ids) => match ids.as_slice() {
                [id] => Ok(RequestDispatchAnswer::Requested(DispatchRequestId(*id))),
                _ => anyhow::bail!(
                    "the shared store made a dispatch request for task {} but it is not in \
                     this board's subscription",
                    task.0
                ),
            },
            ReducerOutcome::Refused(why) => Ok(RequestDispatchAnswer::Refused(why)),
        }
    }

    async fn claim_dispatch_request(&self, id: DispatchRequestId) -> Result<bool> {
        Ok(applied(
            self.caller
                .claim_dispatch_request(id, self.host.clone())
                .await?,
            "dispatch request claim",
            id,
        ))
    }

    async fn report_dispatch_provisioned(
        &self,
        id: DispatchRequestId,
        worktree: &str,
        tmux_window: &TmuxWindow,
    ) -> Result<bool> {
        Ok(applied(
            self.caller
                .report_dispatch_provisioned(
                    id,
                    self.host.clone(),
                    worktree.to_string(),
                    tmux_window.as_str().to_string(),
                )
                .await?,
            "dispatched report",
            id,
        ))
    }

    async fn report_dispatch_failed(&self, id: DispatchRequestId, reason: &str) -> Result<bool> {
        Ok(applied(
            self.caller
                .report_dispatch_failed(id, self.host.clone(), reason.to_string())
                .await?,
            "dispatch failure report",
            id,
        ))
    }

    async fn fail_interrupted_dispatch_requests(&self) -> Result<()> {
        self.caller
            .fail_interrupted_dispatch_requests(self.host.clone())
            .await?
            .applied()
    }
}
