use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{Epic, EpicId, TaskStatus, Visibility};
use crate::service::{CreateEpicParams, UpdateEpicParams};

use super::types::{
    deserialize_flexible_id, deserialize_nullable_flexible_i64, deserialize_nullable_flexible_id,
    deserialize_nullable_string, deserialize_optional_flexible_i64,
    deserialize_optional_flexible_id, deserialize_optional_limit, parse_args, reject_alongside,
    service_err_to_response, tool_error, JsonRpcResponse,
};

// ---------------------------------------------------------------------------
// Typed argument structs (JSON-RPC layer)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateEpicArgs {
    pub(super) title: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) parent_epic_id: Option<EpicId>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) workspace_id: Option<i64>,
    #[serde(default)]
    pub(super) visibility: Option<Visibility>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListEpicsArgs {
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) epic_id: Option<EpicId>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) parent_epic_id: Option<EpicId>,
    #[serde(default)]
    pub(super) recursive: Option<bool>,
    #[serde(default)]
    pub(super) query: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    pub(super) limit: Option<std::num::NonZeroUsize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateEpicArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) epic_id: EpicId,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) status: Option<TaskStatus>,
    #[serde(default)]
    pub(super) plan_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub(super) feed_command: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_i64")]
    pub(super) feed_interval_secs: Option<Option<i64>>,
    #[serde(default)]
    pub(super) group_by_repo: Option<bool>,
    #[serde(default)]
    pub(super) feed_append_only: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_id")]
    pub(super) parent_epic_id: Option<Option<EpicId>>,
    #[serde(default)]
    pub(super) visibility: Option<Visibility>,
}

// ---------------------------------------------------------------------------
// Epic tool handlers (thin wrappers over EpicService)
// ---------------------------------------------------------------------------

