use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::LearningFilter;
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{LearningId, LearningKind, LearningScope, LearningStatus, RetrievalSource};

// RAG similarity threshold: candidates below this cosine similarity are filtered out
pub const QUERY_LEARNINGS_RAG_THRESHOLD: f32 = 0.25;
use crate::service::embeddings::{
    deserialize_embedding, embed_text_for_query, rag_rank_learnings,
};
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
    /// Optional semantic query string. When omitted, task title + description
    /// are used as the query text fed into the embedding model.
    #[serde(default)]
    pub(super) query: Option<String>,
    /// Optional list of tags; learnings whose tags overlap receive a soft score
    /// boost but entries without matching tags are **not** excluded.
    #[serde(default)]
    pub(super) tag_filter: Option<Vec<String>>,
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

pub(super) async fn handle_record_learning(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<RecordLearningArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let task_id = crate::models::TaskId(parsed.task_id);
    let task = match state.db.get_task(task_id).await {
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
    let svc = LearningService::new(state.db.clone(), state.embedding_service.clone());
    match svc
        .create_learning(crate::service::CreateLearningParams {
            kind: parsed.kind,
            summary: parsed.summary,
            detail: parsed.detail,
            scope: parsed.scope,
            scope_ref,
            tags: parsed.tags,
            source_task_id: Some(task_id),
        })
        .await
    {
        Ok(learning_id) => {
            let similar: Vec<_> = match state
                .db
                .list_learnings(LearningFilter {
                    status: Some(LearningStatus::Approved),
                    scope: Some(parsed.scope),
                    scope_ref: similar_scope_ref,
                    ..Default::default()
                })
                .await
            {
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
                        "\n  [{}] {} (upvoted {}x) \
                         -> upvote_learning(learning_id={}, task_id={})",
                        l.id, l.summary, l.upvote_count, l.id, task_id.0
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

pub(super) async fn handle_query_learnings(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<QueryLearningsArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let task_id = crate::models::TaskId(parsed.task_id);
    let task = match state.db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return JsonRpcResponse::err(id, -32602, format!("task {task_id} not found")),
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("database error: {e}")),
    };

    // Build query text: use explicit query param when provided, otherwise fall
    // back to the task's title + description.
    let query_text = match parsed.query {
        Some(q) => q,
        None => embed_text_for_query(&task.title, &task.description),
    };

    // Embed the query string.
    let query_vec = match state.embedding_service.embed(query_text).await {
        Ok(v) => v,
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("embedding error: {e}")),
    };

    // Fetch all approved non-task-scoped candidates with their stored embeddings.
    let candidates_raw = match state.db.list_all_approved_non_task_learnings().await {
        Ok(c) => c,
        Err(e) => return JsonRpcResponse::err(id, -32603, format!("database error: {e}")),
    };

    let candidates: Vec<(crate::models::Learning, Vec<f32>)> = candidates_raw
        .into_iter()
        .filter_map(|(l, emb_bytes)| {
            let bytes = emb_bytes?;
            Some((l, deserialize_embedding(&bytes)))
        })
        .collect();

    // RAG ranking with scope boosts and soft tag boost.
    let threshold = QUERY_LEARNINGS_RAG_THRESHOLD;
    let tag_filter = parsed.tag_filter.unwrap_or_default();
    let limit = parsed.limit.unwrap_or(50).min(50) as usize;

    let epic_id_str = task.epic_id.map(|e| e.0.to_string());
    let project_id_str = task.project_id.0.to_string();
    let ranked = rag_rank_learnings(
        &candidates,
        &query_vec,
        epic_id_str.as_deref(),
        Some(task.repo_path.as_str()),
        Some(project_id_str.as_str()),
        threshold,
        &tag_filter,
        limit,
    );

    // Record retrievals for analytics.
    for l in &ranked {
        if let Err(e) = state
            .db
            .record_retrieval(task_id, l.id, RetrievalSource::QueryLearnings)
            .await
        {
            tracing::warn!(
                task_id = task_id.0,
                learning_id = l.id.0,
                error = ?e,
                "failed to record learning retrieval"
            );
        }
    }

    if ranked.is_empty() {
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

    let text = ranked
        .iter()
        .map(|l| {
            let tags = if l.tags.is_empty() {
                "none".to_string()
            } else {
                l.tags.join(", ")
            };
            format!(
                "[{}] ({}/{}) {}\n  Tags: {} | Upvotes: {}",
                l.id, l.kind, l.scope, l.summary, tags, l.upvote_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

pub(super) async fn handle_upvote_learning(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
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

    let svc = LearningService::new(state.db.clone(), state.embedding_service.clone());
    match svc.upvote_learning(LearningId(parsed.learning_id)).await {
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
