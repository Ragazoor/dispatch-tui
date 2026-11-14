//! A read-only record of a task a person deleted (`core.allium: ArchivedTask`).
//!
//! Read from the store on demand, never held by the board: no board
//! subscribes to the archive.

use chrono::{DateTime, Utc};

use super::{EpicId, TaskId, TaskTag, TaskUrl};

/// What an agent can still learn about a deleted task: what it was, where it
/// sat and who worked on it. Live-session facts are not kept.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedTask {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub base_branch: String,
    pub tag: Option<TaskTag>,
    pub url: Option<TaskUrl>,
    pub plan_path: Option<String>,
    /// The epic it sat in; that epic may since be gone.
    pub epic_id: Option<EpicId>,
    /// That epic's title at archive time.
    pub epic_title: Option<String>,
    /// The task's display agent at archive time.
    pub agent_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub archived_at: DateTime<Utc>,
}
