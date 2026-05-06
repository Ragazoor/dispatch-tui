use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::TaskId;

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

    pub fn display_label(self) -> &'static str {
        match self {
            LearningKind::Pitfall => "Pitfall",
            LearningKind::Convention => "Convention",
            LearningKind::Preference => "Preference",
            LearningKind::ToolRecommendation => "Tool recommendation",
            LearningKind::Procedural => "Procedural",
            LearningKind::Landscape => "Landscape",
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
    pub confirmed_count: i64,
    pub last_confirmed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
