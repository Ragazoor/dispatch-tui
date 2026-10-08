//! What a running agent reports through Claude Code hooks, and how the board
//! reads it: hook event kinds, subagent and stop bookkeeping, notification
//! kinds, and the active/stale activity classification.

use super::SubStatus;

/// A Claude Code hook event kind reported via the `dispatch hook` CLI.
///
/// Each event kind drives a different side effect on a Running task; non-Running
/// tasks ignore hook events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    /// Refreshes `last_pre_tool_use_at`. Covers both the Claude Code
    /// `PreToolUse` and `PostToolUse` hook events — the shell hook
    /// (`task-status-hook`) maps both to `pre_tool_use` so the Rust side
    /// sees a single activity signal regardless of which fired.
    PreToolUse,
    /// Fires on the Claude Code `Notification` hook. Carries the payload's
    /// `notification_type` (forwarded by the shell hook as `--kind`) when
    /// present; `None` when the field is absent (older Claude Code) or the
    /// value is unrecognised, both of which map to the raise/`needs_input`
    /// path for backward compatibility. See `record_hook_event`.
    Notification(Option<NotificationKind>),
    Stop,
    /// Fires when the user submits a new prompt, before the agent has taken
    /// any action. Unlike the other kinds, this is not gated to already-
    /// Running tasks: it drives Review -> Running so a task reflects the
    /// human resuming the conversation immediately, without waiting for the
    /// agent's first tool call (which may be seconds away, or never fire at
    /// all for a pure-text turn).
    UserPromptSubmit,
}

impl HookEventKind {
    /// Parse the event name (`pre_tool_use` | `notification` | `stop`). The
    /// `notification_type` subtype arrives via a separate `--kind` argument
    /// and is attached by the caller, so `notification` parses to
    /// `Notification(None)` here.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pre_tool_use" => Some(Self::PreToolUse),
            "notification" => Some(Self::Notification(None)),
            "stop" => Some(Self::Stop),
            "user_prompt_submit" => Some(Self::UserPromptSubmit),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "pre_tool_use",
            Self::Notification(_) => "notification",
            Self::Stop => "stop",
            Self::UserPromptSubmit => "user_prompt_submit",
        }
    }
}

/// A Claude Code subagent lifecycle event, forwarded by `task-status-hook`
/// via `dispatch hook-subagent`. Deliberately separate from [`HookEventKind`]:
/// these carry an `agent_id` and `session_id` and mutate `task_subagents`,
/// where `HookEventKind` variants are timestamp-only signals.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentEvent {
    /// Claude Code `SubagentStart`.
    Start {
        agent_id: String,
        session_id: String,
    },
    /// Claude Code `SubagentStop`.
    Stop {
        agent_id: String,
        session_id: String,
    },
    /// Drop every entry for the task, then run the drain path. Reached only from
    /// `DetachTmux` — detaching removes the agent that was going to drain the
    /// count itself. `SessionStart` clears too, but *without* draining, so it
    /// goes through `clear_subagents_no_drain` rather than this variant.
    Clear,
}

/// Whether clearing a task's subagent entries also runs the drain path.
///
/// Exactly one of the four structural clear points drains. See the drain-path
/// `@guidance` on `HookSubagentStop` (`docs/specs/agent-health.allium`), which
/// names the clear points on `DetectCrashedAgent`, `DetachTmux`
/// (`split-pane.allium`) and `DispatchTask` (`dispatch.allium`), and the
/// `ClearSubagentsOnSessionStart` rule (`docs/specs/agent-health.allium`).
///
/// Lives here beside [`SubagentEvent`] rather than in the TUI command module
/// that first named it: the drain/no-drain split is spec'd domain behaviour, and
/// the runtime and service layers both need the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainMode {
    /// Run the drain path: a Stop deferred while subagents were live lands now
    /// as a Review flip. `DetachTmux` is the only caller — it assigns no
    /// outcome status of its own, so applying the deferred Stop is safe there.
    Drain,
    /// Clear the entries and `stop_pending`, but leave status alone. For callers
    /// that already own the resulting status (crash, dispatch-claim): draining
    /// alongside their own write would leave the task in both states at once.
    NoDrain,
}

