use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Path utilities
// ---------------------------------------------------------------------------

/// Expand a leading `~` or `~/` to the user's home directory.
/// Returns the path unchanged if it doesn't start with `~` or `$HOME` is unset.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{rest}", home.to_string_lossy());
        }
    } else if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

// ---------------------------------------------------------------------------
// ProjectId / Project
// ---------------------------------------------------------------------------

pub type ProjectId = i64;

#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub sort_order: i64,
    pub is_default: bool,
}

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[serde(alias = "ready")]
    Backlog,
    Running,
    Review,
    Done,
    Archived,
}

impl TaskStatus {
    pub const ALL: &'static [TaskStatus] = &[
        TaskStatus::Backlog,
        TaskStatus::Running,
        TaskStatus::Review,
        TaskStatus::Done,
    ];

    pub const COLUMN_COUNT: usize = Self::ALL.len();

    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Running => "running",
            TaskStatus::Review => "review",
            TaskStatus::Done => "done",
            TaskStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "backlog" | "ready" => Some(TaskStatus::Backlog),
            "running" => Some(TaskStatus::Running),
            "review" => Some(TaskStatus::Review),
            "done" => Some(TaskStatus::Done),
            "archived" => Some(TaskStatus::Archived),
            _ => None,
        }
    }

    /// Advance to the next status (wraps at Done -> Done).
    pub fn next(self) -> Self {
        match self {
            TaskStatus::Backlog => TaskStatus::Running,
            TaskStatus::Running => TaskStatus::Review,
            TaskStatus::Review => TaskStatus::Done,
            TaskStatus::Done => TaskStatus::Done,
            TaskStatus::Archived => TaskStatus::Archived,
        }
    }

    /// Retreat to the previous status (wraps at Backlog -> Backlog).
    pub fn prev(self) -> Self {
        match self {
            TaskStatus::Backlog => TaskStatus::Backlog,
            TaskStatus::Running => TaskStatus::Backlog,
            TaskStatus::Review => TaskStatus::Running,
            TaskStatus::Done => TaskStatus::Review,
            TaskStatus::Archived => TaskStatus::Archived,
        }
    }

    /// Zero-based column index for kanban board layout.
    pub fn column_index(self) -> usize {
        match self {
            TaskStatus::Backlog => 0,
            TaskStatus::Running => 1,
            TaskStatus::Review => 2,
            TaskStatus::Done => 3,
            TaskStatus::Archived => TaskStatus::COLUMN_COUNT, // Virtual archive column, rightmost
        }
    }

    /// Construct from a column index; returns None if out of range.
    pub fn from_column_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(TaskStatus::Backlog),
            1 => Some(TaskStatus::Running),
            2 => Some(TaskStatus::Review),
            3 => Some(TaskStatus::Done),
            _ => None,
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown status: {s}"))
    }
}

// ---------------------------------------------------------------------------
// TipsShowMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipsShowMode {
    Always,
    NewOnly,
    Never,
}

impl TipsShowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TipsShowMode::Always => "always",
            TipsShowMode::NewOnly => "new_only",
            TipsShowMode::Never => "never",
        }
    }
}

impl std::str::FromStr for TipsShowMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "always" => Ok(TipsShowMode::Always),
            "new_only" => Ok(TipsShowMode::NewOnly),
            "never" => Ok(TipsShowMode::Never),
            _ => Err(format!("unknown tips show mode: {s}")),
        }
    }
}

// ---------------------------------------------------------------------------
// SubStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatus {
    None,
    Active,
    NeedsInput,
    Stale,
    Crashed,
    Conflict,
    AwaitingReview,
    ChangesRequested,
    Approved,
}

impl SubStatus {
    pub const ALL: &'static [SubStatus] = &[
        SubStatus::None,
        SubStatus::Active,
        SubStatus::NeedsInput,
        SubStatus::Stale,
        SubStatus::Crashed,
        SubStatus::Conflict,
        SubStatus::AwaitingReview,
        SubStatus::ChangesRequested,
        SubStatus::Approved,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            SubStatus::None => "none",
            SubStatus::Active => "active",
            SubStatus::NeedsInput => "needs_input",
            SubStatus::Stale => "stale",
            SubStatus::Crashed => "crashed",
            SubStatus::Conflict => "conflict",
            SubStatus::AwaitingReview => "awaiting_review",
            SubStatus::ChangesRequested => "changes_requested",
            SubStatus::Approved => "approved",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(SubStatus::None),
            "active" => Some(SubStatus::Active),
            "needs_input" => Some(SubStatus::NeedsInput),
            "stale" => Some(SubStatus::Stale),
            "crashed" => Some(SubStatus::Crashed),
            "conflict" => Some(SubStatus::Conflict),
            "awaiting_review" => Some(SubStatus::AwaitingReview),
            "changes_requested" => Some(SubStatus::ChangesRequested),
            "approved" => Some(SubStatus::Approved),
            _ => None,
        }
    }

    /// Check whether this sub-status is valid for the given parent status.
    pub fn is_valid_for(&self, status: TaskStatus) -> bool {
        match status {
            TaskStatus::Backlog => matches!(self, SubStatus::None),
            TaskStatus::Running => matches!(
                self,
                SubStatus::Active
                    | SubStatus::NeedsInput
                    | SubStatus::Stale
                    | SubStatus::Crashed
                    | SubStatus::Conflict
            ),
            TaskStatus::Review => matches!(
                self,
                SubStatus::AwaitingReview
                    | SubStatus::ChangesRequested
                    | SubStatus::Approved
                    | SubStatus::Conflict
            ),
            TaskStatus::Done => matches!(self, SubStatus::None),
            TaskStatus::Archived => matches!(self, SubStatus::None),
        }
    }

    /// Return the default sub-status for a given parent status.
    pub fn default_for(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Backlog => SubStatus::None,
            TaskStatus::Running => SubStatus::Active,
            TaskStatus::Review => SubStatus::AwaitingReview,
            TaskStatus::Done => SubStatus::None,
            TaskStatus::Archived => SubStatus::None,
        }
    }

    /// Sort priority for column grouping (lower = more urgent = top of column).
    pub fn column_priority(self) -> u8 {
        match self {
            SubStatus::Conflict => 0,
            SubStatus::Crashed => 1,
            SubStatus::Stale => 2,
            SubStatus::NeedsInput => 3,
            SubStatus::ChangesRequested => 4,
            SubStatus::Active => 5,
            SubStatus::AwaitingReview => 5, // same slot as Active
            SubStatus::None => 5,
            SubStatus::Approved => 6,
        }
    }

    /// Sort priority for column grouping, detach-aware variant.
    /// Detached review tasks sort below Approved so they sink to the bottom.
    pub fn column_priority_detached(self, is_detached: bool) -> u8 {
        match (self, is_detached) {
            (SubStatus::AwaitingReview, true) => 7,
            _ => self.column_priority(),
        }
    }

    /// Label for section header lines within a column.
    pub fn header_label(self) -> &'static str {
        match self {
            SubStatus::None => "",
            SubStatus::Active => "active",
            SubStatus::NeedsInput => "needs input",
            SubStatus::Stale => "stale",
            SubStatus::Crashed => "crashed",
            SubStatus::Conflict => "conflict",
            SubStatus::AwaitingReview => "awaiting review",
            SubStatus::ChangesRequested => "changes requested",
            SubStatus::Approved => "approved",
        }
    }

    /// Detach-aware section header label.
    /// Detached awaiting_review tasks show "awaiting merge" instead.
    pub fn header_label_detached(self, is_detached: bool) -> &'static str {
        match (self, is_detached) {
            (SubStatus::AwaitingReview, true) => "awaiting merge",
            _ => self.header_label(),
        }
    }
}

impl std::fmt::Display for SubStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SubStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown sub-status: {s}"))
    }
}

// ---------------------------------------------------------------------------
// VisualColumn — the 8 visual columns for the kanban board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VisualColumn {
    pub label: &'static str,
    pub parent_status: TaskStatus,
    pub sub_statuses: &'static [SubStatus],
}

impl VisualColumn {
    pub const COUNT: usize = 8;
    pub const ALL: &'static [VisualColumn] = &[
        VisualColumn {
            label: "Backlog",
            parent_status: TaskStatus::Backlog,
            sub_statuses: &[SubStatus::None],
        },
        VisualColumn {
            label: "Active",
            parent_status: TaskStatus::Running,
            sub_statuses: &[SubStatus::Active],
        },
        VisualColumn {
            label: "Blocked",
            parent_status: TaskStatus::Running,
            sub_statuses: &[SubStatus::NeedsInput],
        },
        VisualColumn {
            label: "Stale",
            parent_status: TaskStatus::Running,
            sub_statuses: &[SubStatus::Stale, SubStatus::Crashed, SubStatus::Conflict],
        },
        VisualColumn {
            label: "PR Created",
            parent_status: TaskStatus::Review,
            sub_statuses: &[SubStatus::AwaitingReview, SubStatus::Conflict],
        },
        VisualColumn {
            label: "Revise",
            parent_status: TaskStatus::Review,
            sub_statuses: &[SubStatus::ChangesRequested],
        },
        VisualColumn {
            label: "Approved",
            parent_status: TaskStatus::Review,
            sub_statuses: &[SubStatus::Approved],
        },
        VisualColumn {
            label: "Done",
            parent_status: TaskStatus::Done,
            sub_statuses: &[SubStatus::None],
        },
    ];

    pub fn contains(&self, sub_status: SubStatus) -> bool {
        self.sub_statuses.contains(&sub_status)
    }

    pub fn parent_group_start(status: TaskStatus) -> usize {
        Self::ALL
            .iter()
            .position(|vc| vc.parent_status == status)
            .unwrap_or(0)
    }

    pub fn parent_group_span(status: TaskStatus) -> usize {
        Self::ALL
            .iter()
            .filter(|vc| vc.parent_status == status)
            .count()
    }
}

