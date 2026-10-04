//! A SQLite reading of the board, for the tests whose subject is a handle with
//! no store attached (`Database::open_in_memory_unattached`).
//!
//! Not a production seam and not a second `BoardReads` to keep in step: an
//! attached handle serves its own (`Database::board_reads`,
//! `spacetime-memory-store.allium: TestBoardReadsShareTheHandlesRows`). This
//! exists because an unattached handle has none, and because the sync tests
//! compare the subscription against what SQLite holds.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::db::{Database, TaskReadStore};
use crate::models::{Epic, EpicId, PollScopeId, Task, TaskId};
use crate::sync::BoardReads;

pub(crate) struct SqliteBoardReads {
    db: Arc<dyn TaskReadStore>,
}

impl SqliteBoardReads {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

/// The handle's own board reads when it has them, else a SQLite reading.
pub(crate) fn board_reads_of(db: &Arc<Database>) -> Arc<dyn BoardReads> {
    db.board_reads()
        .unwrap_or_else(|| Arc::new(SqliteBoardReads::new(db.clone())))
}

#[async_trait]
impl BoardReads for SqliteBoardReads {
    async fn list_tasks(&self) -> Result<Vec<Task>> {
        self.db.list_all().await
    }
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        self.db.get_task(id).await
    }
    async fn list_tasks_for_epic(&self, epic: EpicId) -> Result<Vec<Task>> {
        self.db.list_tasks_for_epic(epic).await
    }
    async fn list_epics(&self) -> Result<Vec<Epic>> {
        self.db.list_epics().await
    }
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>> {
        self.db.get_epic(id).await
    }
    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        self.db.list_repo_paths().await
    }
    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>> {
        self.db.list_all_base_branches().await
    }
    /// SQLite has no `PollOwner`: every scope reads as unclaimed.
    async fn poll_owner(&self, _target: PollScopeId) -> Result<Option<String>> {
        Ok(None)
    }
    async fn revision(&self) -> Option<u64> {
        self.db.get_total_changes().await.ok().map(|n| n as u64)
    }
}
