use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EpicId, ProjectId};

define_id_newtype!(TaskId, task_id_tests);

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
            TaskStatus::Archived => TaskStatus::COLUMN_COUNT,
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
    /// Free-form badges rendered on the kanban card alongside derived
    /// indicators. Order is preserved so feed scripts can control rendering
    /// order.
    pub labels: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_pre_tool_use_at: Option<DateTime<Utc>>,
    pub last_notification_at: Option<DateTime<Utc>>,
    pub wrap_up_mode: Option<WrapUpMode>,
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
    /// Required: feed scripts must declare which TaskTag the inserted task
    /// receives, so dispatch routes feed-derived tasks to the correct agent
    /// (e.g. `pr-review` for Dependabot PRs, `fix` for security alerts).
    pub tag: TaskTag,
    /// Free-form labels copied to `Task.labels` on insert and on conflict.
    /// `#[serde(default)]` keeps wire compatibility with scripts written
    /// before this field existed.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Ordering hint copied to `Task.sort_order` (lower sorts first). Used
    /// by the CVE feed to surface CRITICAL alerts above HIGH/MEDIUM/LOW.
    #[serde(default)]
    pub sort_order: Option<i64>,
}

// ---------------------------------------------------------------------------
// DispatchMode
// ---------------------------------------------------------------------------

/// Determines how a backlog task should be dispatched. Most tasks route to
/// `Dispatch`, which produces the unified prompt skeleton (with-plan or
/// no-plan variant). The `research` tag is the only one with a dedicated
/// agent — its prompt keeps the agent in read-only mode while it presents
/// findings to the user. Other tags (`pr_review`, `fix`, `dependabot`) are
/// kanban labels and route through the unified `Dispatch` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    Dispatch,
    Research,
}

impl DispatchMode {
    pub fn label(self) -> &'static str {
        match self {
            DispatchMode::Dispatch => "Dispatch",
            DispatchMode::Research => "Research",
        }
    }

    /// Select the dispatch mode for a task: tasks with a plan always go
    /// through the unified `Dispatch` path; otherwise only the `research`
    /// tag routes to its dedicated agent.
    pub fn for_task(task: &Task) -> Self {
        if task.plan_path.is_some() {
            DispatchMode::Dispatch
        } else {
            match task.tag {
                Some(TaskTag::Research) => DispatchMode::Research,
                _ => DispatchMode::Dispatch,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TaskTag
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskTag {
    Bug,
    Feature,
    Chore,
    #[serde(rename = "pr-review")]
    PrReview,
    Research,
    Fix,
    Dependabot,
}

impl TaskTag {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskTag::Bug => "bug",
            TaskTag::Feature => "feature",
            TaskTag::Chore => "chore",
            TaskTag::PrReview => "pr-review",
            TaskTag::Research => "research",
            TaskTag::Fix => "fix",
            TaskTag::Dependabot => "dependabot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bug" => Some(TaskTag::Bug),
            "feature" => Some(TaskTag::Feature),
            "chore" => Some(TaskTag::Chore),
            "pr-review" => Some(TaskTag::PrReview),
            "research" => Some(TaskTag::Research),
            "fix" => Some(TaskTag::Fix),
            "dependabot" => Some(TaskTag::Dependabot),
            _ => None,
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            TaskTag::Bug => "bug",
            TaskTag::Feature => "feat",
            TaskTag::Chore => "chore",
            TaskTag::PrReview => "pr-rev",
            TaskTag::Research => "research",
            TaskTag::Fix => "fix",
            TaskTag::Dependabot => "dep",
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
// WrapUpMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WrapUpMode {
    Rebase,
    Pr,
    Done,
}

impl WrapUpMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WrapUpMode::Rebase => "rebase",
            WrapUpMode::Pr => "pr",
            WrapUpMode::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rebase" => Some(WrapUpMode::Rebase),
            "pr" => Some(WrapUpMode::Pr),
            "done" => Some(WrapUpMode::Done),
            _ => None,
        }
    }
}

impl std::fmt::Display for WrapUpMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WrapUpMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown wrap-up mode: {s}"))
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
    Fresh,
    Aging,
    Stale,
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

/// A Claude Code hook event kind reported via the `dispatch hook` CLI.
///
/// Each event kind drives a different side effect on a Running task; non-Running
/// tasks ignore hook events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEventKind {
    PreToolUse,
    Notification,
    Stop,
}

impl HookEventKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pre_tool_use" => Some(Self::PreToolUse),
            "notification" => Some(Self::Notification),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "pre_tool_use",
            Self::Notification => "notification",
            Self::Stop => "stop",
        }
    }
}