// ---------------------------------------------------------------------------
// TaskId
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub i64);

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// EpicId
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EpicId(pub i64);

impl std::fmt::Display for EpicId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
            Self::Blocked(_) => 3, // NeedsInput equivalent
            Self::Active => 5,     // Active equivalent
            Self::WrappingUp => 6, // Approved equivalent
            Self::InReview => 5,   // AwaitingReview equivalent
            Self::Unplanned => 5,
            Self::Planned => 5,
            Self::Done => 5,
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

// ---------------------------------------------------------------------------
// ReviewDecision — review status for a PR
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    ReviewRequired,
    WaitingForResponse,
    ChangesRequested,
    Approved,
}

impl ReviewDecision {
    pub const ALL: [Self; 4] = [
        Self::ReviewRequired,
        Self::WaitingForResponse,
        Self::ChangesRequested,
        Self::Approved,
    ];

    pub const COLUMN_COUNT: usize = Self::ALL.len();

    pub fn column_index(self) -> usize {
        match self {
            Self::ReviewRequired => 0,
            Self::WaitingForResponse => 1,
            Self::ChangesRequested => 2,
            Self::Approved => 3,
        }
    }

    pub fn from_column_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::ReviewRequired),
            1 => Some(Self::WaitingForResponse),
            2 => Some(Self::ChangesRequested),
            3 => Some(Self::Approved),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReviewRequired => "Needs Review",
            Self::WaitingForResponse => "Waiting for Response",
            Self::ChangesRequested => "Changes Requested",
            Self::Approved => "Approved",
        }
    }

    /// Stable string for database storage. Not the same as `as_str()` (display)
    /// or `parse()` (GitHub wire format).
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::ReviewRequired => "ReviewRequired",
            Self::WaitingForResponse => "WaitingForResponse",
            Self::ChangesRequested => "ChangesRequested",
            Self::Approved => "Approved",
        }
    }

    /// Parse from database string. Inverse of `as_db_str`.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "ReviewRequired" => Some(Self::ReviewRequired),
            "WaitingForResponse" => Some(Self::WaitingForResponse),
            "ChangesRequested" => Some(Self::ChangesRequested),
            "Approved" => Some(Self::Approved),
            _ => None,
        }
    }

    /// Parse from GitHub GraphQL `reviewDecision` field value.
    /// Note: `WaitingForResponse` has no wire value — it is derived client-side.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "REVIEW_REQUIRED" => Some(Self::ReviewRequired),
            "CHANGES_REQUESTED" => Some(Self::ChangesRequested),
            "APPROVED" => Some(Self::Approved),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CiStatus — CI check status for a PR
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiStatus {
    Pending,
    Success,
    Failure,
    None,
}

impl CiStatus {
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Pending => "\u{23f3}", // ⏳
            Self::Success => "\u{2713}", // ✓
            Self::Failure => "\u{2717}", // ✗
            Self::None => "\u{00b7}",    // ·
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Success => "Success",
            Self::Failure => "Failure",
            Self::None => "None",
        }
    }

    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::None => "none",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "success" => Self::Success,
            "failure" => Self::Failure,
            _ => Self::None,
        }
    }

    /// Column index for the dependabot board: Passing=0, Failing=1, Pending=2.
    /// `None` maps to Pending (col 2) as a safe default.
    pub fn column_index(self) -> usize {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Pending | Self::None => 2,
        }
    }

    pub fn from_github(s: Option<&str>) -> Self {
        match s {
            Some("SUCCESS") => Self::Success,
            Some("FAILURE") | Some("ERROR") => Self::Failure,
            Some("PENDING") | Some("EXPECTED") => Self::Pending,
            _ => Self::None,
        }
    }
}

// ---------------------------------------------------------------------------
// Reviewer — a reviewer on a PR
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewer {
    pub login: String,
    pub decision: Option<ReviewDecision>,
}

// ---------------------------------------------------------------------------
// ReviewAgentStatus — lifecycle state of a dispatched review/fix agent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAgentStatus {
    Reviewing,
    FindingsReady,
    Idle,
}

impl ReviewAgentStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Reviewing => "reviewing",
            Self::FindingsReady => "findings_ready",
            Self::Idle => "idle",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "reviewing" => Some(Self::Reviewing),
            "findings_ready" => Some(Self::FindingsReady),
            "idle" => Some(Self::Idle),
            _ => None,
        }
    }
}

impl std::fmt::Display for ReviewAgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reviewing => write!(f, "reviewing"),
            Self::FindingsReady => write!(f, "ready"),
            Self::Idle => write!(f, "idle"),
        }
    }
}

// ---------------------------------------------------------------------------
// ReviewWorkflowState + ReviewWorkflowSubState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewWorkflowState {
    Backlog,
    Ongoing,
    ActionRequired,
    Done,
}

impl ReviewWorkflowState {
    pub const COLUMN_COUNT: usize = 4;
    pub const ALL: [Self; 4] = [
        Self::Backlog,
        Self::Ongoing,
        Self::ActionRequired,
        Self::Done,
    ];

    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Ongoing => "ongoing",
            Self::ActionRequired => "action_required",
            Self::Done => "done",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "backlog" => Some(Self::Backlog),
            "ongoing" => Some(Self::Ongoing),
            "action_required" => Some(Self::ActionRequired),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn column_index(self) -> usize {
        match self {
            Self::Backlog => 0,
            Self::Ongoing => 1,
            Self::ActionRequired => 2,
            Self::Done => 3,
        }
    }

    pub fn column_label(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::Ongoing => "Ongoing",
            Self::ActionRequired => "Action Required",
            Self::Done => "Done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewWorkflowSubState {
    // Ongoing sub-states
    Reviewing,
    Idle,
    Stale,
    // ActionRequired sub-states
    FindingsReady,
    ChangesRequested,
    AwaitingResponse,
    CiFailing,
    ReadyToMerge,
}

impl ReviewWorkflowSubState {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Reviewing => "reviewing",
            Self::Idle => "idle",
            Self::Stale => "stale",
            Self::FindingsReady => "findings_ready",
            Self::ChangesRequested => "changes_requested",
            Self::AwaitingResponse => "awaiting_response",
            Self::CiFailing => "ci_failing",
            Self::ReadyToMerge => "ready_to_merge",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "reviewing" => Some(Self::Reviewing),
            "idle" => Some(Self::Idle),
            "stale" => Some(Self::Stale),
            "findings_ready" => Some(Self::FindingsReady),
            "changes_requested" => Some(Self::ChangesRequested),
            "awaiting_response" => Some(Self::AwaitingResponse),
            "ci_failing" => Some(Self::CiFailing),
            "ready_to_merge" => Some(Self::ReadyToMerge),
            _ => None,
        }
    }

    pub fn section_label(self) -> &'static str {
        match self {
            Self::Reviewing => "reviewing",
            Self::Idle => "idle",
            Self::Stale => "stale",
            Self::FindingsReady => "findings ready",
            Self::ChangesRequested => "changes requested",
            Self::AwaitingResponse => "awaiting response",
            Self::CiFailing => "ci failing",
            Self::ReadyToMerge => "ready to merge",
        }
    }
}

// ---------------------------------------------------------------------------
// SecurityWorkflowState + SecurityWorkflowSubState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecurityWorkflowState {
    Backlog,
    Ongoing,
    ActionRequired,
    Done,
}

impl SecurityWorkflowState {
    pub const COLUMN_COUNT: usize = 4;
    pub const ALL: [Self; 4] = [
        Self::Backlog,
        Self::Ongoing,
        Self::ActionRequired,
        Self::Done,
    ];

    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Ongoing => "ongoing",
            Self::ActionRequired => "action_required",
            Self::Done => "done",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "backlog" => Some(Self::Backlog),
            "ongoing" => Some(Self::Ongoing),
            "action_required" => Some(Self::ActionRequired),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn column_index(self) -> usize {
        match self {
            Self::Backlog => 0,
            Self::Ongoing => 1,
            Self::ActionRequired => 2,
            Self::Done => 3,
        }
    }

