use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::EpicPatch;
use crate::models::{EpicId, TaskStatus};
use crate::mcp::McpState;

use super::types::{JsonRpcResponse, deserialize_flexible_i64, deserialize_optional_flexible_i64, parse_args};
use super::validation;

// ---------------------------------------------------------------------------
// Typed argument structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct CreateEpicArgs {
    pub(super) title: String,
    pub(super) repo_path: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct GetEpicArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) epic_id: i64,
}

#[derive(Deserialize)]
pub(super) struct UpdateEpicArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) epic_id: i64,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) plan: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default)]
    pub(super) repo_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Epic tool handlers
// ---------------------------------------------------------------------------

pub(super) fn handle_create_epic(state: &McpState, id: Option<Value>, args: Value) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(title = %parsed.title, "MCP create_epic");

    match state.db.create_epic(&parsed.title, &parsed.description, &parsed.repo_path) {
        Ok(epic) => {
            if let Some(so) = parsed.sort_order {
                if let Err(e) = state.db.patch_epic(epic.id, &EpicPatch::new().sort_order(Some(so))) {
                    return JsonRpcResponse::err(id, -32603, format!("Failed to set sort_order: {e}"));
                }
            }
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Epic {} created: {}", epic.id, epic.title)}]}),
            )
        }
        Err(e) => JsonRpcResponse::err(id, -32603, format!("Database error: {e}")),
    }
}

pub(super) fn handle_get_epic(state: &McpState, id: Option<Value>, args: Value) -> JsonRpcResponse {
    let parsed = match parse_args::<GetEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(epic_id = parsed.epic_id, "MCP get_epic");

    match state.db.get_epic(EpicId(parsed.epic_id)) {
        Ok(Some(epic)) => {
            let subtasks = state.db.list_tasks_for_epic(epic.id).unwrap_or_else(|e| {
                tracing::warn!(epic_id = epic.id.0, "failed to list subtasks for epic detail: {e}");
                Vec::new()
            });
            let done_count = subtasks.iter().filter(|t| t.status == TaskStatus::Done).count();
            let total = subtasks.len();
            let mut text = format!(
                "Epic {id}: {title}\nDescription: {desc}\nRepo: {repo}\nStatus: {status}",
                id = epic.id,
                title = epic.title,
                desc = epic.description,
                repo = epic.repo_path,
                status = epic.status.as_str(),
            );
            if let Some(ref p) = epic.plan {
                text.push_str(&format!("\nPlan: {p}"));
            }
            if let Some(sort_order) = epic.sort_order {
                text.push_str(&format!("\nSort order: {sort_order}"));
            }
            text.push_str(&format!("\nCreated: {}", epic.created_at.format("%Y-%m-%d %H:%M:%S UTC")));
            text.push_str(&format!("\nUpdated: {}", epic.updated_at.format("%Y-%m-%d %H:%M:%S UTC")));
            text.push_str(&format!("\nSubtasks: {done_count}/{total} done"));
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
        Ok(None) => JsonRpcResponse::err(id, -32602, format!("Epic {} not found", parsed.epic_id)),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("Database error: {e}")),
    }
}

pub(super) fn handle_list_epics(state: &McpState, id: Option<Value>, _args: Value) -> JsonRpcResponse {
    tracing::info!("MCP list_epics");

    match state.db.list_epics() {
        Ok(epics) => {
            if epics.is_empty() {
                return JsonRpcResponse::ok(
                    id,
                    json!({"content": [{"type": "text", "text": "No epics found"}]}),
                );
            }
            let lines: Vec<String> = epics.iter().map(|e| {
                let subtasks = state.db.list_tasks_for_epic(e.id).unwrap_or_else(|err| {
                    tracing::warn!(epic_id = e.id.0, "failed to list subtasks for epic listing: {err}");
                    Vec::new()
                });
                let done = subtasks.iter().filter(|t| t.status == TaskStatus::Done).count();
                let plan_indicator = if e.plan.is_some() { " [plan]" } else { "" };
                let status_indicator = if e.status != TaskStatus::Backlog { format!(" [{}]", e.status.as_str()) } else { String::new() };
                format!("- [{}] {} ({}/{} done){}{}: {}", e.id, e.title, done, subtasks.len(), plan_indicator, status_indicator, e.description)
            }).collect();
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": lines.join("\n")}]}))
        }
        Err(e) => JsonRpcResponse::err(id, -32603, format!("Database error: {e}")),
    }
}

pub(super) fn handle_update_epic(state: &McpState, id: Option<Value>, args: Value) -> JsonRpcResponse {
    let parsed = match parse_args::<UpdateEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(epic_id = parsed.epic_id, "MCP update_epic");

    if let Err(resp) = validation::require_some_update(
        &[
            ("title", parsed.title.is_some()),
            ("description", parsed.description.is_some()),
            ("status", parsed.status.is_some()),
            ("plan", parsed.plan.is_some()),
            ("sort_order", parsed.sort_order.is_some()),
            ("repo_path", parsed.repo_path.is_some()),
        ],
        &id,
    ) {
        return resp;
    }

    let repo_path = parsed.repo_path.as_deref().map(crate::models::expand_tilde);
    let mut patch = EpicPatch::new();
    if let Some(ref t) = parsed.title { patch = patch.title(t); }
    if let Some(ref d) = parsed.description { patch = patch.description(d); }
    if let Some(ref s) = parsed.status {
        match validation::parse_status_or_error(s, &id) {
            Ok(status) => { patch = patch.status(status); }
            Err(resp) => return resp,
        }
    }
    if let Some(ref p) = parsed.plan { patch = patch.plan(Some(p.as_str())); }
    if let Some(so) = parsed.sort_order { patch = patch.sort_order(Some(so)); }
    if let Some(ref rp) = repo_path {
        patch = patch.repo_path(rp);
    }

    if let Err(e) = state.db.patch_epic(EpicId(parsed.epic_id), &patch) {
        return JsonRpcResponse::err(id, -32603, format!("Database error: {e}"));
    }

    state.notify();
    let mut updated = Vec::new();
    if parsed.title.is_some() { updated.push("title"); }
    if parsed.description.is_some() { updated.push("description"); }
    if parsed.status.is_some() { updated.push("status"); }
    if parsed.plan.is_some() { updated.push("plan"); }
    if parsed.sort_order.is_some() { updated.push("sort_order"); }
    if parsed.repo_path.is_some() { updated.push("repo_path"); }

    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": format!("Epic {} updated ({})", parsed.epic_id, updated.join(", "))}]}),
    )
}
