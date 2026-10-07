//! Where a learning READ goes — Phase 10 of the SpacetimeDB migration.
//!
//! Spec: `docs/specs/learnings.allium`'s Storage Backend section.
//!
//! The read twin of `shared_writer.rs`: with no reader attached, a read
//! answers from SQLite exactly as it always has (the unchanged, unconfigured
//! board). With one attached, the reader answers INSTEAD of SQLite — proven
//! here by making the two disagree and asserting the reader's answer wins,
//! the same way `shared_writer.rs` proves a routed write bypasses SQLite by
//! asserting the local table stays empty.

use super::*;
use crate::models::{Learning, LearningId, LearningKind, LearningRetrieval, LearningScope, TaskId};
use crate::store::{CreateLearningRow, LearningFilter, SharedLearningReader};
use std::sync::Mutex;

/// A reader that answers from a fixed, caller-supplied set of rows and
/// remembers which methods were called.
#[derive(Default)]
struct RecordingReader {
    calls: Mutex<Vec<&'static str>>,
    learnings: Vec<Learning>,
    retrievals: Vec<LearningRetrieval>,
}

impl RecordingReader {
    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, what: &'static str) {
        self.calls.lock().unwrap().push(what);
    }
}

#[async_trait::async_trait]
impl SharedLearningReader for RecordingReader {
    async fn get_learning(&self, id: LearningId) -> anyhow::Result<Option<Learning>> {
        self.record("get_learning");
        Ok(self.learnings.iter().find(|l| l.id == id).cloned())
    }

    async fn list_learnings(&self, _filter: LearningFilter) -> anyhow::Result<Vec<Learning>> {
        self.record("list_learnings");
        Ok(self.learnings.clone())
    }

    async fn list_all_approved_non_task_learnings(
        &self,
    ) -> anyhow::Result<Vec<(Learning, Vec<u8>)>> {
        self.record("list_all_approved_non_task_learnings");
        Ok(self
            .learnings
            .iter()
            .cloned()
            .map(|l| (l, vec![1, 2, 3]))
            .collect())
    }

    async fn list_learnings_missing_embedding(&self) -> anyhow::Result<Vec<Learning>> {
        self.record("list_learnings_missing_embedding");
        Ok(self.learnings.clone())
    }

    async fn list_retrievals_for_task(
        &self,
        _task_id: TaskId,
    ) -> anyhow::Result<Vec<LearningRetrieval>> {
        self.record("list_retrievals_for_task");
        Ok(self.retrievals.clone())
    }
}

fn a_learning(id: i64, summary: &str) -> Learning {
    Learning {
        id: LearningId(id),
        kind: LearningKind::Convention,
        summary: summary.to_string(),
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: vec![],
        status: crate::models::LearningStatus::Approved,
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// The unchanged board: no reader, so a read answers from SQLite.
#[tokio::test]
async fn with_no_reader_a_learning_read_goes_to_sqlite() {
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "local",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    let found = db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(found.summary, "local");
}

/// The configured board: the reader answers, not SQLite. Proven by making
/// them disagree — a row created locally (no writer attached, so it landed
/// in SQLite) is invisible, and the reader's own row is what comes back.
#[tokio::test]
async fn with_a_reader_attached_a_learning_read_routes_to_it() {
    let db = in_memory_db().await;
    let local_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "local only",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    let reader = Arc::new(RecordingReader {
        learnings: vec![a_learning(99, "from the store")],
        ..Default::default()
    });
    let db = db.with_shared_learning_reader(reader.clone());

    // The locally-created row is invisible: the reader answers instead.
    assert!(db.get_learning(local_id).await.unwrap().is_none());

    let found = db.get_learning(LearningId(99)).await.unwrap().unwrap();
    assert_eq!(found.summary, "from the store");

    let all = db.list_learnings(LearningFilter::default()).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, LearningId(99));

    let (candidates, _embeddings) = db
        .list_all_approved_non_task_learnings()
        .await
        .unwrap()
        .into_iter()
        .next()
        .map(|(l, e)| (vec![l], e))
        .unwrap();
    assert_eq!(candidates[0].id, LearningId(99));

    let missing = db.list_learnings_missing_embedding().await.unwrap();
    assert_eq!(missing[0].id, LearningId(99));

    let retrievals = db.list_retrievals_for_task(TaskId(1)).await.unwrap();
    assert!(retrievals.is_empty());

    assert_eq!(
        reader.calls(),
        vec![
            "get_learning",
            "get_learning",
            "list_learnings",
            "list_all_approved_non_task_learnings",
            "list_learnings_missing_embedding",
            "list_retrievals_for_task",
        ]
    );
}
