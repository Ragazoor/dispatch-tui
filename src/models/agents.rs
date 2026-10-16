//! An agent row (agents A1).
//!
//! Spec: `docs/specs/agents.allium`'s `Agent`. The Claude Code session that
//! works a task, as an entity of its own: its name, when it started, and
//! whether it has ended. The module owns the row; the board reads it through
//! its subscription (`sync.allium: SubscribeToAgents`), which delivers every
//! agent unfiltered.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DispatchMode, TaskId, TmuxWindow};
use crate::define_id_newtype;
use crate::define_str_enum;

define_id_newtype!(AgentId, agent_id_tests);

/// Whether an agent's session can still be resumed (`AgentState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentState {
    Current,
    Ended,
}

define_str_enum!(AgentState, "agent state" {
    Current => "current",
    Ended => "ended",
});

/// Why an agent ended (`AgentEndReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentEndReason {
    Replaced,
    WorktreeGone,
    LaunchFailed,
}

define_str_enum!(AgentEndReason, "agent end reason" {
    Replaced => "replaced",
    WorktreeGone => "worktree_gone",
    LaunchFailed => "launch_failed",
});

/// One `agents` row as the board reads it. An absent optional column (`""` in
/// the module) is `None` here. The start mode is a [`DispatchMode`]: the two
/// vocabularies are the same three launch kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: AgentId,
    pub task_id: TaskId,
    pub name: String,
    pub host: String,
    pub start_mode: DispatchMode,
    pub state: AgentState,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<AgentEndReason>,
    pub tmux_window: Option<TmuxWindow>,
}

impl Agent {
    /// `Agent.is_current`.
    pub fn is_current(&self) -> bool {
        self.state == AgentState::Current
    }

    /// `Agent.is_live`: current and holding a window.
    pub fn is_live(&self) -> bool {
        self.is_current() && self.tmux_window.is_some()
    }
}
