//! `override_poll_owner` — the MCP surface for
//! `pr-workflow.allium: OverridePrPollOwner` / `feeds.allium:
//! OverrideFeedOwner`. See `mcp-task-tools.allium: OverridePollOwnerViaMcp`.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{EpicId, PollScopeId, TaskId};
use crate::service::ServiceError;

use super::types::{
    deserialize_optional_flexible_id, parse_args, service_err_to_response, JsonRpcResponse,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverridePollOwnerArgs {
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) task_id: Option<TaskId>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) epic_id: Option<EpicId>,
}

/// `mcp-task-tools.allium: OverridePollOwnerViaMcp`. Exactly one of
/// `task_id`/`epic_id` — the caller states which scope it means rather than
/// the handler guessing from whichever id happens to be present.
///
/// No confirmation gate here, unlike the epic scope's EditEpic-hosted prompt
/// (`epics.allium: EditEpic`): a direct call to this tool — by a human via
/// CLI, or by an agent — already IS the explicit decision the design calls
/// for.
pub(crate) async fn handle_override_poll_owner(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed: OverridePollOwnerArgs = match parse_args(&id, args) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let result = match (parsed.task_id, parsed.epic_id) {
        (Some(task_id), None) => override_task(state, task_id).await,
        (None, Some(epic_id)) => override_epic(state, epic_id).await,
        (Some(_), Some(_)) => Err(ServiceError::Validation(
            "provide exactly one of task_id or epic_id, not both".into(),
        )),
        (None, None) => Err(ServiceError::Validation(
            "provide one of task_id or epic_id".into(),
        )),
    };

    match result {
        Ok(text) => JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]})),
        Err(e) => service_err_to_response(id, e),
    }
}

/// Resolved first, matching every other tool in this crate: an id that
/// resolves to nothing is a `NotFound` error, not a `PollOwner` row silently
/// created for a task/epic that was never real.
async fn override_task(state: &McpState, task_id: TaskId) -> Result<String, ServiceError> {
    state.task_svc.get_task(task_id).await?;
    state
        .db
        .override_poll_owner(PollScopeId::Task(task_id))
        .await
        .map(|()| format!("PR-poll ownership for task {task_id} reassigned to this host."))
        .map_err(ServiceError::Internal)
}

async fn override_epic(state: &McpState, epic_id: EpicId) -> Result<String, ServiceError> {
    state.epic_svc.get_epic(epic_id).await?;
    state
        .db
        .override_poll_owner(PollScopeId::Epic(epic_id))
        .await
        .map(|()| format!("Feed-poll ownership for epic {epic_id} reassigned to this host."))
        .map_err(ServiceError::Internal)
}
