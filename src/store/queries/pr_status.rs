//! PR state (store-split M6): the module's PR reducers, always naming THIS host.

use anyhow::Result;

use crate::models::TaskId;
use crate::store::PrStatusStore;
use crate::sync::ReducerOutcome;

use super::Store;

/// [`ReducerOutcome::won`] for a host-side call — and if it lost, WHY, in the
/// log: a state the module does not know is worth reading about.
fn won_logged(outcome: ReducerOutcome, what: &str, task: TaskId) -> bool {
    match outcome {
        ReducerOutcome::Applied(_) => true,
        ReducerOutcome::Refused(why) => {
            tracing::info!(
                task_id = task.0,
                "the shared store refused the {what}: {why}"
            );
            false
        }
    }
}

#[async_trait::async_trait]
impl PrStatusStore for Store {
    async fn report_pr_status(
        &self,
        task: TaskId,
        state: &str,
        review_decision: &str,
    ) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .report_pr_status(
                    task,
                    self.host.clone(),
                    state.to_string(),
                    review_decision.to_string(),
                )
                .await?,
            "PR status report",
            task,
        ))
    }

    async fn report_pr_unreadable(&self, task: TaskId, reason: &str) -> Result<bool> {
        Ok(won_logged(
            self.caller
                .report_pr_unreadable(task, self.host.clone(), reason.to_string())
                .await?,
            "PR unreadable report",
            task,
        ))
    }
}
