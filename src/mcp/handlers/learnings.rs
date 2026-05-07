use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::LearningFilter;
use crate::mcp::McpState;
use crate::models::{LearningId, LearningKind, LearningScope, LearningStatus, RetrievalSource};
use crate::service::LearningService;

use super::types::{
    deserialize_flexible_i64, deserialize_optional_flexible_i64, parse_args,
    service_err_to_response, JsonRpcResponse,
};

// ---------------------------------------------------------------------------
// Typed argument structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct RecordLearningArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    pub(super) kind: LearningKind,
    pub(super) summary: String,
    pub(super) scope: LearningScope,
    #[serde(default)]
    pub(super) detail: Option<String>,
    #[serde(default)]
    pub(super) scope_ref: Option<String>,
    #[serde(default)]
    pub(super) tags: Vec<String>,
}

#[derive(Deserialize)]
pub(super) struct QueryLearningsArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    #[serde(default)]
    pub(super) tag_filter: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) limit: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct UpvoteLearningArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) learning_id: i64,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(super) fn handle_record_learning(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<RecordLearningArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let task_id = crate::models::TaskId(parsed.task_id);
    let task = match state.db.get_task(task_id) {
        Ok(Some(t)) => t,
        Ok(None) => return JsonRpcResponse::err(id, -32602, format!("task {task_id} not found")),
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("database error: {e}")),
    };

    let scope_ref = match parsed.scope_ref {
        Some(r) => Some(r),
        None => match parsed.scope {
            LearningScope::User => None,
            LearningScope::Repo => Some(task.repo_path.clone()),
            LearningScope::Project => Some(task.project_id.to_string()),
            LearningScope::Epic => match task.epic_id {
                Some(eid) => Some(eid.0.to_string()),
                None => {
                    return JsonRpcResponse::err(
                        id,
                        -32602,
                        "scope=epic requires the task to belong to an epic".to_string(),
                    )
                }
            },
            LearningScope::Task => Some(task.id.0.to_string()),
        },
    };

    let similar_scope_ref = scope_ref.clone();
    let svc = LearningService::new(state.db.clone());
    match svc.create_learning(crate::service::CreateLearningParams {
        kind: parsed.kind,
        summary: parsed.summary,
        detail: parsed.detail,
        scope: parsed.scope,
        scope_ref,
        tags: parsed.tags,
        source_task_id: Some(task_id),
    }) {
        Ok(learning_id) => {
            let similar: Vec<_> = match state.db.list_learnings(LearningFilter {
                status: Some(LearningStatus::Approved),
                scope: Some(parsed.scope),
                scope_ref: similar_scope_ref,
                ..Default::default()
            }) {
                Ok(entries) => entries
                    .into_iter()
                    .filter(|l| l.kind == parsed.kind && l.id != learning_id)
                    .take(5)
                    .collect(),
                Err(e) => {
                    tracing::warn!("record_learning: failed to query similar entries: {e}");
                    vec![]
                }
            };

            let mut text = format!(
                "Learning {learning_id} recorded and active. \
                 It will be injected into future dispatch prompts for matching tasks."
            );

            if !similar.is_empty() {
                text.push_str(&format!(
                    "\n\nSimilar approved learnings already exist for \
                     (kind={kind}, scope={scope}):",
                    kind = parsed.kind,
                    scope = parsed.scope,
                ));
                for l in &similar {
                    text.push_str(&format!(
                        "\n  [{}] {} (confirmed {}x) \
                         → upvote_learning(learning_id={}, task_id={})",
                        l.id, l.summary, l.confirmed_count, l.id, task_id.0
                    ));
                }
                text.push_str(
                    "\n\nIf one of these already captures what you intended, \
                     consider calling upvote_learning on it instead of keeping this new entry.",
                );
            }

            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) fn handle_query_learnings(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<QueryLearningsArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let task_id = crate::models::TaskId(parsed.task_id);
    let task = match state.db.get_task(task_id) {
        Ok(Some(t)) => t,
        Ok(None) => return JsonRpcResponse::err(id, -32602, format!("task {task_id} not found")),
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("database error: {e}")),
    };

    let mut learnings = match state.db.list_learnings_for_dispatch(
        Some(task.project_id),
        &task.repo_path,
        task.epic_id,
    ) {
        Ok(l) => l,
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("database error: {e}")),
    };

    // Post-filter by tag when requested.
    if let Some(ref tag) = parsed.tag_filter {
        learnings.retain(|l| l.tags.iter().any(|t| t == tag));
    }

    let limit = parsed.limit.unwrap_or(20).min(50) as usize;
    learnings.truncate(limit);

    // Best-effort: record a retrieval row per returned learning so the
    // wrap-up verdict step can validate that the agent saw it. A failed
    // insert must not fail the query response.
    for l in &learnings {
        let _ = state
            .db
            .record_retrieval(task_id, l.id, RetrievalSource::QueryLearnings);
    }

    if learnings.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": "No approved learnings found for this task's context."
                }]
            }),
        );
    }

    let text = learnings
        .iter()
        .map(|l| {
            let tags = if l.tags.is_empty() {
                "none".to_string()
            } else {
                l.tags.join(", ")
            };
            format!(
                "[{}] ({}/{}) {}\n  Tags: {} | Confirmed: {}x",
                l.id, l.kind, l.scope, l.summary, tags, l.confirmed_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

pub(super) fn handle_upvote_learning(
    state: &McpState,
    id: Option<Value>,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<UpvoteLearningArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    tracing::info!(
        task_id = parsed.task_id,
        learning_id = parsed.learning_id,
        "MCP upvote_learning"
    );

    let svc = LearningService::new(state.db.clone());
    match svc.upvote_learning(LearningId(parsed.learning_id)) {
        Ok(()) => JsonRpcResponse::ok(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": format!("Learning {} upvoted.", parsed.learning_id)
                }]
            }),
        ),
        Err(e) => service_err_to_response(id, e),
    }
}
