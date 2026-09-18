//! Where the board gets the rows it draws.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardReadsFromTheSubscription`.
//!
//! # Why a seam rather than a swapped store
//!
//! `TaskReadStore` — the handle the runtime holds today — spans both halves of
//! the store seam: task and epic reads on the shared side, settings, learnings
//! and usage on the local one. The board's DRAWING needs only the shared half,
//! and that is the only half a subscription can serve. So this trait names
//! exactly the reads the board performs to put cards on screen, and nothing
//! else.
//!
//! The narrowness is the point. A trait wide enough to also cover the knowledge
//! base would need a subscription implementation that answered questions the
//! shared store has no rows for, and the honest answer to those is "ask the
//! local store" — which is what the runtime still does, through the handle it
//! already has.
//!
//! # Two implementations, and which one runs
//!
//! [`LocalBoardReads`] reads SQLite, and is what every board runs today.
//! [`SubscriptionBoardReads`] reads the live view of this board's
//! subscriptions. The runtime picks between them by whether a shared store is
//! configured, so an install with none is not a degraded board — it is the
//! single-machine dispatch that has always existed, and `sync.allium`'s header
//! says so.
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

use crate::models::{Epic, EpicId, Task, TaskId, Todo};

use super::SharedRows;

/// The reads a board performs to draw itself.
#[async_trait]
pub trait BoardReads: Send + Sync {
    async fn list_tasks(&self) -> Result<Vec<Task>>;
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>>;
    async fn list_tasks_for_epic(&self, epic: EpicId) -> Result<Vec<Task>>;
    async fn list_epics(&self) -> Result<Vec<Epic>>;
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>>;
    async fn list_todos(&self) -> Result<Vec<Todo>>;
    async fn list_repo_paths(&self) -> Result<Vec<String>>;
    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>>;

    /// A number that changes when the rows do.
    ///
    /// `-1` means "cannot tell" — take it as changed. That is the answer a
    /// failed read gives, and erring towards one wasted refresh is the right
    /// side to err on: the other side is a board that stops updating and says
    /// nothing.
    async fn revision(&self) -> i64;
}

/// Reads from this machine's SQLite database. What every board runs today.
pub struct LocalBoardReads {
    db: Arc<dyn crate::db::TaskReadStore>,
    /// A second handle to the same database, for the one table `TaskReadStore`
    /// does not reach.
    ///
    /// `TodoStore` carries its writes on the same trait as its read, and the
    /// runtime must not hold those — so it cannot simply be folded into
    /// `TaskReadStore`. Two handles to one object is the narrower of the two
    /// wrong-looking options, and this one keeps the mutation boundary
    /// (`docs/conventions.md`) intact. Only `list_todos` is ever called on it.
    todos: Arc<dyn crate::db::TodoStore>,
}

impl LocalBoardReads {
    pub fn new(
        db: Arc<dyn crate::db::TaskReadStore>,
        todos: Arc<dyn crate::db::TodoStore>,
    ) -> Self {
        Self { db, todos }
    }
}

#[async_trait]
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

    async fn list_todos(&self) -> Result<Vec<Todo>> {
        self.todos.list_todos().await
    }

    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        self.db.list_repo_paths().await
    }

    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>> {
        self.db.list_all_base_branches().await
    }

    async fn revision(&self) -> i64 {
        self.db.get_total_changes().await.unwrap_or(-1)
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

    async fn list_todos(&self) -> Result<Vec<Todo>> {
        Ok(self.rows.todos())
    }

    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        Ok(self.rows.repo_paths())
    }

    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>> {
        Ok(self.rows.base_branches())
    }

    async fn revision(&self) -> i64 {
        // Saturating rather than wrapping: a board that ran long enough to
        // overflow an i64 of row changes would, on wrap, report a revision it
        // had already reported and skip a refresh. Pinning at the ceiling makes
        // every later read look changed instead, which is the harmless side.
        i64::try_from(self.rows.generation()).unwrap_or(i64::MAX)
    }
}
