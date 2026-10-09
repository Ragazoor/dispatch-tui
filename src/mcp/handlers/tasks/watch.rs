use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::service::SubscribeOutcome;

use super::{parse_args, service_err_to_response, JsonRpcResponse, WatchTaskArgs};

pub(crate) async fn handle_watch_task(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed: WatchTaskArgs = match parse_args(&id, args) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if parsed.watch {
        subscribe(state, id, identity, parsed).await
    } else {
        unsubscribe(state, id, identity, parsed).await
    }
}

async fn subscribe(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    parsed: WatchTaskArgs,
) -> JsonRpcResponse {
    match state
        .task_svc
        .subscribe_to_task(parsed.watcher_task_id, parsed.target_task_id)
        .await
    {
        Ok(SubscribeOutcome::AlreadyFinished(status)) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format!(
                "Task {} has already finished (status: {}). Not subscribing.",
                parsed.target_task_id, status.as_str()
            )}]}),
        ),
        Ok(SubscribeOutcome::Subscribed) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format!(
                "Now watching task {} for completion.",
                parsed.target_task_id
            )}]}),
        ),
        Err(e) => service_err_to_response(id, e),
    }
}

async fn unsubscribe(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    parsed: WatchTaskArgs,
) -> JsonRpcResponse {
    match state
        .task_svc
        .unsubscribe_from_task(parsed.watcher_task_id, parsed.target_task_id)
        .await
    {
        Ok(()) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format!(
                "No longer watching task {}.",
                parsed.target_task_id
            )}]}),
        ),
        Err(e) => service_err_to_response(id, e),
    }
}
