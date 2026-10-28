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
    if let Some(why) = refusal(&task) {
        return JsonRpcResponse::err(id, INVALID_PARAMS, why);
    }
    let Some(tx) = &state.notify_tx else {
        return JsonRpcResponse::err(
            id,
            INTERNAL_ERROR,
            "no board is attached to this server, so nothing can resume the agent".to_string(),
        );
    };
    let event = BoardEvent::ResumeTask {
        id: task_id,
        message: parsed.message,
    };
    if tx.send(event).is_err() {
        return JsonRpcResponse::err(
            id,
            INTERNAL_ERROR,
            "the board is shutting down; the agent was not resumed".to_string(),
        );
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

/// Why `task` has no session to resume, or `None` when it has one. Mirrors the
/// board's own `ResumeTask` preconditions that the row shows (the host gate is
/// the board's: only it knows this install's host id).
fn refusal(task: &Task) -> Option<String> {
    let n = task.id.0;
    if !matches!(
        task.status,
        TaskStatus::Running | TaskStatus::Review | TaskStatus::Done
    ) {
        return Some(format!(
            "task #{n} has no resumable session: it is {} and has not run yet. \
             Use dispatch_task to start it.",
            task.status
        ));
    }
    if task.worktree.is_none() {
        return Some(format!(
            "task #{n} has no resumable session: it has no worktree, so there is no \
             conversation to continue. Set it back to backlog with update_tasks and \
             dispatch_task it to start a new agent."
        ));
    }
    if task.tmux_window.is_some() {
        return Some(format!(
            "task #{n} already has an agent attached; reach it with ListAgents and \
             SendMessage instead of resuming it."
        ));
    }
    None
}