    pub fn column_label(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::Ongoing => "Ongoing",
            Self::ActionRequired => "Action Required",
            Self::Done => "Done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecurityWorkflowSubState {
    // Ongoing sub-states
    Investigating,
    Idle,
    Stale,
    // ActionRequired — no fix PR
    FindingsReady,
    NeedsManualFix,
    // ActionRequired — fix PR exists
    PrOpen,
    ChangesRequested,
    CiFailing,
    ReadyToMerge,
}

impl SecurityWorkflowSubState {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Investigating => "investigating",
            Self::Idle => "idle",
            Self::Stale => "stale",
            Self::FindingsReady => "findings_ready",
            Self::NeedsManualFix => "needs_manual_fix",
            Self::PrOpen => "pr_open",
            Self::ChangesRequested => "changes_requested",
            Self::CiFailing => "ci_failing",
            Self::ReadyToMerge => "ready_to_merge",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "investigating" => Some(Self::Investigating),
            "idle" => Some(Self::Idle),
            "stale" => Some(Self::Stale),
            "findings_ready" => Some(Self::FindingsReady),
            "needs_manual_fix" => Some(Self::NeedsManualFix),
            "pr_open" => Some(Self::PrOpen),
            "changes_requested" => Some(Self::ChangesRequested),
            "ci_failing" => Some(Self::CiFailing),
            "ready_to_merge" => Some(Self::ReadyToMerge),
            _ => None,
        }
    }

    pub fn section_label(self) -> &'static str {
        match self {
            Self::Investigating => "investigating",
            Self::Idle => "idle",
            Self::Stale => "stale",
            Self::FindingsReady => "findings ready",
            Self::NeedsManualFix => "needs manual fix",
            Self::PrOpen => "pr open",
            Self::ChangesRequested => "changes requested",
            Self::CiFailing => "ci failing",
            Self::ReadyToMerge => "ready to merge",
        }
    }
}

// ---------------------------------------------------------------------------
// WorkflowItemKind — identifies which board/table a pr_workflow_states row belongs to
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkflowItemKind {
    ReviewerPr,
    DependabotPr,
    DependabotAlert,
    CodeScanAlert,
}

impl WorkflowItemKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::ReviewerPr => "reviewer_pr",
            Self::DependabotPr => "dependabot_pr",
            Self::DependabotAlert => "dependabot_alert",
            Self::CodeScanAlert => "code_scan_alert",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "reviewer_pr" => Some(Self::ReviewerPr),
            "dependabot_pr" => Some(Self::DependabotPr),
            "dependabot_alert" => Some(Self::DependabotAlert),
            "code_scan_alert" => Some(Self::CodeScanAlert),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ReviewPr — a PR the user is expected to review
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReviewPr {
    pub number: i64,
    pub title: String,
    pub author: String,
    pub repo: String,
    pub url: String,
    pub is_draft: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub additions: i64,
    pub deletions: i64,
    pub review_decision: ReviewDecision,
    pub labels: Vec<String>,
    pub body: String,
    pub head_ref: String,
    pub ci_status: CiStatus,
    pub reviewers: Vec<Reviewer>,
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

pub const DEFAULT_QUICK_TASK_TITLE: &str = "Quick task";
pub const DEFAULT_BASE_BRANCH: &str = "main";

#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: TaskStatus,
    pub worktree: Option<String>,
    pub tmux_window: Option<String>,
    pub plan_path: Option<String>,
    pub epic_id: Option<EpicId>,
    pub sub_status: SubStatus,
    pub pr_url: Option<String>,
    pub tag: Option<TaskTag>,
    pub sort_order: Option<i64>,
    pub base_branch: String,
    pub external_id: Option<String>,
    pub project_id: ProjectId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    /// Whether this task has a worktree but no tmux window (agent session ended).
    /// Excludes conflict state which is handled separately.
    pub fn is_detached(&self) -> bool {
        self.worktree.is_some()
            && self.tmux_window.is_none()
            && matches!(self.status, TaskStatus::Running | TaskStatus::Review)
            && self.sub_status != SubStatus::Conflict
    }
}

// ---------------------------------------------------------------------------
// FeedItem — an item from a programmable epic feed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedItem {
    pub external_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub url: String,
    pub status: TaskStatus,
}

// ---------------------------------------------------------------------------
// DispatchMode
// ---------------------------------------------------------------------------

/// Determines how a backlog task should be dispatched based on whether it has
/// a plan and its tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    Dispatch,
    Brainstorm,
    Plan,
}

impl DispatchMode {
    /// Select the dispatch mode for a task: tasks with a plan always get
    /// `Dispatch`; otherwise the tag drives the choice.
    pub fn for_task(task: &Task) -> Self {
        if task.plan_path.is_some() {
            DispatchMode::Dispatch
        } else {
            match task.tag {
                Some(TaskTag::Epic) => DispatchMode::Brainstorm,
                Some(TaskTag::Feature) => DispatchMode::Plan,
                _ => DispatchMode::Dispatch,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TaskTag
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskTag {
    Bug,
    Feature,
    Chore,
    Epic,
}

impl TaskTag {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskTag::Bug => "bug",
            TaskTag::Feature => "feature",
            TaskTag::Chore => "chore",
            TaskTag::Epic => "epic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bug" => Some(TaskTag::Bug),
            "feature" => Some(TaskTag::Feature),
            "chore" => Some(TaskTag::Chore),
            "epic" => Some(TaskTag::Epic),
            _ => None,
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            TaskTag::Bug => "bug",
            TaskTag::Feature => "feat",
            TaskTag::Chore => "chore",
            TaskTag::Epic => "epic",
        }
    }
}

impl std::fmt::Display for TaskTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TaskTag {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown tag: {s}"))
    }
}

// ---------------------------------------------------------------------------
// DispatchResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub worktree_path: String,
    pub tmux_window: String,
}

// ---------------------------------------------------------------------------
// ResumeResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResumeResult {
    pub tmux_window: String,
}

// ---------------------------------------------------------------------------
// TaskUsage
// ---------------------------------------------------------------------------

/// Usage metrics for a single reporting interval (no task_id or timestamp).
#[derive(Debug, Clone)]
pub struct UsageReport {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
}

/// Accumulated usage stored in the database, keyed by task.
#[derive(Debug, Clone)]
pub struct TaskUsage {
    pub task_id: TaskId,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// slugify
// ---------------------------------------------------------------------------

/// Convert an arbitrary string into a URL/filesystem-safe slug.
/// - Lowercased
/// - Non-alphanumeric characters replaced with `-`
/// - Consecutive dashes collapsed to one
/// - Leading/trailing dashes trimmed
/// - Returns `"task"` if the result would be empty
pub fn slugify(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut slug = String::with_capacity(lower.len());
    let mut last_was_dash = false;

    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else {
            if !last_was_dash && !slug.is_empty() {
                slug.push('-');
                last_was_dash = true;
            }
        }
    }

    // Trim trailing dash
    let slug = slug.trim_end_matches('-').to_string();

    if slug.is_empty() {
        "task".to_string()
    } else {
        slug
    }
}

// ---------------------------------------------------------------------------
// Staleness
// ---------------------------------------------------------------------------

/// Tasks updated within this many hours are considered fresh.
const FRESH_THRESHOLD_HOURS: i64 = 3 * 24; // 3 days
/// Tasks updated within this many hours are aging (not yet stale).
const AGING_THRESHOLD_HOURS: i64 = 7 * 24; // 7 days
/// Days threshold above which format_age switches to weeks.
const WEEKS_THRESHOLD_DAYS: i64 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    Fresh, // < 3 days
    Aging, // 3-7 days
    Stale, // > 7 days
}

impl Staleness {
    /// Determine staleness tier from the age of `timestamp` relative to `now`.
    pub fn from_age(timestamp: DateTime<Utc>, now: DateTime<Utc>) -> Self {
        let age = now.signed_duration_since(timestamp);
        let hours = age.num_hours().max(0);
        if hours < FRESH_THRESHOLD_HOURS {
            Staleness::Fresh
        } else if hours < AGING_THRESHOLD_HOURS {
            Staleness::Aging
        } else {
            Staleness::Stale
        }
    }
}

// ---------------------------------------------------------------------------
// format_age
// ---------------------------------------------------------------------------

/// Format the age of `updated_at` relative to `now` as a compact label.
/// Returns strings like "<1h", "3h", "2d", "3w".
pub fn format_age(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let age = now.signed_duration_since(updated_at);
    let hours = age.num_hours().max(0);

    if hours < 1 {
        "<1h".to_string()
    } else if hours < 24 {
        format!("{hours}h")
    } else {
        let days = hours / 24;
        if days < WEEKS_THRESHOLD_DAYS {
            format!("{days}d")
        } else {
            format!("{}w", days / 7)
        }
    }
}

// ---------------------------------------------------------------------------
// format_detail_age
// ---------------------------------------------------------------------------

/// Format age for the detail panel — slightly more verbose than card labels.
/// Returns strings like "less than 1 hour", "1 hour", "5 hours", "1 day", "3 days".
pub fn format_detail_age(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let age = now.signed_duration_since(updated_at);
    let total_hours = age.num_hours().max(0);

    if total_hours < 1 {
        "less than 1 hour".to_string()
    } else if total_hours == 1 {
        "1 hour".to_string()
    } else if total_hours < 24 {
        format!("{total_hours} hours")
    } else {
        let days = total_hours / 24;
        if days == 1 {
            "1 day".to_string()
        } else {
            format!("{days} days")
        }
    }
}

// ---------------------------------------------------------------------------
// AlertSeverity — severity level for security alerts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl AlertSeverity {
    pub const ALL: [Self; 4] = [Self::Critical, Self::High, Self::Medium, Self::Low];

    pub const COLUMN_COUNT: usize = Self::ALL.len();

    pub fn column_index(self) -> usize {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }

    pub fn from_column_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::Critical),
            1 => Some(Self::High),
            2 => Some(Self::Medium),
            3 => Some(Self::Low),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }

    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "critical" => Some(Self::Critical),
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "critical" => Some(Self::Critical),
            "high" => Some(Self::High),
            "medium" | "moderate" => Some(Self::Medium),
            "low" => Some(Self::Low),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// AlertKind — type of security alert
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Dependabot,
    CodeScanning,
}

