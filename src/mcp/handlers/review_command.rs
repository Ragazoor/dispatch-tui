use serde::Deserialize;
use serde_json::{json, Value};

use crate::dispatch::{effective_review_command, REVIEW_COMMAND_KEY};
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::{parse_args, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetReviewCommandArgs {
    #[serde(default)]
    pub(super) command: Option<String>,
}

/// Read or set the review command a `pr_review` task runs. Omitting `command`
/// is a read. See `SetReviewCommandViaMcp` in `docs/specs/mcp-task-tools.allium`.
pub(super) async fn handle_set_review_command(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SetReviewCommandArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!("MCP set_review_command");
    if let Some(command) = parsed.command.as_deref() {
        if command.contains('\n') || command.contains('\r') {
            return JsonRpcResponse::err(id, INVALID_PARAMS, "command must be a single line");
        }
        // A blank is saved as-is: it reads back as the default.
        if let Err(e) = state
            .db
            .set_setting_string(REVIEW_COMMAND_KEY, command.trim())
            .await
        {
            return JsonRpcResponse::err(
                id,
                INTERNAL_ERROR,
                format!("failed to set review command: {e}"),
            );
        }
    }
    let saved = match state.db.get_setting_string(REVIEW_COMMAND_KEY).await {
        Ok(v) => v,
        Err(e) => {
            return JsonRpcResponse::err(
                id,
                INTERNAL_ERROR,
                format!("failed to read review command: {e}"),
            )
        }
    };
    let text = format!(
        "Review command for PR review tasks: `{}`",
        effective_review_command(saved)
    );
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}
