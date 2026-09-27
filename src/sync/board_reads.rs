//! Where the board gets the rows it draws.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardReadsFromTheSubscription`.
//!
//! # Why a seam rather than a swapped store
//!
//! This trait names exactly the reads the board performs to put cards on
//! screen, and nothing else — the reads the row-change pump and the tick's
//! revision guard refresh. Every other read goes through the runtime's
//! `database`, which since task #4916 answers from the same rows through
//! `db::SharedReader`.
//!
//! # Two implementations, and which one runs
//!
//! [`SubscriptionBoardReads`] reads the live view of this board's
//! subscriptions, and is what every board runs: the store is mandatory.
//! [`LocalBoardReads`] reads SQLite and survives only as the test suite's
//! stand-in until Phase 12b (#4975).
//!
//! # The revision number
//!
//! [`BoardReads::revision`] is the cheap "has anything changed?" both backings
//! can answer: SQLite's cumulative change counter, or the subscription's
//! generation. The tick-driven refresh compares it before re-reading, which is
//! what keeps a speculative refresh free. Its VALUES are not comparable across
//! implementations and nothing persists one, so swapping the backing simply
//! costs one extra refresh.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::models::{Epic, EpicId, Task, TaskId};

use super::SharedRows;

/// The reads a board performs to draw itself.
#[async_trait]
pub trait BoardReads: Send + Sync {
    async fn list_tasks(&self) -> Result<Vec<Task>>;
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>>;
    async fn list_tasks_for_epic(&self, epic: EpicId) -> Result<Vec<Task>>;
    async fn list_epics(&self) -> Result<Vec<Epic>>;
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>>;
    async fn list_repo_paths(&self) -> Result<Vec<String>>;
    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>>;

    /// The `Host.id` allowed to run recurring background polling for
    /// `(scope, scope_id)`, or `None` if unclaimed (`core.allium: PollOwner`).
    /// `scope` is `"task"` or `"epic"` — see `pr-workflow.allium: PollPrStatus`
    /// and `feeds.allium: FeedTick`, the two callers.
    async fn poll_owner(&self, scope: &str, scope_id: i64) -> Result<Option<String>>;

    /// A number that changes when the rows do.
    ///
    /// `None` means "cannot tell" — take it as changed. That is the answer a
    /// failed read gives, and erring towards one wasted refresh is the right
    /// side to err on: the other side is a board that stops updating and says
    /// nothing.
    ///
    /// `Option<u64>` rather than a signed sentinel. The caller also has to
    /// represent "never read yet", and with one `-1` standing for both that
    /// value meant two different absences on the same line. It also spared the
    /// subscription backing a saturating conversion it only needed because the
    /// trait had chosen a signed type for an unsigned counter.
    async fn revision(&self) -> Option<u64>;
}

/// Reads from this machine's SQLite database. The test suite's stand-in for
/// [`SubscriptionBoardReads`]; no board runs it.
#[cfg(any(test, feature = "test-support"))]
pub struct LocalBoardReads {
    db: Arc<dyn crate::db::TaskReadStore>,
}

#[cfg(any(test, feature = "test-support"))]
impl LocalBoardReads {
    pub fn new(db: Arc<dyn crate::db::TaskReadStore>) -> Self {
        Self { db }
    }
}

#[async_trait]
#[cfg(any(test, feature = "test-support"))]
impl BoardReads for LocalBoardReads {
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

    /// `PollOwner` has no SQLite counterpart at all (`core.allium: PollOwner`):
    /// a single-machine install has no other host to contend a claim with,
    /// so every scope reads as unclaimed.
    async fn poll_owner(&self, _scope: &str, _scope_id: i64) -> Result<Option<String>> {
        Ok(None)
    }

    async fn revision(&self) -> Option<u64> {
        self.db.get_total_changes().await.ok().map(|n| n as u64)
    }
}

/// Reads from the live view of this board's subscriptions.
///
/// Every method is infallible in practice — the rows are already decoded and in
/// memory — but keeps the `Result` its twin has, so the runtime has one code
/// path rather than two. A board that is not connected holds no rows and
/// answers empty; it does not answer an error, because "disconnected" is the
/// connection's state to report (`sync.allium`'s `ConnectionIndicator`) and
/// reporting it again per read would put the same outage on screen eight times.
pub struct SubscriptionBoardReads {
    rows: Arc<SharedRows>,
}

impl SubscriptionBoardReads {
    pub fn new(rows: Arc<SharedRows>) -> Self {
        Self { rows }
    }
}

#[async_trait]
impl BoardReads for SubscriptionBoardReads {
    async fn list_tasks(&self) -> Result<Vec<Task>> {
        Ok(self.rows.tasks())
    }

    async fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        Ok(self.rows.task(id))
    }

    async fn list_tasks_for_epic(&self, epic: EpicId) -> Result<Vec<Task>> {
        Ok(self.rows.tasks_for_epic(epic))
    }

    async fn list_epics(&self) -> Result<Vec<Epic>> {
        Ok(self.rows.epics())
    }

    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>> {
        Ok(self.rows.epic(id))
    }

    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        Ok(self.rows.repo_paths())
    }

    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>> {
        Ok(self.rows.base_branches())
    }

    async fn poll_owner(&self, scope: &str, scope_id: i64) -> Result<Option<String>> {
        Ok(self.rows.poll_owner(scope, scope_id).map(|row| row.host))
    }

    async fn revision(&self) -> Option<u64> {
        Some(self.rows.generation())
    }
}

/// The same rows answer every other shared read `Database` routes
/// (`db::SharedReader`, `sync.allium`'s `BoardReadsFromTheSubscription`) — one
/// adapter over [`SharedRows`], not two kept in step.
#[async_trait]
impl crate::db::SharedReader for SubscriptionBoardReads {
    async fn list_all(&self) -> Result<Vec<Task>> {
        Ok(self.rows.tasks())
    }

    async fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        Ok(self.rows.task(id))
    }

    async fn task_exists(&self, id: TaskId) -> Result<bool> {
        Ok(self.rows.has_task(id))
    }

    async fn list_live_agent_tasks(&self) -> Result<Vec<Task>> {
        Ok(self.rows.live_agent_tasks())
    }

    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>> {
        Ok(self.rows.task_by_plan(plan))
    }

    async fn list_tasks_for_epic(&self, epic: EpicId) -> Result<Vec<Task>> {
        Ok(self.rows.tasks_for_epic(epic))
    }

    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<Task>> {
        Ok(self.rows.tasks_with_epic())
    }

    async fn list_watchers_of(&self, target: TaskId) -> Result<Vec<TaskId>> {
        Ok(self.rows.watchers_of(target))
    }

    async fn list_epics(&self) -> Result<Vec<Epic>> {
        Ok(self.rows.epics())
    }

    async fn list_epics_with_parent(&self, parent: Option<EpicId>) -> Result<Vec<Epic>> {
        Ok(self.rows.epics_with_parent(parent))
    }

    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>> {
        Ok(self.rows.epic(id))
    }

    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        Ok(self.rows.repo_paths())
    }

    async fn get_verify_command(&self, path: &str) -> Result<Option<String>> {
        Ok(self.rows.verify_command(path))
    }

    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>> {
        Ok(self.rows.base_branches())
    }

    async fn subscribed_epics(&self, subscriber: &str) -> Result<Vec<i64>> {
        Ok(self.rows.subscribed_epics(subscriber))
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.rows.setting(key))
    }
}
