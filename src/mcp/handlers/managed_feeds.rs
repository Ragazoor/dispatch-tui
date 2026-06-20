use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::{
    deserialize_nullable_flexible_i64, deserialize_nullable_string, parse_args, JsonRpcResponse,
};

#[derive(Deserialize)]
pub(super) struct SetManagedFeedConfigArgs {
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub(super) reviews_command: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_i64")]
    pub(super) reviews_interval_secs: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub(super) cve_command: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_i64")]
    pub(super) cve_interval_secs: Option<Option<i64>>,
}

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
        Err(e) => JsonRpcResponse::err(
            id,
            -32603,
            format!("failed to read managed feed config: {e}"),
        ),
    }
}

pub(super) async fn handle_set_managed_feed_config(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SetManagedFeedConfigArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!("MCP set_managed_feed_config");

    // Reject negative intervals up front (0 is valid: "poll every tick").
    for (label, field) in [
        ("reviews_interval_secs", parsed.reviews_interval_secs),
        ("cve_interval_secs", parsed.cve_interval_secs),
    ] {
        if let Some(Some(n)) = field {
            if n < 0 {
                return JsonRpcResponse::err(id, -32602, format!("{label} must be >= 0"));
            }
        }
    }

    // Persist only the provided fields; an omitted field (None) is left as-is.
    let write = async {
        if let Some(v) = &parsed.reviews_command {
            state.db.set_reviews_feed_command(v.as_deref()).await?;
        }
        if let Some(v) = parsed.reviews_interval_secs {
            state.db.set_reviews_feed_interval_secs(v).await?;
        }
        if let Some(v) = &parsed.cve_command {
            state.db.set_cve_feed_command(v.as_deref()).await?;
        }
        if let Some(v) = parsed.cve_interval_secs {
            state.db.set_cve_feed_interval_secs(v).await?;
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = write {
        return JsonRpcResponse::err(
            id,
            -32603,
            format!("failed to persist managed feed config: {e}"),
        );
    }

    // Re-materialise the managed epic tree, mirroring the TUI [C] save path.
    if let Err(e) = crate::service::provision_managed_feeds_from_settings(&*state.db).await {
        return JsonRpcResponse::err(
            id,
            -32603,
            format!("failed to provision managed feeds: {e}"),
        );
    }
    state.notify();

    match config_summary(state, "Managed-feed config saved:").await {
        Ok(text) => JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]})),
        // Persist + provision already succeeded; a summary read failure is non-fatal.
        Err(_) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": "Managed-feed config saved."}]}),
        ),
    }
}
