use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::db;
use crate::dispatch;
use crate::mcp::McpState;
use crate::models::{DispatchMode, EpicId, SubStatus, Task, TaskStatus, TaskTag};
use crate::service::{
    ClaimTaskParams, CreateTaskParams, ListTasksFilter, ServiceError, TaskService, UpdateTaskParams,
};

use super::types::{
    deserialize_flexible_i64, deserialize_optional_flexible_i64, parse_args, resolve_project_id,
    service_err_to_response, JsonRpcResponse,
};

// ---------------------------------------------------------------------------
// Typed argument structs (JSON-RPC layer)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct UpdateTaskArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    #[serde(default)]
    pub(super) status: Option<TaskStatus>,
    #[serde(default)]
    pub(super) plan_path: Option<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) repo_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default)]
    pub(super) pr_url: Option<String>,
    #[serde(default)]
    pub(super) tag: Option<TaskTag>,
    #[serde(default)]
    pub(super) sub_status: Option<SubStatus>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) epic_id: Option<i64>,
    #[serde(default)]
    pub(super) base_branch: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) project_id: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct GetTaskArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
}

#[derive(Deserialize)]
pub(super) struct ListTasksArgs {
    #[serde(default)]
    pub(super) status: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) epic_id: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct ClaimTaskArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    pub(super) worktree: String,
    pub(super) tmux_window: String,
}

#[derive(Deserialize)]
pub(super) struct ReportUsageArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    pub(super) cost_usd: f64,
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
    #[serde(default)]
    pub(super) cache_read_tokens: i64,
    #[serde(default)]
    pub(super) cache_write_tokens: i64,
}

#[derive(Deserialize)]
pub(super) struct CreateTaskWithEpicArgs {
    pub(super) title: String,
    pub(super) repo_path: String,
    #[serde(default)]
    pub(super) description: String,
    pub(super) plan_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) epic_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default)]
    pub(super) tag: Option<TaskTag>,
    #[serde(default)]
    pub(super) base_branch: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) project_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum WrapUpAction {
    Rebase,
    Pr,
}

#[derive(Deserialize)]
pub(super) struct WrapUpArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    pub(super) action: WrapUpAction,
}

#[derive(Deserialize)]
pub(super) struct SendMessageArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) from_task_id: i64,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) to_task_id: i64,
    pub(super) body: String,
}

#[derive(Deserialize)]
pub(super) struct DispatchNextArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) epic_id: i64,
}

#[derive(Deserialize)]
pub(super) struct DispatchTaskArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
}

// ---------------------------------------------------------------------------
// Response formatting (presentation layer)
// ---------------------------------------------------------------------------

fn build_epic_titles(state: &McpState) -> HashMap<EpicId, String> {
    state
        .db
        .list_epics()
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, e.title))
        .collect()
}

