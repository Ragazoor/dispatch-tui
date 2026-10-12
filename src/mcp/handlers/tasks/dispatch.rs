use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{
    DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchRequestState, DispatchResult,
    EpicId, TaskId,
};
use crate::store::RequestDispatchAnswer;

use super::{
    parse_args, service_err_to_response, DispatchTaskArgs, JsonRpcResponse, INTERNAL_ERROR,
    INVALID_PARAMS,
};

/// Requests the next backlog subtask of `epic_id`, if any. Returns
/// `Some((id, title))` when a request was written, `None` when the chain stops
/// here (auto_dispatch off, no backlog subtask, or a lookup failure). Never
/// returns an error: a chain problem must not fail the caller.
///
/// Implements `AutoDispatchNextSubtask` in `docs/specs/epics.allium`, fired by
/// `handle_exit_session` as the last thing a session close does. Since
/// store-split M4 the chain only ASKS: the request (origin `epic_chain`, no
/// mode) names this board's host, so this board's host reactor provisions it,
/// and a failure is raised from the row itself (`ReportChainedDispatchFailure`),
/// not from here.
pub(in crate::mcp::handlers) async fn auto_dispatch_next(
    state: &McpState,
    epic_id: EpicId,
) -> Option<(TaskId, String)> {
    // Fail closed: nothing *requested* this chain, so a DB hiccup must not
    // launch an agent on an epic whose operator turned chaining off. Every
    // non-dispatch outcome here is indistinguishable from the documented
    // normal stops.
    let epic = match state.epic_svc.get_epic(epic_id).await {
        Ok(epic) => epic,
        Err(crate::service::ServiceError::NotFound(_)) => {
            tracing::warn!("auto_dispatch_next: epic #{} not found", epic_id.0);
            return None;
        }
        Err(e) => {
            tracing::warn!(
                "auto_dispatch_next: failed to fetch epic #{}: {e}",
                epic_id.0
            );
            return None;
        }
    };
    if !epic.auto_dispatch {
        return None;
    }

    // The request is the claim, and the chain offers the subtasks one at a
    // time: a concurrent close on the same epic can never win the same one.
    let next_task = match state.task_svc.request_next_backlog_subtask(epic_id).await {
        Ok(Some(next)) => next,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(
                "auto_dispatch_next: failed to request the next subtask of epic #{}: {e}",
                epic_id.0
            );
            return None;
        }
    };

    state.notify_task_changed(next_task.id);
    state.notify_epic_changed(epic_id);
    Some((next_task.id, next_task.title))
}

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
                (Some(worktree), Some(window)) => {
                    match state.task_svc.dispatched_worktree_reused(request).await {
                        Some(reused_worktree) => dispatched_text(
                            task_id,
                            &DispatchResult {
                                worktree_path: worktree.to_string(),
                                tmux_window: window.clone(),
                                reused_worktree,
                            },
                        ),
                        // Not provisioned by this board's service, so which
                        // path ran is unknown and not guessed.
                        None => format!(
                            "dispatched task #{} — worktree: {worktree}, tmux: {window}",
                            task_id.0
                        ),
                    }
                }
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
