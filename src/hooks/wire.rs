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

use crate::models::{HookEventKind, ShellEvent, SubagentEvent};

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
    Event { task_id: i64, kind: HookEventKind },
    /// `dispatch hook-subagent <id> start|stop`.
    ///
    /// Carries the identifiers rather than a [`SubagentEvent`], which also has
    /// a `Clear` variant that only ever arises in-process (from detach) and so
    /// must not be representable on the wire. The board builds the
    /// [`SubagentEvent`] from these.
    Subagent {
        task_id: i64,
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
    SubagentClear { task_id: i64 },
    /// `dispatch hook-shell <id> start|stop`. `stop` reads as in `Subagent`.
    Shell {
        task_id: i64,
        shell_id: String,
        session_id: String,
        stop: bool,
    },
    /// `dispatch hook-peer-message <id> --target <to> --body <body>`.
    PeerMessage {
        task_id: i64,
        target: String,
        body: String,
    },
}

impl ObservedEvent {
    pub fn task_id(&self) -> i64 {
        match self {
            Self::Event { task_id, .. }
            | Self::Subagent { task_id, .. }
            | Self::SubagentClear { task_id }
            | Self::Shell { task_id, .. }
            | Self::PeerMessage { task_id, .. } => *task_id,
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

    pub fn shell_event(shell_id: String, session_id: String, stop: bool) -> ShellEvent {
        if stop {
            ShellEvent::Stop {
                shell_id,
                session_id,
            }
        } else {
            ShellEvent::Start {
                shell_id,
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
    PrGate { task_id: i64 },
}

impl Question {
    pub fn task_id(&self) -> i64 {
        match self {
            Self::PrGate { task_id } => *task_id,
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
    /// The board could not decide. The gate treats this as permission to
    /// proceed — see `PrLearningsGate` in `docs/specs/pr-workflow.allium`.
    Failed { reason: String },
}
