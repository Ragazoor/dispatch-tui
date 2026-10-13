use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{
    SessionRequest, SessionRequestId, SessionRequestKind, SessionRequestState, Task, TaskId,
};
use crate::service::WrapUpChecks;
use crate::store::SessionRequestAnswer;

use super::{
    fetch_caller_task, parse_args, service_err_to_response, ExitSessionArgs, JsonRpcResponse,
    WrapUpAction, WrapUpArgs, INTERNAL_ERROR, INVALID_PARAMS,
};

const ERR_NO_TOKEN: &str = "no exit token — call wrap_up first";

fn exit_instruction(action: WrapUpAction) -> String {
    let extra_arg = match action {
        WrapUpAction::Pr => ", and pr_url (the URL returned by `gh pr create`)",
        WrapUpAction::Rebase | WrapUpAction::Done => "",
    };
    format!(
        "call `exit_session` with action=\"{}\" and this token{extra_arg}",
        action.as_str()
    )
}

/// The sentence reporting that `wrap_up` itself ran the verify command, or
/// nothing when the repo has none. The agent is not asked to verify: the tool
/// already did, and refused otherwise (`WrapUpRefusesUnfinishedWork` in
/// `docs/specs/mcp-task-tools.allium`).
fn verified_line(checks: &WrapUpChecks) -> String {
    match &checks.verified {
        Some(cmd) => format!(" Verified: `{cmd}` passed in your worktree."),
        None => String::new(),
    }
}

/// Checks the task is wrappable, returning the JSON-RPC error response to
/// return immediately if not. The module checks it again when the request is
/// written; this is the early refusal, with the same wording, before the
/// commit and verify checks (`ReviewTasksAreNotWrappedUp`).
async fn validate_wrap_up_request(
    state: &McpState,
    id: &Option<Value>,
    task_id: TaskId,
) -> Result<Task, JsonRpcResponse> {
    state
        .task_svc
        .validate_wrap_up(task_id)
        .await
        .map_err(|e| service_err_to_response(id.clone(), e))
}

/// The success response of a wrap-up whose request row is done: report the
/// verification, hand over the row's id as the exit token recording `action`,
/// and name the exit_session call. Only the surrounding prose differs per
/// action.
fn wrap_up_complete(
    id: Option<Value>,
    task_id: TaskId,
    checks: &WrapUpChecks,
    action: WrapUpAction,
    token: SessionRequestId,
) -> JsonRpcResponse {
    let verify_line = verified_line(checks);
    let exit_line = exit_instruction(action);
    let tail = match action {
        WrapUpAction::Rebase => {
            " You MUST call `exit_session` next as your final action — without it, the tmux window stays alive \
             and the task remains in its current status."
                .to_string()
        }
        WrapUpAction::Done | WrapUpAction::Pr => {
            let rate_learning_nudge = match action {
                WrapUpAction::Pr => {
                    "\n\n\
                    Before you finish: if any knowledge base entry was surfaced to you this task \
                    and you haven't rated it yet, call `rate_learning` now (helped or wrong). \
                    You can only rate learnings that were surfaced to you during this session."
                }
                WrapUpAction::Done | WrapUpAction::Rebase => "",
            };
            format!("{rate_learning_nudge} You MUST call `exit_session` next as your final action.")
        }
    };
    let no_git_note = match action {
        WrapUpAction::Done => " No git operations performed.",
        WrapUpAction::Pr | WrapUpAction::Rebase => "",
    };
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": format!(
            "wrap_up complete (task {tid}, action: {action_str}).{no_git_note} \
        The session is NOT yet closed.{verify_line} \
        Exit token: {token} — {exit_line}.{tail}",
            tid = task_id.0,
            action_str = action.as_str(),
            token = token.0,
        )}]}),
    )
}

