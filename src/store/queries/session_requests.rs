//! Session requests (store-split M5): reads from this host's subscribed rows,
//! writes as the module's request reducers, always naming THIS host.

use anyhow::Result;

use crate::models::{SessionRequest, SessionRequestId, TaskId, WrapUpMode};
use crate::store::{SessionRequestAnswer, SessionRequestRead, SessionRequestStore};
use crate::sync::ReducerOutcome;

use super::Store;

/// [`ReducerOutcome::won`] for a host-side call — and if it lost, WHY, in the
/// log. A redelivered claim is ordinary; a report refused because the backstop
/// failed the row, or the deadline passed first, is worth reading about.
fn won_logged(outcome: ReducerOutcome, what: &str, id: SessionRequestId) -> bool {
    match outcome {
        ReducerOutcome::Applied(_) => true,
        ReducerOutcome::Refused(why) => {
            tracing::info!(request = id.0, "the shared store refused the {what}: {why}");
            false
        }
    }
}

/// The one id an applied request reads back.
fn requested(outcome: ReducerOutcome, task: TaskId, what: &str) -> Result<SessionRequestAnswer> {
    match outcome {
        ReducerOutcome::Applied(ids) => match ids.as_slice() {
            [id] => Ok(SessionRequestAnswer::Requested(SessionRequestId(*id))),
            _ => anyhow::bail!(
                "the shared store accepted the {what} for task {} but its row is not in this \
                 board's subscription",
                task.0
            ),
        },
        ReducerOutcome::Refused(why) => Ok(SessionRequestAnswer::Refused(why)),
    }
}

#[async_trait::async_trait]
impl SessionRequestRead for Store {
    async fn session_request(&self, id: SessionRequestId) -> Option<SessionRequest> {
        self.rows.session_request(id)
    }

    async fn session_requests(&self) -> Vec<SessionRequest> {
        self.rows.session_requests()
    }

    fn session_request_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.rows.changed()
    }
}

#[async_trait::async_trait]
impl SessionRequestStore for Store {
    /// The requester is this handle's host, always: the rebase runs where the
    /// worktree is, and only that host's board serves the row (W2, W11).
    async fn request_wrap_up(
        &self,
        task: TaskId,
        action: WrapUpMode,
    ) -> Result<SessionRequestAnswer> {
        let outcome = self
            .caller
            .request_wrap_up(task, self.host.clone(), action.as_str().to_string())
            .await?;
        requested(outcome, task, "wrap-up request")
    }

    async fn request_exit_session(
        &self,
        task: TaskId,
        token: SessionRequestId,
        action: WrapUpMode,
        pr_url: Option<&str>,
    ) -> Result<SessionRequestAnswer> {
        let outcome = self
            .caller
            .request_exit_session(
                task,
                token,
                action.as_str().to_string(),
                pr_url.unwrap_or_default().to_string(),
            )
            .await?;
        requested(outcome, task, "exit-session request")
    }

    async fn close_task(&self, task: TaskId) -> Result<()> {
        self.caller
            .close_task(task, self.host.clone())
            .await?
            .applied()
    }

    async fn claim_session_request(&self, id: SessionRequestId) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .claim_session_request(id, self.host.clone())
                .await?,
            "session request claim",
            id,
        ))
    }

    async fn report_session_request_done(&self, id: SessionRequestId) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .report_session_request_done(id, self.host.clone())
                .await?,
            "session request done report",
            id,
        ))
    }

    async fn report_session_request_failed(
        &self,
        id: SessionRequestId,
        reason: &str,
        conflict: bool,
    ) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .report_session_request_failed(id, self.host.clone(), reason.to_string(), conflict)
                .await?,
            "session request failure report",
            id,
        ))
    }

    async fn fail_interrupted_session_requests(&self) -> Result<()> {
        self.caller
            .fail_interrupted_session_requests(self.host.clone())
            .await?
            .applied()
    }
}