impl AlertKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dependabot => "Dependabot",
            Self::CodeScanning => "Code Scanning",
        }
    }

    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Dependabot => "dependabot",
            Self::CodeScanning => "code_scanning",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "dependabot" => Some(Self::Dependabot),
            "code_scanning" => Some(Self::CodeScanning),
            _ => None,
        }
    }

    pub fn indicator(&self) -> &'static str {
        match self {
            Self::Dependabot => "D",
            Self::CodeScanning => "S",
        }
    }
}

// ---------------------------------------------------------------------------
// SecurityWorkflowColumn — workflow stage for security alerts (left → right)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityWorkflowColumn {
    Backlog,
    InProgress,
    Review,
}

impl SecurityWorkflowColumn {
    pub const COLUMN_COUNT: usize = 3;
    pub const ALL: [Self; 3] = [Self::Backlog, Self::InProgress, Self::Review];

    pub fn column_index(self) -> usize {
        match self {
            Self::Backlog => 0,
            Self::InProgress => 1,
            Self::Review => 2,
        }
    }

    pub fn from_column_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::Backlog),
            1 => Some(Self::InProgress),
            2 => Some(Self::Review),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::InProgress => "In Progress",
            Self::Review => "Review",
        }
    }
}

// ---------------------------------------------------------------------------
// SecurityAlert — a security vulnerability alert from GitHub
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SecurityAlert {
    pub number: i64,
    pub repo: String,
    pub severity: AlertSeverity,
    pub kind: AlertKind,
    pub title: String,
    pub package: Option<String>,
    pub vulnerable_range: Option<String>,
    pub fixed_version: Option<String>,
    pub cvss_score: Option<f64>,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub state: String,
    pub description: String,
}

// ---------------------------------------------------------------------------
// PrRef — newtype for (repo, PR number) tuples
// ---------------------------------------------------------------------------

/// A reference to a pull request or security alert, identified by repo and number.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrRef {
    repo: String,
    number: i64,
}

impl PrRef {
    pub fn new(repo: String, number: i64) -> Self {
        Self { repo, number }
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn number(&self) -> i64 {
        self.number
    }

    pub fn matches(&self, number: i64, repo: &str) -> bool {
        self.number == number && self.repo == repo
    }
}

impl std::fmt::Display for PrRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

// ---------------------------------------------------------------------------
// pr_number_from_url
// ---------------------------------------------------------------------------

/// Extract the PR number from a GitHub PR URL.
/// Handles trailing slashes, query parameters, and fragment identifiers.
pub fn pr_number_from_url(url: &str) -> Option<i64> {
    url.split(['?', '#'])
        .next()
        .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
        .and_then(|s| s.parse::<i64>().ok())
}

/// Returns the URL type word: `"PR"` for pull requests, `"Issue"` for issues, `"Link"` otherwise.
pub fn url_type(url: &str) -> &'static str {
    let clean = url.split(['?', '#']).next().unwrap_or(url);
    if clean.contains("/pull/") {
        "PR"
    } else if clean.contains("/issues/") {
        "Issue"
    } else {
        "Link"
    }
}

/// Returns the display label for a URL stored in the `pr_url` field.
///
/// - URLs containing `/pull/<N>` → `"PR #N"` (or `"PR"` if no number follows)
/// - URLs containing `/issues/<N>` → `"Issue #N"` (or `"Issue"` if no number follows)
/// - Anything else → `"Link"`
pub fn url_label(url: &str) -> String {
    let clean = url.split(['?', '#']).next().unwrap_or(url);
    let type_label = url_type(url);

    if type_label != "Link" {
        let segment = if type_label == "PR" {
            "/pull/"
        } else {
            "/issues/"
        };
        if let Some((_, after)) = clean.split_once(segment) {
            if let Some(n) = after
                .trim_end_matches('/')
                .split('/')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
            {
                return format!("{type_label} #{n}");
            }
            return type_label.to_string();
        }
    }

    "Link".to_string()
}

