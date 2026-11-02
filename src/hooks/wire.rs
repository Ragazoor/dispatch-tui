//! The hook delivery payload, shared by the hook process that sends it and
//! the board handler that applies it.
//!
//! One tagged request rather than one endpoint per hook kind: the hook side
//! has exactly one place that builds a request and one that sends it, and the
//! board side has one handler. See `HookDelivery` in
//! `docs/specs/agent-health.allium`.
//!
//! The top-level split is the one distinction that matters to every layer: a
//! hook either **observes** something that already happened, or **asks** a
//! question whose answer the agent's tool call is waiting on. They differ in
//! what an unreachable board costs — an observation is dropped, a question
//! has to be decided anyway — so keeping them apart in the types means no
//! layer carries an arm for an answer it cannot receive.

use serde::{Deserialize, Serialize};

use crate::models::{EpicId, HookEventKind, SubagentEvent, TaskId};

/// The path the board serves hook deliveries on, beside `/mcp`.
pub const HOOK_PATH: &str = "/hook";

/// One hook delivery, already parsed. Every field the board needs is decided
/// in the hook process, which does no I/O of its own to decide them — an
/// unparseable argument fails there, before delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hook", rename_all = "snake_case")]
pub enum HookRequest {
    /// Something happened; record it. The tool call does not wait on this.
    Observe(ObservedEvent),
    /// Something needs deciding before the tool call may proceed.
    Ask(Question),
}

/// A hook event the board records. Nothing the agent does depends on the
/// answer, which is what lets an unreachable board drop one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ObservedEvent {
    /// `dispatch hook <id> <kind>`: a timestamp-or-status signal.
    Event {
        task_id: TaskId,
        kind: HookEventKind,
    },
    /// `dispatch hook-subagent <id> start|stop`.
    ///
    /// Carries the identifiers rather than a [`SubagentEvent`], which also has
    /// a `Clear` variant that only ever arises in-process (from detach) and so
    /// must not be representable on the wire. The board builds the
    /// [`SubagentEvent`] from these.
    Subagent {
        task_id: TaskId,
        agent_id: String,
        session_id: String,
        /// `false` is a start, `true` a stop. The two differ in nothing but
        /// which end of the subagent's life they mark.
        stop: bool,
    },
    /// `dispatch hook-subagent <id> clear` — the non-draining clear that
    /// `SessionStart` produces. A different event from a subagent stopping,
    /// not a third value of one: it voids a deferred Stop where a stop would
    /// apply it.
    SubagentClear { task_id: TaskId },
    /// `dispatch hook-peer-message <id> --target <to> --body <body>`.
    PeerMessage {
        task_id: TaskId,
        target: String,
        body: String,
    },
    /// A key pressed in the task's agent-tree or diff pane that took effect:
    /// the row's action id and the key as the board records it. The pane has
    /// no store connection, so the board records the usage event
    /// (`PanesRecordUsageLikeTheBoard` in `docs/specs/keybindings.allium`).
    PaneKey {
        task_id: TaskId,
        action: String,
        key: String,
    },
}

impl ObservedEvent {
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::Event { task_id, .. }
            | Self::Subagent { task_id, .. }
            | Self::SubagentClear { task_id }
            | Self::PeerMessage { task_id, .. }
            | Self::PaneKey { task_id, .. } => *task_id,
        }
    }

    /// The board-side event this records, for the two kinds that carry one.
    pub fn subagent_event(agent_id: String, session_id: String, stop: bool) -> SubagentEvent {
        if stop {
            SubagentEvent::Stop {
                agent_id,
                session_id,
            }
        } else {
            SubagentEvent::Start {
                agent_id,
                session_id,
            }
        }
    }
}

/// A hook the agent's tool call is blocked on. See `PrLearningsGate` in
/// `docs/specs/pr-workflow.allium`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "question", rename_all = "snake_case")]
pub enum Question {
    /// `dispatch pr-gate <id>`: may this task create its PR yet?
    PrGate { task_id: TaskId },
    /// `dispatch hook-peer-message <id> --target <to> --body <body>
    /// --undelivered`: the task's `SendMessage` reached nobody. What should
    /// the sender be told? See `UndeliveredPeerMessage` in
    /// `docs/specs/agent-health.allium`. Asked rather than observed because
    /// the sender reads the answer; the board still records the send, as it
    /// does for [`ObservedEvent::PeerMessage`].
    UndeliveredPeerMessage {
        task_id: TaskId,
        target: String,
        body: String,
    },
}

impl Question {
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::PrGate { task_id } | Self::UndeliveredPeerMessage { task_id, .. } => *task_id,
        }
    }
}

/// What the board did with a delivery. Mirrors [`HookRequest`], so a caller
/// that sent one kind cannot be handed the other's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookResponse {
    Observed(ObserveOutcome),
    Answer(Answer),
}

