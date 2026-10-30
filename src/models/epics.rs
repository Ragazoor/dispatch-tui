use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::{ColumnSection, Task, TaskId, TaskStatus};
use crate::define_id_newtype;

define_id_newtype!(EpicId, epic_id_tests);

// ---------------------------------------------------------------------------
// Epic
// ---------------------------------------------------------------------------

// `PartialEq` so a decoded row can be compared against an expected one field
// for field. See `sync::tests::decode`.
#[derive(Debug, Clone, PartialEq)]
pub struct Epic {
    pub id: EpicId,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    /// When this epic last entered Done; `None` until it first does.
    ///
    /// The twin of [`crate::models::Task::completed_at`], on the same terms.
    /// Unlike the task field it is also written *without* a status transition:
    /// a manual reorder of this epic's card in the Done column persists an
    /// override here, which `EpicPlacement::sort_key` prefers over the key
    /// derived from the epic's done subtasks. See "Done Column Ordering" in
    /// `docs/specs/board-layout.allium`.
    pub completed_at: Option<DateTime<Utc>>,
    pub auto_dispatch: bool,
    pub parent_epic_id: Option<EpicId>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
    pub group_by_repo: bool,
    /// When true this epic's feed only ever ADDS: the stale-removal pass never
    /// runs for it, whatever the emission omits. For a source that emits events
    /// rather than mirroring state — a log scan — absence is not a retraction.
    /// See `docs/specs/feeds.allium`: `AppendOnlyFeed`.
    pub feed_append_only: bool,
    pub feed_role: FeedRole,
    pub origin: EpicOrigin,
    /// The workspace this epic lives in, as the store's workspace id. `0`
    /// for a row from before workspaces existed.
    pub workspace_id: i64,
    pub visibility: crate::models::Visibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Epic {
    /// The card's generic ordering key — the twin of [`crate::models::Task::sort_key`].
    pub fn sort_key(&self) -> i64 {
        self.sort_order.unwrap_or(self.id.0)
    }

    /// True when this epic participates in MANUAL repo-grouping: it has
    /// group_by_repo enabled and is not a feed epic (feed epics group via
    /// ingestion, not the manual router).
    pub fn can_auto_group(&self) -> bool {
        self.group_by_repo && self.feed_command.is_none()
    }
}

// ---------------------------------------------------------------------------
// FeedRole — routing role of an epic within a PR-review feed hierarchy
// ---------------------------------------------------------------------------

/// The routing role an epic plays within a feed hierarchy. `None` is the
/// default for ordinary epics; the other variants tag a feed sub-epic so PR
/// items can be routed to the right bucket (reviews requested of me, my team,
/// bots, CVEs, etc.). Stored in the store's `epics.feed_role` text column as the
/// kebab-case string (mirrors `TaskTag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeedRole {
    #[default]
    None,
    ReviewsParent,
    MyReviews,
    TeamReviews,
    Bots,
    Cve,
}

impl FeedRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedRole::None => "none",
            FeedRole::ReviewsParent => "reviews-parent",
            FeedRole::MyReviews => "my-reviews",
            FeedRole::TeamReviews => "team-reviews",
            FeedRole::Bots => "bots",
            FeedRole::Cve => "cve",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(FeedRole::None),
            "reviews-parent" => Some(FeedRole::ReviewsParent),
            "my-reviews" => Some(FeedRole::MyReviews),
            "team-reviews" => Some(FeedRole::TeamReviews),
            "bots" => Some(FeedRole::Bots),
            "cve" => Some(FeedRole::Cve),
            _ => None,
        }
    }
}

impl std::fmt::Display for FeedRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for FeedRole {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FeedRole::parse(s).ok_or(())
    }
}

// ---------------------------------------------------------------------------
// EpicOrigin — how an epic came to exist
// ---------------------------------------------------------------------------

/// How an epic was created. `Manual` is any user-created epic (TUI / MCP).
/// `RepoGroup` is an auto-created per-repo sub-epic produced by repo grouping.
/// Stored in the store's `epics.origin` text column as the kebab-case string.
/// This is the safety boundary for grouping: routing, flatten, and reroute only
/// ever touch `RepoGroup` sub-epics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EpicOrigin {
    #[default]
    Manual,
    RepoGroup,
}

impl EpicOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            EpicOrigin::Manual => "manual",
            EpicOrigin::RepoGroup => "repo-group",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(EpicOrigin::Manual),
            "repo-group" => Some(EpicOrigin::RepoGroup),
            _ => None,
        }
    }
}

impl std::fmt::Display for EpicOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for EpicOrigin {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EpicOrigin::parse(s).ok_or(())
    }
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
            Self::Done => "done".into(),
        }
    }

    /// The section an epic card renders under, so an epic and a task in the
    /// same state group together rather than the epic floating above every
    /// header. `None` in the two columns that have no sections.
    pub const fn column_section(&self) -> Option<ColumnSection> {
        match self {
            Self::Blocked(_) => Some(ColumnSection::NeedsInput),
            Self::Active => Some(ColumnSection::Active),
            Self::InReview => Some(ColumnSection::AwaitingReview),
            Self::Unplanned | Self::Planned | Self::Done => None,
        }
    }

    /// Header label for section grouping in the UI, from the same table.
    pub const fn header_label(&self) -> &'static str {
        match self.column_section() {
            Some(section) => section.header_label(),
            None => "",
        }
    }
}