fn format_task_detail(task: &Task, epic_titles: &HashMap<EpicId, String>) -> String {
    let mut text = format!(
        "Task {id}: {title}\nStatus: {status}\nRepo: {repo}\nDescription: {desc}",
        id = task.id,
        title = task.title,
        status = task.status.as_str(),
        repo = task.repo_path,
        desc = task.description,
    );
    text.push_str(&format!("\nSub-status: {}", task.sub_status.as_str()));
    if let Some(epic_id) = task.epic_id {
        let epic_label = match epic_titles.get(&epic_id) {
            Some(title) => format!("{title} (#{epic_id})"),
            None => format!("#{epic_id}"),
        };
        text.push_str(&format!("\nEpic: {epic_label}"));
    }
    if let Some(ref tag) = task.tag {
        text.push_str(&format!("\nTag: {tag}"));
    }
    if let Some(ref plan) = task.plan_path {
        text.push_str(&format!("\nPlan: {plan}"));
    }
    if let Some(ref pr_url) = task.pr_url {
        text.push_str(&format!("\nPR: {pr_url}"));
    }
    if let Some(ref worktree) = task.worktree {
        text.push_str(&format!("\nWorktree: {worktree}"));
    }
    if let Some(ref tmux_window) = task.tmux_window {
        text.push_str(&format!("\nTmux window: {tmux_window}"));
    }
    if let Some(sort_order) = task.sort_order {
        text.push_str(&format!("\nSort order: {sort_order}"));
    }
    text.push_str(&format!(
        "\nCreated: {}",
        task.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    text.push_str(&format!(
        "\nUpdated: {}",
        task.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    text
}

fn format_task_line(t: &Task, epic_titles: &HashMap<EpicId, String>) -> String {
    let desc_preview = if t.description.len() > 200 {
        let end = t
            .description
            .char_indices()
            .take_while(|(i, _)| *i < 200)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &t.description[..end])
    } else {
        t.description.clone()
    };
    let plan_indicator = if t.plan_path.is_some() { " [plan]" } else { "" };
    let tag_indicator = match t.tag {
        Some(tag) => format!(" [{}]", tag.as_str()),
        None => String::new(),
    };
    let epic_indicator = match t.epic_id {
        Some(eid) => match epic_titles.get(&eid) {
            Some(title) => format!(" (epic:{eid} {title})"),
            None => format!(" (epic:{eid})"),
        },
        None => String::new(),
    };
    format!(
        "- [{}] {} ({}/{}){}{}{}: {}",
        t.id,
        t.title,
        t.status.as_str(),
        t.sub_status.as_str(),
        plan_indicator,
        tag_indicator,
        epic_indicator,
        desc_preview
    )
}

// ---------------------------------------------------------------------------
// Task tool handlers (thin wrappers over TaskService)
// ---------------------------------------------------------------------------

pub(super) fn handle_update_task(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<UpdateTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id, status = ?parsed.status, "MCP update_task");

    // MCP-specific restriction: agents cannot set status to done or archived
    if matches!(parsed.status, Some(TaskStatus::Done | TaskStatus::Archived)) {
        return service_err_to_response(
            id,
            ServiceError::Validation(
                "Cannot set status to done or archived via MCP. Please ask the human operator to manage this from the TUI.".into(),
            ),
        );
    }

    let mut params = UpdateTaskParams::for_task(parsed.task_id)
        .tag(parsed.tag)
        .base_branch(parsed.base_branch);
    if let Some(status) = parsed.status {
        params = params.status(status);
    }
    if let Some(plan_path) = parsed.plan_path {
        params = params.plan_path(Some(plan_path));
    }
    if let Some(title) = parsed.title {
        params = params.title(title);
    }
    if let Some(description) = parsed.description {
        params = params.description(description);
    }
    if let Some(repo_path) = parsed.repo_path {
        params = params.repo_path(repo_path);
    }
    if let Some(sort_order) = parsed.sort_order {
        params = params.sort_order(sort_order);
    }
    if let Some(pr_url_str) = parsed.pr_url {
        let fu = if pr_url_str.is_empty() {
            crate::service::FieldUpdate::Clear
        } else {
            crate::service::FieldUpdate::Set(pr_url_str)
        };
        params = params.pr_url(fu);
    }
    if let Some(sub_status) = parsed.sub_status {
        params = params.sub_status(sub_status);
    }
    if let Some(epic_id) = parsed.epic_id {
        params = params.epic_id(epic_id);
    }
    if let Some(project_id) = parsed.project_id {
        if !state
            .db
            .list_projects()
            .unwrap_or_default()
            .iter()
            .any(|p| p.id == project_id)
        {
            return service_err_to_response(
                id,
                crate::service::ServiceError::Validation(format!(
                    "project {project_id} does not exist"
                )),
            );
        }
        params = params.project_id(project_id);
    }
    let fields_display = params.updated_field_names().join(", ");

    let svc = TaskService::new(state.db.clone());
    match svc.update_task(params) {
        Ok(task_id) => {
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Task {} updated ({})", task_id, fields_display)}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) fn handle_create_task(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateTaskWithEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(title = %parsed.title, epic_id = ?parsed.epic_id, "MCP create_task");

    let project_id = match resolve_project_id(&id, parsed.project_id, &*state.db) {
        Ok(pid) => pid,
        Err(resp) => return resp,
    };

    let svc = TaskService::new(state.db.clone());
    match svc.create_task(CreateTaskParams {
        title: parsed.title,
        description: parsed.description,
        repo_path: parsed.repo_path,
        plan_path: parsed.plan_path,
        epic_id: parsed.epic_id,
        sort_order: parsed.sort_order,
        tag: parsed.tag,
        base_branch: parsed.base_branch,
        project_id,
    }) {
        Ok(task_id) => {
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Task {task_id} created")}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) fn handle_get_task(state: &McpState, id: Option<Value>, args: Value) -> JsonRpcResponse {
    let parsed = match parse_args::<GetTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id, "MCP get_task");

    let svc = TaskService::new(state.db.clone());
    match svc.get_task(parsed.task_id) {
        Ok(task) => {
            let epic_titles = build_epic_titles(state);
            let text = format_task_detail(&task, &epic_titles);
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) fn handle_list_tasks(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListTasksArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(status = ?parsed.status, "MCP list_tasks");

    // Parse status filter (supports string or array) — this is JSON-RPC parsing logic
    let status_filter: Option<Vec<TaskStatus>> = match parsed.status {
        Some(Value::String(ref s)) => match TaskStatus::parse(s) {
            Some(st) => Some(vec![st]),
            None => {
                return JsonRpcResponse::err(
                    id,
                    -32602,
                    format!("Unknown status: {s}. Valid values: backlog, running, review, done"),
                );
            }
        },
        Some(Value::Array(ref arr)) => {
            let mut statuses = Vec::new();
            for v in arr {
                match v.as_str().and_then(TaskStatus::parse) {
                    Some(st) => statuses.push(st),
                    None => {
                        return JsonRpcResponse::err(
                            id,
                            -32602,
                            format!("Invalid status in array: {v}"),
                        );
                    }
                }
            }
            Some(statuses)
        }
        Some(_) => {
            return JsonRpcResponse::err(id, -32602, "status must be a string or array of strings");
        }
        None => None,
    };

    let svc = TaskService::new(state.db.clone());
    match svc.list_tasks(ListTasksFilter {
        statuses: status_filter,
        epic_id: parsed.epic_id.map(EpicId),
    }) {
        Ok(filtered) => {
            if filtered.is_empty() {
                return JsonRpcResponse::ok(
                    id,
                    json!({"content": [{"type": "text", "text": "No tasks found"}]}),
                );
            }
            let epic_titles = build_epic_titles(state);
            let lines: Vec<String> = filtered
                .iter()
                .map(|t| format_task_line(t, &epic_titles))
                .collect();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) fn handle_claim_task(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ClaimTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id, worktree = %parsed.worktree, "MCP claim_task");

    let svc = TaskService::new(state.db.clone());
    match svc.claim_task(ClaimTaskParams {
        task_id: parsed.task_id,
        worktree: parsed.worktree,
        tmux_window: parsed.tmux_window,
    }) {
        Ok(task) => {
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Task {} claimed: {}", parsed.task_id, task.title)}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_wrap_up(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<WrapUpArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id, action = ?parsed.action, "MCP wrap_up");

    let svc = TaskService::new(state.db.clone());
    let task = match svc.validate_wrap_up(parsed.task_id) {
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
    let tmux_window = task.tmux_window.clone();
    let runner = state.runner.clone();
    let notify_tx = state.notify_tx.clone();
    let task_id = task.id;

    match parsed.action {
        WrapUpAction::Rebase => {
            let db = state.db.clone();
            // Optimistically clear conflict sub_status before rebasing,
            // matching the TUI behavior.
            if task.sub_status == SubStatus::Conflict {
                let clear_patch =
                    db::TaskPatch::new().sub_status(SubStatus::default_for(task.status));
                let _ = db.patch_task(task_id, &clear_patch);
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
                    let patch = db::TaskPatch::new()
                        .status(TaskStatus::Done)
                        .tmux_window(None);
                    if let Err(e) = db.patch_task(task_id, &patch) {
                        tracing::warn!(
                            task_id = task_id.0,
                            "MCP wrap_up: failed to set task to done: {e}"
                        );
                    }
                    if let Some(epic_id) = task.epic_id {
                        if let Err(err) = db.recalculate_epic_status(epic_id) {
                            tracing::warn!(
                                "failed to recalculate epic status for epic {}: {err}",
                                epic_id.0
                            );
                        }
                    }
                    let ad_runner = state.runner.clone();
                    tokio::task::spawn_blocking(move || {
                        if let Some(window) = &tmux_window {
                            let _ = crate::tmux::kill_window(window, &*ad_runner);
                        }
                        if let Some(tx) = notify_tx {
                            let _ = tx.send(crate::mcp::McpEvent::Refresh);
                        }
                    });
                    JsonRpcResponse::ok(
                        id,
                        json!({"content": [{"type": "text", "text": format!("wrap_up complete (task {}, action: rebase)", parsed.task_id)}]}),
                    )
                }
                Err(e) => {
                    if matches!(e, dispatch::FinishError::RebaseConflict(_)) {
                        let patch = db::TaskPatch::new().sub_status(SubStatus::Conflict);
                        let _ = db.patch_task(task_id, &patch);
                    }
                    if let Some(tx) = notify_tx {
                        let _ = tx.send(crate::mcp::McpEvent::Refresh);
                    }
                    JsonRpcResponse::err(id, -32603, format!("wrap_up failed: {e}"))
                }
            }
        }
        WrapUpAction::Pr => {
            let db = state.db.clone();
            let pr_runner = runner.clone();
            let title = task.title.clone();
            let description = task.description.clone();
            let pr_result = match tokio::task::spawn_blocking(move || {
                tracing::info!(task_id = task_id.0, %branch, "MCP wrap_up pr starting");
                dispatch::create_pr(
                    &worktree,
                    &branch,
                    &title,
                    &description,
                    &base_branch,
                    &*pr_runner,
                )
            })
            .await
            {
                Ok(r) => r,
                Err(e) => return JsonRpcResponse::err(id, -32603, format!("internal error: {e}")),
            };

            match pr_result {
                Ok(result) => {
                    let patch = db::TaskPatch::new()
                        .status(TaskStatus::Review)
                        .pr_url(Some(result.pr_url.as_str()));
                    if let Err(e) = db.patch_task(task_id, &patch) {
                        tracing::warn!(
                            task_id = task_id.0,
                            "MCP wrap_up: failed to save PR fields: {e}"
                        );
                    }
                    if let Some(epic_id) = task.epic_id {
                        if let Err(err) = db.recalculate_epic_status(epic_id) {
                            tracing::warn!(
                                "failed to recalculate epic status for epic {}: {err}",
                                epic_id.0
                            );
                        }
                    }
                    let pr_url = result.pr_url.clone();
                    if let Some(tx) = notify_tx {
                        let _ = tx.send(crate::mcp::McpEvent::Refresh);
                    }
                    JsonRpcResponse::ok(
                        id,
                        json!({"content": [{"type": "text", "text": format!("wrap_up complete (task {}, action: pr, pr_url: {})", parsed.task_id, pr_url)}]}),
                    )
                }
                Err(e) => {
                    if let Some(tx) = notify_tx {
                        let _ = tx.send(crate::mcp::McpEvent::Refresh);
                    }
                    JsonRpcResponse::err(id, -32603, format!("wrap_up failed: {e}"))
                }
            }
        }
    }
}

fn do_dispatch(
    task: &crate::models::Task,
    db: &dyn crate::db::TaskStore,
    runner: &dyn crate::process::ProcessRunner,
) -> anyhow::Result<crate::models::DispatchResult> {
    let epic_ctx = dispatch::EpicContext::from_db(task, db);
    match DispatchMode::for_task(task) {
        DispatchMode::Dispatch => dispatch::dispatch_agent(task, runner, epic_ctx.as_ref()),
        DispatchMode::Brainstorm => dispatch::brainstorm_agent(task, runner, epic_ctx.as_ref()),
        DispatchMode::Plan => dispatch::plan_agent(task, runner, epic_ctx.as_ref()),
    }
}

pub(super) async fn handle_dispatch_next(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DispatchNextArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(epic_id = parsed.epic_id, "MCP dispatch_next");

    // Check auto_dispatch flag before doing any work
    match state.db.get_epic(crate::models::EpicId(parsed.epic_id)) {
        Ok(Some(epic)) if !epic.auto_dispatch => {
            return JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "auto dispatch is disabled for epic #{} — dispatch the next task manually",
                    parsed.epic_id
                )}]}),
            );
        }
        Ok(_) => {} // auto_dispatch is true, or epic not found — proceed normally
        Err(e) => {
            tracing::warn!(
                "dispatch_next: failed to fetch epic #{}: {e}",
                parsed.epic_id
            );
            // Don't block dispatch on a DB error reading the flag
        }
    }

    let svc = TaskService::new(state.db.clone());
    let next_task = match svc.next_backlog_task(parsed.epic_id) {
        Ok(Some(task)) => task,
        Ok(None) => {
            return JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "no backlog tasks to dispatch for epic #{}",
                    parsed.epic_id
                )}]}),
            );
        }
        Err(e) => return service_err_to_response(id, e),
    };

    let next_id = next_task.id;
    let next_title = next_task.title.clone();
    let db = state.db.clone();
    let runner = state.runner.clone();
    let notify_tx = state.notify_tx.clone();

    tokio::task::spawn_blocking(move || {
        let result = do_dispatch(&next_task, &*db, &*runner);

        match result {
            Ok(dispatch_result) => {
                let patch = db::TaskPatch::new()
                    .status(TaskStatus::Running)
                    .worktree(Some(&dispatch_result.worktree_path))
                    .tmux_window(Some(&dispatch_result.tmux_window));
                if let Err(e) = db.patch_task(next_id, &patch) {
                    tracing::warn!(
                        task_id = next_id.0,
                        "dispatch_next: failed to update task: {e}"
                    );
                }
                if let Some(epic_id) = next_task.epic_id {
                    if let Err(err) = db.recalculate_epic_status(epic_id) {
                        tracing::warn!(
                            "failed to recalculate epic status for epic {}: {err}",
                            epic_id.0
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(task_id = next_id.0, "dispatch_next: dispatch failed: {e:#}");
            }
        }

        if let Some(tx) = notify_tx {
            let _ = tx.send(crate::mcp::McpEvent::Refresh);
        }
    });

    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": format!(
            "dispatching task #{} '{}'",
            next_id.0, next_title
        )}]}),
    )
}

