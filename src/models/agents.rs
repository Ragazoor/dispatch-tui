//! An agent row (agents A1).
//!
//! Spec: `docs/specs/agents.allium`'s `Agent`. The Claude Code session that
//! works a task, as an entity of its own: its name, when it started, and
//! whether it has ended. The module owns the row; the board reads it through
//! its subscription (`sync.allium: SubscribeToAgents`), which delivers every
//! agent unfiltered.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DispatchMode, Task, TaskId, TmuxWindow};
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
    // The live-session facts, moved off the task by agents A4. Each keeps the
    // meaning `core.allium` gives it. `stop_pending_at` is not carried: only
    // the module compares it, and it does so as a string.
    pub tmux_window: Option<TmuxWindow>,
    pub last_pre_tool_use_at: Option<DateTime<Utc>>,
    pub last_notification_at: Option<DateTime<Utc>>,
    pub live_subagents: i64,
    pub stop_pending: bool,
}

/// `task` as the board shows it: the six live-session facts read off its
/// current agent (agents A4). The stored Task row holds none of them, so the
/// board's `Task` keeps the field names as a derived view, and every reader of
/// `task.tmux_window`, `is_detached` and the rest reads the agent without
/// knowing it. A task with no current agent has no session: no window, no
/// hook timestamps, no subagents, no pending Stop.
pub fn with_session_of(task: &Task, agent: Option<&Agent>) -> Task {
    let mut task = task.clone();
    match agent {
        Some(agent) => {
            task.tmux_window = agent.tmux_window.clone();
            task.last_pre_tool_use_at = agent.last_pre_tool_use_at;
            task.last_notification_at = agent.last_notification_at;
            task.live_subagents = agent.live_subagents;
            task.stop_pending = agent.stop_pending;
        }
        None => {
            task.tmux_window = None;
            task.last_pre_tool_use_at = None;
            task.last_notification_at = None;
            task.live_subagents = 0;
            task.stop_pending = false;
        }
    }
    task
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
