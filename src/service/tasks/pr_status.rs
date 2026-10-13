//! PR state (store-split M6, task #53259): the board posts what `gh` printed
//! and the module decides every task write (`store-split.allium:
//! ReportPrStatus`, `ReportPrUnreadable`). Nothing here interprets the strings.

use crate::models::TaskId;
use crate::service::ServiceError;

use super::crud::TaskService;

impl TaskService {
    /// `ReportPrStatus`: `true` when the module accepted the report, `false`
    /// when it refused a string it does not know.
    pub async fn report_pr_status(
        &self,
        task_id: TaskId,
        state: String,
        review_decision: String,
    ) -> Result<bool, ServiceError> {
        Ok(self
            .db
            .report_pr_status(task_id, &state, &review_decision)
            .await?)
    }

    /// `ReportPrUnreadable`: the host gave up reading the task's PR.
    pub async fn report_pr_unreadable(
        &self,
        task_id: TaskId,
        reason: String,
    ) -> Result<bool, ServiceError> {
        Ok(self.db.report_pr_unreadable(task_id, &reason).await?)
    }
}
