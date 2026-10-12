use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{
    DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchRequestState, DispatchResult,
    TaskId,
};
use crate::store::RequestDispatchAnswer;

use super::{
    parse_args, service_err_to_response, DispatchTaskArgs, JsonRpcResponse, INTERNAL_ERROR,
    INVALID_PARAMS,
};

pub(crate) async fn handle_dispatch_task(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DispatchTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let task_id = parsed.task_id;

    let task = match state.task_svc.get_task(task_id).await {
        Ok(t) => t,
        Err(e) => return service_err_to_response(id, e),
    };
    let epic_id = task.epic_id;

    // Subscribed BEFORE the request is written, so no change to the row can
    // land between writing it and starting to wait on it.
    let mut changes = state.db.dispatch_request_changes();

    // The request is the claim (`DispatchClaimExclusive` in
    // `docs/specs/dispatch.allium`): the backlog guard is the module's, in the
    // same transaction that writes the row, not the status read above.
    let request = match state
        .task_svc
        .request_dispatch(task_id, DispatchOrigin::Mcp, None)
        .await
    {
        Ok(RequestDispatchAnswer::Requested(request)) => request,
        Ok(RequestDispatchAnswer::Refused(why)) => {
            return refused_response(state, id, task_id, why).await
        }
        Err(e) => return service_err_to_response(id, e),
    };
    state.notify_task_changed(task_id);
    if let Some(eid) = epic_id {
        state.notify_epic_changed(eid);
    }

    let row = wait_for_request(state, request, &mut changes).await;
    match row {
        Some(row) if row.state == DispatchRequestState::Dispatched => {
            let text = match (row.worktree.as_deref(), row.tmux_window.as_ref()) {
                // Whether the worktree was reused is provisioning's own
                // answer, recorded on the row with the dispatched report.
                (Some(worktree), Some(window)) => dispatched_text(
                    task_id,
                    &DispatchResult {
                        worktree_path: worktree.to_string(),
                        tmux_window: window.clone(),
                        reused_worktree: row.reused_worktree,
                    },
                ),
                _ => format!("dispatched task #{} (request #{})", task_id.0, request.0),
            };
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
        Some(row) if row.state == DispatchRequestState::Failed => {
            let reason = row
                .reason
                .unwrap_or_else(|| "no reason recorded".to_string());
            JsonRpcResponse::err(id, INTERNAL_ERROR, format!("dispatch failed: {reason}"))
        }
        // Still open when the wait ended: NOT an error. The dispatch carries
        // on; get_task's [Dispatch request] line shows how it ends.
        open => {
            let state_word = open
                .as_ref()
                .map(|r| r.state.as_str())
                .unwrap_or(DispatchRequestState::Requested.as_str());
            let text = format!(
                "dispatch request #{} for task #{} is still {state_word}; it carries on \
                 without this call. Call get_task(task_id={}) to see how it ends — its \
                 \"Dispatch request:\" line shows the request's state, and the worktree and \
                 tmux window once it is dispatched.",
                request.0, task_id.0, task_id.0
            );
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
    }
}

/// Wait for `request` to finish, for at most the state's wait cap
/// (`config.dispatch_task_wait_cap`). Wakes on the subscription's change
/// signal, never on a timer of its own. Returns the row as last seen.
async fn wait_for_request(
    state: &McpState,
    request: DispatchRequestId,
    changes: &mut tokio::sync::watch::Receiver<u64>,
) -> Option<DispatchRequest> {
    let until = tokio::time::Instant::now() + state.dispatch_task_wait_cap;
    loop {
        let row = state.db.dispatch_request(request).await;
        if row.as_ref().is_some_and(|r| !r.is_open()) {
            return row;
        }
        match tokio::time::timeout_at(until, changes.changed()).await {
            Ok(Ok(())) => continue,
            // The cap passed, or the rows' sender is gone (the board is going
            // away): answer with what is known.
            Ok(Err(_)) | Err(_) => return row,
        }
    }
}

/// The error for a refused request. "Not in backlog" when the task holds
/// another status now — the same error a task that was already running
/// produced before M4 — and the module's own reason otherwise (its worktree
/// is on another machine, or an open request already exists).
async fn refused_response(
    state: &McpState,
    id: Option<Value>,
    task_id: TaskId,
    why: String,
) -> JsonRpcResponse {
    let current = match state.task_svc.get_task(task_id).await {
        Ok(t) => t.status,
        Err(e) => return service_err_to_response(id, e),
    };
    if current != crate::models::TaskStatus::Backlog {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            format!("task #{} is not in backlog (current: {current})", task_id.0),
        );
    }
    JsonRpcResponse::err(
        id,
        INVALID_PARAMS,
        format!("task #{} cannot be dispatched: {why}", task_id.0),
    )
}

/// The success text `dispatch_task` returns.
///
/// Names which provisioning path ran — created or reused — rather than one
/// wording for both. See rule-guidance.DispatchTaskViaMcp in
/// docs/specs/mcp-task-tools.allium for why the caller is told.
pub(in crate::mcp::handlers) fn dispatched_text(task_id: TaskId, dr: &DispatchResult) -> String {
    let provisioning = if dr.reused_worktree {
        "reused existing worktree"
    } else {
        "created worktree"
    };
    format!(
        "dispatched task #{} — {provisioning}: {}, tmux: {}",
        task_id.0, dr.worktree_path, dr.tmux_window
    )
}