pub(super) async fn handle_dispatch_task(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DispatchTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let task_id = crate::models::TaskId(parsed.task_id);

    let task = match state.db.get_task(task_id) {
        Ok(Some(t)) => t,
        Ok(None) => {
            return service_err_to_response(
                id,
                crate::service::ServiceError::NotFound(format!("task #{} not found", task_id.0)),
            )
        }
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("db error: {e:#}")),
    };

    if task.status != TaskStatus::Backlog {
        return JsonRpcResponse::err(
            id,
            -32602,
            format!(
                "task #{} is not in backlog (current: {})",
                task_id.0, task.status
            ),
        );
    }

    let db = state.db.clone();
    let runner = state.runner.clone();
    let notify_tx = state.notify_tx.clone();
    let epic_id = task.epic_id;

    let result = tokio::task::spawn_blocking(move || do_dispatch(&task, &*db, &*runner)).await;

    match result {
        Ok(Ok(dr)) => {
            let patch = db::TaskPatch::new()
                .status(TaskStatus::Running)
                .worktree(Some(&dr.worktree_path))
                .tmux_window(Some(&dr.tmux_window));
            let _ = state.db.patch_task(task_id, &patch);
            if let Some(eid) = epic_id {
                let _ = state.db.recalculate_epic_status(eid);
            }
            if let Some(tx) = notify_tx {
                let _ = tx.send(crate::mcp::McpEvent::Refresh);
            }
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "dispatched task #{} — worktree: {}, tmux: {}",
                    task_id.0, dr.worktree_path, dr.tmux_window
                )}]}),
            )
        }
        Ok(Err(e)) => JsonRpcResponse::err(id, -32603, format!("dispatch failed: {e:#}")),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("dispatch join error: {e}")),
    }
}

