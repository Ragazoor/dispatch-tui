use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::service::repo_index::RepoIndexService;

use super::types::{deserialize_flexible_i64, parse_args, tool_error, JsonRpcResponse};

#[derive(Deserialize)]
pub(super) struct IndexRepoArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    #[serde(default)]
    pub(super) repo_path: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SearchDocsArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    pub(super) task_id: i64,
    pub(super) query: String,
    #[serde(default)]
    pub(super) repo_path: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

async fn resolve_repo_path(
    id: &Option<Value>,
    task_id: i64,
    override_path: Option<String>,
    state: &McpState,
) -> Result<std::path::PathBuf, JsonRpcResponse> {
    if let Some(p) = override_path {
        return Ok(std::path::PathBuf::from(p));
    }
    let tid = crate::models::TaskId(task_id);
    match state.db.get_task(tid).await {
        Ok(Some(t)) => Ok(std::path::PathBuf::from(t.repo_path)),
        Ok(None) => Err(JsonRpcResponse::err(
            id.clone(),
            -32602,
            format!("task {task_id} not found — pass repo_path explicitly"),
        )),
        Err(e) => Err(JsonRpcResponse::err(
            id.clone(),
            -32603,
            format!("db error: {e}"),
        )),
    }
}

pub(super) async fn handle_index_repo(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<IndexRepoArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let repo_path =
        match resolve_repo_path(&id, parsed.task_id, parsed.repo_path, state).await {
            Ok(p) => p,
            Err(e) => return e,
        };

    let svc = RepoIndexService::new(state.embedding_service.clone());
    match svc.index_repo(&repo_path).await {
        Ok(result) => JsonRpcResponse::ok(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Indexed {repo} — files_indexed: {fi}, files_skipped: {fs}, chunks_total: {ct}, duration_ms: {ms}",
                        repo = repo_path.display(),
                        fi = result.files_indexed,
                        fs = result.files_skipped,
                        ct = result.chunks_total,
                        ms = result.duration_ms,
                    )
                }]
            }),
        ),
        Err(e) => tool_error(id, format!("index_repo failed: {e}")),
    }
}

#[allow(clippy::expect_used)]
pub(super) async fn handle_search_docs(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SearchDocsArgs>(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let repo_path =
        match resolve_repo_path(&id, parsed.task_id, parsed.repo_path, state).await {
            Ok(p) => p,
            Err(e) => return e,
        };

    let limit = parsed.limit.unwrap_or(5).min(20);
    let svc = RepoIndexService::new(state.embedding_service.clone());

    match svc.search_docs(&repo_path, &parsed.query, limit).await {
        Ok(results) => {
            let count = results.len();
            let items: Vec<Value> = results
                .into_iter()
                .map(|r| {
                    json!({
                        "file_path": r.file_path,
                        "chunk_text": r.chunk_text,
                        "score": r.score,
                    })
                })
                .collect();
            JsonRpcResponse::ok(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&json!({
                            "results": items,
                            "count": count,
                        }))
                        .expect("Value built from json! macro is always serializable")

                    }]
                }),
            )
        }
        Err(e) => tool_error(id, format!("search_docs failed: {e}")),
    }
}
