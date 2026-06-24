use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EpicId, UrlType};
use crate::define_id_newtype;

define_id_newtype!(TaskId, task_id_tests);

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
// BranchName
// ---------------------------------------------------------------------------

/// A validated git branch name. Wraps a `String` and provides a type-safe
/// boundary between branch-name arguments and other stringly-typed fields
/// such as `worktree` or `repo_path`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BranchName(pub String);

impl BranchName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for BranchName {
    fn default() -> Self {
        BranchName(DEFAULT_BASE_BRANCH.to_string())
    }
}

impl std::fmt::Display for BranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for BranchName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for BranchName {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<String> for BranchName {
    fn from(s: String) -> Self {
        BranchName(s)
    }
}

impl From<&str> for BranchName {
    fn from(s: &str) -> Self {
        BranchName(s.to_string())
    }
}

impl std::str::FromStr for BranchName {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(BranchName(s.to_string()))
    }
}

impl rusqlite::types::FromSql for BranchName {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        String::column_result(value).map(BranchName)
    }
}

impl PartialEq<str> for BranchName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for BranchName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for BranchName {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

pub const DEFAULT_QUICK_TASK_TITLE: &str = "Quick task";
pub const DEFAULT_BASE_BRANCH: &str = "main";

#[derive(Debug, Clone, PartialEq)]
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
    pub url: Option<crate::models::TaskUrl>,
    pub tag: Option<TaskTag>,
    pub sort_order: Option<i64>,
    pub base_branch: BranchName,
    pub external_id: Option<String>,
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
    /// Optional explicit type for `url`. When set, the inserted task's
    /// url_type is taken verbatim; when absent it is inferred from the URL
    /// string. Lets a feed declare types inference cannot reach (e.g.
    /// `security_alert` for Dependabot alert URLs). `#[serde(default)]`
    /// keeps wire compatibility with scripts written before this field
    /// existed. Ignored when `url` is empty.
    #[serde(default)]
    pub url_type: Option<UrlType>,
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
    /// Routing signals attached by the feed script (e.g. `direct-request`,
    /// `author-bot`). Used by later WPs to route PR items into the right
    /// feed bucket. Unrecognised values are dropped with a warning rather
    /// than failing the whole item: signals are additive routing metadata,
    /// so a value introduced by a newer feed script must not break ingest
    /// on an older binary. This is a deliberate, scoped exception to the
    /// "parse failures must surface" boundary rule in docs/conventions.md —
    /// a single unknown signal should not poison an otherwise-valid item.
    #[serde(default, deserialize_with = "deserialize_lenient_signals")]
    pub signals: Vec<Signal>,
}

// ---------------------------------------------------------------------------
// Signal — routing hints a feed script attaches to a FeedItem
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Signal {
    DirectRequest,
    TeamRequest,
    Reviewed,
    Commented,
    AuthorBot,
    AuthorMe,
}