pub(super) async fn handle_create_epic(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(title = %parsed.title, "MCP create_epic");

    // `workspaces.allium: NameWorkspaceAndVisibilityViaMcp`. A sub-epic takes
    // its parent's workspace, so naming one is refused rather than ignored.
    let workspace_id = match (parsed.parent_epic_id, parsed.workspace_id) {
        (Some(_), Some(_)) => {
            return tool_error(
                id,
                "workspace_id is for a root epic; a sub-epic takes its parent's workspace",
            )
        }
        (Some(_), None) => 0,
        (None, Some(workspace_id)) => workspace_id,
        (None, None) => {
            return tool_error(
                id,
                format!(
                    "a root epic needs a workspace_id. {}",
                    workspace_choices(state).await
                ),
            )
        }
    };
    if workspace_id != 0 {
        if let Some(refusal) = unknown_workspace(state, workspace_id).await {
            return tool_error(id, refusal);
        }
    }
    let created = state
        .epic_svc
        .create_epic(CreateEpicParams {
            title: parsed.title,
            description: parsed.description,
            sort_order: parsed.sort_order,
            parent_epic_id: parsed.parent_epic_id,
            feed_command: None,
            feed_interval_secs: None,
            workspace_id,
            visibility: parsed.visibility.unwrap_or_default(),
        })
        .await;
    let epic = match created {
        Ok(epic) => epic,
        Err(e) => return service_err_to_response(id, e),
    };
    state.notify_epic_changed(epic.id);
    let text = format!(
        "Epic {} created: {} (workspace {}, {})",
        epic.id, epic.title, epic.workspace_id, epic.visibility
    );
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// The refusal for a `workspace_id` the caller does not belong to. A workspace
/// that does not exist and one the caller is not in read the same, so the
/// answer does not tell which ids are taken.
pub(super) async fn unknown_workspace(state: &McpState, workspace_id: i64) -> Option<String> {
    let mine = state.people_svc.my_workspaces().await;
    if mine.iter().any(|w| w.id == workspace_id) {
        return None;
    }
    Some(format!(
        "unknown workspace_id {workspace_id}. {}",
        format_workspace_choices(&mine)
    ))
}

/// The workspaces the caller belongs to, for a refusal that asks for a
/// `workspace_id` (`workspaces.allium: NameWorkspaceAndVisibilityViaMcp`).
pub(super) async fn workspace_choices(state: &McpState) -> String {
    format_workspace_choices(&state.people_svc.my_workspaces().await)
}

fn format_workspace_choices(mine: &[crate::models::WorkspaceRef]) -> String {
    if mine.is_empty() {
        return format!("{}.", crate::models::NO_WORKSPACE);
    }
    let list: Vec<String> = mine
        .iter()
        .map(|w| format!("{} ({})", w.id, w.name))
        .collect();
    format!("Your workspaces: {}.", list.join(", "))
}

/// epics.allium: GetEpicViaMcp, the `epic_id` form of list_epics.
async fn get_epic(state: &McpState, id: Option<Value>, epic_id: EpicId) -> JsonRpcResponse {
    tracing::info!(epic_id = epic_id.0, "MCP list_epics one epic");

    let (epic, done_count, total) = match state.epic_svc.get_epic_with_progress(epic_id).await {
        Ok(found) => found,
        Err(e) => return service_err_to_response(id, e),
    };
    let mut text = format!(
        "Epic {id}: {title}\nDescription: {desc}\nStatus: {status}",
        id = epic.id,
        title = epic.title,
        desc = epic.description,
        status = epic.status.as_str(),
    );
    if let Some(parent_id) = epic.parent_epic_id {
        text.push_str(&format!("\nParent: {parent_id}"));
        if let Ok(parent) = state.epic_svc.get_epic(parent_id).await {
            text.push_str(&format!(" {}", parent.title));
        }
    }
    if let Some(ref p) = epic.plan_path {
        text.push_str(&format!("\nPlan: {p}"));
    }
    if let Some(sort_order) = epic.sort_order {
        text.push_str(&format!("\nSort order: {sort_order}"));
    }
    if let Some(ref fc) = epic.feed_command {
        text.push_str(&format!("\nFeed command: {fc}"));
    }
    if let Some(fi) = epic.feed_interval_secs {
        text.push_str(&format!("\nFeed interval: {fi}s"));
    }
    text.push_str(&format!(
        "\nCreated: {}",
        epic.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    text.push_str(&format!(
        "\nUpdated: {}",
        epic.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    text.push_str(&format!(
        "\nWorkspace: {}\nVisibility: {}",
        epic.workspace_id, epic.visibility
    ));
    text.push_str(&format!("\nSubtasks: {done_count}/{total} done"));
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

pub(super) async fn handle_list_epics(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListEpicsArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if let Some(epic_id) = parsed.epic_id {
        if let Some(refusal) = reject_alongside(
            &id,
            "epic_id",
            &[
                ("parent_epic_id", parsed.parent_epic_id.is_some()),
                ("recursive", parsed.recursive.is_some()),
                ("query", parsed.query.is_some()),
                ("limit", parsed.limit.is_some()),
            ],
        ) {
            return refusal;
        }
        return get_epic(state, id, epic_id).await;
    }
    let recursive = parsed.recursive.unwrap_or(false);
    tracing::info!(parent_epic_id = ?parsed.parent_epic_id, recursive, "MCP list_epics");

    let epics = match state
        .epic_svc
        .list_epics_with_progress_under(parsed.parent_epic_id, recursive)
        .await
    {
        Ok(epics) => epics,
        Err(e) => return service_err_to_response(id, e),
    };
    let mut epics: Vec<_> = match parsed.query.as_deref() {
        Some(q) => epics
            .into_iter()
            .filter(|(e, _, _)| crate::models::text_query_matches(q, &e.title, &e.description))
            .collect(),
        None => epics,
    };
    if epics.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": "No epics found"}]}),
        );
    }
    let cut_hint = super::types::apply_limit(&mut epics, parsed.limit);
    let mut lines: Vec<String> = epics
        .iter()
        .map(|(e, done, total)| format_epic_list_line(e, *done, *total))
        .collect();
    lines.extend(cut_hint);
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
    )
}

fn format_epic_list_line(e: &Epic, done: usize, total: usize) -> String {
    let plan_indicator = if e.plan_path.is_some() { " [plan]" } else { "" };
    let status_indicator = if e.status != TaskStatus::Backlog {
        format!(" [{}]", e.status.as_str())
    } else {
        String::new()
    };
    let parent_indicator = match e.parent_epic_id {
        Some(p) => format!(" (parent:{p})"),
        None => String::new(),
    };
    format!(
        "- [{}] {} ({}/{} done) [workspace {}, {}]{}{}{}: {}",
        e.id,
        e.title,
        done,
        total,
        e.workspace_id,
        e.visibility,
        plan_indicator,
        status_indicator,
        parent_indicator,
        e.description
    )
}

pub(super) async fn handle_update_epic(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<UpdateEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(epic_id = parsed.epic_id.0, "MCP update_epic");

    let params = UpdateEpicParams {
        epic_id: parsed.epic_id,
        title: parsed.title,
        description: parsed.description,
        status: parsed.status,
        plan_path: parsed.plan_path,
        sort_order: parsed.sort_order,
        completed_at: None,
        auto_dispatch: None,
        feed_command: parsed.feed_command.map(|v| match v {
            Some(s) => crate::service::FieldUpdate::Set(s),
            None => crate::service::FieldUpdate::Clear,
        }),
        feed_interval_secs: parsed.feed_interval_secs,
        group_by_repo: parsed.group_by_repo,
        feed_append_only: parsed.feed_append_only,
        parent_epic_id: parsed.parent_epic_id,
        visibility: parsed.visibility,
    };
    let field_names: Vec<String> = params
        .updated_field_names()
        .into_iter()
        .map(String::from)
        .collect();

    let result = match state.epic_svc.update_epic(params).await {
        Ok(result) => result,
        Err(e) => return service_err_to_response(id, e),
    };
    let epic_id = result.epic_id;
    state.notify_epic_changed(epic_id);
    let text = format!("Epic {} updated ({})", epic_id, field_names.join(", "));
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}
