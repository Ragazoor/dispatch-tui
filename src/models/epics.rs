use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::{SubStatus, Task, TaskId, TaskStatus};

define_id_newtype!(EpicId, epic_id_tests);

// ---------------------------------------------------------------------------
// Epic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Epic {
    pub id: EpicId,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    pub auto_dispatch: bool,
    pub parent_epic_id: Option<EpicId>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
    pub group_by_repo: bool,
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

/// Build a parent→children adjacency map over `epics`.
///
/// Calling this once and passing the result to [`descendant_epic_ids_with_map`]
/// for each epic in a loop reduces the cost from O(epics²) to O(epics).
pub fn build_children_map(epics: &[Epic]) -> std::collections::HashMap<EpicId, Vec<EpicId>> {
    let mut children: std::collections::HashMap<EpicId, Vec<EpicId>> =
        std::collections::HashMap::new();
    for epic in epics {
        if let Some(parent) = epic.parent_epic_id {
            children.entry(parent).or_default().push(epic.id);
        }
    }
    children
}

/// Collect all descendant epic IDs of `root` using a prebuilt children map.
///
/// Equivalent to [`descendant_epic_ids`] but skips rebuilding the map.
/// Use when calling for multiple roots over the same epic list.
pub fn descendant_epic_ids_with_map(
    root: EpicId,
    children: &std::collections::HashMap<EpicId, Vec<EpicId>>,
) -> HashSet<EpicId> {
    let mut out = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if out.insert(id) {
            if let Some(kids) = children.get(&id) {
                stack.extend_from_slice(kids);
            }
        }
    }
    out
}