/// Extract the GitHub repo slug (e.g. "org/repo") from a PR URL.
/// Handles query parameters and fragment identifiers.
/// Returns None for non-GitHub URLs or malformed input.
pub fn github_repo_from_pr_url(url: &str) -> Option<String> {
    url.split(['?', '#'])
        .next()
        .and_then(|u| u.strip_prefix("https://github.com/"))
        .and_then(|rest| rest.split("/pull/").next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // --- expand_tilde ---

    #[test]
    fn expand_tilde_with_path() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_tilde("~/projects/foo"),
            format!("{home}/projects/foo")
        );
    }

    #[test]
    fn expand_tilde_bare() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expand_tilde_absolute_unchanged() {
        assert_eq!(expand_tilde("/home/user/foo"), "/home/user/foo");
    }

    // --- TaskStatus ---

    #[test]
    fn status_roundtrip() {
        for &status in TaskStatus::ALL {
            let s = status.as_str();
            let parsed = TaskStatus::parse(s).expect("roundtrip failed");
            assert_eq!(status, parsed, "roundtrip failed for {:?}", status);
        }
    }

    #[test]
    fn status_invalid_from_str() {
        assert!(TaskStatus::parse("").is_none());
        assert!(TaskStatus::parse("unknown").is_none());
        assert!(
            TaskStatus::parse("Backlog").is_none(),
            "should be case-sensitive"
        );
    }

    #[test]
    fn archived_column_index_is_column_count() {
        assert_eq!(
            TaskStatus::Archived.column_index(),
            TaskStatus::COLUMN_COUNT
        );
    }

    #[test]
    fn parse_ready_maps_to_backlog() {
        assert_eq!(TaskStatus::parse("ready"), Some(TaskStatus::Backlog));
    }

    #[test]
    fn status_next() {
        assert_eq!(TaskStatus::Backlog.next(), TaskStatus::Running);
        assert_eq!(TaskStatus::Running.next(), TaskStatus::Review);
        assert_eq!(TaskStatus::Review.next(), TaskStatus::Done);
        assert_eq!(
            TaskStatus::Done.next(),
            TaskStatus::Done,
            "Done.next() should stay Done"
        );
    }

    #[test]
    fn status_prev() {
        assert_eq!(TaskStatus::Done.prev(), TaskStatus::Review);
        assert_eq!(TaskStatus::Review.prev(), TaskStatus::Running);
        assert_eq!(TaskStatus::Running.prev(), TaskStatus::Backlog);
        assert_eq!(
            TaskStatus::Backlog.prev(),
            TaskStatus::Backlog,
            "Backlog.prev() should stay Backlog"
        );
    }

    #[test]
    fn status_column_index_roundtrip() {
        for &status in TaskStatus::ALL {
            let idx = status.column_index();
            let back = TaskStatus::from_column_index(idx).expect("column roundtrip failed");
            assert_eq!(status, back);
        }
    }

    #[test]
    fn column_index_out_of_range() {
        assert!(TaskStatus::from_column_index(4).is_none());
        assert!(TaskStatus::from_column_index(999).is_none());
    }

    // --- SubStatus ---

    #[test]
    fn substatus_roundtrip() {
        for &sub in SubStatus::ALL {
            let s = sub.as_str();
            let parsed: SubStatus = s
                .parse()
                .unwrap_or_else(|e| panic!("roundtrip failed for {s}: {e}"));
            assert_eq!(sub, parsed, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn substatus_as_str_is_snake_case() {
        assert_eq!(SubStatus::None.as_str(), "none");
        assert_eq!(SubStatus::Active.as_str(), "active");
        assert_eq!(SubStatus::NeedsInput.as_str(), "needs_input");
        assert_eq!(SubStatus::Stale.as_str(), "stale");
        assert_eq!(SubStatus::Crashed.as_str(), "crashed");
        assert_eq!(SubStatus::Conflict.as_str(), "conflict");
        assert_eq!(SubStatus::AwaitingReview.as_str(), "awaiting_review");
        assert_eq!(SubStatus::ChangesRequested.as_str(), "changes_requested");
        assert_eq!(SubStatus::Approved.as_str(), "approved");
    }

    #[test]
    fn substatus_from_str_invalid() {
        assert!("bogus".parse::<SubStatus>().is_err());
        assert!("".parse::<SubStatus>().is_err());
        assert!(
            "None".parse::<SubStatus>().is_err(),
            "should be case-sensitive"
        );
    }

    #[test]
    fn substatus_display() {
        assert_eq!(format!("{}", SubStatus::NeedsInput), "needs_input");
        assert_eq!(format!("{}", SubStatus::AwaitingReview), "awaiting_review");
    }

    #[test]
    fn substatus_valid_combinations() {
        // Backlog: only None
        assert!(SubStatus::None.is_valid_for(TaskStatus::Backlog));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Backlog));
        assert!(!SubStatus::NeedsInput.is_valid_for(TaskStatus::Backlog));
        assert!(!SubStatus::AwaitingReview.is_valid_for(TaskStatus::Backlog));

        // Running: Active, NeedsInput, Stale, Crashed
        assert!(!SubStatus::None.is_valid_for(TaskStatus::Running));
        assert!(SubStatus::Active.is_valid_for(TaskStatus::Running));
        assert!(SubStatus::NeedsInput.is_valid_for(TaskStatus::Running));
        assert!(SubStatus::Stale.is_valid_for(TaskStatus::Running));
        assert!(SubStatus::Crashed.is_valid_for(TaskStatus::Running));
        assert!(!SubStatus::AwaitingReview.is_valid_for(TaskStatus::Running));

        // Review: AwaitingReview, ChangesRequested, Approved
        assert!(!SubStatus::None.is_valid_for(TaskStatus::Review));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Review));
        assert!(SubStatus::AwaitingReview.is_valid_for(TaskStatus::Review));
        assert!(SubStatus::ChangesRequested.is_valid_for(TaskStatus::Review));
        assert!(SubStatus::Approved.is_valid_for(TaskStatus::Review));

        // Done: only None
        assert!(SubStatus::None.is_valid_for(TaskStatus::Done));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Done));

        // Archived: only None
        assert!(SubStatus::None.is_valid_for(TaskStatus::Archived));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Archived));
    }

    #[test]
    fn substatus_default_for() {
        assert_eq!(SubStatus::default_for(TaskStatus::Backlog), SubStatus::None);
        assert_eq!(
            SubStatus::default_for(TaskStatus::Running),
            SubStatus::Active
        );
        assert_eq!(
            SubStatus::default_for(TaskStatus::Review),
            SubStatus::AwaitingReview
        );
        assert_eq!(SubStatus::default_for(TaskStatus::Done), SubStatus::None);
        assert_eq!(
            SubStatus::default_for(TaskStatus::Archived),
            SubStatus::None
        );
    }

    #[test]
    fn substatus_rules_consistent_with_visual_columns() {
        // Every SubStatus in a VisualColumn must be valid for that column's parent_status
        for vc in VisualColumn::ALL {
            for &sub in vc.sub_statuses {
                assert!(
                    sub.is_valid_for(vc.parent_status),
                    "{sub:?} in column {:?} but not valid for {:?}",
                    vc.label,
                    vc.parent_status
                );
            }
        }
        // Every valid (status, substatus) pair must appear in exactly one VisualColumn
        for &status in TaskStatus::ALL {
            for &sub in SubStatus::ALL {
                if sub.is_valid_for(status) {
                    let count = VisualColumn::ALL
                        .iter()
                        .filter(|vc| vc.parent_status == status && vc.contains(sub))
                        .count();
                    assert_eq!(
                        count, 1,
                        "{sub:?}/{status:?} is valid but appears in {count} VisualColumns"
                    );
                }
            }
        }
        // default_for() must be valid
        for &status in TaskStatus::ALL {
            let default = SubStatus::default_for(status);
            assert!(
                default.is_valid_for(status),
                "default_for({status:?}) = {default:?} is not valid"
            );
        }
    }

    // --- VisualColumn ---

    #[test]
    fn visual_columns_count_is_8() {
        assert_eq!(VisualColumn::ALL.len(), 8);
        assert_eq!(VisualColumn::COUNT, 8);
        assert_eq!(VisualColumn::ALL.len(), VisualColumn::COUNT);
    }

    #[test]
    fn visual_column_parent_status_mapping() {
        assert_eq!(VisualColumn::ALL[0].parent_status, TaskStatus::Backlog);
        assert_eq!(VisualColumn::ALL[1].parent_status, TaskStatus::Running);
        assert_eq!(VisualColumn::ALL[2].parent_status, TaskStatus::Running);
        assert_eq!(VisualColumn::ALL[3].parent_status, TaskStatus::Running);
        assert_eq!(VisualColumn::ALL[4].parent_status, TaskStatus::Review);
        assert_eq!(VisualColumn::ALL[5].parent_status, TaskStatus::Review);
        assert_eq!(VisualColumn::ALL[6].parent_status, TaskStatus::Review);
        assert_eq!(VisualColumn::ALL[7].parent_status, TaskStatus::Done);
    }

    #[test]
    fn visual_column_contains_substatus() {
        // Column 3 ("Stale") contains Stale and Crashed, but not Active
        let stale_col = &VisualColumn::ALL[3];
        assert!(stale_col.contains(SubStatus::Stale));
        assert!(stale_col.contains(SubStatus::Crashed));
        assert!(!stale_col.contains(SubStatus::Active));
    }

    #[test]
    fn visual_column_parent_group_start() {
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Backlog), 0);
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Running), 1);
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Review), 4);
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Done), 7);
    }

    #[test]
    fn visual_column_parent_group_span() {
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Backlog), 1);
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Running), 3);
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Review), 3);
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Done), 1);
    }

    // --- slugify ---

    #[test]
    fn slugify_normal() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Foo & Bar! (baz)"), "foo-bar-baz");
    }

    #[test]
    fn slugify_empty() {
        assert_eq!(slugify(""), "task");
    }

    #[test]
    fn slugify_only_special() {
        assert_eq!(slugify("!!!"), "task");
    }

    #[test]
    fn slugify_collapsed_dashes() {
        assert_eq!(slugify("a---b"), "a-b");
        assert_eq!(slugify("a & & b"), "a-b");
    }

    #[test]
    fn slugify_leading_trailing_special() {
        assert_eq!(slugify("  hello  "), "hello");
        assert_eq!(slugify("---hello---"), "hello");
    }

    #[test]
    fn slugify_numbers() {
        assert_eq!(slugify("Task 42"), "task-42");
    }

    proptest! {
        #[test]
        fn slugify_never_panics(input in "\\PC{0,2000}") {
            // slugify should never panic on arbitrary input
            let _ = slugify(&input);
        }
    }

    #[test]
    fn column_count_matches_all_len() {
        assert_eq!(TaskStatus::COLUMN_COUNT, TaskStatus::ALL.len());
        assert_eq!(TaskStatus::COLUMN_COUNT, 4);
    }

    #[test]
    fn task_status_display() {
        for &status in TaskStatus::ALL {
            assert_eq!(format!("{status}"), status.as_str());
        }
    }

    #[test]
    fn task_status_from_str_roundtrip() {
        for &status in TaskStatus::ALL {
            let parsed: TaskStatus = status.as_str().parse().unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn task_status_from_str_error() {
        let result: Result<TaskStatus, _> = "bogus".parse();
        assert!(result.is_err());
    }

    #[test]
    fn status_archived_roundtrip() {
        let s = TaskStatus::Archived.as_str();
        assert_eq!(s, "archived");
        let parsed = TaskStatus::parse(s).expect("roundtrip failed");
        assert_eq!(parsed, TaskStatus::Archived);
    }

    #[test]
    fn status_archived_is_terminal() {
        assert_eq!(TaskStatus::Archived.next(), TaskStatus::Archived);
        assert_eq!(TaskStatus::Archived.prev(), TaskStatus::Archived);
    }

    #[test]
    fn status_archived_has_no_column() {
        // Archived is not a kanban column — COLUMN_COUNT stays 4
        assert_eq!(TaskStatus::COLUMN_COUNT, 4);
    }

    // --- Staleness ---

    #[test]
    fn staleness_fresh() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(71);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Fresh);
    }

    #[test]
    fn staleness_fresh_boundary() {
        let now = Utc::now();
        // Exactly 3 days minus 1 second => still Fresh
        let updated = now - chrono::Duration::seconds(3 * 24 * 3600 - 1);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Fresh);
    }

    #[test]
    fn staleness_aging() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(3);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Aging);
    }

    #[test]
    fn staleness_aging_boundary() {
        let now = Utc::now();
        // Exactly 7 days minus 1 second => still Aging
        let updated = now - chrono::Duration::seconds(7 * 24 * 3600 - 1);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Aging);
    }

    #[test]
    fn staleness_stale() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(7);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Stale);
    }

    #[test]
    fn staleness_very_stale() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(30);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Stale);
    }

    #[test]
    fn staleness_future_is_fresh() {
        let now = Utc::now();
        let updated = now + chrono::Duration::hours(1);
        assert_eq!(Staleness::from_age(updated, now), Staleness::Fresh);
    }

    // --- format_age ---

    #[test]
    fn format_age_minutes() {
        let now = Utc::now();
        let updated = now - chrono::Duration::minutes(30);
        assert_eq!(format_age(updated, now), "<1h");
    }

    #[test]
    fn format_age_one_hour() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(1);
        assert_eq!(format_age(updated, now), "1h");
    }

    #[test]
    fn format_age_hours() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(23);
        assert_eq!(format_age(updated, now), "23h");
    }

    #[test]
    fn format_age_one_day() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(24);
        assert_eq!(format_age(updated, now), "1d");
    }

    #[test]
    fn format_age_days() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(5);
        assert_eq!(format_age(updated, now), "5d");
    }

    #[test]
    fn format_age_thirteen_days() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(13);
        assert_eq!(format_age(updated, now), "13d");
    }

    #[test]
    fn format_age_two_weeks() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(14);
        assert_eq!(format_age(updated, now), "2w");
    }

    #[test]
    fn format_age_three_weeks() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(21);
        assert_eq!(format_age(updated, now), "3w");
    }

    #[test]
    fn format_age_future() {
        let now = Utc::now();
        let updated = now + chrono::Duration::hours(5);
        assert_eq!(format_age(updated, now), "<1h");
    }

    // --- format_detail_age ---

    #[test]
    fn format_detail_age_minutes() {
        let now = Utc::now();
        let updated = now - chrono::Duration::minutes(30);
        assert_eq!(format_detail_age(updated, now), "less than 1 hour");
    }

    #[test]
    fn format_detail_age_one_hour() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(1);
        assert_eq!(format_detail_age(updated, now), "1 hour");
    }

    #[test]
    fn format_detail_age_hours() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(5);
        assert_eq!(format_detail_age(updated, now), "5 hours");
    }

    #[test]
    fn format_detail_age_one_day() {
        let now = Utc::now();
        let updated = now - chrono::Duration::hours(24);
        assert_eq!(format_detail_age(updated, now), "1 day");
    }

    #[test]
    fn format_detail_age_days() {
        let now = Utc::now();
        let updated = now - chrono::Duration::days(10);
        assert_eq!(format_detail_age(updated, now), "10 days");
    }

    #[test]
    fn format_detail_age_future() {
        let now = Utc::now();
        let updated = now + chrono::Duration::hours(3);
        assert_eq!(format_detail_age(updated, now), "less than 1 hour");
    }

    // --- EpicId ---

    #[test]
    fn epic_id_display() {
        let id = EpicId(42);
        assert_eq!(format!("{id}"), "42");
    }

    #[test]
    fn epic_id_equality() {
        assert_eq!(EpicId(1), EpicId(1));
        assert_ne!(EpicId(1), EpicId(2));
    }

    #[test]
    fn task_epic_id_defaults_to_none() {
        let now = Utc::now();
        let task = Task {
            id: TaskId(1),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: None,
            sub_status: SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        };
        assert!(task.epic_id.is_none());
    }

    #[test]
    fn task_with_epic_id() {
        let now = Utc::now();
        let task = Task {
            id: TaskId(1),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: Some(EpicId(5)),
            sub_status: SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        };
        assert_eq!(task.epic_id, Some(EpicId(5)));
    }

    #[test]
    fn epic_struct_fields() {
        let now = Utc::now();
        let epic = Epic {
            id: EpicId(1),
            title: "Auth Rewrite".to_string(),
            description: "Rewrite auth system".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        };
        assert_eq!(epic.id, EpicId(1));
        assert_eq!(epic.status, TaskStatus::Backlog);
    }

    // --- Epic.status direct access ---
    // epic_status() was a wrapper that once derived status from subtasks.
    // It was deleted; callers should access epic.status directly.

    fn make_epic_for_status(status: TaskStatus) -> Epic {
        Epic {
            id: EpicId(1),
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: 1,
        }
    }

    #[test]
    fn epic_has_status_field_directly_accessible() {
        // Regression guard: epic.status is public and accessible directly.
        // Previously callers used epic_status(&epic) — that wrapper no longer exists.
        let epic = make_epic_for_status(TaskStatus::Done);
        assert_eq!(epic.status, TaskStatus::Done);

        let epic = make_epic_for_status(TaskStatus::Backlog);
        assert_eq!(epic.status, TaskStatus::Backlog);

        let epic = make_epic_for_status(TaskStatus::Running);
        assert_eq!(epic.status, TaskStatus::Running);

        let epic = make_epic_for_status(TaskStatus::Review);
        assert_eq!(epic.status, TaskStatus::Review);
    }

    // --- ReviewDecision ---

    #[test]
    fn review_decision_column_count() {
        assert_eq!(ReviewDecision::COLUMN_COUNT, 4);
        assert_eq!(ReviewDecision::ALL.len(), 4);
    }

    #[test]
    fn review_decision_column_index_roundtrip() {
        for (i, decision) in ReviewDecision::ALL.iter().enumerate() {
            assert_eq!(decision.column_index(), i);
        }
    }

    #[test]
    fn review_decision_from_column_index() {
        assert_eq!(
            ReviewDecision::from_column_index(0),
            Some(ReviewDecision::ReviewRequired)
        );
        assert_eq!(
            ReviewDecision::from_column_index(1),
            Some(ReviewDecision::WaitingForResponse)
        );
        assert_eq!(
            ReviewDecision::from_column_index(2),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(
            ReviewDecision::from_column_index(3),
            Some(ReviewDecision::Approved)
        );
        assert_eq!(ReviewDecision::from_column_index(4), None);
    }

    #[test]
    fn review_decision_as_str() {
        assert_eq!(ReviewDecision::ReviewRequired.as_str(), "Needs Review");
        assert_eq!(
            ReviewDecision::ChangesRequested.as_str(),
            "Changes Requested"
        );
        assert_eq!(ReviewDecision::Approved.as_str(), "Approved");
    }

    #[test]
    fn review_decision_parse() {
        assert_eq!(
            ReviewDecision::parse("REVIEW_REQUIRED"),
            Some(ReviewDecision::ReviewRequired)
        );
        assert_eq!(
            ReviewDecision::parse("CHANGES_REQUESTED"),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(
            ReviewDecision::parse("APPROVED"),
            Some(ReviewDecision::Approved)
        );
        assert_eq!(ReviewDecision::parse("bogus"), None);
        assert_eq!(ReviewDecision::parse(""), None);
    }

    // --- pr_number_from_url ---

    #[test]
    fn pr_number_from_standard_url() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42"),
            Some(42)
        );
    }

    #[test]
    fn pr_number_from_url_with_trailing_slash() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42/"),
            Some(42)
        );
    }

    #[test]
    fn pr_number_from_url_with_query_params() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42?diff=split"),
            Some(42)
        );
    }

    #[test]
    fn pr_number_from_url_no_number() {
        assert_eq!(pr_number_from_url("https://github.com/org/repo"), None);
    }

    #[test]
    fn pr_number_from_empty_url() {
        assert_eq!(pr_number_from_url(""), None);
    }

    #[test]
    fn pr_number_from_url_with_fragment() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42#issuecomment-123"),
            Some(42)
        );
    }

    // --- url_label ---

    #[test]
    fn url_label_github_pr() {
        assert_eq!(url_label("https://github.com/org/repo/pull/42"), "PR #42");
    }

    #[test]
    fn url_label_github_pr_no_number() {
        assert_eq!(url_label("https://github.com/org/repo/pull/"), "PR");
    }

    #[test]
    fn url_label_github_pr_with_query() {
        assert_eq!(
            url_label("https://github.com/org/repo/pull/42?diff=split"),
            "PR #42"
        );
    }

    #[test]
    fn url_label_github_pr_with_fragment() {
        assert_eq!(
            url_label("https://github.com/org/repo/pull/42#issuecomment-123"),
            "PR #42"
        );
    }

    #[test]
    fn url_label_github_issue() {
        assert_eq!(
            url_label("https://github.com/org/repo/issues/7"),
            "Issue #7"
        );
    }

    #[test]
    fn url_label_github_issue_no_number() {
        assert_eq!(url_label("https://github.com/org/repo/issues/"), "Issue");
    }

    #[test]
    fn url_label_github_issue_with_fragment() {
        assert_eq!(
            url_label("https://github.com/org/repo/issues/7#issuecomment-999"),
            "Issue #7"
        );
    }

    #[test]
    fn url_label_arbitrary_url() {
        assert_eq!(
            url_label("https://jira.example.com/browse/PROJ-123"),
            "Link"
        );
    }

    #[test]
    fn url_label_empty_string() {
        assert_eq!(url_label(""), "Link");
    }

    // --- github_repo_from_pr_url ---

    #[test]
    fn github_repo_from_standard_url() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn github_repo_from_url_with_query_params() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42?diff=split"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn github_repo_from_url_with_fragment() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42#issuecomment-123"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn github_repo_from_non_github_url() {
        assert_eq!(
            github_repo_from_pr_url("https://gitlab.com/org/repo/pull/42"),
            None
        );
    }

    #[test]
    fn github_repo_from_empty_url() {
        assert_eq!(github_repo_from_pr_url(""), None);
    }

    #[test]
    fn github_repo_from_url_with_trailing_slash() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42/"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn review_decision_db_roundtrip() {
        for decision in ReviewDecision::ALL {
            let s = decision.as_db_str();
            let parsed = ReviewDecision::from_db_str(s)
                .unwrap_or_else(|| panic!("failed to parse db str: {s}"));
            assert_eq!(parsed, decision);
        }
    }

    #[test]
    fn ci_status_db_roundtrip() {
        for status in [
            CiStatus::Pending,
            CiStatus::Success,
            CiStatus::Failure,
            CiStatus::None,
        ] {
            let s = status.as_db_str();
            assert_eq!(CiStatus::from_db_str(s), status, "roundtrip failed for {s}");
        }
    }

    // --- EpicSubstatus ---

    fn test_epic() -> Epic {
        Epic {
            id: EpicId(1),
            title: "Test".to_string(),
            description: "".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: 1,
        }
    }

    fn test_task() -> Task {
        Task {
            id: TaskId(1),
            title: "T".to_string(),
            description: "".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            sub_status: SubStatus::None,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: 1,
        }
    }

    #[test]
    fn epic_substatus_unplanned() {
        let epic = Epic {
            plan_path: None,
            status: TaskStatus::Backlog,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Unplanned);
    }

    #[test]
    fn epic_substatus_planned() {
        let epic = Epic {
            plan_path: Some("plan.md".into()),
            status: TaskStatus::Backlog,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Planned);
    }

    #[test]
    fn epic_substatus_active_with_backlog() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
            Task {
                status: TaskStatus::Backlog,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Active
        );
    }

    #[test]
    fn epic_substatus_active_all_running() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
            Task {
                status: TaskStatus::Done,
                sub_status: SubStatus::None,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Active
        );
    }

    #[test]
    fn epic_substatus_blocked_stale() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Stale,
                ..test_task()
            },
            Task {
                status: TaskStatus::Backlog,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Blocked(1)
        );
    }

    #[test]
    fn epic_substatus_blocked_needs_input() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::NeedsInput,
                ..test_task()
            },
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Blocked(1)
        );
    }

    #[test]
    fn epic_substatus_blocked_count() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::NeedsInput,
                ..test_task()
            },
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Stale,
                ..test_task()
            },
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Blocked(2)
        );
    }

    #[test]
    fn epic_substatus_in_review() {
        let epic = Epic {
            status: TaskStatus::Review,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::InReview);
    }

    #[test]
    fn epic_substatus_wrapping_up() {
        let epic = Epic {
            status: TaskStatus::Review,
            ..test_epic()
        };
        assert_eq!(
            epic_substatus(&epic, &[], Some(EpicId(1))),
            EpicSubstatus::WrappingUp
        );
    }

    #[test]
    fn epic_substatus_done() {
        let epic = Epic {
            status: TaskStatus::Done,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Done);
    }

    // --- DispatchMode ---

    fn make_task_with(plan: Option<&str>, tag: Option<TaskTag>) -> Task {
        let now = chrono::Utc::now();
        Task {
            id: TaskId(1),
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: plan.map(String::from),
            epic_id: None,
            sub_status: SubStatus::None,
            pr_url: None,
            tag,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        }
    }

    #[test]
    fn dispatch_mode_with_plan_always_dispatches() {
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), None)),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Epic))),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Feature))),
            DispatchMode::Dispatch
        );
    }

    // --- descendant_task_ids / descendant_epic_ids ---

    fn epic_with(id: i64, parent: Option<i64>) -> Epic {
        let now = Utc::now();
        Epic {
            id: EpicId(id),
            title: format!("Epic {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: false,
            parent_epic_id: parent.map(EpicId),
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        }
    }

    fn task_under(id: i64, epic: Option<i64>) -> Task {
        let now = Utc::now();
        Task {
            id: TaskId(id),
            title: format!("Task {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: epic.map(EpicId),
            sub_status: SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            created_at: now,
            updated_at: now,
            project_id: 1,
        }
    }

    #[test]
    fn descendant_task_ids_includes_direct_children() {
        let epics = vec![epic_with(1, None)];
        let tasks = vec![task_under(10, Some(1)), task_under(11, None)];
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(!ids.contains(&TaskId(11)));
    }

    #[test]
    fn descendant_task_ids_is_recursive() {
        // root(1) -> mid(2) -> leaf(3)
        let epics = vec![
            epic_with(1, None),
            epic_with(2, Some(1)),
            epic_with(3, Some(2)),
        ];
        let tasks = vec![
            task_under(10, Some(1)),
            task_under(20, Some(2)),
            task_under(30, Some(3)),
        ];
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(ids.contains(&TaskId(20)));
        assert!(ids.contains(&TaskId(30)));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn descendant_task_ids_excludes_sibling_subtree() {
        // root_a(1) with child 10; root_b(2) with child 20
        let epics = vec![epic_with(1, None), epic_with(2, None)];
        let tasks = vec![task_under(10, Some(1)), task_under(20, Some(2))];
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(!ids.contains(&TaskId(20)));
    }

    #[test]
    fn descendant_task_ids_is_cycle_safe() {
        // Malformed: epic 1 points to epic 2, epic 2 points back to epic 1.
        let epics = vec![epic_with(1, Some(2)), epic_with(2, Some(1))];
        let tasks = vec![task_under(10, Some(1)), task_under(20, Some(2))];
        // Must terminate. From root=1, descendants include {1, 2}, so both tasks.
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(ids.contains(&TaskId(20)));
    }

    #[test]
    fn dispatch_mode_without_plan_uses_tag() {
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Epic))),
            DispatchMode::Brainstorm
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Feature))),
            DispatchMode::Plan
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Chore))),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, None)),
            DispatchMode::Dispatch
        );
    }

    mod property_tests {
        use super::*;

        const TASK_STATUSES: &[TaskStatus] = &[
            TaskStatus::Backlog,
            TaskStatus::Running,
            TaskStatus::Review,
            TaskStatus::Done,
            TaskStatus::Archived,
        ];

        const TASK_TAGS: &[TaskTag] = &[
            TaskTag::Bug,
            TaskTag::Feature,
            TaskTag::Chore,
            TaskTag::Epic,
        ];

        fn task_status_strategy() -> impl Strategy<Value = TaskStatus> {
            (0..TASK_STATUSES.len()).prop_map(|i| TASK_STATUSES[i])
        }

        fn task_tag_strategy() -> impl Strategy<Value = TaskTag> {
            (0..TASK_TAGS.len()).prop_map(|i| TASK_TAGS[i])
        }

        fn sub_status_strategy() -> impl Strategy<Value = SubStatus> {
            (0..SubStatus::ALL.len()).prop_map(|i| SubStatus::ALL[i])
        }

        proptest! {
            #[test]
            fn taskstatus_parse_roundtrip(idx in 0..TaskStatus::ALL.len()) {
                let status = TaskStatus::ALL[idx];
                let parsed = TaskStatus::parse(status.as_str());
                prop_assert_eq!(parsed, Some(status));
            }

            #[test]
            fn tasktag_parse_roundtrip(tag in task_tag_strategy()) {
                let parsed = TaskTag::parse(tag.as_str());
                prop_assert_eq!(parsed, Some(tag));
            }

            #[test]
            fn substatus_default_is_valid_for_status(status in task_status_strategy()) {
                let default_ss = SubStatus::default_for(status);
                prop_assert!(
                    default_ss.is_valid_for(status),
                    "default_for({:?}) = {:?} is not valid for that status",
                    status,
                    default_ss
                );
            }

            #[test]
            fn substatus_none_is_only_valid_for_terminal_statuses(ss in sub_status_strategy()) {
                // For Backlog, Done, and Archived only SubStatus::None is valid.
                // Running and Review require a specific active sub-status.
                for &terminal in &[TaskStatus::Backlog, TaskStatus::Done, TaskStatus::Archived] {
                    let valid = ss.is_valid_for(terminal);
                    let expected = matches!(ss, SubStatus::None);
                    prop_assert_eq!(valid, expected);
                }
            }

            #[test]
            fn substatus_column_priority_never_panics(ss in sub_status_strategy()) {
                // column_priority() is a pure exhaustive match — just confirm it always
                // returns a value for every variant.
                let _ = ss.column_priority();
            }
        }
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn alert_severity_column_count() {
        assert_eq!(AlertSeverity::COLUMN_COUNT, 4);
        assert_eq!(AlertSeverity::ALL.len(), 4);
    }

    #[test]
    fn alert_severity_column_index_roundtrip() {
        for (i, severity) in AlertSeverity::ALL.iter().enumerate() {
            assert_eq!(severity.column_index(), i);
            assert_eq!(AlertSeverity::from_column_index(i), Some(*severity));
        }
    }

    #[test]
    fn alert_severity_from_column_index_out_of_range() {
        assert_eq!(AlertSeverity::from_column_index(4), None);
        assert_eq!(AlertSeverity::from_column_index(999), None);
    }

    #[test]
    fn alert_severity_db_roundtrip() {
        for severity in AlertSeverity::ALL {
            let s = severity.as_db_str();
            let parsed =
                AlertSeverity::from_db_str(s).unwrap_or_else(|| panic!("roundtrip failed: {s}"));
            assert_eq!(parsed, severity);
        }
    }

    #[test]
    fn alert_severity_parse() {
        assert_eq!(
            AlertSeverity::parse("critical"),
            Some(AlertSeverity::Critical)
        );
        assert_eq!(AlertSeverity::parse("HIGH"), Some(AlertSeverity::High));
        assert_eq!(
            AlertSeverity::parse("moderate"),
            Some(AlertSeverity::Medium)
        );
        assert_eq!(AlertSeverity::parse("Medium"), Some(AlertSeverity::Medium));
        assert_eq!(AlertSeverity::parse("low"), Some(AlertSeverity::Low));
        assert_eq!(AlertSeverity::parse("bogus"), None);
    }

    #[test]
    fn alert_kind_db_roundtrip() {
        for kind in [AlertKind::Dependabot, AlertKind::CodeScanning] {
            let s = kind.as_db_str();
            let parsed =
                AlertKind::from_db_str(s).unwrap_or_else(|| panic!("roundtrip failed: {s}"));
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn alert_kind_indicator() {
        assert_eq!(AlertKind::Dependabot.indicator(), "D");
        assert_eq!(AlertKind::CodeScanning.indicator(), "S");
    }

    #[test]
    fn alert_kind_as_str() {
        assert_eq!(AlertKind::Dependabot.as_str(), "Dependabot");
        assert_eq!(AlertKind::CodeScanning.as_str(), "Code Scanning");
    }

    #[test]
    fn alert_severity_as_str() {
        assert_eq!(AlertSeverity::Critical.as_str(), "Critical");
        assert_eq!(AlertSeverity::High.as_str(), "High");
        assert_eq!(AlertSeverity::Medium.as_str(), "Medium");
        assert_eq!(AlertSeverity::Low.as_str(), "Low");
    }

    #[test]
    fn review_agent_status_db_roundtrip() {
        for status in [
            ReviewAgentStatus::Reviewing,
            ReviewAgentStatus::FindingsReady,
            ReviewAgentStatus::Idle,
        ] {
            let s = status.as_db_str();
            let parsed = ReviewAgentStatus::from_db_str(s)
                .unwrap_or_else(|| panic!("roundtrip failed for: {s}"));
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn review_agent_status_from_db_str_invalid() {
        assert_eq!(ReviewAgentStatus::from_db_str("bogus"), None);
        assert_eq!(ReviewAgentStatus::from_db_str(""), None);
    }

    #[test]
    fn review_agent_status_display() {
        assert_eq!(ReviewAgentStatus::Reviewing.to_string(), "reviewing");
        assert_eq!(ReviewAgentStatus::FindingsReady.to_string(), "ready");
        assert_eq!(ReviewAgentStatus::Idle.to_string(), "idle");
    }

    // --- ReviewWorkflowState + ReviewWorkflowSubState ---

    #[test]
    fn review_workflow_state_roundtrip() {
        use ReviewWorkflowState::*;
        for (s, expected) in [
            (Backlog, "backlog"),
            (Ongoing, "ongoing"),
            (ActionRequired, "action_required"),
            (Done, "done"),
        ] {
            assert_eq!(s.as_db_str(), expected);
            assert_eq!(ReviewWorkflowState::from_db_str(expected), Some(s));
        }
        assert_eq!(ReviewWorkflowState::from_db_str("bogus"), None);
    }

    #[test]
    fn review_workflow_sub_state_roundtrip() {
        use ReviewWorkflowSubState::*;
        for s in [
            Reviewing,
            Idle,
            Stale,
            FindingsReady,
            ChangesRequested,
            AwaitingResponse,
            CiFailing,
            ReadyToMerge,
        ] {
            let db_str = s.as_db_str();
            assert_eq!(ReviewWorkflowSubState::from_db_str(db_str), Some(s));
        }
    }

    #[test]
    fn review_workflow_state_column_index_is_sequential() {
        use ReviewWorkflowState::*;
        assert_eq!(Backlog.column_index(), 0);
        assert_eq!(Ongoing.column_index(), 1);
        assert_eq!(ActionRequired.column_index(), 2);
        assert_eq!(Done.column_index(), 3);
    }

    #[test]
    fn review_workflow_state_column_count_matches_all() {
        use ReviewWorkflowState::*;
        assert_eq!(ReviewWorkflowState::COLUMN_COUNT, 4);
        assert_eq!(ReviewWorkflowState::ALL.len(), 4);
        assert_eq!(
            ReviewWorkflowState::COLUMN_COUNT,
            ReviewWorkflowState::ALL.len()
        );
        // Verify that the 4 variants are in the ALL array
        let all_states = ReviewWorkflowState::ALL.to_vec();
        assert!(all_states.contains(&Backlog));
        assert!(all_states.contains(&Ongoing));
        assert!(all_states.contains(&ActionRequired));
        assert!(all_states.contains(&Done));
    }

    #[test]
    fn security_workflow_state_column_count_matches_all() {
        use SecurityWorkflowState::*;
        assert_eq!(SecurityWorkflowState::COLUMN_COUNT, 4);
        assert_eq!(SecurityWorkflowState::ALL.len(), 4);
        assert_eq!(
            SecurityWorkflowState::COLUMN_COUNT,
            SecurityWorkflowState::ALL.len()
        );
        // Verify that the 4 variants are in the ALL array
        let all_states = SecurityWorkflowState::ALL.to_vec();
        assert!(all_states.contains(&Backlog));
        assert!(all_states.contains(&Ongoing));
        assert!(all_states.contains(&ActionRequired));
        assert!(all_states.contains(&Done));
    }

    // --- SecurityWorkflowState + SecurityWorkflowSubState ---

    #[test]
    fn security_workflow_state_roundtrip() {
        use SecurityWorkflowState::*;
        for (s, expected) in [
            (Backlog, "backlog"),
            (Ongoing, "ongoing"),
            (ActionRequired, "action_required"),
            (Done, "done"),
        ] {
            assert_eq!(s.as_db_str(), expected);
            assert_eq!(SecurityWorkflowState::from_db_str(expected), Some(s));
        }
    }

    #[test]
    fn security_workflow_sub_state_roundtrip() {
        use SecurityWorkflowSubState::*;
        for s in [
            Investigating,
            Idle,
            Stale,
            FindingsReady,
            NeedsManualFix,
            PrOpen,
            ChangesRequested,
            CiFailing,
            ReadyToMerge,
        ] {
            let db_str = s.as_db_str();
            assert_eq!(SecurityWorkflowSubState::from_db_str(db_str), Some(s));
        }
    }

    // --- WorkflowItemKind ---

    #[test]
    fn workflow_item_kind_roundtrip() {
        use WorkflowItemKind::*;
        for (k, expected) in [
            (ReviewerPr, "reviewer_pr"),
            (DependabotPr, "dependabot_pr"),
            (DependabotAlert, "dependabot_alert"),
            (CodeScanAlert, "code_scan_alert"),
        ] {
            assert_eq!(k.as_db_str(), expected);
            assert_eq!(WorkflowItemKind::from_db_str(expected), Some(k));
        }
    }

    // --- PrRef ---

    #[test]
    fn pr_ref_new_and_accessors() {
        let pr = PrRef::new("org/repo".to_string(), 42);
        assert_eq!(pr.repo(), "org/repo");
        assert_eq!(pr.number(), 42);
    }

    #[test]
    fn pr_ref_equality() {
        let a = PrRef::new("org/repo".to_string(), 1);
        let b = PrRef::new("org/repo".to_string(), 1);
        let c = PrRef::new("org/other".to_string(), 1);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn pr_ref_hash_works_in_hashmap() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        let pr = PrRef::new("org/repo".to_string(), 42);
        map.insert(pr.clone(), "value");
        assert_eq!(
            map.get(&PrRef::new("org/repo".to_string(), 42)),
            Some(&"value")
        );
    }

    #[test]
    fn pr_ref_display() {
        let pr = PrRef::new("org/repo".to_string(), 42);
        assert_eq!(pr.to_string(), "org/repo#42");
    }

    #[test]
    fn default_base_branch_is_main() {
        assert_eq!(DEFAULT_BASE_BRANCH, "main");
    }
}

// ---------------------------------------------------------------------------
// Learning domain types
// ---------------------------------------------------------------------------

pub type LearningId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningKind {
    Pitfall,
    Convention,
    Preference,
    ToolRecommendation,
    Procedural,
    Episodic,
}

impl LearningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LearningKind::Pitfall => "pitfall",
            LearningKind::Convention => "convention",
            LearningKind::Preference => "preference",
            LearningKind::ToolRecommendation => "tool_recommendation",
            LearningKind::Procedural => "procedural",
            LearningKind::Episodic => "episodic",
        }
    }

    pub fn display_label(self) -> &'static str {
        match self {
            LearningKind::Pitfall => "Pitfall",
            LearningKind::Convention => "Convention",
            LearningKind::Preference => "Preference",
            LearningKind::ToolRecommendation => "Tool recommendation",
            LearningKind::Procedural => "Procedural",
            LearningKind::Episodic => "Episodic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pitfall" => Some(LearningKind::Pitfall),
            "convention" => Some(LearningKind::Convention),
            "preference" => Some(LearningKind::Preference),
            "tool_recommendation" => Some(LearningKind::ToolRecommendation),
            "procedural" => Some(LearningKind::Procedural),
            "episodic" => Some(LearningKind::Episodic),
            _ => None,
        }
    }
}

