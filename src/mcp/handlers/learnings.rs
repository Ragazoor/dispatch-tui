use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::LearningFilter;
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{
    LearningId, LearningKind, LearningScope, LearningStatus, LearningVerdict, TaskId,
};

use super::types::{
    deserialize_flexible_id, deserialize_optional_flexible_i64, fetch_caller_task, parse_args,
    service_err_to_response, JsonRpcResponse,
};

// ---------------------------------------------------------------------------
// Typed argument structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordLearningArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) task_id: TaskId,
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
#[serde(deny_unknown_fields)]
pub(super) struct QueryLearningsArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) task_id: TaskId,
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
#[serde(deny_unknown_fields)]
pub(super) struct RateLearningArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) learning_id: LearningId,
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) task_id: TaskId,
    pub(super) verdict: LearningVerdict,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteLearningArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) learning_id: LearningId,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// The `scope_ref` a new learning is filed under: the caller's explicit one, else
/// the one its `scope` implies for `task`.
fn resolve_scope_ref(
    task: &crate::models::Task,
    scope: LearningScope,
    explicit: Option<String>,
) -> Result<Option<String>, crate::service::ServiceError> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    match scope {
        LearningScope::User => Ok(None),
        LearningScope::Repo => Ok(Some(task.repo_path.clone())),
        LearningScope::Epic => match task.epic_id {
            Some(eid) => Ok(Some(eid.0.to_string())),
            None => Err(crate::service::ServiceError::Validation(
                "scope=epic requires the task to belong to an epic".to_string(),
            )),
        },
        LearningScope::Task => Ok(Some(task.id.0.to_string())),
    }
}

/// Up to five approved learnings of the same kind and scope as a new one, so the
/// recorder can prefer an existing entry over a duplicate. A failed query only
/// loses the hint.
async fn similar_learnings(
    state: &McpState,
    kind: LearningKind,
    scope: LearningScope,
    scope_ref: Option<String>,
    new_id: LearningId,
) -> Vec<crate::models::Learning> {
    match state
        .db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Approved),
            scope: Some(scope),
            scope_ref,
            ..Default::default()
        })
        .await
    {
        Ok(entries) => entries
            .into_iter()
            .filter(|l| l.kind == kind && l.id != new_id)
            .take(5)
            .collect(),
        Err(e) => {
            tracing::warn!("record_learning: failed to query similar entries: {e}");
            vec![]
        }
    }
}

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

    let task_id = parsed.task_id;
    let task = match fetch_caller_task(&*state.db, &id, task_id).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let scope_ref = match resolve_scope_ref(&task, parsed.scope, parsed.scope_ref) {
        Ok(r) => r,
        Err(e) => return service_err_to_response(id, e),
    };

    let scope_filter = scope_ref.clone();
    match state
        .learning_svc
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
            let similar =
                similar_learnings(state, parsed.kind, parsed.scope, scope_filter, learning_id)
                    .await;

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
                        "\n  [{}] {} (upvoted {}x)",
                        l.id, l.summary, l.upvote_count
                    ));
                }
                text.push_str(
                    "\n\nIf one of these already captures what you intended, \
                     prefer it over keeping this duplicate.",
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

    let task_id = parsed.task_id;
    let limit = parsed.limit.unwrap_or(50).min(50) as usize;

    let ranked = match state
        .learning_svc
        .query_learnings(crate::service::QueryLearningsParams {
            task_id,
            query: parsed.query,
            tag_filter: parsed.tag_filter.unwrap_or_default(),
            limit,
        })
        .await
    {
        Ok(r) => r,
        Err(e) => return service_err_to_response(id, e),
    };

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

pub(super) async fn handle_rate_learning(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<RateLearningArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    tracing::info!(
        task_id = parsed.task_id.0,
        learning_id = parsed.learning_id.0,
        verdict = parsed.verdict.as_str(),
        "MCP rate_learning"
    );

    match state
        .learning_svc
        .apply_verdicts(parsed.task_id, vec![(parsed.learning_id, parsed.verdict)])
        .await
    {
        Ok(()) => {
            let note = match parsed.verdict {
                LearningVerdict::Helped => "recorded as helped (upvoted)",
                LearningVerdict::Wrong => "recorded as wrong (downvoted; no review step)",
            };
            JsonRpcResponse::ok(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Learning {} {note}.", parsed.learning_id.0)
                    }]
                }),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_delete_learning(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DeleteLearningArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let learning_id = parsed.learning_id;

    tracing::info!(learning_id = learning_id.0, "MCP delete_learning");

    match state.learning_svc.delete_learning(learning_id).await {
        Ok(()) => JsonRpcResponse::ok(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": format!("Learning {} deleted.", learning_id.0)
                }]
            }),
        ),
        Err(e) => service_err_to_response(id, e),
    }
}
