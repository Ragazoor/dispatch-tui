//! `resume_task` (`mcp-task-tools.allium: ResumeTaskViaMcp`).
//!
//! Wakes a detached agent with its conversation. The handler refuses what the
//! task row already shows to be impossible, then hands the resume to the board
//! as an event; the board runs the launch behind the Space key
//! (`dispatch.allium: ResumeTask`), so there is one resume path.

use serde_json::{json, Value};

use crate::board_event::BoardEvent;
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{Task, TaskStatus};

use super::{
    parse_args, service_err_to_response, JsonRpcResponse, ResumeTaskArgs, INTERNAL_ERROR,
    INVALID_PARAMS,
};

pub(crate) async fn handle_resume_task(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ResumeTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let task_id = parsed.task_id;
    let task = match state.task_svc.get_task(task_id).await {
        Ok(t) => t,
        Err(e) => return service_err_to_response(id, e),
    };
    if let Some(why) = unresumable(&task, &state.host_id).await {
        return JsonRpcResponse::err(id, INVALID_PARAMS, refusal_text(task_id.0, &why));
    }
    if let Err(why) = hand_resume_to_board(state, task_id, parsed.message) {
        return JsonRpcResponse::err(id, INTERNAL_ERROR, why.to_string());
    }
    let text = format!(
        "resume requested for task #{n}: the board is restarting its agent in {window} with the \
         previous conversation. The task moves to running once the session starts; \
         get_task(task_id={n}) shows the window. A launch failure appears in the board's \
         status line.",
        n = task_id.0,
        window = crate::models::TmuxWindow::for_task(task_id),
    );
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// Hand a checked resume to the board, which runs the launch behind the Space
/// key (`dispatch.allium: ResumeTask`). The one place a resume is sent from, so
/// `resume_task` and the agent-tree pane's resume (`agent-tree.allium:
/// ResumeIsTheBoardsOwnResume`) are the same resume. `Err` says why nothing
/// was sent.
pub(crate) fn hand_resume_to_board(
    state: &McpState,
    task_id: crate::models::TaskId,
    message: Option<String>,
) -> Result<(), &'static str> {
    let Some(tx) = &state.notify_tx else {
        return Err("no board is attached to this server, so nothing can resume the agent");
    };
    tx.send(BoardEvent::ResumeTask {
        id: task_id,
        message,
    })
    .map_err(|_| "the board is shutting down; the agent was not resumed")
}

/// Why a task has no session `resume_task` can wake. Shared with the hook
/// that answers an undelivered `SendMessage`
/// (`agent-health.allium: UndeliveredPeerMessage`), which words each reason
/// for the sending agent rather than for a `resume_task` caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unresumable {
    /// Backlog: no agent has run yet.
    NeverRan(TaskStatus),
    /// No worktree, so no conversation to continue.
    NoWorktree,
    /// The worktree is on another machine.
    OtherMachine,
    /// The board still holds a window for the task.
    Attached,
    /// The worktree directory is gone from this machine's disk.
    WorktreeGone(String),
}

/// Why `task` has no session to resume, or `None` when it has one. Mirrors the
/// board's own `ResumeTask` preconditions, host gate included: `host_id` is
/// this install's own id. The worktree check runs last, after the row says
/// the worktree is set and owned here. A bare metadata stat, no read
/// (docs/conventions.md, "No std::fs inside async handlers").
pub(crate) async fn unresumable(task: &Task, host_id: &str) -> Option<Unresumable> {
    if !matches!(
        task.status,
        TaskStatus::Running | TaskStatus::Review | TaskStatus::Done
    ) {
        return Some(Unresumable::NeverRan(task.status));
    }
    let Some(worktree) = task.worktree.as_deref() else {
        return Some(Unresumable::NoWorktree);
    };
    if !task.is_locally_owned(Some(host_id)) {
        return Some(Unresumable::OtherMachine);
    }
    if task.tmux_window.is_some() {
        return Some(Unresumable::Attached);
    }
    match tokio::fs::metadata(worktree).await {
        Ok(m) if m.is_dir() => None,
        _ => Some(Unresumable::WorktreeGone(worktree.to_string())),
    }
}

/// The agent-tree pane's refusal for `why`: one line for the pane's notice,
/// which has no tools to suggest (`AgentTreeResumeRefusedIsVisible`).
pub(crate) fn pane_refusal_text(name: &str, why: &Unresumable) -> String {
    match why {
        Unresumable::NeverRan(status) => {
            format!("{name} cannot be resumed: it is {status} and has not run yet")
        }
        Unresumable::NoWorktree => format!("{name} cannot be resumed: it has no worktree"),
        Unresumable::OtherMachine => {
            format!("{name} cannot be resumed here: its worktree is on another machine")
        }
        Unresumable::Attached => format!("{name} already has an agent attached"),
        Unresumable::WorktreeGone(worktree) => {
            format!("{name} cannot be resumed: its worktree {worktree} is gone")
        }
    }
}

/// The `resume_task` refusal for `why`.
fn refusal_text(n: i64, why: &Unresumable) -> String {
    match why {
        Unresumable::NeverRan(status) => format!(
            "task #{n} has no resumable session: it is {status} and has not run yet. \
             Use dispatch_task to start it."
        ),
        Unresumable::NoWorktree => format!(
            "task #{n} has no resumable session: it has no worktree, so there is no \
             conversation to continue. Set it back to backlog with update_tasks and \
             dispatch_task it to start a new agent."
        ),
        Unresumable::OtherMachine => format!(
            "task #{n}'s worktree is on another machine, so this machine cannot resume \
             it; resume it from the machine that holds the worktree."
        ),
        Unresumable::Attached => format!(
            "task #{n} already has an agent attached; reach it with ListAgents and \
             SendMessage instead of resuming it."
        ),
        Unresumable::WorktreeGone(worktree) => format!(
            "task #{n} has no resumable session: its worktree {worktree} is no longer on \
             disk, so the agent is gone. Set it back to backlog with update_tasks and \
             dispatch_task it to start a new agent."
        ),
    }
}