impl std::fmt::Display for LearningKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for LearningKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown learning kind: {s}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningScope {
    User,
    Project,
    Repo,
    Epic,
    Task,
}

impl LearningScope {
    pub fn as_str(self) -> &'static str {
        match self {
            LearningScope::User => "user",
            LearningScope::Project => "project",
            LearningScope::Repo => "repo",
            LearningScope::Epic => "epic",
            LearningScope::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(LearningScope::User),
            "project" => Some(LearningScope::Project),
            "repo" => Some(LearningScope::Repo),
            "epic" => Some(LearningScope::Epic),
            "task" => Some(LearningScope::Task),
            _ => None,
        }
    }
}

impl std::fmt::Display for LearningScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for LearningScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown learning scope: {s}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningStatus {
    Proposed,
    Approved,
    Rejected,
    Archived,
}

impl LearningStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LearningStatus::Proposed => "proposed",
            LearningStatus::Approved => "approved",
            LearningStatus::Rejected => "rejected",
            LearningStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "proposed" => Some(LearningStatus::Proposed),
            "approved" => Some(LearningStatus::Approved),
            "rejected" => Some(LearningStatus::Rejected),
            "archived" => Some(LearningStatus::Archived),
            _ => None,
        }
    }

    /// Returns true if this status is terminal (no further transitions allowed).
    pub fn is_terminal(self) -> bool {
        matches!(self, LearningStatus::Rejected | LearningStatus::Archived)
    }
}

impl std::fmt::Display for LearningStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for LearningStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown learning status: {s}"))
    }
}

#[derive(Debug, Clone)]
pub struct Learning {
    pub id: LearningId,
    pub kind: LearningKind,
    pub summary: String,
    pub detail: Option<String>,
    pub scope: LearningScope,
    pub scope_ref: Option<String>,
    pub tags: Vec<String>,
    pub status: LearningStatus,
    pub source_task_id: Option<TaskId>,
    pub confirmed_count: i64,
    pub last_confirmed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
