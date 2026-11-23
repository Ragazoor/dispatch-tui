//! The orchestrator switch on an epic and the ready mark on a backlog task
//! (`core.allium: EpicOrchestration`, `ReadyMark`).

use chrono::{DateTime, Utc};

use crate::define_str_enum;

use super::{EpicId, TaskId};

/// `core.allium: StartRule`: which backlog tasks an orchestrator may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRule {
    MarkedOnly,
    Everything,
}

define_str_enum!(StartRule, "start rule" {
    MarkedOnly => "marked_only",
    Everything => "everything",
});

impl StartRule {
    pub const ALL: &'static [StartRule] = &[StartRule::MarkedOnly, StartRule::Everything];
}

/// `core.allium: ReadyMarker`: who set a ready mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyMarker {
    Operator,
    Agent,
}

define_str_enum!(ReadyMarker, "ready marker" {
    Operator => "operator",
    Agent => "agent",
});

/// `core.allium: config.orchestrator_max_parallel`, the limit of an epic that
/// sets none: the store's, which refuses a start past it.
pub const DEFAULT_ORCHESTRATOR_LIMIT: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicOrchestration {
    pub epic_id: EpicId,
    pub start_rule: StartRule,
    /// `None` follows [`DEFAULT_ORCHESTRATOR_LIMIT`].
    pub max_parallel: Option<i64>,
    pub switched_on_at: DateTime<Utc>,
}

impl EpicOrchestration {
    pub fn limit(&self) -> i64 {
        self.max_parallel.unwrap_or(DEFAULT_ORCHESTRATOR_LIMIT)
    }
}

/// `core.allium: Orchestrator`: an orchestrated epic's orchestrator on one
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orchestrator {
    pub id: i64,
    pub epic_id: EpicId,
    pub host: String,
    pub name: String,
    pub tmux_window: Option<super::TmuxWindow>,
    pub started_at: Option<DateTime<Utc>>,
    pub restarted_once: bool,
    pub session_died: bool,
    pub last_turn_at: Option<DateTime<Utc>>,
}

impl Orchestrator {
    pub fn is_live(&self) -> bool {
        self.tmux_window.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyMark {
    pub task_id: TaskId,
    pub marked_by: ReadyMarker,
    /// The marking agent's name when `marked_by` is [`ReadyMarker::Agent`].
    pub agent_name: Option<String>,
    pub marked_at: DateTime<Utc>,
}