/// What the `Stop` hook's conditional write actually did.
///
/// The three arms are decided by the row's committed state at write time, not
/// by a prior read: every Claude Code hook is its own `dispatch` process, so a
/// snapshot taken before the write can be stale by the time it lands. See
/// `HookStop` in `docs/specs/agent-health.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// No subagent was live: the task moved to `Review`.
    Flipped,
    /// Subagents were still live: the flip was withheld and `stop_pending` set.
    /// The last `SubagentStop` applies it.
    Deferred,
    /// The task was not `Running` (or does not exist). Nothing was written.
    NoOp,
}

/// What the `UserPromptSubmit` hook's conditional write actually did.
///
/// Production reads one bit of this: whether to recalculate the task's epic,
/// which is owed for a status change and so only for `Resumed`. The other two
/// arms are split because tests assert on them — a refresh and a no-op are very
/// different outcomes to get wrong — and for symmetry with [`StopOutcome`].
/// Like it, the arms are decided by the row's committed state at write time. See
/// `HookUserPromptSubmit` in `docs/specs/agent-health.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserPromptOutcome {
    /// The task was in `Review`: the human's prompt moved it back to `Running`.
    Resumed,
    /// The task was already `Running`: a plain activity refresh, no status move.
    Refreshed,
    /// The task was in neither `Running` nor `Review` (or does not exist).
    /// Nothing was written.
    NoOp,
}

/// Result of a subagent mutation that can drain the last live subagent.
///
/// `applied_pending_stop` is reported rather than re-derived by the caller
/// because the flip happens inside the same transaction that recomputed the
/// count — there is no point at which a caller could observe the two
/// separately. See `HookSubagentStop` in `docs/specs/agent-health.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentDrain {
    /// `live_subagents` after the mutation.
    ///
    /// Informational — mirrors the count `subagent_start` returns. Do **not**
    /// branch on it to decide whether a deferred `Stop` should apply: by the
    /// time you read it the transaction has already made that decision, and
    /// re-deciding out here is the read-then-write shape that made the
    /// stranded state reachable in the first place. Use
    /// `applied_pending_stop`.
    pub live: i64,
    /// Whether this write also applied a deferred `Stop`.
    pub applied_pending_stop: bool,
}

/// The `notification_type` field on Claude Code's `Notification` hook payload,
/// forwarded by `task-status-hook` as the `--kind` argument. The agent-view-only
/// values `agent_needs_input` / `agent_completed` are intentionally absent:
/// dispatch runs a plain `claude` process in tmux, never `claude agents`, so
/// they never reach the hook. See the `NotificationKind` enum in
/// `docs/specs/core.allium` and `HookNotification` in `agent-health.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// Agent is blocked on a permission decision.
    PermissionPrompt,
    /// Agent has gone idle awaiting human input.
    IdlePrompt,
    /// Informational (auth succeeded); not human-actionable.
    AuthSuccess,
    /// Agent is asking a question / showing a form.
    ElicitationDialog,
    /// An elicitation just resolved.
    ElicitationComplete,
    /// An elicitation response was received.
    ElicitationResponse,
}

impl NotificationKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "permission_prompt" => Some(Self::PermissionPrompt),
            "idle_prompt" => Some(Self::IdlePrompt),
            "auth_success" => Some(Self::AuthSuccess),
            "elicitation_dialog" => Some(Self::ElicitationDialog),
            "elicitation_complete" => Some(Self::ElicitationComplete),
            "elicitation_response" => Some(Self::ElicitationResponse),
            _ => None,
        }
    }

    /// Classify into the three behaviours `record_hook_event` acts on. See
    /// `NotificationBehavior` and `HookNotification` in `agent-health.allium`.
    pub fn behavior(self) -> NotificationBehavior {
        match self {
            Self::PermissionPrompt | Self::IdlePrompt | Self::ElicitationDialog => {
                NotificationBehavior::Raise
            }
            Self::ElicitationComplete | Self::ElicitationResponse => NotificationBehavior::Clear,
            Self::AuthSuccess => NotificationBehavior::Ignore,
        }
    }
}

