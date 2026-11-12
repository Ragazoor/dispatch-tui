//! An agent row (agents A1).
//!
//! Spec: `docs/specs/agents.allium`'s `Agent`. The Claude Code session that
//! works a task, as an entity of its own: its name, when it started, and
//! whether it has ended. The module owns the row; the board reads it through
//! its subscription (`sync.allium: SubscribeToAgents`), which delivers every
//! agent unfiltered.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BackgroundWork, DispatchMode, Task, TaskId, TmuxWindow};
use crate::define_id_newtype;
use crate::define_str_enum;

define_id_newtype!(AgentId, agent_id_tests);

/// Where an agent is in its life (`AgentState`): assigned at task creation and
/// never launched (A17), current while its session can be resumed, or ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentState {
    Assigned,
    Current,
    Ended,
}

define_str_enum!(AgentState, "agent state" {
    Assigned => "assigned",
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
    /// `None` while assigned: it has not launched.
    pub start_mode: Option<DispatchMode>,
    pub state: AgentState,
    /// `None` while assigned: it has not launched.
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<AgentEndReason>,
    // The live-session facts, moved off the task by agents A4. Each keeps the
    // meaning `core.allium` gives it. The module alone compares
    // `stop_pending_at`, as a string; the board reads it only to know when to
    // ask the store to end a background wait.
    pub tmux_window: Option<TmuxWindow>,
    pub last_pre_tool_use_at: Option<DateTime<Utc>>,
    pub last_notification_at: Option<DateTime<Utc>>,
    pub live_subagents: i64,
    pub stop_pending: bool,
    pub stop_pending_at: Option<DateTime<Utc>>,
    pub background_work: BackgroundWork,
}

/// `display_agent(task)` over the task's agents (agents A18): the one not
/// ended (assigned or current), else the one that ended last.
pub fn display_agent<'a>(agents: impl IntoIterator<Item = &'a Agent>) -> Option<&'a Agent> {
    let mut last_ended: Option<&Agent> = None;
    for agent in agents {
        if agent.state != AgentState::Ended {
            return Some(agent);
        }
        if last_ended.is_none_or(|l| agent.ended_at > l.ended_at) {
            last_ended = Some(agent);
        }
    }
    last_ended
}

/// `task` as the board shows it: the live-session facts read off its current
/// agent (agents A4), and its display name (A18). The stored Task row holds
/// none of them, so the board's `Task` keeps the field names as a derived
/// view, and every reader of `task.tmux_window`, `is_detached` and the rest
/// reads the agent without knowing it. A task with no current agent has no
/// session: no window, no hook timestamps, no subagents, no pending Stop.
pub fn with_session_of(task: &Task, agent: Option<&Agent>, display_name: Option<&str>) -> Task {
    let mut task = task.clone();
    task.agent_name = display_name.map(str::to_string);
    match agent {
        Some(agent) => {
            task.tmux_window = agent.tmux_window.clone();
            task.last_pre_tool_use_at = agent.last_pre_tool_use_at;
            task.last_notification_at = agent.last_notification_at;
            task.live_subagents = agent.live_subagents;
            task.stop_pending = agent.stop_pending;
            task.stop_pending_at = agent.stop_pending_at;
            task.background_work = agent.background_work;
        }
        None => {
            task.tmux_window = None;
            task.last_pre_tool_use_at = None;
            task.last_notification_at = None;
            task.live_subagents = 0;
            task.stop_pending = false;
            task.stop_pending_at = None;
            task.background_work = BackgroundWork::default();
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
