use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::JsonRpcResponse;

fn fmt_opt_str(v: Option<&str>) -> String {
    match v {
        Some(s) => format!("`{s}`"),
        None => "(unset)".to_string(),
    }
}

fn fmt_opt_int(v: Option<i64>) -> String {
    match v {
        Some(n) => n.to_string(),
        None => "(unset)".to_string(),
    }
}

/// Compose the four-line text summary of the current managed-feed config.
async fn config_summary(state: &McpState, heading: &str) -> anyhow::Result<String> {
    let rc = state.db.get_reviews_feed_command().await?;
    let ri = state.db.get_reviews_feed_interval_secs().await?;
    let cc = state.db.get_cve_feed_command().await?;
    let ci = state.db.get_cve_feed_interval_secs().await?;
    Ok(format!(
        "{heading}\n\
         - reviews_command: {}\n\
         - reviews_interval_secs: {}\n\
         - cve_command: {}\n\
         - cve_interval_secs: {}",
        fmt_opt_str(rc.as_deref()),
        fmt_opt_int(ri),
        fmt_opt_str(cc.as_deref()),
        fmt_opt_int(ci),
    ))
}

pub(super) async fn handle_get_managed_feed_config(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    _args: Value,
) -> JsonRpcResponse {
    tracing::info!("MCP get_managed_feed_config");
    match config_summary(state, "Managed-feed config:").await {
        Ok(text) => JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]})),
        Err(e) => {
            JsonRpcResponse::err(id, -32603, format!("failed to read managed feed config: {e}"))
        }
    }
}