/// What became of an observed event. Every variant is a successful delivery:
/// the board answered. Only a board that answered nothing at all is a failure,
/// and that is the caller's own verdict, not one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ObserveOutcome {
    /// The event was recorded.
    Applied,
    /// The task named by the event does not exist on this board. Not an error
    /// the agent can act on — see `MissingTaskSucceeds` in
    /// `docs/specs/agent-health.allium`.
    TaskNotFound,
    /// The board could not record it. Carries the reason to report.
    Failed { reason: String },
}

// ---------------------------------------------------------------------------
// BoardPaneView (agent-tree.allium): what the board answers a companion pane.
// ---------------------------------------------------------------------------

/// The path the board answers a companion pane's read on, beside `/mcp` and
/// [`HOOK_PATH`]. POST, with a [`PaneViewRequest`] body; the answer is a
/// [`PaneView`].
pub const PANE_VIEW_PATH: &str = "/pane-view";

/// A pane renderer's question: what does the board hold for this task?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneViewRequest {
    pub task_id: TaskId,
}

/// `agent-tree.allium`'s `PaneView`: the pane's task, when the board's rows
/// hold it, every task with a live agent window, and every task whose agent is
/// detached.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneView {
    /// Null until the board's rows hold the task -- "not arrived yet", which
    /// is not the same answer as "arrived without a worktree".
    pub task: Option<PaneTask>,
    /// Exactly the tasks `Task::is_live_agent` holds for in the board's rows,
    /// across every repo and epic (`LiveIsTheBoardsOwnDefinition`).
    pub live_agents: Vec<PaneAgent>,
    /// Exactly the tasks `Task::is_detached` holds for in the board's rows
    /// (`DetachedIsTheBoardsOwnDefinition`), each with no `tmux_window`.
    /// A separate list rather than a flag on [`PaneAgent`] so a pane that
    /// predates it ignores it, and lists the attached agents as before;
    /// absent from a board that predates it.
    #[serde(default)]
    pub detached_agents: Vec<PaneAgent>,
}

/// `agent-tree.allium`'s `PaneTask`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneTask {
    pub worktree: Option<String>,
    pub base_branch: String,
}

/// One listed agent, as much of the task as the agents section draws and
/// jumps to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneAgent {
    pub id: TaskId,
    pub title: String,
    /// The task's recorded tmux window: always present for a live agent, and
    /// absent for a detached one, which has none. Only ever absent in
    /// `detached_agents`, a list a pane that predates it ignores.
    #[serde(default)]
    pub tmux_window: Option<String>,
    /// The name of the task's current agent, the word the section shows in
    /// place of its id (`agent-tree.allium: AgentRowShowsAgentName`). Empty
    /// from a board that predates agent names, or for a task with no current
    /// agent.
    #[serde(default)]
    pub agent_name: String,
    /// The task's own epic, which the section groups agents under
    /// (`agent-tree.allium: AgentListGroupsByEpic`). Absent for a task with
    /// no epic, and from a board that predates grouping.
    #[serde(default)]
    pub epic: Option<PaneEpic>,
}

/// The epic a [`PaneAgent`] belongs to, as its group header draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneEpic {
    pub id: EpicId,
    /// The epic's own title, as the one segment. The header names only the
    /// epic itself (`agent-tree.allium`, `AgentListGroupsByEpic`). The field
    /// keeps its name and list shape from when it carried the ancestor path,
    /// and the pane draws its last segment, so a pane and board of different
    /// builds still agree.
    pub breadcrumb: Vec<String>,
}

/// The path a companion pane asks the board to resume a detached agent on,
/// beside [`PANE_VIEW_PATH`]. POST, with a [`PaneResumeRequest`] body; the
/// answer is a [`PaneResumeAnswer`]. See `BoardPaneView.resume_agent` and
/// `ResumeIsTheBoardsOwnResume` in `docs/specs/agent-tree.allium`.
pub const PANE_RESUME_PATH: &str = "/pane-resume";

/// A pane's request: resume this detached agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneResumeRequest {
    pub task_id: TaskId,
}

/// `agent-tree.allium`'s `PaneResumeAnswer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum PaneResumeAnswer {
    /// The board raised `ResumeTask`; the launch is its to finish.
    Accepted,
    /// The board refused. `reason` is worded for the pane's notice.
    Refused { reason: String },
}

/// The board's answer to a [`Question`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum Answer {
    /// The PR gate's verdict. `reminder` carries the text to show when this is
    /// the first attempt, and is absent when the attempt may simply proceed.
    ///
    /// The **board** supplies the wording, not the hook process: the hook
    /// binary is whichever `dispatch` is on `PATH`, which need not be the
    /// build the board is running, so a reminder composed by the courier
    /// would let a stale installed binary silently override the deciding
    /// board's text.
    PrGate { reminder: Option<String> },
    /// What to tell an agent whose `SendMessage` reached nobody: how to wake
    /// the target, or why it cannot be woken. Absent when `to` names no task
    /// on this board, so Claude Code's own error stands alone. Worded by the
    /// board for the reason [`Answer::PrGate`] is.
    UndeliveredPeerMessage { hint: Option<String> },
    /// The board could not decide. The gate treats this as permission to
    /// proceed — see `PrLearningsGate` in `docs/specs/pr-workflow.allium`.
    Failed { reason: String },
}
