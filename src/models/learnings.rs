use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::TaskId;
use crate::define_id_newtype;

define_id_newtype!(LearningId, learning_id_tests);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningKind {
    Pitfall,
    Convention,
    Preference,
    ToolRecommendation,
    Procedural,
    Landscape,
}

impl LearningKind {
    pub const ALL: &'static [LearningKind] = &[
        LearningKind::Pitfall,
        LearningKind::Convention,
        LearningKind::Preference,
        LearningKind::ToolRecommendation,
        LearningKind::Procedural,
        LearningKind::Landscape,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LearningKind::Pitfall => "pitfall",
            LearningKind::Convention => "convention",
            LearningKind::Preference => "preference",
            LearningKind::ToolRecommendation => "tool_recommendation",
            LearningKind::Procedural => "procedural",
            LearningKind::Landscape => "landscape",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pitfall" => Some(LearningKind::Pitfall),
            "convention" => Some(LearningKind::Convention),
            "preference" => Some(LearningKind::Preference),
            "tool_recommendation" => Some(LearningKind::ToolRecommendation),
            "procedural" => Some(LearningKind::Procedural),
            "landscape" => Some(LearningKind::Landscape),
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
    Repo,
    Epic,
    Task,
}

impl LearningScope {
    pub const ALL: &'static [LearningScope] = &[
        LearningScope::User,
        LearningScope::Repo,
        LearningScope::Epic,
        LearningScope::Task,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LearningScope::User => "user",
            LearningScope::Repo => "repo",
            LearningScope::Epic => "epic",
            LearningScope::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(LearningScope::User),
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
    Approved,
    Rejected,
    Archived,
}

impl LearningStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LearningStatus::Approved => "approved",
            LearningStatus::Rejected => "rejected",
            LearningStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "approved" => Ok(LearningStatus::Approved),
            "rejected" => Ok(LearningStatus::Rejected),
            "archived" => Ok(LearningStatus::Archived),
            other => Err(format!("unknown learning status: {other}")),
        }
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
        Self::parse(s)
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
    pub upvote_count: i64,
    pub last_upvoted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The store's workspace id; `0` for none, which makes the learning
    /// private to its author (`workspaces.allium: LearningVisibility`).
    pub workspace_id: i64,
    /// Who recorded it, in the store's identity spelling; empty on a learning
    /// from before workspaces.
    pub author: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningVerdict {
    Helped,
    Wrong,
    /// Read, and did not fit this task. Counts 0: a signal for measuring the
    /// store, not a ranking input (`learnings.allium: RateLearningViaMcp`).
    NotApplicable,
}

impl LearningVerdict {
    pub const ALL: &'static [LearningVerdict] = &[
        LearningVerdict::Helped,
        LearningVerdict::Wrong,
        LearningVerdict::NotApplicable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LearningVerdict::Helped => "helped",
            LearningVerdict::Wrong => "wrong",
            LearningVerdict::NotApplicable => "not_applicable",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "helped" => Ok(LearningVerdict::Helped),
            "wrong" => Ok(LearningVerdict::Wrong),
            "not_applicable" => Ok(LearningVerdict::NotApplicable),
            other => Err(format!("unknown verdict: {other}")),
        }
    }
}

impl std::fmt::Display for LearningVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for LearningVerdict {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    PromptInjection,
    /// Legacy — no longer produced; kept for existing DB records.
    Procedural,
    QueryLearnings,
}

impl RetrievalSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RetrievalSource::PromptInjection => "prompt_injection",
            RetrievalSource::Procedural => "procedural",
            RetrievalSource::QueryLearnings => "query_learnings",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "prompt_injection" => Ok(RetrievalSource::PromptInjection),
            "procedural" => Ok(RetrievalSource::Procedural),
            "query_learnings" => Ok(RetrievalSource::QueryLearnings),
            other => Err(format!("unknown retrieval source: {other}")),
        }
    }
}

impl std::fmt::Display for RetrievalSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for RetrievalSource {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone)]
pub struct LearningRetrieval {
    pub id: i64,
    pub task_id: TaskId,
    pub learning_id: LearningId,
    pub source: RetrievalSource,
    pub retrieved_at: DateTime<Utc>,
}

