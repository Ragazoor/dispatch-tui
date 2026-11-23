//! The `display-name_set` tool (`docs/specs/people.allium:
//! SetDisplayNameViaMcp`): a thin wrapper over `PeopleService`.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;

use super::types::{parse_args, service_err_to_response, JsonRpcResponse};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDisplayNameArgs {
    name: String,
}

pub(super) async fn handle_display_name_set(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SetDisplayNameArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!("MCP display-name_set");
    match state.people_svc.set_display_name(&parsed.name).await {
        Ok(saved) => {
            state.notify();
            JsonRpcResponse::ok(
                id,
                json!({"content": [{"type": "text", "text": format!("Display name set to {saved}")}]}),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}
