use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::db;
use crate::dispatch;
use crate::mcp::McpState;
use crate::models::{
    DispatchMode, EpicId, LearningId, LearningVerdict, ProjectId, SubStatus, Task, TaskId,
    TaskStatus, TaskTag,
};
use crate::service::{
    ClaimTaskParams, CreateTaskParams, FieldUpdate, LearningService, ListTasksFilter, ServiceError,
    TaskService, UpdateTaskParams,
};

use super::types::{
    deserialize_flexible_i64, deserialize_nullable_flexible_i64, deserialize_optional_flexible_i64,
    parse_args, service_err_to_response, JsonRpcResponse, StatusFilter,
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
    pub(super) status: Option<StatusFilter>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) epic_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) caller_task_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) project_id: Option<i64>,
    #[serde(default)]
    pub(super) repo_paths: Option<Vec<String>>,
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
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) project_id: Option<i64>,
    #[serde(default)]
    pub(super) description: String,
    pub(super) plan_path: Option<String>,
    /// Double-Option distinguishes "absent" (→ outer None: inherit from
    /// caller_task_id if any) from "explicit null" (→ Some(None): clear /
    /// no epic).
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_i64")]
    pub(super) epic_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default)]
    pub(super) tag: Option<TaskTag>,
    #[serde(default)]
    pub(super) base_branch: Option<String>,
    /// Task ID of the dispatched agent calling this tool. When set, a
    /// missing project_id inherits from this task; an absent epic_id
    /// inherits from this task's epic (if any).
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) caller_task_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum WrapUpAction {
    Rebase,
}

#[derive(Debug, Deserialize)]
pub(super) struct VerdictArg {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) learning_id: i64,
    pub(super) verdict: LearningVerdict,
}

#[derive(Deserialize)]
pub(super) struct WrapUpArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    pub(super) action: WrapUpAction,
    #[serde(default)]
    pub(super) learning_verdicts: Option<Vec<VerdictArg>>,
}

#[derive(Deserialize)]
pub(super) struct ExitSessionArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    #[serde(default)]
    pub(super) has_learnings: Option<bool>,
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

async fn build_epic_titles(state: &McpState) -> HashMap<EpicId, String> {
    state
        .db
        .list_epics()
        .await
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

fn plan_goal(path: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let description = crate::plan::parse_plan(&content).ok()?.description;
    (!description.is_empty()).then_some(description)
}

fn description_preview(s: &str) -> String {
    if s.len() > 200 {
        let end = s
            .char_indices()
            .take_while(|(i, _)| *i < 200)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &s[..end])
    } else {
        s.to_owned()
    }
}

fn format_task_line(t: &Task, epic_titles: &HashMap<EpicId, String>, goal: &str) -> String {
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
    let pr_part = t
        .pr_url
        .as_deref()
        .map(|url| format!(" | PR: {url}"))
        .unwrap_or_default();
    let goal_part = if goal.is_empty() {
        String::new()
    } else {
        format!(" | Goal: {goal}")
    };
    format!(
        "- [{}] {} ({}/{}){}{}{}{}",
        t.id,
        t.title,
        t.status.as_str(),
        t.sub_status.as_str(),
        tag_indicator,
        epic_indicator,
        pr_part,
        goal_part,
    )
}

// ---------------------------------------------------------------------------
// Task tool handlers (thin wrappers over TaskService)
// ---------------------------------------------------------------------------

/// Look up a `caller_task_id` and map "not found" / DB errors to the
/// JSON-RPC errors agents see (-32602 / -32603). Shared by create_task and
/// list_tasks so the two handlers stay in sync.
async fn lookup_caller_task(
    state: &McpState,
    id: &Option<Value>,
    caller_id: i64,
) -> Result<Task, JsonRpcResponse> {
    match state.db.get_task(TaskId(caller_id)).await {
        Ok(Some(t)) => Ok(t),
        Ok(None) => Err(JsonRpcResponse::err(
            id.clone(),
            -32602,
            format!("Unknown caller_task_id: {caller_id}"),
        )),
        Err(e) => Err(JsonRpcResponse::err(
            id.clone(),
            -32603,
            format!("Database error: {e}"),
        )),
    }
}

async fn validate_project_id(
    state: &McpState,
    id: &Option<Value>,
    project_id: i64,
) -> Result<(), JsonRpcResponse> {
    if state
        .db
        .list_projects()
        .await
        .unwrap_or_default()
        .iter()
        .any(|p| p.id == ProjectId(project_id))
    {
        return Ok(());
    }
    Err(service_err_to_response(
        id.clone(),
        crate::service::ServiceError::Validation(format!("project {project_id} does not exist")),
    ))
}

