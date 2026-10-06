use anyhow::{bail, Result};

use crate::models::{
    EpicId, Learning, LearningId, LearningRetrieval, LearningVerdict, RetrievalSource, TaskId,
};

use super::super::{CreateLearningRow, Database, LearningFilter, LearningPatch};

/// Learnings live only in the shared store (`learnings.allium`); this handle
/// has no SQLite table for them. A handle with no store attached refuses.
fn no_store<T>(op: &str) -> Result<T> {
    bail!("{op}: no shared store attached, and learnings are not kept in SQLite")
}

#[async_trait::async_trait]
impl super::super::LearningStore for Database {
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId> {
        match self.shared_writer() {
            Some(writer) => writer.create_learning(row).await,
            None => no_store("create_learning"),
        }
    }

    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>> {
        match self.shared_learning_reader() {
            Some(reader) => reader.get_learning(id).await,
            None => no_store("get_learning"),
        }
    }

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>> {
        match self.shared_learning_reader() {
            Some(reader) => reader.list_learnings(filter).await,
            None => no_store("list_learnings"),
        }
    }

    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        match self.shared_writer() {
            Some(writer) => writer.patch_learning(id, patch).await,
            None => no_store("patch_learning"),
        }
    }

    async fn delete_learning(&self, id: LearningId) -> Result<bool> {
        match self.shared_writer() {
            Some(writer) => writer.delete_learning(id).await,
            None => no_store("delete_learning"),
        }
    }

    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>> {
        match self.shared_learning_reader() {
            Some(reader) => reader.list_all_approved_non_task_learnings().await,
            None => no_store("list_all_approved_non_task_learnings"),
        }
    }

    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>> {
        match self.shared_learning_reader() {
            Some(reader) => reader.list_learnings_missing_embedding().await,
            None => no_store("list_learnings_missing_embedding"),
        }
    }

    async fn archive_stale_learnings(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        match self.shared_writer() {
            Some(writer) => writer.archive_stale_learnings(cutoff).await,
            None => no_store("archive_stale_learnings"),
        }
    }

    async fn rescope_epic_learnings(&self, from: EpicId, to: EpicId) -> Result<()> {
        match self.shared_writer() {
            Some(writer) => writer.rescope_epic_learnings(from, to).await,
            None => no_store("rescope_epic_learnings"),
        }
    }
}

#[async_trait::async_trait]
impl super::super::LearningRetrievalStore for Database {
    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<()> {
        match self.shared_writer() {
            Some(writer) => {
                writer
                    .record_learning_retrieval(task_id, learning_id, source)
                    .await
            }
            None => no_store("record_retrieval"),
        }
    }

    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>> {
        match self.shared_learning_reader() {
            Some(reader) => reader.list_retrievals_for_task(task_id).await,
            None => no_store("list_retrievals_for_task"),
        }
    }

    async fn apply_verdicts_tx(&self, verdicts: &[(LearningId, LearningVerdict)]) -> Result<()> {
        match self.shared_writer() {
            Some(writer) => writer.apply_learning_verdicts(verdicts).await,
            None => no_store("apply_verdicts_tx"),
        }
    }
}
