use serde::Deserialize;
use serde_json::{json, Value};

use crate::dispatch::{effective_review_command, REVIEW_COMMAND_KEY};
use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::{parse_args, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetReviewCommandArgs {
    pub(super) command: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadReviewCommandArgs {}

/// Read the review command a `pr_review` task runs; writes nothing. See
/// `ReadReviewCommandViaMcp` in `docs/specs/mcp-task-tools.allium`.
pub(super) async fn handle_review_command_read(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    if let Err(resp) = parse_args::<ReadReviewCommandArgs>(&id, args) {
        return resp;
    }
    tracing::info!("MCP review-command_read");
    command_reply(state, id).await
}

/// Set the review command a `pr_review` task runs. See `SetReviewCommandViaMcp`
/// in `docs/specs/mcp-task-tools.allium`.
pub(super) async fn handle_review_command_set(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SetReviewCommandArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!("MCP review-command_set");
    let command = parsed.command.as_str();
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
    command_reply(state, id).await
}

/// The reply both tools give: the command now in effect.
async fn command_reply(state: &McpState, id: Option<Value>) -> JsonRpcResponse {
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