pub(super) async fn handle_update_task(
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

    let mut params = UpdateTaskParams::for_task(TaskId(parsed.task_id))
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
    if let Some(fu) = FieldUpdate::from_optional_string(parsed.pr_url) {
        params = params.pr_url(fu);
    }
    if let Some(sub_status) = parsed.sub_status {
        params = params.sub_status(sub_status);
    }
    if let Some(epic_id) = parsed.epic_id {
        params = params.epic_id(EpicId(epic_id));
    }
    if let Some(project_id) = parsed.project_id {
        if let Err(resp) = validate_project_id(state, &id, project_id).await {
            return resp;
        }
        params = params.project_id(ProjectId(project_id));
    }
    let fields_display = params.updated_field_names().join(", ");

    let svc = TaskService::new(state.db.clone());
    match svc.update_task(params).await {
        Ok(result) => {
            state.notify_task_changed(TaskId(parsed.task_id));
            let nudge = if result.was_pr_finalisation {
                reflection_nudge(&*state.db).await
            } else {
                ""
            };
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Task {} updated ({}){}", result.task_id, fields_display, nudge)}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_create_task(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateTaskWithEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(
        title = %parsed.title,
        epic_id = ?parsed.epic_id,
        project_id = ?parsed.project_id,
        caller_task_id = ?parsed.caller_task_id,
        "MCP create_task"
    );

    if parsed.project_id.is_none() && parsed.caller_task_id.is_none() {
        return JsonRpcResponse::err(
            id,
            -32602,
            "must provide project_id or caller_task_id".to_string(),
        );
    }

    // Look up the caller task once, if provided, so we can inherit fields.
    let caller = if let Some(caller_id) = parsed.caller_task_id {
        match lookup_caller_task(state, &id, caller_id).await {
            Ok(t) => Some(t),
            Err(resp) => return resp,
        }
    } else {
        None
    };

    // Effective project: explicit > caller's. The earlier guard ensures at
    // least one source is available, so the chain below cannot yield None.
    let effective_project_id = parsed
        .project_id
        .or_else(|| caller.as_ref().map(|c| c.project_id.0))
        .unwrap_or_else(|| unreachable!("guard above ensures project_id or caller_task_id is set"));

    if let Err(resp) = validate_project_id(state, &id, effective_project_id).await {
        return resp;
    }

    // Effective epic: explicit (incl. JSON null) > caller's epic > None.
    let effective_epic_id: Option<EpicId> = match parsed.epic_id {
        Some(inner) => inner.map(EpicId),
        None => caller.as_ref().and_then(|c| c.epic_id),
    };

    let svc = TaskService::new(state.db.clone());
    match svc
        .create_task(CreateTaskParams {
            title: parsed.title,
            description: parsed.description,
            repo_path: parsed.repo_path,
            plan_path: parsed.plan_path,
            epic_id: effective_epic_id,
            sort_order: parsed.sort_order,
            tag: parsed.tag,
            base_branch: parsed.base_branch,
            project_id: ProjectId(effective_project_id),
        })
        .await
    {
        Ok(task_id) => {
            state.notify_task_changed(task_id);
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Task {task_id} created")}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_get_task(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<GetTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id, "MCP get_task");

    let svc = TaskService::new(state.db.clone());
    match svc.get_task(TaskId(parsed.task_id)).await {
        Ok(task) => {
            let epic_titles = build_epic_titles(state).await;
            let text = format_task_detail(&task, &epic_titles);
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_list_tasks(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListTasksArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(status = ?parsed.status, "MCP list_tasks");

    let status_filter: Option<Vec<TaskStatus>> = parsed.status.map(StatusFilter::into_vec);

    let (derived_epic_id, derived_project_id, exclude_task_id) = if let Some(caller_id) =
        parsed.caller_task_id
    {
        let caller = match lookup_caller_task(state, &id, caller_id).await {
            Ok(t) => t,
            Err(resp) => return resp,
        };
        let has_explicit_scope =
            parsed.epic_id.is_some() || parsed.project_id.is_some() || parsed.repo_paths.is_some();
        let (epic, proj) = if has_explicit_scope {
            (None, None)
        } else if let Some(eid) = caller.epic_id {
            (Some(eid), None)
        } else {
            (None, Some(caller.project_id))
        };
        (epic, proj, Some(caller.id))
    } else {
        (None, None, None)
    };

    let epic_id = parsed.epic_id.map(EpicId).or(derived_epic_id);
    let project_id = parsed.project_id.map(ProjectId).or(derived_project_id);

    let svc = TaskService::new(state.db.clone());
    match svc
        .list_tasks(ListTasksFilter {
            statuses: status_filter,
            epic_id,
            project_id,
            repo_paths: parsed.repo_paths,
            exclude_task_id,
        })
        .await
    {
        Ok(filtered) => {
            if filtered.is_empty() {
                return JsonRpcResponse::ok(
                    id,
                    json!({"content": [{"type": "text", "text": "No tasks found"}]}),
                );
            }
            let epic_titles = build_epic_titles(state).await;
            // Read each unique plan file once to avoid repeated blocking I/O per task.
            let plan_goals: HashMap<String, String> = {
                let mut cache = HashMap::new();
                for t in &filtered {
                    if let Some(path) = t.plan_path.as_deref() {
                        cache
                            .entry(path.to_owned())
                            .or_insert_with(|| plan_goal(path).unwrap_or_default());
                    }
                }
                cache
            };
            let lines: Vec<String> = filtered
                .iter()
                .map(|t| {
                    let goal = match t.plan_path.as_deref().and_then(|p| plan_goals.get(p)) {
                        Some(g) if !g.is_empty() => g.clone(),
                        _ => description_preview(&t.description),
                    };
                    format_task_line(t, &epic_titles, &goal)
                })
                .collect();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_claim_task(
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
    match svc
        .claim_task(ClaimTaskParams {
            task_id: TaskId(parsed.task_id),
            worktree: parsed.worktree,
            tmux_window: parsed.tmux_window,
        })
        .await
    {
        Ok(task) => {
            state.notify_task_changed(task.id);
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Task {} claimed: {}", parsed.task_id, task.title)}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

async fn reflection_nudge(db: &dyn crate::db::TaskStore) -> &'static str {
    let enabled = db
        .get_setting_bool("learning_reflection_enabled")
        .await
        .unwrap_or(None)
        .unwrap_or(true);
    if enabled {
        " Before finishing, did you discover anything non-obvious about \
this repo or task? If so, call record_learning with a brief summary."
    } else {
        ""
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
    let task = match svc.validate_wrap_up(TaskId(parsed.task_id)).await {
        Ok(t) => t,
        Err(e) => return service_err_to_response(id, e),
    };

    // Apply learning verdicts BEFORE the rebase. The agent's evaluation of
    // surfaced knowledge is independent of branch state — if the rebase fails,
    // the verdicts have still been recorded against the retrieval rows that
    // existed when the agent decided to wrap up.
    if let Some(vs) = parsed.learning_verdicts {
        let parsed_verdicts: Vec<(LearningId, LearningVerdict)> = vs
            .into_iter()
            .map(|v| (LearningId(v.learning_id), v.verdict))
            .collect();
        let learning_svc = LearningService::new(state.db.clone());
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

    // Only WrapUpAction::Rebase is supported; PR creation is agent-driven
    // (see plugin/skills/wrap-up/SKILL.md).
    let db = state.db.clone();
    // Optimistically clear conflict sub_status before rebasing,
    // matching the TUI behavior.
    if task.sub_status == SubStatus::Conflict {
        let clear_patch = db::TaskPatch::new().sub_status(SubStatus::default_for(task.status));
        let _ = db.patch_task(task_id, &clear_patch).await;
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
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "wrap_up complete (task {}, action: rebase). The session is NOT yet closed. \
                You MUST call `exit_session` next as your final action — without it, the tmux window stays alive \
                and the task remains in its current status. Do not stop, and do not call any other tool first.",
                    parsed.task_id
                )}]}),
            )
        }
        Err(e) => {
            if matches!(e, dispatch::FinishError::RebaseConflict(_)) {
                let patch = db::TaskPatch::new().sub_status(SubStatus::Conflict);
                let _ = db.patch_task(task_id, &patch).await;
            }
            if let Some(tx) = notify_tx {
                let _ = tx.send(crate::mcp::McpEvent::TaskChanged(task_id));
            }
            JsonRpcResponse::err(id, -32603, format!("wrap_up failed: {e}"))
        }
    }
}

pub(super) async fn handle_exit_session(
    state: &McpState,
    id: Option<Value>,
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

    if task.tmux_window.is_none() {
        return JsonRpcResponse::err(
            id,
            -32602,
            format!("task #{} has no active session", parsed.task_id),
        );
    }

    // Stateless three-way dispatch driven by `has_learnings`:
    //   None       → ask the agent to reflect (call again with true/false)
    //   Some(true) → instruct agent to record_learning, then call again
    //   Some(false)→ close the session
    match parsed.has_learnings {
        None => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": "\
Before closing, reflect on this session.\n\
\n\
Did you encounter any of the following?\n\
  \u{2022} A pitfall \u{2014} something that wasted time or caused surprise\n\
  \u{2022} A convention \u{2014} a pattern worth following consistently\n\
  \u{2022} A tool tip or preference\n\
\n\
Call exit_session(has_learnings=true) if yes, or exit_session(has_learnings=false) to close now."}]}),
        ),
        Some(true) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": "\
Record each finding with record_learning before closing:\n\
  \u{2022} kind: pitfall | convention | preference | tool_recommendation\n\
  \u{2022} summary: one clear sentence\n\
  \u{2022} scope: repo | project | user | epic | task\n\
\n\
Check query_learnings first to avoid duplicates.\n\
Call exit_session(has_learnings=false) when done to close the session."}]}),
        ),
        Some(false) => {
            let patch = crate::db::TaskPatch::new()
                .status(TaskStatus::Done)
                .sub_status(SubStatus::default_for(TaskStatus::Done))
                .tmux_window(None);
            if let Err(e) = state.db.patch_task(task_id, &patch).await {
                tracing::warn!(
                    task_id = task_id.0,
                    "exit_session: failed to apply closing patch: {e}"
                );
            }
            if let Some(epic_id) = task.epic_id {
                if let Err(err) = state.db.recalculate_epic_status(epic_id).await {
                    tracing::warn!(
                        "failed to recalculate epic status for epic {}: {err}",
                        epic_id.0
                    );
                }
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
    }
}

fn do_dispatch(
    task: &crate::models::Task,
    runner: &dyn crate::process::ProcessRunner,
    project_ctx: dispatch::ProjectContext,
    epic_ctx: Option<dispatch::EpicContext>,
    procedural: &[crate::models::Learning],
    tiered: &[crate::models::Learning],
) -> anyhow::Result<crate::models::DispatchResult> {
    let injections = dispatch::LearningInjections {
        procedural: procedural.iter().collect(),
        tiered: tiered.iter().collect(),
    };
    match DispatchMode::for_task(task) {
        DispatchMode::Dispatch => dispatch::dispatch_agent(
            task,
            runner,
            epic_ctx.as_ref(),
            Some(&project_ctx),
            &injections,
        ),
        DispatchMode::PrReview => {
            dispatch::pr_review_agent(task, runner, epic_ctx.as_ref(), Some(&project_ctx))
        }
        DispatchMode::Research => {
            dispatch::research_agent(task, runner, epic_ctx.as_ref(), Some(&project_ctx))
        }
        DispatchMode::Fix => {
            dispatch::fix_task_agent(task, runner, epic_ctx.as_ref(), Some(&project_ctx))
        }
        DispatchMode::Dependabot => {
            dispatch::dependabot_review_agent(task, runner, epic_ctx.as_ref(), Some(&project_ctx))
        }
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
    match state
        .db
        .get_epic(crate::models::EpicId(parsed.epic_id))
        .await
    {
        Ok(Some(epic)) if !epic.auto_dispatch => {
            return JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!(
                    "auto dispatch is disabled for epic #{} — dispatch the next task manually",
                    parsed.epic_id
                )}]}),
            );
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            return JsonRpcResponse::err(id, -32602, format!("epic #{} not found", parsed.epic_id));
        }
        Err(e) => {
            tracing::warn!(
                "dispatch_next: failed to fetch epic #{}: {e}",
                parsed.epic_id
            );
            // Don't block dispatch on a DB error reading the flag
        }
    }

    let svc = TaskService::new(state.db.clone());
    let next_task = match svc.next_backlog_task(EpicId(parsed.epic_id)).await {
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
    let next_epic_id = next_task.epic_id;
    let project_ctx = dispatch::ProjectContext::from_db(&next_task, &*state.db).await;
    let epic_ctx = dispatch::EpicContext::from_db(&next_task, &*state.db).await;
    let db = state.db.clone();
    let runner = state.runner.clone();
    let notify_tx = state.notify_tx.clone();

    let (procedural, tiered) = dispatch::build_and_record_injections(&*db, &next_task).await;

    tokio::spawn(async move {
        let next_task_for_blocking = next_task.clone();
        let result = tokio::task::spawn_blocking(move || {
            do_dispatch(
                &next_task_for_blocking,
                &*runner,
                project_ctx,
                epic_ctx,
                &procedural,
                &tiered,
            )
        })
        .await;

        match result {
            Ok(Ok(dispatch_result)) => {
                // Seed last_pre_tool_use_at so ClassifyAgentActivity treats
                // the freshly running task as Active until the agent's first
                // PreToolUse hook fires — otherwise the TUI tick flickers it
                // into Stale.
                let patch = db::TaskPatch::new()
                    .status(TaskStatus::Running)
                    .worktree(Some(&dispatch_result.worktree_path))
                    .tmux_window(Some(&dispatch_result.tmux_window))
                    .last_pre_tool_use_at(Some(chrono::Utc::now()));
                if let Err(e) = db.patch_task(next_id, &patch).await {
                    tracing::warn!(
                        task_id = next_id.0,
                        "dispatch_next: failed to update task: {e}"
                    );
                }
                if let Some(epic_id) = next_epic_id {
                    if let Err(err) = db.recalculate_epic_status(epic_id).await {
                        tracing::warn!(
                            "failed to recalculate epic status for epic {}: {err}",
                            epic_id.0
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(task_id = next_id.0, "dispatch_next: dispatch failed: {e:#}");
            }
            Err(e) => {
                tracing::warn!(
                    task_id = next_id.0,
                    "dispatch_next: blocking task panicked: {e}"
                );
            }
        }

        if let Some(tx) = notify_tx {
            let _ = tx.send(crate::mcp::McpEvent::TaskChanged(next_id));
            if let Some(epic_id) = next_epic_id {
                let _ = tx.send(crate::mcp::McpEvent::EpicChanged(epic_id));
            }
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

    let task = match state.db.get_task(task_id).await {
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

    let project_ctx = dispatch::ProjectContext::from_db(&task, &*state.db).await;
    let epic_ctx = dispatch::EpicContext::from_db(&task, &*state.db).await;
    let db = state.db.clone();
    let runner = state.runner.clone();
    let notify_tx = state.notify_tx.clone();
    let epic_id = task.epic_id;

    let (procedural, tiered) = dispatch::build_and_record_injections(&*db, &task).await;
    let result = tokio::task::spawn_blocking(move || {
        do_dispatch(&task, &*runner, project_ctx, epic_ctx, &procedural, &tiered)
    })
    .await;

    match result {
        Ok(Ok(dr)) => {
            // Seed last_pre_tool_use_at so ClassifyAgentActivity treats the
            // freshly running task as Active until the agent's first
            // PreToolUse hook fires.
            let patch = db::TaskPatch::new()
                .status(TaskStatus::Running)
                .worktree(Some(&dr.worktree_path))
                .tmux_window(Some(&dr.tmux_window))
                .last_pre_tool_use_at(Some(chrono::Utc::now()));
            let _ = state.db.patch_task(task_id, &patch).await;
            if let Some(eid) = epic_id {
                let _ = state.db.recalculate_epic_status(eid).await;
            }
            if let Some(tx) = notify_tx {
                let _ = tx.send(crate::mcp::McpEvent::TaskChanged(task_id));
                if let Some(eid) = epic_id {
                    let _ = tx.send(crate::mcp::McpEvent::EpicChanged(eid));
                }
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

pub(super) async fn handle_send_message(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed: SendMessageArgs = match parse_args(&id, args) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let svc = TaskService::new(state.db.clone());
    let (from_task, to_task) = match svc
        .validate_send_message(TaskId(parsed.from_task_id), TaskId(parsed.to_task_id))
        .await
    {
        Ok(pair) => pair,
        Err(e) => return service_err_to_response(id, e),
    };

    let Some(worktree) = to_task.worktree.as_ref() else {
        return JsonRpcResponse::err(id, -32603, "target task has no worktree (internal error)");
    };
    let Some(tmux_window) = to_task.tmux_window.as_ref() else {
        return JsonRpcResponse::err(
            id,
            -32603,
            "target task has no tmux window (internal error)",
        );
    };

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

pub(super) async fn handle_report_usage(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ReportUsageArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(task_id = parsed.task_id, "MCP report_usage");

    let svc = TaskService::new(state.db.clone());
    match svc
        .report_usage(
            TaskId(parsed.task_id),
            &crate::models::UsageReport {
                input_tokens: parsed.input_tokens,
                output_tokens: parsed.output_tokens,
                cache_read_tokens: parsed.cache_read_tokens,
                cache_write_tokens: parsed.cache_write_tokens,
            },
        )
        .await
    {
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