/// Deserialize `FeedItem.signals`, dropping any entry that is not a recognised
/// `Signal` (logging each at `warn`). See the field doc for why this is lenient
/// rather than surfacing the error like the rest of the feed-JSON boundary.
fn deserialize_lenient_signals<'de, D>(deserializer: D) -> Result<Vec<Signal>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let mut signals = Vec::with_capacity(raw.len());
    for value in raw {
        // Deserialize from a borrow so `value` stays available for the warn.
        match Signal::deserialize(&value) {
            Ok(sig) => signals.push(sig),
            Err(_) => tracing::warn!(value = %value, "dropping unrecognised feed signal"),
        }
    }
    Ok(signals)
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

    /// Whether this tag routes to a read-only PR-review agent (PR review or
    /// Dependabot). Review tasks skip the plan/implement flow and, when they
    /// carry a PR URL, base their worktree on the PR's branch.
    pub fn is_review(&self) -> bool {
        matches!(self, TaskTag::PrReview | TaskTag::Dependabot)
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
    /// Refreshes `last_pre_tool_use_at`. Covers both the Claude Code
    /// `PreToolUse` and `PostToolUse` hook events — the shell hook
    /// (`task-status-hook`) maps both to `pre_tool_use` so the Rust side
    /// sees a single activity signal regardless of which fired.
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
mod branch_name_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn default_is_main() {
        assert_eq!(BranchName::default().as_str(), DEFAULT_BASE_BRANCH);
    }

    #[test]
    fn from_str_ref() {
        let b = BranchName::from("develop");
        assert_eq!(b.as_str(), "develop");
    }

    #[test]
    fn from_string() {
        let b = BranchName::from("feature-x".to_string());
        assert_eq!(b.as_str(), "feature-x");
    }

    #[test]
    fn display() {
        assert_eq!(BranchName::from("main").to_string(), "main");
    }

    #[test]
    fn clone_and_eq() {
        let a = BranchName::from("main");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn deref_to_str() {
        let b = BranchName::from("main");
        let s: &str = &b;
        assert_eq!(s, "main");
    }

    #[test]
    fn as_ref_str() {
        let b = BranchName::from("staging");
        let s: &str = b.as_ref();
        assert_eq!(s, "staging");
    }

    #[test]
    fn from_str_parse() {
        let b: BranchName = "release/v2".parse().unwrap();
        assert_eq!(b.as_str(), "release/v2");
    }

    #[test]
    fn serde_roundtrip() {
        let b = BranchName::from("main");
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, "\"main\"");
        let back: BranchName = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
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
        let past = now - ACTIVE_THRESHOLD - Duration::seconds(1);
        assert_eq!(
            classify_agent_activity(Some(past), None, now),
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod model_tests {
    use super::*;
    use chrono::Utc;

    // --- Signal / FeedItem.signals ---

    #[test]
    fn signal_deserializes_kebab_case() {
        let s: Vec<Signal> = serde_json::from_str(r#"["direct-request","author-bot"]"#).unwrap();
        assert_eq!(s, vec![Signal::DirectRequest, Signal::AuthorBot]);
    }

    #[test]
    fn feed_item_signals_default_empty_and_unknown_skipped() {
        // missing field -> empty
        let item: FeedItem = serde_json::from_str(
            r#"{"external_id":"x","title":"t","description":"","status":"backlog","tag":"pr-review"}"#,
        )
        .unwrap();
        assert!(item.signals.is_empty());
        // unknown signal value is dropped, not fatal
        let item2: FeedItem = serde_json::from_str(
            r#"{"external_id":"x","title":"t","description":"","status":"backlog","tag":"pr-review","signals":["reviewed","bogus"]}"#,
        )
        .unwrap();
        assert_eq!(item2.signals, vec![Signal::Reviewed]);
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

    // --- DispatchMode / TaskTag ---

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
            url: None,
            tag,
            sort_order: None,
            base_branch: "main".into(),
            external_id: None,
            labels: Vec::new(),
            created_at: now,
            updated_at: now,
            last_pre_tool_use_at: None,
            last_notification_at: None,
            wrap_up_mode: None,
        }
    }

    #[test]
    fn dispatch_mode_with_plan_always_dispatches() {
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), None)),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Feature))),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::PrReview))),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Research))),
            DispatchMode::Dispatch
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Fix))),
            DispatchMode::Dispatch
        );
    }

    #[test]
    fn task_tag_parse_roundtrip_new_tags() {
        for (tag, expected_str, expected_short) in [
            (TaskTag::PrReview, "pr-review", "pr-rev"),
            (TaskTag::Research, "research", "research"),
            (TaskTag::Fix, "fix", "fix"),
        ] {
            assert_eq!(tag.as_str(), expected_str, "as_str mismatch for {tag:?}");
            assert_eq!(
                TaskTag::parse(expected_str),
                Some(tag),
                "parse mismatch for {expected_str}"
            );
            assert_eq!(
                tag.short_label(),
                expected_short,
                "short_label mismatch for {tag:?}"
            );
            assert_eq!(
                tag.to_string(),
                expected_str,
                "Display mismatch for {tag:?}"
            );
            assert_eq!(
                expected_str.parse::<TaskTag>().unwrap(),
                tag,
                "FromStr mismatch for {expected_str}"
            );
        }
    }

    #[test]
    fn task_tag_is_review_only_for_pr_review_and_dependabot() {
        assert!(TaskTag::PrReview.is_review());
        assert!(TaskTag::Dependabot.is_review());
        for tag in [
            TaskTag::Bug,
            TaskTag::Feature,
            TaskTag::Chore,
            TaskTag::Research,
            TaskTag::Fix,
        ] {
            assert!(!tag.is_review(), "{tag:?} should not be a review tag");
        }
    }

    #[test]
    fn dispatch_mode_without_plan_routes_only_research() {
        for tag in [
            None,
            Some(TaskTag::Feature),
            Some(TaskTag::Bug),
            Some(TaskTag::Chore),
            Some(TaskTag::PrReview),
            Some(TaskTag::Fix),
            Some(TaskTag::Dependabot),
        ] {
            assert_eq!(
                DispatchMode::for_task(&make_task_with(None, tag)),
                DispatchMode::Dispatch,
                "tag {tag:?} should fall through to Dispatch"
            );
        }
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Research))),
            DispatchMode::Research
        );
    }

    #[test]
    fn task_tag_dependabot_serde_roundtrip() {
        let tag = TaskTag::Dependabot;
        let s = serde_json::to_string(&tag).unwrap();
        assert_eq!(s, "\"dependabot\"");
        let back: TaskTag = serde_json::from_str(&s).unwrap();
        assert_eq!(back, TaskTag::Dependabot);
    }

    #[test]
    fn task_tag_dependabot_parse_and_labels() {
        assert_eq!(TaskTag::parse("dependabot"), Some(TaskTag::Dependabot));
        assert_eq!(TaskTag::Dependabot.as_str(), "dependabot");
        assert_eq!(TaskTag::Dependabot.short_label(), "dep");
    }

    #[test]
    fn default_base_branch_is_main() {
        assert_eq!(DEFAULT_BASE_BRANCH, "main");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

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
        TaskTag::PrReview,
        TaskTag::Research,
        TaskTag::Fix,
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
        fn slugify_never_panics(input in "\\PC{0,2000}") {
            // slugify should never panic on arbitrary input
            let _ = slugify(&input);
        }

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