/// The substatus table, keyed on the three things it actually depends on.
///
/// The one owner of the mapping. Two questions reach it from different angles —
/// "what state is this epic in?" ([`epic_substatus`], keyed on the epic's own
/// status over its whole subtree) and "what state is this epic's card in *this
/// column*?" (`EpicPlacement::substatus_in`, keyed on the column over that
/// column's slice) — and the answer must be the same table either way. Adding a
/// variant or changing the Planned/Unplanned rule is then one edit, not two that
/// nothing ties together.
pub fn epic_substatus_for(
    status: TaskStatus,
    planned: bool,
    blocked_running: usize,
) -> EpicSubstatus {
    match status {
        TaskStatus::Done => EpicSubstatus::Done,
        TaskStatus::Review => EpicSubstatus::InReview,
        TaskStatus::Running if blocked_running > 0 => EpicSubstatus::Blocked(blocked_running),
        TaskStatus::Running => EpicSubstatus::Active,
        TaskStatus::Backlog if planned => EpicSubstatus::Planned,
        TaskStatus::Backlog => EpicSubstatus::Unplanned,
    }
}

/// Derive epic substatus from current state: the epic's own status, read over
/// its whole subtree. See [`epic_substatus_for`] for the table itself.
pub fn epic_substatus(epic: &Epic, subtasks: &[&Task]) -> EpicSubstatus {
    let blocked = subtasks
        .iter()
        .filter(|t| t.status == TaskStatus::Running && t.sub_status.is_blocked())
        .count();
    epic_substatus_for(epic.status, epic.plan_path.is_some(), blocked)
}

/// Build an id→epic lookup over `epics`.
///
/// Calling this once and indexing it in a loop reduces a repeated linear
/// `iter().find(|e| e.id == id)` from O(epics²) to O(epics).
pub fn epic_id_lookup(epics: &[Epic]) -> std::collections::HashMap<EpicId, &Epic> {
    epics.iter().map(|e| (e.id, e)).collect()
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

/// Titles of `epic`'s ancestor chain, root first, ending with `epic` itself.
///
/// Walks `parent_epic_id` upward through `epics`. A visited-set guards against
/// malformed cycles (which the service layer prevents, but the renderer must
/// not hang on). A root epic returns `vec![epic.title]`.
pub fn ancestor_titles<'a>(epic: &'a Epic, epics: &'a [Epic]) -> Vec<&'a str> {
    let by_id = epic_id_lookup(epics);
    let mut seen: HashSet<EpicId> = HashSet::new();
    let mut chain: Vec<&str> = Vec::new();
    let mut cursor = epic;
    loop {
        if !seen.insert(cursor.id) {
            break; // cycle guard
        }
        chain.push(cursor.title.as_str());
        match cursor.parent_epic_id.and_then(|pid| by_id.get(&pid)) {
            Some(parent) => cursor = parent,
            None => break,
        }
    }
    chain.reverse();
    chain
}

/// The chain of epic ids from the root epic down to `target`, both included.
/// Cycle-safe like [`ancestor_titles`].
pub fn epic_path(target: EpicId, epics: &[Epic]) -> Vec<EpicId> {
    let by_id = epic_id_lookup(epics);
    let mut seen: HashSet<EpicId> = HashSet::new();
    let mut chain = Vec::new();
    let mut cursor = Some(target);
    while let Some(id) = cursor {
        if !seen.insert(id) {
            break;
        }
        chain.push(id);
        cursor = by_id.get(&id).and_then(|e| e.parent_epic_id);
    }
    chain.reverse();
    chain
}

/// Where Enter on `root`'s card lands in the column `status`
/// (`core/Epic.deepest_epic_with`): stay on an epic with a direct task in
/// `status`, step into the one sub-epic whose subtree holds such a task, and
/// stop at an epic where none or several sub-epics do.
pub fn deepest_epic_with(
    root: EpicId,
    status: TaskStatus,
    epics: &[Epic],
    tasks: &[Task],
) -> EpicId {
    let children = build_children_map(epics);
    let holders: HashSet<EpicId> = tasks
        .iter()
        .filter(|t| t.status == status)
        .filter_map(|t| t.epic_id)
        .collect();
    let mut current = root;
    // Each step moves strictly down the tree, so the bound only guards cycles.
    for _ in 0..=epics.len() {
        if holders.contains(&current) {
            return current;
        }
        let mut carriers = children
            .get(&current)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&kid| {
                descendant_epic_ids_with_map(kid, &children)
                    .iter()
                    .any(|e| holders.contains(e))
            });
        match (carriers.next(), carriers.next()) {
            (Some(only), None) => current = only,
            _ => return current,
        }
    }
    current
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
mod tests;