pub(super) fn handle_send_message(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed: SendMessageArgs = match parse_args(&id, args) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let svc = TaskService::new(state.db.clone());
    let (from_task, to_task) =
        match svc.validate_send_message(parsed.from_task_id, parsed.to_task_id) {
            Ok(pair) => pair,
            Err(e) => return service_err_to_response(id, e),
        };

    let worktree = to_task.worktree.as_ref().expect("validated by service");
    let tmux_window = to_task.tmux_window.as_ref().expect("validated by service");

    // Write message to a uniquely-named file in target's worktree
    let messages_dir = format!("{worktree}/.claude-messages");
    if let Err(e) = std::fs::create_dir_all(&messages_dir) {
        return JsonRpcResponse::err(id, -32603, format!("failed to create messages dir: {e}"));
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("{}-{}.md", from_task.id.0, timestamp);
    let message_path = format!("{messages_dir}/{filename}");

    let message_content = format!(
        "[Message from task {}: \"{}\"]\n{}",
        from_task.id.0, from_task.title, parsed.body
    );
    if let Err(e) = std::fs::write(&message_path, &message_content) {
        return JsonRpcResponse::err(id, -32603, format!("failed to write message file: {e}"));
    }

    // Inject notification into the target's tmux window
    let notification = format!(
        "You received a message from task {}. Read .claude-messages/{} for the full content, then delete the file.",
        from_task.id.0, filename
    );
    if let Err(e) = crate::tmux::send_keys(tmux_window, &notification, &*state.runner) {
        let _ = std::fs::remove_file(&message_path);
        return JsonRpcResponse::err(
            id,
            -32603,
            format!("failed to send notification to target agent: {e}"),
        );
    }

    state.notify_message_sent(to_task.id);

    tracing::info!(
        from_task_id = parsed.from_task_id,
        to_task_id = parsed.to_task_id,
        "message sent between agents"
    );

    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": format!(
            "Message sent to task {} ({})",
            to_task.id.0, to_task.title
        )}]}),
    )
}