/// Time without a PreToolUse event before a running agent is considered Stale.
pub const ACTIVE_THRESHOLD: chrono::Duration = chrono::Duration::minutes(10);

/// Live activity classification for a running agent, derived from hook event
/// timestamps. Distinct from the wallclock `Staleness` enum (which colors card
/// ages across all statuses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivity {
    Active,
    Waiting,
    Stale,
}

impl AgentActivity {
    /// Map the classifier output to the visible `SubStatus` for a Running task.
    pub fn to_sub_status(self) -> SubStatus {
        match self {
            AgentActivity::Active => SubStatus::Active,
            AgentActivity::Waiting => SubStatus::NeedsInput,
            AgentActivity::Stale => SubStatus::Stale,
        }
    }
}

/// Classify a running agent's activity from its hook event timestamps.
pub fn classify_agent_activity(
    last_pre_tool_use_at: Option<chrono::DateTime<chrono::Utc>>,
    last_notification_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> AgentActivity {
    if let Some(notif) = last_notification_at {
        let notif_is_newer = last_pre_tool_use_at.is_none_or(|p| notif > p);
        if notif_is_newer {
            return AgentActivity::Waiting;
        }
    }
    match last_pre_tool_use_at {
        Some(ts) if now.signed_duration_since(ts) <= ACTIVE_THRESHOLD => AgentActivity::Active,
        _ => AgentActivity::Stale,
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn at(min_ago: i64, now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
        now - Duration::minutes(min_ago)
    }

    #[test]
    fn no_events_classifies_stale() {
        let now = Utc::now();
        assert_eq!(
            classify_agent_activity(None, None, now),
            AgentActivity::Stale
        );
    }

    #[test]
    fn recent_pre_tool_use_classifies_active() {
        let now = Utc::now();
        assert_eq!(
            classify_agent_activity(Some(at(1, now)), None, now),
            AgentActivity::Active
        );
    }

    #[test]
    fn old_pre_tool_use_classifies_stale() {
        let now = Utc::now();
        assert_eq!(
            classify_agent_activity(Some(at(10, now)), None, now),
            AgentActivity::Stale
        );
    }

    #[test]
    fn notification_after_pre_tool_use_classifies_waiting() {
        let now = Utc::now();
        assert_eq!(
            classify_agent_activity(Some(at(5, now)), Some(at(1, now)), now),
            AgentActivity::Waiting
        );
    }

    #[test]
    fn pre_tool_use_after_notification_classifies_active() {
        let now = Utc::now();
        assert_eq!(
            classify_agent_activity(Some(at(1, now)), Some(at(5, now)), now),
            AgentActivity::Active
        );
    }

    #[test]
    fn notification_only_classifies_waiting() {
        let now = Utc::now();
        assert_eq!(
            classify_agent_activity(None, Some(at(1, now)), now),
            AgentActivity::Waiting
        );
    }

    #[test]
    fn boundary_exactly_at_threshold_classifies_active() {
        let now = Utc::now();
        let exactly = now - ACTIVE_THRESHOLD;
        assert_eq!(
            classify_agent_activity(Some(exactly), None, now),
            AgentActivity::Active
        );
    }

    #[test]
    fn just_past_threshold_classifies_stale() {
        let now = Utc::now();
        let past = now - ACTIVE_THRESHOLD - Duration::seconds(1);
        assert_eq!(
            classify_agent_activity(Some(past), None, now),
            AgentActivity::Stale
        );
    }
}

#[cfg(test)]
mod wrap_up_mode_tests {
    use super::*;

    #[test]
    fn wrap_up_mode_roundtrip() {
        for mode in [WrapUpMode::Rebase, WrapUpMode::Pr, WrapUpMode::Done] {
            let s = mode.as_str();
            let parsed = WrapUpMode::parse(s).expect("parse should succeed");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn wrap_up_mode_from_str() {
        assert_eq!("rebase".parse::<WrapUpMode>().unwrap(), WrapUpMode::Rebase);
        assert_eq!("pr".parse::<WrapUpMode>().unwrap(), WrapUpMode::Pr);
        assert_eq!("done".parse::<WrapUpMode>().unwrap(), WrapUpMode::Done);
        assert!("unknown".parse::<WrapUpMode>().is_err());
    }

    #[test]
    fn wrap_up_mode_display() {
        assert_eq!(WrapUpMode::Rebase.to_string(), "rebase");
        assert_eq!(WrapUpMode::Pr.to_string(), "pr");
        assert_eq!(WrapUpMode::Done.to_string(), "done");
    }
}