/// Wait for `request` to finish, for at most the state's wait cap
/// ([`crate::mcp::WRAP_UP_WAIT_CAP`]). Wakes on the subscription's change
/// signal, never on a timer of its own. Returns the row as last seen.
async fn wait_for_session_request(
    state: &McpState,
    request: SessionRequestId,
    changes: &mut tokio::sync::watch::Receiver<u64>,
) -> Option<SessionRequest> {
    let until = tokio::time::Instant::now() + state.wrap_up_wait_cap;
    loop {
        let row = state.db.session_request(request).await;
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

/// Shapes the outcome of a rebase request row (`WrapUpRebase` in
/// `docs/specs/pr-workflow.allium`) into a JSON-RPC response. The git work, the
/// `Conflict` sub_status and the report are this board's host and the module's;
/// what is left here is which error code each outcome earns and the prose.
async fn answer_rebase(
    state: &McpState,
    id: Option<Value>,
    task_id: TaskId,
    checks: &WrapUpChecks,
    request: SessionRequestId,
    changes: &mut tokio::sync::watch::Receiver<u64>,
) -> JsonRpcResponse {
    let row = wait_for_session_request(state, request, changes).await;
    // The module sets the Conflict flag with a failed report, and the host's
    // refresh follows the row; nudge the board to re-read the task now.
    state.notify_task_changed(task_id);
    match row {
        Some(row) if row.state == SessionRequestState::Done => {
            wrap_up_complete(id, task_id, checks, WrapUpAction::Rebase, request)
        }
        Some(row) if row.state == SessionRequestState::Failed => {
            let reason = row
                .reason
                .unwrap_or_else(|| "no reason recorded".to_string());
            JsonRpcResponse::err(id, INTERNAL_ERROR, format!("wrap_up failed: {reason}"))
        }
        // Still open when the wait ended (the store unreachable, or the host
        // gone without the backstop yet firing): not a token, and the next
        // request is refused while this one is still open.
        open => {
            let state_word = open
                .as_ref()
                .map(|r| r.state.as_str())
                .unwrap_or(SessionRequestState::Requested.as_str());
            JsonRpcResponse::err(
                id,
                INTERNAL_ERROR,
                format!(
                    "wrap_up request #{} for task #{} is still {state_word} and no exit token was \
                     issued. Wait a moment and call wrap_up again — a second request is refused \
                     while this one is still open.",
                    request.0, task_id.0
                ),
            )
        }
    }
}

pub(crate) async fn handle_wrap_up(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<WrapUpArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id.0, action = ?parsed.action, "MCP wrap_up");

    let task = match validate_wrap_up_request(state, &id, parsed.task_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    // The host-local checks, before anything is written: a refusal here writes
    // no request row and issues no token. `WrapUpRefusesUnfinishedWork`.
    let checks = match state.task_svc.check_wrap_up_ready(task.clone()).await {
        Ok(c) => c,
        Err(e) => return service_err_to_response(id, e),
    };
    // For rebase the worktree path names a branch (the rebase needs one):
    // refused as a bad request rather than reported as a failed rebase.
    if parsed.action == WrapUpAction::Rebase {
        let worktree = task.worktree.clone().unwrap_or_default();
        if crate::dispatch::branch_from_worktree(&worktree).is_none() {
            return JsonRpcResponse::err(
                id,
                INVALID_PARAMS,
                format!("Cannot derive branch from worktree: {worktree}"),
            );
        }
    }

    // Subscribed BEFORE the request is written, so no change to the row can
    // land between writing it and starting to wait on it.
    let mut changes = state.db.session_request_changes();
    let request = match state
        .task_svc
        .request_wrap_up(task.id, parsed.action.mode())
        .await
    {
        Ok(SessionRequestAnswer::Requested(request)) => request,
        Ok(SessionRequestAnswer::Refused(why)) => {
            return JsonRpcResponse::err(id, INVALID_PARAMS, why)
        }
        Err(e) => return service_err_to_response(id, e),
    };

    match parsed.action {
        // No host work: the module wrote the row already done.
        WrapUpAction::Done | WrapUpAction::Pr => {
            wrap_up_complete(id, task.id, &checks, parsed.action, request)
        }
        WrapUpAction::Rebase => {
            answer_rebase(state, id, task.id, &checks, request, &mut changes).await
        }
    }
}

/// The accepted close's response: "Session closed.", or, when the epic chain
/// requested a subtask in the same transaction (the exit_session row's
/// `chained_task`), the one it names.
async fn session_closed(
    state: &McpState,
    id: Option<Value>,
    task: &Task,
    request: SessionRequestId,
) -> JsonRpcResponse {
    state.notify_task_changed(task.id);
    if let Some(epic_id) = task.epic_id {
        state.notify_epic_changed(epic_id);
    }
    let chained = match state
        .db
        .session_request(request)
        .await
        .and_then(|r| r.chained_task)
    {
        Some(next) => {
            state.notify_task_changed(next);
            Some((
                next,
                state.task_svc.get_task(next).await.ok().map(|t| t.title),
            ))
        }
        None => None,
    };
    let text = match chained {
        Some((next, Some(title))) => format!(
            "Session closed. Dispatching next epic subtask #{} '{title}'.",
            next.0
        ),
        Some((next, None)) => format!("Session closed. Dispatching next epic subtask #{}.", next.0),
        None => "Session closed.".to_string(),
    };
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// A call that got no answer from the store may or may not have landed. The
/// rows settle it: an exit_session row that consumed this token means the close
/// took effect.
async fn settled_request(
    state: &McpState,
    task_id: TaskId,
    token: SessionRequestId,
) -> Option<SessionRequestId> {
    state
        .db
        .session_requests()
        .await
        .into_iter()
        .find(|r| {
            r.kind == SessionRequestKind::ExitSession
                && r.task_id == task_id
                && r.wrap_up_request == Some(token)
        })
        .map(|r| r.id)
}

pub(crate) async fn handle_exit_session(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ExitSessionArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let task_id = parsed.task_id;

    let task = match fetch_caller_task(&*state.db, &id, task_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    // Checked here, before any call: there is nothing to ask the module
    // without them.
    let Some(token_text) = parsed.token else {
        return JsonRpcResponse::err(id, INVALID_PARAMS, ERR_NO_TOKEN);
    };
    let Some(action) = parsed.action else {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "action is required — pass the same action used in wrap_up",
        );
    };
    // A token that is not a row id names no row; the module refuses it in its
    // own words (id 0 is never a real row).
    let token = SessionRequestId(token_text.trim().parse().unwrap_or(0));

    match state
        .task_svc
        .request_exit_session(task_id, token, action.mode(), parsed.pr_url)
        .await
    {
        // The module applied the close, consumed the token and wrote the
        // teardown row. The answer does NOT wait for the teardown: it kills this
        // agent's own window, so a reply that waited would never be read.
        Ok(SessionRequestAnswer::Requested(request)) => {
            session_closed(state, id, &task, request).await
        }
        Ok(SessionRequestAnswer::Refused(why)) => JsonRpcResponse::err(id, INVALID_PARAMS, why),
        Err(e) => match settled_request(state, task_id, token).await {
            Some(request) => session_closed(state, id, &task, request).await,
            None => {
                tracing::warn!(task_id = task_id.0, "exit_session got no answer: {e}");
                JsonRpcResponse::err(
                    id,
                    INTERNAL_ERROR,
                    format!(
                        "exit_session got no answer from the shared store and the close did not \
                         take effect: task #{} is unchanged, your tmux session is still alive and \
                         the token is still valid — call exit_session again.",
                        task_id.0
                    ),
                )
            }
        },
    }
}
