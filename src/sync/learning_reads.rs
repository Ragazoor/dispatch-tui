//! Where a shared-table learning READ goes, when it does not go to SQLite.
//!
//! Spec: `docs/specs/learnings.allium`'s Storage Backend section.
//!
//! # The mirror of `SharedWriter`, not of `BoardReads`
//!
//! `db::SharedWriter` is the port a routed WRITE goes through when a store is
//! attached, defined in `db` and implemented here in `sync`; `Database`
//! itself is still the local SQL path, reached when no writer is attached.
//! [`db::SharedLearningReader`] is the same shape for reads: no "local"
//! implementation exists here, because `Database`'s own existing SQL methods
//! already ARE the local implementation — the same way there is no "local
//! writer" type beside `SharedWriter`.
//!
//! This is deliberately NOT `BoardReads`. That seam names exactly the reads a
//! board performs to put cards on screen (`board_reads.rs`'s own header), and
//! a learning is never drawn on the board — every read here is reached from
//! an MCP tool handler or the dispatch-prompt builder. Folding the two
//! together would blur a boundary that exists on purpose.
//!
//! # Why this exists at all, unlike Phase 9's settings
//!
//! Phase 9 (`docs/specs/settings.allium`) routed `Setting`
//! writes to the shared store but left their reads on local SQLite —
//! correct there, because a setting is scoped to `host` and this machine
//! never needs to see another one's. Learnings are the opposite: the whole
//! point of this phase is that a learning recorded on one host's board is
//! visible from every other host's. A read that stayed local would silently
//! show an empty or stale knowledge base the moment a write routed to the
//! store instead — exactly the gap `rate_learning`'s retrieval-precondition
//! check would hit first, since the retrieval it looks for was written to
//! the store and never touched local SQLite.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::db::{LearningFilter, SharedLearningReader};
use crate::models::{Learning, LearningId, LearningRetrieval, TaskId};

use super::SharedRows;

/// Reads from the live view of this board's subscriptions.
///
/// Every method is infallible in practice — the rows are already decoded and
/// in memory — but keeps the `Result` its twin has, so callers have one code
/// path rather than two. A board that is not connected holds no rows and
/// answers empty, the same bargain `SubscriptionBoardReads` makes.
pub struct SubscriptionLearningReads {
    rows: Arc<SharedRows>,
}

impl SubscriptionLearningReads {
    pub fn new(rows: Arc<SharedRows>) -> Self {
        Self { rows }
    }
}

#[async_trait]
impl SharedLearningReader for SubscriptionLearningReads {
    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>> {
        Ok(self.rows.learning(id))
    }

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>> {
        Ok(self.rows.learnings_matching(&filter))
    }

    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>> {
        Ok(self.rows.approved_non_task_learnings_with_embedding())
    }

    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>> {
        Ok(self.rows.learnings_missing_embedding())
    }

    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>> {
        Ok(self.rows.retrievals_for_task(task_id))
    }
}
