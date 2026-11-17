//! Soft delete over MCP (mcp-task-tools.allium: GetArchivedTaskViaMcp,
//! ListArchivedTasksViaMcp, ArchivedTaskIsReadOnly). Archived records are
//! asked of the store on demand: the board holds none of them.

use serde_json::{json, Value};

use crate::mcp::McpState;
use crate::models::{ArchivedTask, EpicId, TaskId};

use super::super::types::NOT_FOUND_CODE;
use super::{goal_line, utc_stamp, JsonRpcResponse, INTERNAL_ERROR};

/// The record's epic as `"<title> (#<id>)"`, or `"#<id>"` with no title.
fn epic_label(a: &ArchivedTask) -> Option<String> {
    let id = a.epic_id?;
    Some(match &a.epic_title {
        Some(title) => format!("{title} (#{id})"),
        None => format!("#{id}"),
    })
}

/// `GetArchivedTaskViaMcp`'s detail lines.
pub(super) fn archived_detail_text(a: &ArchivedTask) -> String {
    let mut text = format!(
        "Task {id}: {title}\nStatus: archived\nArchived: {archived}\nRepo: {repo}\nDescription: {desc}",
        id = a.id,
        title = a.title,
        archived = utc_stamp(a.archived_at),
        repo = a.repo_path,
        desc = a.description,
    );
    if let Some(agent) = &a.agent_name {
        text.push_str(&format!("\nAgent: {agent} (archived)"));
    }
    if let Some(label) = epic_label(a) {
        text.push_str(&format!("\nEpic: {label}"));
    }
    if let Some(tag) = &a.tag {
        text.push_str(&format!("\nTag: {tag}"));
    }
    if let Some(plan) = &a.plan_path {
        text.push_str(&format!("\nPlan: {plan}"));
    }
    if let Some(u) = &a.url {
        text.push_str(&format!("\n{}: {}", u.label(), u.url));
    }
    text.push_str(&format!("\nBase branch: {}", a.base_branch));
    text.push_str(&format!("\nCreated: {}", utc_stamp(a.created_at)));
    if let Some(done) = a.completed_at {
        text.push_str(&format!("\nCompleted: {}", utc_stamp(done)));
    }
    text
}

/// One `ListArchivedTasksViaMcp` row.
fn archived_line(a: &ArchivedTask) -> String {
    let epic = epic_label(a)
        .map(|label| format!(" | epic: {label}"))
        .unwrap_or_default();
    let agent = a
        .agent_name
        .as_ref()
        .map(|n| format!(" | agent:{n}"))
        .unwrap_or_default();
    let url = a
        .url
        .as_ref()
        .map(|u| format!(" | {}: {}", u.label(), u.url))
        .unwrap_or_default();
    format!(
        "#{id}: {title} | archived {archived}{epic}{agent} | Base: {base}{url} | Goal: {goal}",
        id = a.id,
        title = a.title,
        archived = utc_stamp(a.archived_at),
        base = a.base_branch,
        goal = goal_line(&a.description),
    )
}

/// The filters `task_search(archived: true)` applies.
pub(super) struct ArchivedFilter {
    pub(super) epic_id: Option<EpicId>,
    pub(super) repo_paths: Vec<String>,
    pub(super) base_branch: Option<String>,
    pub(super) query: Option<String>,
    pub(super) limit: Option<std::num::NonZeroUsize>,
    /// The filters the call applied, for the empty reply.
    pub(super) filters: Option<String>,
}

/// `ListArchivedTasksViaMcp`, after the caller's scope is resolved.
pub(super) async fn list_archived(
    state: &McpState,
    id: Option<Value>,
    filter: ArchivedFilter,
) -> JsonRpcResponse {
    let mut records = match state
        .db
        .archived_tasks(filter.epic_id, &filter.repo_paths)
        .await
    {
        Ok(records) => records,
        Err(e) => return JsonRpcResponse::err(id, INTERNAL_ERROR, e.to_string()),
    };
    if let Some(base) = &filter.base_branch {
        records.retain(|a| &a.base_branch == base);
    }
    if let Some(query) = &filter.query {
        records.retain(|a| crate::models::text_query_matches(query, &a.title, &a.description));
    }
    if records.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": super::crud::empty_reply("archived tasks", filter.filters)}]}),
        );
    }
    let cut_hint = super::crud::cut_rows(
        &mut records,
        filter.limit,
        "newest archived first",
        // Only the filters this list accepts: status and the time filters are
        // refused here (ListArchivedTasksViaMcp).
        "epic_id, repo_paths, base_branch or query",
    );
    let mut lines: Vec<String> = records.iter().map(archived_line).collect();
    lines.extend(cut_hint);
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
    )
}

/// `ArchivedTaskIsReadOnly`: the refusal for an id with no live task but an
/// archived record the caller may see, or `None`. A store that cannot be
/// asked yields `None`, so the caller falls through to its usual not-found.
pub(super) async fn archived_refusal(
    state: &McpState,
    id: &Option<Value>,
    task_id: TaskId,
    verb: &str,
) -> Option<JsonRpcResponse> {
    if state.db.task_exists(task_id).await {
        return None;
    }
    match state.db.archived_task(task_id).await {
        Ok(Some(_)) => Some(JsonRpcResponse::err(
            id.clone(),
            NOT_FOUND_CODE,
            format!("task {} is archived and cannot be {verb}", task_id.0),
        )),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(task_id = task_id.0, error = %e, "archived task read failed");
            None
        }
    }
}