/// Collect all descendant epic IDs of `root`, inclusive of `root` itself.
///
/// Uses a DFS stack over a children map for O(N) traversal. Cycle-safe: each
/// node is visited at most once via the `out` visited set.
///
/// When computing descendants for multiple roots over the same epic list,
/// prefer [`build_children_map`] + [`descendant_epic_ids_with_map`] to avoid
/// rebuilding the adjacency map on every call.
pub fn descendant_epic_ids(root: EpicId, epics: &[Epic]) -> HashSet<EpicId> {
    let children = build_children_map(epics);
    descendant_epic_ids_with_map(root, &children)
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::models::{SubStatus, Task, TaskId, TaskStatus};
    use chrono::Utc;

    fn make_epic(
        id: i64,
        status: TaskStatus,
        plan_path: Option<&str>,
        parent: Option<i64>,
    ) -> Epic {
        Epic {
            id: EpicId(id),
            title: format!("Epic {id}"),
            description: String::new(),
            status,
            plan_path: plan_path.map(String::from),
            sort_order: None,
            auto_dispatch: false,
            parent_epic_id: parent.map(EpicId),
            feed_command: None,
            feed_interval_secs: None,
            group_by_repo: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_task(id: i64, status: TaskStatus, sub_status: SubStatus, epic: Option<i64>) -> Task {
        Task {
            id: TaskId(id),
            title: format!("Task {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status,
            sub_status,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: epic.map(EpicId),
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_pre_tool_use_at: None,
            last_notification_at: None,
            wrap_up_mode: None,
        }
    }

    #[test]
    fn epic_substatus_label_all_variants() {
        assert_eq!(EpicSubstatus::Unplanned.label(), "unplanned");
        assert_eq!(EpicSubstatus::Planned.label(), "planned");
        assert_eq!(EpicSubstatus::Active.label(), "active");
        assert_eq!(EpicSubstatus::Blocked(1).label(), "1 blocked");
        assert_eq!(EpicSubstatus::Blocked(5).label(), "5 blocked");
        assert_eq!(EpicSubstatus::InReview.label(), "in review");
        assert_eq!(EpicSubstatus::WrappingUp.label(), "wrapping up");
        assert_eq!(EpicSubstatus::Done.label(), "done");
    }

    #[test]
    fn epic_substatus_header_label_active_states() {
        assert_eq!(EpicSubstatus::Blocked(2).header_label(), "needs input");
        assert_eq!(EpicSubstatus::Active.header_label(), "active");
        assert_eq!(EpicSubstatus::InReview.header_label(), "awaiting review");
        assert_eq!(EpicSubstatus::WrappingUp.header_label(), "approved");
    }

    #[test]
    fn epic_substatus_header_label_terminal_states_are_empty() {
        assert_eq!(EpicSubstatus::Unplanned.header_label(), "");
        assert_eq!(EpicSubstatus::Planned.header_label(), "");
        assert_eq!(EpicSubstatus::Done.header_label(), "");
    }

    #[test]
    fn epic_substatus_column_priority_aligns_with_substatus() {
        assert_eq!(
            EpicSubstatus::Blocked(1).column_priority(),
            SubStatus::NeedsInput.column_priority()
        );
        assert_eq!(
            EpicSubstatus::Active.column_priority(),
            SubStatus::Active.column_priority()
        );
        assert_eq!(
            EpicSubstatus::WrappingUp.column_priority(),
            SubStatus::Approved.column_priority()
        );
        assert_eq!(
            EpicSubstatus::InReview.column_priority(),
            SubStatus::AwaitingReview.column_priority()
        );
        assert_eq!(
            EpicSubstatus::Unplanned.column_priority(),
            SubStatus::None.column_priority()
        );
        assert_eq!(
            EpicSubstatus::Done.column_priority(),
            SubStatus::None.column_priority()
        );
    }

    #[test]
    fn epic_substatus_archived_yields_done() {
        let epic = make_epic(1, TaskStatus::Archived, None, None);
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Done);
    }

    #[test]
    fn epic_substatus_blocked_counts_conflict_and_crashed() {
        for sub in [SubStatus::Conflict, SubStatus::Crashed] {
            let epic = make_epic(1, TaskStatus::Running, None, None);
            let subtasks = vec![make_task(1, TaskStatus::Running, sub, None)];
            assert_eq!(
                epic_substatus(&epic, &subtasks, None),
                EpicSubstatus::Blocked(1),
                "{sub:?}"
            );
        }
    }

    #[test]
    fn epic_substatus_wrapping_up_requires_matching_epic_id() {
        // active_merge_epic is a DIFFERENT epic → InReview, not WrappingUp
        let epic = make_epic(1, TaskStatus::Review, None, None);
        assert_eq!(
            epic_substatus(&epic, &[], Some(EpicId(99))),
            EpicSubstatus::InReview
        );
    }

    #[test]
    fn descendant_epic_ids_includes_root_itself() {
        let epics = vec![make_epic(1, TaskStatus::Backlog, None, None)];
        let ids = descendant_epic_ids(EpicId(1), &epics);
        assert!(ids.contains(&EpicId(1)));
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn descendant_epic_ids_includes_direct_children() {
        let epics = vec![
            make_epic(1, TaskStatus::Backlog, None, None),
            make_epic(2, TaskStatus::Backlog, None, Some(1)),
            make_epic(3, TaskStatus::Backlog, None, Some(1)),
        ];
        let ids = descendant_epic_ids(EpicId(1), &epics);
        assert!(ids.contains(&EpicId(1)));
        assert!(ids.contains(&EpicId(2)));
        assert!(ids.contains(&EpicId(3)));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn descendant_epic_ids_is_recursive() {
        // root(1) → child(2) → grandchild(3)
        let epics = vec![
            make_epic(1, TaskStatus::Backlog, None, None),
            make_epic(2, TaskStatus::Backlog, None, Some(1)),
            make_epic(3, TaskStatus::Backlog, None, Some(2)),
        ];
        let ids = descendant_epic_ids(EpicId(1), &epics);
        assert!(ids.contains(&EpicId(1)));
        assert!(ids.contains(&EpicId(2)));
        assert!(ids.contains(&EpicId(3)));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn descendant_epic_ids_excludes_unrelated_subtree() {
        let epics = vec![
            make_epic(1, TaskStatus::Backlog, None, None),
            make_epic(2, TaskStatus::Backlog, None, None),
            make_epic(3, TaskStatus::Backlog, None, Some(2)),
        ];
        let ids = descendant_epic_ids(EpicId(1), &epics);
        assert!(ids.contains(&EpicId(1)));
        assert!(!ids.contains(&EpicId(2)));
        assert!(!ids.contains(&EpicId(3)));
    }

    #[test]
    fn descendant_epic_ids_is_cycle_safe() {
        // Malformed: epic 1 → parent 2, epic 2 → parent 1
        let epics = vec![
            make_epic(1, TaskStatus::Backlog, None, Some(2)),
            make_epic(2, TaskStatus::Backlog, None, Some(1)),
        ];
        let ids = descendant_epic_ids(EpicId(1), &epics);
        assert!(ids.contains(&EpicId(1)));
        assert!(ids.contains(&EpicId(2)));
    }

    // ---------------------------------------------------------------------------
    // build_children_map / descendant_epic_ids_with_map
    // ---------------------------------------------------------------------------

    #[test]
    fn descendant_epic_ids_with_map_matches_original_for_deep_tree() {
        // root(1) → child(2) → grandchild(3), unrelated(4) → child(5)
        let epics = vec![
            make_epic(1, TaskStatus::Backlog, None, None),
            make_epic(2, TaskStatus::Backlog, None, Some(1)),
            make_epic(3, TaskStatus::Backlog, None, Some(2)),
            make_epic(4, TaskStatus::Backlog, None, None),
            make_epic(5, TaskStatus::Backlog, None, Some(4)),
        ];
        let children = build_children_map(&epics);

        for root in [EpicId(1), EpicId(2), EpicId(3), EpicId(4), EpicId(5)] {
            let original = descendant_epic_ids(root, &epics);
            let with_map = descendant_epic_ids_with_map(root, &children);
            assert_eq!(
                original, with_map,
                "descendant_epic_ids_with_map must match original for root {root:?}"
            );
        }
    }

    #[test]
    fn build_children_map_returns_empty_for_flat_epics() {
        let epics = vec![
            make_epic(1, TaskStatus::Backlog, None, None),
            make_epic(2, TaskStatus::Backlog, None, None),
        ];
        let children = build_children_map(&epics);
        assert!(children.is_empty(), "no parent-child relationships → empty map");
    }
}