impl Learning {
    /// Whether `reader` sees this learning (`workspaces.allium:
    /// LearningVisibility`): a learning with a workspace goes to that
    /// workspace's members, one with none to its author alone, and one from
    /// before workspaces (no workspace, no author) to everyone. Whether an
    /// epic- or task-scoped learning's item is visible is the store's to
    /// decide; a retrieval only reaches epic learnings of the reader's own
    /// epic and never task-scoped ones.
    pub fn seen_by(&self, reader: Option<&str>, reader_workspaces: &[i64]) -> bool {
        if self.workspace_id != 0 {
            return reader_workspaces.contains(&self.workspace_id);
        }
        self.author.is_empty() || reader == Some(self.author.as_str())
    }
}

#[cfg(test)]
mod seen_by_tests {
    use super::*;
    use chrono::TimeZone;

    fn learning(workspace_id: i64, author: &str) -> Learning {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Learning {
            id: LearningId(1),
            kind: LearningKind::Pitfall,
            summary: "s".into(),
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo".into()),
            tags: vec![],
            status: LearningStatus::Approved,
            source_task_id: None,
            upvote_count: 0,
            last_upvoted_at: None,
            created_at: at,
            updated_at: at,
            workspace_id,
            author: author.into(),
        }
    }

    #[test]
    fn a_workspace_learning_goes_to_its_members() {
        let l = learning(7, "alice");
        assert!(l.seen_by(Some("bob"), &[7]));
        assert!(!l.seen_by(Some("bob"), &[8]));
        assert!(!l.seen_by(None, &[]));
    }

    #[test]
    fn a_learning_with_no_workspace_goes_to_its_author_alone() {
        let l = learning(0, "alice");
        assert!(l.seen_by(Some("alice"), &[]));
        assert!(!l.seen_by(Some("bob"), &[7]));
        assert!(!l.seen_by(None, &[]));
    }

    #[test]
    fn a_learning_from_before_workspaces_goes_to_everyone() {
        let l = learning(0, "");
        assert!(l.seen_by(Some("bob"), &[]));
        assert!(l.seen_by(None, &[]));
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    /// `LearningKind::ALL`/`LearningScope::ALL`/`LearningVerdict::ALL` back
    /// record_learning's and rate_learning's MCP schema enums (dispatch.rs)
    /// — a variant added to one of these enums without updating its `ALL`
    /// would silently under-advertise it, the same drift class as #4152's
    /// stale `project` scope.
    #[test]
    fn learning_enum_all_consts_have_every_variant() {
        assert_eq!(LearningKind::ALL.len(), 6);
        assert_eq!(LearningScope::ALL.len(), 4);
        assert_eq!(LearningVerdict::ALL.len(), 3);
    }

    #[test]
    fn needs_review_no_longer_parses() {
        // The needs_review status was removed; parsing it must hard-error per
        // the storage-boundary policy (core.allium).
        assert!(LearningStatus::parse("needs_review").is_err());
    }

    #[test]
    fn verdict_round_trips() {
        assert_eq!(
            LearningVerdict::parse("helped").unwrap(),
            LearningVerdict::Helped
        );
        assert_eq!(
            LearningVerdict::parse("wrong").unwrap(),
            LearningVerdict::Wrong
        );
        // `unused` was removed — it is now an unknown verdict.
        assert!(LearningVerdict::parse("unused").is_err());
        assert!(LearningVerdict::parse("bogus").is_err());
        assert_eq!(LearningVerdict::Helped.as_str(), "helped");
        assert_eq!(LearningVerdict::Wrong.as_str(), "wrong");
        assert_eq!(
            LearningVerdict::parse("not_applicable").unwrap(),
            LearningVerdict::NotApplicable
        );
        assert_eq!(LearningVerdict::NotApplicable.as_str(), "not_applicable");
    }

    #[test]
    fn retrieval_source_round_trips() {
        assert_eq!(
            RetrievalSource::parse("prompt_injection").unwrap(),
            RetrievalSource::PromptInjection
        );
        assert_eq!(
            RetrievalSource::parse("procedural").unwrap(),
            RetrievalSource::Procedural
        );
        assert_eq!(
            RetrievalSource::parse("query_learnings").unwrap(),
            RetrievalSource::QueryLearnings
        );
        assert!(RetrievalSource::parse("nope").is_err());
        assert_eq!(
            RetrievalSource::PromptInjection.as_str(),
            "prompt_injection"
        );
    }
}
