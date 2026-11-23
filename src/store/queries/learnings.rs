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
    /// Unlike settings, nothing here is scoped by host: every board reads
    /// every learning (`learnings.allium`, Scope Model).
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

    async fn list_all_approved_learnings_with_embedding(&self) -> Result<Vec<(Learning, Vec<u8>)>> {
        Ok(self.rows.approved_learnings_with_embedding())
    }

    /// `learnings.allium: RecordLearningViaMcp`, the repair pass.
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

    async fn learnings_off_topic_in_repo(
        &self,
        repo_path: &str,
        threshold: usize,
    ) -> Result<std::collections::BTreeSet<LearningId>> {
        Ok(self.rows.learnings_off_topic_in_repo(repo_path, threshold))
    }

    async fn apply_verdicts_tx(
        &self,
        task_id: TaskId,
        verdicts: &[(LearningId, LearningVerdict)],
    ) -> Result<()> {
        self.caller
            .apply_learning_verdicts(task_id, verdicts.to_vec())
            .await?
            .applied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacetime::bindings;
    use crate::store::{LearningStore, Store, MEMORY_USER_ID};

    fn learning(id: i64, workspace_id: i64, author: &str) -> bindings::Learning {
        bindings::Learning {
            id,
            kind: "pitfall".into(),
            summary: format!("learning {id}"),
            detail: None,
            scope: "repo".into(),
            scope_ref: Some("/repo".into()),
            tags: "[]".into(),
            status: "approved".into(),
            source_task_id: None,
            upvote_count: 0,
            last_upvoted_at: None,
            created_at: "2026-01-01 00:00:00.000".into(),
            updated_at: "2026-01-01 00:00:00.000".into(),
            embedding: None,
            workspace_id,
            author: author.into(),
        }
    }

    /// Every reader sees every learning (`learnings.allium`, Scope Model):
    /// whatever its author or the dead `workspace_id` column holds.
    #[tokio::test]
    async fn every_read_shows_every_learning() {
        let store = Store::open_in_memory().unwrap();
        for row in [
            learning(1, 5, "someone"),
            learning(2, 0, MEMORY_USER_ID),
            learning(3, 0, "someone"),
            learning(4, 0, ""),
        ] {
            store.rows.upsert_learning(&row);
        }

        let mut listed: Vec<i64> = store
            .list_learnings(LearningFilter::default())
            .await
            .unwrap()
            .into_iter()
            .map(|l| l.id.0)
            .collect();
        listed.sort();
        assert_eq!(listed, vec![1, 2, 3, 4]);
        assert!(store.get_learning(LearningId(3)).await.unwrap().is_some());
        let mut missing: Vec<i64> = store
            .list_learnings_missing_embedding()
            .await
            .unwrap()
            .into_iter()
            .map(|l| l.id.0)
            .collect();
        missing.sort();
        assert_eq!(missing, vec![1, 2, 3, 4]);
    }
}
