use serde_json::{json, Value};

use crate::dispatch;
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{SubStatus, TaskId, TaskStatus};
use crate::service::{FieldUpdate, UpdateTaskParams};

use super::{
    parse_args, service_err_to_response, ExitSessionArgs, JsonRpcResponse, WrapUpAction, WrapUpArgs,
};

const ERR_NO_TOKEN: &str = "no exit token — call wrap_up first";

fn retro_instruction(action: WrapUpAction) -> String {
    let extra_arg = match action {
        WrapUpAction::Pr => ", and pr_url (the URL returned by `gh pr create`)",
        WrapUpAction::Rebase | WrapUpAction::Done => "",
    };
    format!(
        "run the /retro skill now, then call `exit_session` with action=\"{}\" and this token{extra_arg}",
        action.as_str()
    )
}

async fn wrap_up_verify_line(db: &dyn crate::db::ReadStore, repo_path: &str) -> String {
    match dispatch::fetch_verify_command(db, repo_path).await {
        Some(cmd) => format!(
            " **Verify before exiting**: run `{cmd}` in your worktree and confirm it passes."
        ),
        None => String::new(),
    }
}

/// Common wrap_up finishing sequence shared by all three actions: fetch the
/// verify-command line, issue the exit token recording `action`, and build
/// the retro instruction. Only the surrounding response prose differs per
/// action.
async fn issue_wrap_up_token(
    state: &McpState,
    task_id: TaskId,
    repo_path: &str,
    action: WrapUpAction,
) -> (String, String, String) {
    let verify_line = wrap_up_verify_line(&*state.db, repo_path).await;
    let token = state.issue_exit_token(task_id, action);
    let retro_line = retro_instruction(action);
    (verify_line, token, retro_line)
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
    tracing::info!(task_id = parsed.task_id, action = ?parsed.action, "MCP wrap_up");

    let task = match state
        .task_svc
        .validate_wrap_up(TaskId(parsed.task_id))
        .await
    {
        Ok(t) => t,
        Err(e) => return service_err_to_response(id, e),
    };

    // Defence in depth: `validate_wrap_up` (via `is_wrappable`) guarantees the
    // worktree is `Some` today, but a future change to the validator could
    // silently break that contract. Returning an internal JSON-RPC error keeps
    // a violation from panicking the runtime.
    let worktree = match task.worktree.clone() {
        Some(w) => w,
        None => {
            return JsonRpcResponse::err(
                id,
                -32603,
                "internal: validate_wrap_up returned task without worktree".to_string(),
            );
        }
    };

    let branch = match dispatch::branch_from_worktree(&worktree) {
        Some(b) => b,
        None => {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("Cannot derive branch from worktree: {worktree}"),
            )
        }
    };

    let repo_path = task.repo_path.clone();
    let base_branch = task.base_branch.clone();
    let runner = state.runner.clone();
    let notify_tx = state.notify_tx.clone();
    let task_id = task.id;

    match parsed.action {
        WrapUpAction::Done => {
            let (verify_line, token, retro_line) =
                issue_wrap_up_token(state, task_id, &task.repo_path, WrapUpAction::Done).await;
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "wrap_up complete (task {}, action: done). No git operations performed. \
                The session is NOT yet closed.{verify_line} \
                Exit token: {token} — {retro_line}. \
                You MUST call `exit_session` next as your final action.",
                    parsed.task_id
                )}]}),
            )
        }
        WrapUpAction::Rebase => {
            // Optimistically clear conflict sub_status before rebasing,
            // matching the TUI behavior.
            if task.sub_status == SubStatus::Conflict {
                let clear = UpdateTaskParams::for_task(task_id)
                    .sub_status(SubStatus::default_for(task.status));
                if let Err(e) = state.task_svc.update_task(clear).await {
                    tracing::warn!(
                        task_id = task_id.0,
                        "wrap_up: failed to clear conflict sub_status: {e}"
                    );
                }
            }
            let rebase_runner = runner.clone();
            let rebase_base = base_branch.clone();
            let rebase_result = match tokio::task::spawn_blocking(move || {
                tracing::info!(task_id = task_id.0, %branch, "MCP wrap_up rebase starting");
                dispatch::finish_task(
                    &dispatch::FinishContext {
                        repo_path: &repo_path,
                        worktree: &worktree,
                        branch: &branch,
                        base_branch: &rebase_base,
                        tmux_window: None,
                    },
                    &*rebase_runner,
                )
            })
            .await
            {
                Ok(r) => r,
                Err(e) => return JsonRpcResponse::err(id, -32603, format!("internal error: {e}")),
            };

            match rebase_result {
                Ok(()) => {
                    // The base branch was just fast-forwarded, so repo_path now
                    // reflects the merged code. Refresh the repo's RAG index in
                    // the background if one exists. Fire-and-forget: never block
                    // the exit-token response, and never surface a failure to the
                    // agent.
                    let reindex_svc = crate::service::repo_index::RepoIndexService::new(
                        state.embedding_service.clone(),
                    );
                    let reindex_repo = task.repo_path.clone();
                    tokio::spawn(async move {
                        match reindex_svc
                            .reindex_if_indexed(std::path::Path::new(&reindex_repo))
                            .await
                        {
                            Ok(Some(r)) => tracing::info!(
                                repo = %reindex_repo,
                                chunks = r.chunks_total,
                                "wrap_up re-indexed repo"
                            ),
                            Ok(None) => tracing::debug!(
                                repo = %reindex_repo,
                                "wrap_up: no RAG index, skipping re-index"
                            ),
                            Err(e) => tracing::warn!(
                                repo = %reindex_repo,
                                "wrap_up re-index failed: {e}"
                            ),
                        }
                    });
                    state.notify_task_changed(task_id);
                    let (verify_line, token, retro_line) =
                        issue_wrap_up_token(state, task_id, &task.repo_path, WrapUpAction::Rebase)
                            .await;
                    JsonRpcResponse::ok(
                        id,
                        json!({"content": [{"type": "text", "text": format!(
                            "wrap_up complete (task {}, action: rebase). The session is NOT yet closed.{verify_line} \
                        Exit token: {token} — {retro_line}. \
                        You MUST call `exit_session` next as your final action — without it, the tmux window stays alive \
                        and the task remains in its current status. Do not stop, and do not call any other tool first.",
                            parsed.task_id
                        )}]}),
                    )
                }
                Err(e) => {
                    if matches!(e, dispatch::FinishError::RebaseConflict(_)) {
                        let patch =
                            UpdateTaskParams::for_task(task_id).sub_status(SubStatus::Conflict);
                        if let Err(e) = state.task_svc.update_task(patch).await {
                            tracing::warn!(
                                task_id = task_id.0,
                                "wrap_up: failed to set conflict sub_status: {e}"
                            );
                        }
                    }
                    if let Some(tx) = notify_tx {
                        let _ = tx.send(crate::mcp::McpEvent::TaskChanged(task_id));
                    }
                    JsonRpcResponse::err(id, -32603, format!("wrap_up failed: {e}"))
                }
            }
        }
        WrapUpAction::Pr => {
            let (verify_line, token, retro_line) =
                issue_wrap_up_token(state, task_id, &task.repo_path, WrapUpAction::Pr).await;
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "wrap_up complete (task {tid}, action: pr). The session is NOT yet closed.{verify_line} \
                Exit token: {token} — {retro_line}. \
                You MUST call `exit_session` next as your final action.\n\n\
                Before you finish: if any knowledge base entry was surfaced to you this task \
                and you haven't rated it yet, call `rate_learning` now (helped or wrong). \
                You can only rate learnings that were surfaced to you during this session.",
                    tid = parsed.task_id
                )}]}),
            )
        }
    }
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
    let task_id = TaskId(parsed.task_id);

    let task = match state.db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return JsonRpcResponse::err(id, -32602, format!("task #{} not found", parsed.task_id))
        }
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("internal error: {e}")),
    };

    let token = match parsed.token {
        Some(t) => t,
        None => return JsonRpcResponse::err(id, -32602, ERR_NO_TOKEN),
    };

    // Single call: no more reflect-then-close two-phase dance — the mandatory
    // reflection is the /retro skill, run before exit_session is ever called.
    // Validate token, action, and window liveness, then remove the token —
    // all in one write-lock so a concurrent second call can't observe a
    // half-consumed token.
    let (action, pr_url) = {
        let mut map = state.exit_tokens.write().unwrap_or_else(|e| e.into_inner());
        let stored_action = match map.get(&task_id) {
            None => return JsonRpcResponse::err(id, -32602, ERR_NO_TOKEN),
            Some(et) if et.token != token => {
                return JsonRpcResponse::err(id, -32602, "invalid exit token")
            }
            Some(et) => et.action,
        };
        let action = match parsed.action {
            Some(a) => a,
            None => {
                return JsonRpcResponse::err(
                    id,
                    -32602,
                    "action is required — pass the same action used in wrap_up",
                )
            }
        };
        if action != stored_action {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!(
                    "exit token was issued for wrap_up(action=\"{}\"), but exit_session was called \
                    with action=\"{}\"",
                    stored_action.as_str(),
                    action.as_str()
                ),
            );
        }
        if task.tmux_window.is_none() {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("task #{} has no active session", parsed.task_id),
            );
        }
        let pr_url = if action == WrapUpAction::Pr {
            match parsed.pr_url.filter(|u| !u.is_empty()) {
                Some(u) => Some(u),
                None => return JsonRpcResponse::err(
                    id,
                    -32602,
                    "pr_url is required for action 'pr' — pass the URL returned by `gh pr create`",
                ),
            }
        } else {
            None
        };
        map.remove(&task_id);
        (action, pr_url)
    };

    let mut params = UpdateTaskParams::for_task(task_id).tmux_window(FieldUpdate::Clear);
    params = match (action, pr_url) {
        (WrapUpAction::Pr, Some(pr_url)) => {
            params
                .status(TaskStatus::Review)
                .url(crate::service::UrlUpdate::Set(crate::models::TaskUrl::new(
                    pr_url,
                    crate::models::UrlType::Pr,
                )))
        }
        // pr_url is validated as required above whenever action = Pr, so this
        // (Pr, None) arm is unreachable in practice — Done is a safe, non-panicking
        // fallback rather than asserting an invariant the compiler can't see.
        (WrapUpAction::Pr, None) | (WrapUpAction::Rebase, _) | (WrapUpAction::Done, _) => {
            params.status(TaskStatus::Done)
        }
    };
    if let Err(e) = state.task_svc.update_task(params).await {
        tracing::warn!(
            task_id = task_id.0,
            "exit_session: failed to apply closing patch: {e}"
        );
    }
    state.notify_task_changed(task_id);
    if let Some(epic_id) = task.epic_id {
        state.notify_epic_changed(epic_id);
    }
    let tmux_window = task.tmux_window;
    let runner = state.runner.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(window) = &tmux_window {
            let _ = crate::tmux::kill_window(window, &*runner);
        }
    });
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": "Session closed."}]}),
    )
}
