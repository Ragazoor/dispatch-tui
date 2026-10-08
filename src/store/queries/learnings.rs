//! The knowledge base (`docs/specs/learnings.allium`, Storage Backend). It is
//! team-shared, so a read answers from the same store rows a write lands in:
//! a learning recorded on one host's board is visible from every other's.

use anyhow::Result;

use crate::models::{
    EpicId, Learning, LearningId, LearningRetrieval, LearningVerdict, RetrievalSource, TaskId,
};
use crate::sync::encode;

use super::super::{CreateLearningRow, LearningFilter, LearningPatch, Store};

#[async_trait::async_trait]
impl super::super::LearningStore for Store {
    /// Unlike settings, nothing here is scoped by host — a learning's
    /// visibility is governed entirely by its own scope/scope_ref.
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId> {
        let row = encode::create_learning_row(&row, &self.now());
        self.caller.create_learning(row).await
    }

    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>> {
        Ok(self.rows.learning(id))
    }

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>> {
        Ok(self.rows.learnings_matching(&filter))
    }

    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        self.caller
            .patch_learning(id, encode::learning_patch(patch))
            .await?
            .applied()
    }

    /// `Ok(false)` for an id that was never created or was already deleted —
    /// `DeleteLearningViaMcp` refuses in that case (`docs/specs/learnings.allium`),
    /// and the store says so; reported as `false` rather than as an error, so
    /// the service layer's not-found mapping stays a plain bool.
    async fn delete_learning(&self, id: LearningId) -> Result<bool> {
        Ok(self.caller.delete_learning(id).await?.won())
    }

    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>> {
        Ok(self.rows.approved_non_task_learnings_with_embedding())
    }

    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>> {
        Ok(self.rows.learnings_missing_embedding())
    }

    /// Always `0`. The archived count is best-effort telemetry, not a
    /// correctness signal — its one caller
    /// (`runtime::learnings::exec_archive_stale_learnings`) only logs it. A
    /// reducer cannot answer with a value
    /// (`sync.allium: EveryMutationIsAtomicAndAnswered`), and the read-back
    /// machinery `create_task` needs for its id is not worth it for this.
    async fn archive_stale_learnings(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        self.caller
            .archive_stale_learnings(cutoff)
            .await?
            .applied()?;
        Ok(0)
    }

    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<()> {
        self.caller
            .rescope_epic_learnings(from, to)
            .await?
            .applied()
    }
}

#[async_trait::async_trait]
impl super::super::LearningRetrievalStore for Store {
    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<()> {
        self.caller
            .record_learning_retrieval(task_id, learning_id, source)
            .await?
            .applied()
    }

    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>> {
        Ok(self.rows.retrievals_for_task(task_id))
    }

    async fn apply_verdicts_tx(&self, verdicts: &[(LearningId, LearningVerdict)]) -> Result<()> {
        self.caller
            .apply_learning_verdicts(verdicts.to_vec())
            .await?
            .applied()
    }
}
