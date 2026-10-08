use anyhow::Result;

use crate::models::{
    EpicId, Learning, LearningId, LearningRetrieval, LearningVerdict, RetrievalSource, TaskId,
};

use super::super::{CreateLearningRow, LearningFilter, LearningPatch, Store};

#[async_trait::async_trait]
impl super::super::LearningStore for Store {
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId> {
        let writer = self.shared_writer()?;
        writer.create_learning(row).await
    }

    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>> {
        let reader = self.shared_learning_reader()?;
        reader.get_learning(id).await
    }

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>> {
        let reader = self.shared_learning_reader()?;
        reader.list_learnings(filter).await
    }

    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        let writer = self.shared_writer()?;
        writer.patch_learning(id, patch).await
    }

    async fn delete_learning(&self, id: LearningId) -> Result<bool> {
        let writer = self.shared_writer()?;
        writer.delete_learning(id).await
    }

    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>> {
        let reader = self.shared_learning_reader()?;
        reader.list_all_approved_non_task_learnings().await
    }

    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>> {
        let reader = self.shared_learning_reader()?;
        reader.list_learnings_missing_embedding().await
    }

    async fn archive_stale_learnings(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        let writer = self.shared_writer()?;
        writer.archive_stale_learnings(cutoff).await
    }

    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.rescope_epic_learnings(from, to).await
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
        let writer = self.shared_writer()?;
        writer
            .record_learning_retrieval(task_id, learning_id, source)
            .await
    }

    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>> {
        let reader = self.shared_learning_reader()?;
        reader.list_retrievals_for_task(task_id).await
    }

    async fn apply_verdicts_tx(&self, verdicts: &[(LearningId, LearningVerdict)]) -> Result<()> {
        let writer = self.shared_writer()?;
        writer.apply_learning_verdicts(verdicts).await
    }
}
