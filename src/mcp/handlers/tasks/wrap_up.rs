use serde_json::{json, Value};

use crate::db;
use crate::dispatch;
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{LearningId, SubStatus, TaskId, TaskStatus};
use crate::service::LearningService;

use super::{
    parse_args, service_err_to_response, JsonRpcResponse, ExitSessionArgs, VerdictArg,
    WrapUpAction, WrapUpArgs,
};

const ERR_NO_TOKEN: &str = "no exit token — call wrap_up first";

async fn wrap_up_verify_line(db: &dyn crate::db::TaskStore, repo_path: &str) -> String {
    match dispatch::fetch_verify_command(db, repo_path).await {
        Some(cmd) => format!(
            " **Verify before exiting**: run `{cmd}` in your worktree and confirm it passes."
        ),
        None => String::new(),
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
    tracing::info!(task_id = parsed.task_id, action = ?parsed.action, "MCP wrap_up");

    let task = match state.task_svc.validate_wrap_up(TaskId(parsed.task_id)).await {
        Ok(t) => t,
        Err(e) => return service_err_to_response(id, e),
    };

    // Apply learning verdicts BEFORE the rebase. The agent's evaluation of
    // surfaced knowledge is independent of branch state — if the rebase fails,
    // the verdicts have still been recorded against the retrieval rows that
    // existed when the agent decided to wrap up.
    if let Some(vs) = parsed.learning_verdicts {
        let parsed_verdicts: Vec<(LearningId, _)> = vs
            .into_iter()
            .map(|v: VerdictArg| (LearningId(v.learning_id), v.verdict))
            .collect();
        let learning_svc = LearningService::new(state.db.clone(), state.embedding_service.clone());
        if let Err(e) = learning_svc.apply_verdicts(task.id, parsed_verdicts).await {
            return service_err_to_response(id, e);
        }
    }

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
    let db = state.db.clone();

    match parsed.action {
        WrapUpAction::Done => {
            let patch = db::TaskPatch::new()
                .status(TaskStatus::Done)
                .sub_status(SubStatus::default_for(TaskStatus::Done));
            if let Err(e) = db.patch_task(task_id, &patch).await {
                return JsonRpcResponse::err(id, -32603, format!("wrap_up done failed: {e}"));
            }
            state.notify_task_changed(task_id);
            let verify_line = wrap_up_verify_line(&*state.db, &task.repo_path).await;
            let token = state.issue_exit_token(task_id);
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "wrap_up complete (task {}, action: done). Task marked as done — no git operations performed. \
                The session is NOT yet closed.{verify_line} \
                Exit token: {token} — pass this token to exit_session to close your session. \
                You MUST call `exit_session` next as your final action.",
                    parsed.task_id
                )}]}),
            )
        }
        WrapUpAction::Rebase => {
            // Optimistically clear conflict sub_status before rebasing,
            // matching the TUI behavior.
            if task.sub_status == SubStatus::Conflict {
                let clear_patch =
                    db::TaskPatch::new().sub_status(SubStatus::default_for(task.status));
                if let Err(e) = db.patch_task(task_id, &clear_patch).await {
                    tracing::warn!(task_id = task_id.0, "wrap_up: failed to clear conflict sub_status: {e}");
                }
            }
            let rebase_runner = runner.clone();
            let rebase_base = base_branch.clone();
            let rebase_result = match tokio::task::spawn_blocking(move || {
                tracing::info!(task_id = task_id.0, %branch, "MCP wrap_up rebase starting");
                dispatch::finish_task(
                    &repo_path,
                    &worktree,
                    &branch,
                    &rebase_base,
                    None,
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
                    state.notify_task_changed(task_id);
                    let verify_line = wrap_up_verify_line(&*state.db, &task.repo_path).await;
                    let token = state.issue_exit_token(task_id);
                    JsonRpcResponse::ok(
                        id,
                        json!({"content": [{"type": "text", "text": format!(
                            "wrap_up complete (task {}, action: rebase). The session is NOT yet closed.{verify_line} \
                        Exit token: {token} — pass this token to exit_session. \
                        You MUST call `exit_session` next as your final action — without it, the tmux window stays alive \
                        and the task remains in its current status. Do not stop, and do not call any other tool first.",
                            parsed.task_id
                        )}]}),
                    )
                }
                Err(e) => {
                    if matches!(e, dispatch::FinishError::RebaseConflict(_)) {
                        let patch = db::TaskPatch::new().sub_status(SubStatus::Conflict);
                        if let Err(e) = db.patch_task(task_id, &patch).await {
                            tracing::warn!(task_id = task_id.0, "wrap_up: failed to set conflict sub_status: {e}");
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
            let pr_url = match parsed.pr_url.as_deref() {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => return JsonRpcResponse::err(
                    id,
                    -32602,
                    "pr_url is required for action 'pr' — pass the URL returned by `gh pr create`",
                ),
            };
            let patch = db::TaskPatch::new()
                .status(TaskStatus::Review)
                .sub_status(SubStatus::default_for(TaskStatus::Review))
                .pr_url(Some(pr_url.as_str()));
            if let Err(e) = state.db.patch_task(task_id, &patch).await {
                return JsonRpcResponse::err(id, -32603, format!("wrap_up pr failed: {e}"));
            }
            state.notify_task_changed(task_id);
            if let Some(epic_id) = task.epic_id {
                if let Err(err) = state.db.recalculate_epic_status(epic_id).await {
                    tracing::warn!(
                        "failed to recalculate epic status for epic {}: {err}",
                        epic_id.0
                    );
                }
                state.notify_epic_changed(epic_id);
            }
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "PR recorded (task {tid}, pr_url: {pr_url}). \
                Your session is complete — do not call `exit_session`. \
                PR polling will move this task to Done when the PR merges.",
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

    // Validate token, check session liveness, flip reflected, and (on second call) remove —
    // all in one write-lock to prevent a concurrent second call from seeing stale reflected
    // state and returning the reflection prompt twice.
    // Token errors are checked before the window check so a closed session yields the right
    // error when both the token and the window are gone simultaneously.
    let already_reflected = {
        let mut map = state.exit_tokens.write().unwrap_or_else(|e| e.into_inner());
        let reflected = match map.get(&task_id) {
            None => return JsonRpcResponse::err(id, -32602, ERR_NO_TOKEN),
            Some(et) if et.token != token => {
                return JsonRpcResponse::err(id, -32602, "invalid exit token")
            }
            Some(et) => et.reflected,
        };
        if task.tmux_window.is_none() {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("task #{} has no active session", parsed.task_id),
            );
        }
        if reflected {
            map.remove(&task_id);
        } else if let Some(entry) = map.get_mut(&task_id) {
            entry.reflected = true;
        }
        reflected
    };

    if !already_reflected {
        // Intentionally inline (not via wrap_up_verify_line): this branch has a
        // non-empty nudge when no command is set, and the text differs from wrap_up's phrasing.
        let verify_line = match dispatch::fetch_verify_command(&*state.db, &task.repo_path).await {
            Some(cmd) => {
                format!("\n\nThis repo's verify command is `{cmd}` — run it before closing.")
            }
            None => "\n\nNo verify command is set for this repo. \
                         If you ran a command to validate your work, \
                         record it with `set_verify_command`."
                .to_string(),
        };
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format!("\
Reflect on this session before closing.\n\
\n\
If you encountered any of the following, call record_learning for each one now:\n\
  \u{2022} A pitfall \u{2014} something that wasted time or caused surprise\n\
  \u{2022} A convention \u{2014} a pattern worth following consistently\n\
  \u{2022} A tool tip or preference\n\
\n\
Then call exit_session again (with the same token) to close the session.{verify_line}")}]}),
        );
    }

    let base_patch = crate::db::TaskPatch::new()
        .sub_status(SubStatus::default_for(TaskStatus::Done))
        .tmux_window(None);
    let patch = if task.status == TaskStatus::Running {
        base_patch.status(TaskStatus::Done)
    } else {
        base_patch
    };
    if let Err(e) = state.db.patch_task(task_id, &patch).await {
        tracing::warn!(
            task_id = task_id.0,
            "exit_session: failed to apply closing patch: {e}"
        );
    }
    state.notify_task_changed(task_id);
    if let Some(epic_id) = task.epic_id {
        if let Err(err) = state.db.recalculate_epic_status(epic_id).await {
            tracing::warn!(
                "failed to recalculate epic status for epic {}: {err}",
                epic_id.0
            );
        }
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