/// How a Notification hook firing should affect a running task's sub_status.
/// Mirrors the classification pattern of `AgentActivity`/`classify_agent_activity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationBehavior {
    /// The agent is genuinely blocked: raise sub_status to `needs_input`.
    Raise,
    /// A prior block just resolved: clear back to the running default.
    Clear,
    /// Informational only: no state change.
    Ignore,
}

impl NotificationBehavior {
    /// Absent/unrecognised `notification_type` (older Claude Code, or a
    /// future value dispatch doesn't know yet) preserves the historical
    /// always-`needs_input` behaviour by defaulting to `Raise`.
    pub fn from_kind(kind: Option<NotificationKind>) -> Self {
        kind.map(NotificationKind::behavior)
            .unwrap_or(NotificationBehavior::Raise)
    }
}

/// The write a `Notification` hook must apply, resolved from the notification
/// kind alone.
///
/// Deliberately a *description* of the write rather than a decision already
/// taken: [`RaiseIfNoOwnWorkLive`](Self::RaiseIfNoOwnWorkLive) carries its
/// condition down into the statement that applies it, so the live-work counts
/// are evaluated against the row's committed state rather than a snapshot read
/// beforehand. Every Claude Code hook runs as its own OS process, so a count
/// read before the write can already be stale by the time the write lands —
/// the same argument `try_record_stop` makes for the identical two counters.
/// See `HookNotification` in `docs/specs/agent-health.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationWrite {
    /// Raise to `needs_input` and stamp `last_notification_at`. The agent is
    /// blocked on a human whatever else it has running.
    Raise,
    /// Raise, but only while the task has no live subagents.
    ///
    /// An agent that dispatches a subagent ends its turn while that work keeps
    /// running — `try_record_stop` defers the flip to Review for exactly that
    /// reason — and Claude Code, seeing a session that stopped producing
    /// output, fires `Notification(idle_prompt)` about a minute later. Nothing
    /// is waiting on a human there, so raising `needs_input` would report a
    /// block that does not exist.
    ///
    /// Declining to stamp `last_notification_at` is the load-bearing half, not
    /// an incidental one: [`classify_agent_activity`] reads only timestamps, so
    /// a stamp newer than `last_pre_tool_use_at` re-pins `needs_input` on every
    /// tick until the next PreToolUse.
    RaiseIfNoOwnWorkLive,
    /// Return to the running default and drop `last_notification_at`.
    Clear,
    /// Write nothing at all.
    Ignore,
}

impl NotificationWrite {
    /// `idle_prompt` is the one kind whose raise is conditional.
    /// `permission_prompt` and `elicitation_dialog` are never demoted — a
    /// permission decision or a question dialog genuinely needs a human while
    /// background work churns — and neither is an absent kind, which may be a
    /// permission prompt from an older Claude Code.
    pub fn from_kind(kind: Option<NotificationKind>) -> Self {
        match NotificationBehavior::from_kind(kind) {
            NotificationBehavior::Raise if kind == Some(NotificationKind::IdlePrompt) => {
                Self::RaiseIfNoOwnWorkLive
            }
            NotificationBehavior::Raise => Self::Raise,
            NotificationBehavior::Clear => Self::Clear,
            NotificationBehavior::Ignore => Self::Ignore,
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

/// Classify a running agent's activity from its hook event timestamps and its
/// live subagent count.
///
/// `live_subagents > 0` outranks the staleness threshold but loses to a pending
/// notification: a permission prompt genuinely needs a human even while
/// subagents churn. See `ClassifyAgentActivity` in
/// `docs/specs/agent-health.allium`.
pub fn classify_agent_activity(
    last_pre_tool_use_at: Option<chrono::DateTime<chrono::Utc>>,
    last_notification_at: Option<chrono::DateTime<chrono::Utc>>,
    live_subagents: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> AgentActivity {
    if let Some(notif) = last_notification_at {
        let notif_is_newer = last_pre_tool_use_at.is_none_or(|p| notif > p);
        if notif_is_newer {
            return AgentActivity::Waiting;
        }
    }
    if live_subagents > 0 {
        return AgentActivity::Active;
    }
    match last_pre_tool_use_at {
        Some(ts) if now.signed_duration_since(ts) <= ACTIVE_THRESHOLD => AgentActivity::Active,
        _ => AgentActivity::Stale,
    }
}

#[cfg(test)]
mod tests;
