use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::{ProjectId, SubStatus, Task, TaskId, TaskStatus};

define_id_newtype!(EpicId, epic_id_tests);

// ---------------------------------------------------------------------------
// Epic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Epic {
    pub id: EpicId,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: TaskStatus,
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    pub auto_dispatch: bool,
    pub parent_epic_id: Option<EpicId>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
    pub group_by_repo: bool,
    pub project_id: ProjectId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// EpicSubstatus — derived display state for epics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpicSubstatus {
    // Backlog
    Unplanned,
    Planned,
    // Running
    Active,
    Blocked(usize),
    // Review
    InReview,
    WrappingUp,
    // Done
    Done,
}

impl EpicSubstatus {
    pub fn label(&self) -> String {
        match self {
            Self::Unplanned => "unplanned".into(),
            Self::Planned => "planned".into(),
            Self::Active => "active".into(),
            Self::Blocked(n) => format!("{n} blocked"),
            Self::InReview => "in review".into(),
            Self::WrappingUp => "wrapping up".into(),
            Self::Done => "done".into(),
        }
    }

    /// Priority for sorting within a column, unified with SubStatus priorities
    /// so that epics and tasks share the same section headers.
    pub fn column_priority(&self) -> u8 {
        match self {
            Self::Blocked(_) => SubStatus::NeedsInput.column_priority(),
            Self::Active => SubStatus::Active.column_priority(),
            Self::WrappingUp => SubStatus::Approved.column_priority(),
            Self::InReview => SubStatus::AwaitingReview.column_priority(),
            Self::Unplanned | Self::Planned | Self::Done => SubStatus::None.column_priority(),
        }
    }

    /// Header label for section grouping in the UI, unified with SubStatus header labels.
    pub fn header_label(&self) -> &'static str {
        match self {
            Self::Blocked(_) => "needs input",
            Self::Active => "active",
            Self::InReview => "awaiting review",
            Self::WrappingUp => "approved",
            Self::Unplanned | Self::Planned | Self::Done => "",
        }
    }
}

/// Derive epic substatus from current state. `active_merge_epic` is the epic_id
/// of the currently active merge queue, if any.
pub fn epic_substatus(
    epic: &Epic,
    subtasks: &[Task],
    active_merge_epic: Option<EpicId>,
) -> EpicSubstatus {
    match epic.status {
        TaskStatus::Done | TaskStatus::Archived => EpicSubstatus::Done,
        TaskStatus::Review => {
            if active_merge_epic == Some(epic.id) {
                EpicSubstatus::WrappingUp
            } else {
                EpicSubstatus::InReview
            }
        }
        TaskStatus::Running => {
            let blocked_count = subtasks
                .iter()
                .filter(|t| {
                    t.status == TaskStatus::Running
                        && matches!(
                            t.sub_status,
                            SubStatus::NeedsInput
                                | SubStatus::Stale
                                | SubStatus::Crashed
                                | SubStatus::Conflict
                        )
                })
                .count();
            if blocked_count > 0 {
                EpicSubstatus::Blocked(blocked_count)
            } else {
                EpicSubstatus::Active
            }
        }
        TaskStatus::Backlog => {
            if epic.plan_path.is_some() {
                EpicSubstatus::Planned
            } else {
                EpicSubstatus::Unplanned
            }
        }
    }
}

/// Collect all descendant epic IDs of `root`, inclusive of `root` itself.
///
/// Walks `parent_epic_id` links iteratively and is cycle-safe: if two epics
/// form a malformed cycle, each is visited at most once.
pub fn descendant_epic_ids(root: EpicId, epics: &[Epic]) -> HashSet<EpicId> {
    let mut out = HashSet::new();
    out.insert(root);
    let mut changed = true;
    while changed {
        changed = false;
        for epic in epics {
            if let Some(parent) = epic.parent_epic_id {
                if out.contains(&parent) && !out.contains(&epic.id) {
                    out.insert(epic.id);
                    changed = true;
                }
            }
        }
    }
    out
}

/// Collect all tasks whose `epic_id` is in the subtree rooted at `root`.
///
/// Returns every task directly under `root` or under any of its descendant
/// sub-epics, recursively. Cycle-safe: malformed epic parent cycles terminate.
pub fn descendant_task_ids(root: EpicId, epics: &[Epic], tasks: &[Task]) -> HashSet<TaskId> {
    let epic_ids = descendant_epic_ids(root, epics);
    tasks
        .iter()
        .filter(|t| matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid)))
        .map(|t| t.id)
        .collect()
}
