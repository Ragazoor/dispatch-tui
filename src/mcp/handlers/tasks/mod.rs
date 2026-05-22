use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::mcp::McpState;
use crate::models::{EpicId, LearningVerdict, ProjectId, SubStatus, Task, TaskStatus, TaskTag, WrapUpMode};

// Promoted to pub(super) so sub-modules can `use super::{parse_args, ...}`
pub(super) use super::types::{
    deserialize_flexible_i64, deserialize_nullable_flexible_i64, deserialize_nullable_wrap_up_mode,
    deserialize_optional_flexible_i64, fetch_caller_task, parse_args, service_err_to_response,
    JsonRpcResponse, StatusFilter,
};

mod crud;
mod dispatch;
mod verify;
mod wrap_up;

pub(super) use crud::{
    handle_create_task, handle_get_task, handle_list_tasks, handle_query_usage, handle_update_task,
};
pub(super) use dispatch::{
    handle_claim_task, handle_dispatch_next, handle_dispatch_task, handle_send_message,
};
pub(super) use verify::handle_set_verify_command;
pub(super) use wrap_up::{handle_exit_session, handle_wrap_up};

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
    #[serde(default, deserialize_with = "deserialize_nullable_wrap_up_mode")]
    pub(super) wrap_up_mode: Option<Option<WrapUpMode>>,
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
pub(super) struct CreateTaskWithEpicArgs {
    pub(super) title: String,
    pub(super) repo_path: String,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) project_id: Option<i64>,
    #[serde(default)]
    pub(super) description: String,
    pub(super) plan_path: Option<String>,
    /// Double-Option distinguishes "absent" (→ outer None: inherit from
    /// CallerIdentity if Task) from "explicit null" (→ Some(None): clear /
    /// no epic).
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_i64")]
    pub(super) epic_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default)]
    pub(super) tag: Option<TaskTag>,
    #[serde(default)]
    pub(super) base_branch: Option<String>,
    #[serde(default)]
    pub(super) wrap_up_mode: Option<WrapUpMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum WrapUpAction {
    Rebase,
    Done,
    Pr,
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
    #[serde(default)]
    pub(super) pr_url: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ExitSessionArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    #[serde(default)]
    pub(super) token: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SetVerifyCommandArgs {
    pub(super) repo_path: String,
    pub(super) command: Option<String>,
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

#[derive(Deserialize)]
pub(super) struct QueryUsageArgs {
    #[serde(default)]
    pub(super) category: Option<String>,
    #[serde(default)]
    pub(super) actor: Option<String>,
    #[serde(default)]
    pub(super) since: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<i64>,
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
    if let Some(wrap_up_mode) = task.wrap_up_mode {
        text.push_str(&format!("\nWrap-up mode: {wrap_up_mode}"));
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

async fn plan_goal(path: &str) -> Option<String> {
    let content = tokio::fs::read_to_string(path).await.ok()?;
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