// ---------------------------------------------------------------------------
// update_review_status
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct UpdateReviewStatusArgs {
    repo: String,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    number: i64,
    status: crate::models::ReviewAgentStatus,
}

pub(super) fn handle_update_review_status(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed: UpdateReviewStatusArgs = match parse_args(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };

    match state
        .db
        .update_agent_status(&parsed.repo, parsed.number, Some(parsed.status.as_db_str()))
    {
        Ok(_table) => {
            state.notify();
            if parsed.status == crate::models::ReviewAgentStatus::FindingsReady {
                // Move the workflow item to ActionRequired so the board reflects the new state
                if let Some(kind) = find_workflow_kind_for(&state.db, &parsed.repo, parsed.number) {
                    let _ = state.db.upsert_pr_workflow(
                        &parsed.repo,
                        parsed.number,
                        kind,
                        crate::models::ReviewWorkflowState::ActionRequired.as_db_str(),
                        Some(crate::models::ReviewWorkflowSubState::FindingsReady.as_db_str()),
                    );
                }
            }
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "Updated agent status for {}#{} to {}",
                    parsed.repo, parsed.number, parsed.status.as_db_str()
                )}]}),
            )
        }
        Err(e) => JsonRpcResponse::err(id, -32602, format!("Failed: {e}")),
    }
}

fn find_workflow_kind_for(
    db: &std::sync::Arc<dyn crate::db::TaskStore>,
    repo: &str,
    number: i64,
) -> Option<crate::models::WorkflowItemKind> {
    db.find_pr_workflow_kind(repo, number).ok().flatten()
}

pub(super) fn handle_report_usage(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ReportUsageArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(
        task_id = parsed.task_id,
        cost_usd = parsed.cost_usd,
        "MCP report_usage"
    );

    let svc = TaskService::new(state.db.clone());
    match svc.report_usage(
        parsed.task_id,
        &crate::models::UsageReport {
            cost_usd: parsed.cost_usd,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            cache_read_tokens: parsed.cache_read_tokens,
            cache_write_tokens: parsed.cache_write_tokens,
        },
    ) {
        Ok(()) => {
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Usage recorded for task {}", parsed.task_id)}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}
