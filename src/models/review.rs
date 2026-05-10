use chrono::{DateTime, Utc};
use serde::Deserialize;

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
        let all_states = ReviewWorkflowState::ALL.to_vec();
        assert!(all_states.contains(&Backlog));
        assert!(all_states.contains(&Ongoing));
        assert!(all_states.contains(&ActionRequired));
        assert!(all_states.contains(&Done));
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
}
