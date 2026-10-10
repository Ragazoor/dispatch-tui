//! Work-package tools (`docs/specs/work-packages.allium`, "MCP tools"): thin
//! wrappers over `WorkPackageServiceApi`.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{EpicId, TaskId, WorkPackageId, WorkPackageItem};
use crate::service::{CreateWorkPackageParams, UpdateWorkPackageParams, WorkPackageView};

use super::types::{
    deserialize_flexible_id, deserialize_optional_flexible_id, parse_args, reject_alongside,
    service_err_to_response, FlexibleId, JsonRpcResponse,
};

// ---------------------------------------------------------------------------
// Typed argument structs (JSON-RPC layer)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateWorkPackageArgs {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    link_task_ids: Vec<FlexibleId<TaskId>>,
    #[serde(default)]
    link_epic_ids: Vec<FlexibleId<EpicId>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListWorkPackagesArgs {
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    work_package_id: Option<WorkPackageId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateWorkPackageArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    work_package_id: WorkPackageId,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    link_task_ids: Vec<FlexibleId<TaskId>>,
    #[serde(default)]
    unlink_task_ids: Vec<FlexibleId<TaskId>>,
    #[serde(default)]
    link_epic_ids: Vec<FlexibleId<EpicId>>,
    #[serde(default)]
    unlink_epic_ids: Vec<FlexibleId<EpicId>>,
    #[serde(default)]
    delete: bool,
}

fn items(tasks: Vec<FlexibleId<TaskId>>, epics: Vec<FlexibleId<EpicId>>) -> Vec<WorkPackageItem> {
    tasks
        .into_iter()
        .map(|FlexibleId(t)| WorkPackageItem::Task(t))
        .chain(
            epics
                .into_iter()
                .map(|FlexibleId(e)| WorkPackageItem::Epic(e)),
        )
        .collect()
}

fn text(id: Option<Value>, text: String) -> JsonRpcResponse {
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(super) async fn handle_create_work_package(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateWorkPackageArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(title = %parsed.title, "MCP create_work_package");
    let link = items(parsed.link_task_ids, parsed.link_epic_ids);
    let linked = link.len();
    match state
        .work_package_svc
        .create_work_package(CreateWorkPackageParams {
            title: parsed.title,
            description: parsed.description,
            link,
        })
        .await
    {
        Ok(package_id) => {
            state.notify();
            text(
                id,
                format!("Work package {package_id} created with {linked} linked item(s)"),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_list_work_packages(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListWorkPackagesArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(work_package_id = ?parsed.work_package_id, "MCP list_work_packages");
    match state
        .work_package_svc
        .list_work_packages(parsed.work_package_id)
        .await
    {
        Ok(views) if views.is_empty() => text(
            id,
            "No work packages. Create one with create_work_package.".to_string(),
        ),
        Ok(views) => text(
            id,
            views
                .iter()
                .map(format_view)
                .collect::<Vec<_>>()
                .join("\n\n"),
        ),
        Err(e) => service_err_to_response(id, e),
    }
}

fn format_view(view: &WorkPackageView) -> String {
    let p = &view.package;
    let mut out = format!("Work package {}: {}", p.id, p.title);
    if !p.description.is_empty() {
        out.push_str(&format!("\nDescription: {}", p.description));
    }
    out.push_str(&format!(
        "\nCreated: {}\nUpdated: {}",
        p.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
        p.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    if view.items.is_empty() {
        out.push_str("\nItems: none");
    } else {
        out.push_str(&format!("\nItems ({}):", view.items.len()));
        for i in &view.items {
            out.push_str(&format!(
                "\n- {} {} [{}] {}",
                i.item.kind(),
                i.item.raw_id(),
                i.status.as_str(),
                i.title
            ));
        }
    }
    out
}

pub(super) async fn handle_update_work_package(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<UpdateWorkPackageArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let package_id = parsed.work_package_id;
    tracing::info!(work_package_id = package_id.0, "MCP update_work_package");

    let named = [
        ("title", parsed.title.is_some()),
        ("description", parsed.description.is_some()),
        ("link_task_ids", !parsed.link_task_ids.is_empty()),
        ("unlink_task_ids", !parsed.unlink_task_ids.is_empty()),
        ("link_epic_ids", !parsed.link_epic_ids.is_empty()),
        ("unlink_epic_ids", !parsed.unlink_epic_ids.is_empty()),
    ];
    if parsed.delete {
        if let Some(refusal) = reject_alongside(&id, "delete", &named) {
            return refusal;
        }
        return delete_work_package(state, id, package_id).await;
    }
    let fields: Vec<&str> = named
        .iter()
        .filter_map(|&(name, set)| set.then_some(name))
        .collect();
    let params = UpdateWorkPackageParams {
        id: package_id,
        title: parsed.title,
        description: parsed.description,
        link: items(parsed.link_task_ids, parsed.link_epic_ids),
        unlink: items(parsed.unlink_task_ids, parsed.unlink_epic_ids),
    };
    match state.work_package_svc.update_work_package(params).await {
        Ok(()) => {
            state.notify();
            text(
                id,
                format!("Work package {package_id} updated ({})", fields.join(", ")),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

/// work-packages.allium: DeleteWorkPackageViaMcp, the `delete: true` form of
/// update_work_package.
async fn delete_work_package(
    state: &McpState,
    id: Option<Value>,
    package_id: WorkPackageId,
) -> JsonRpcResponse {
    tracing::info!(
        work_package_id = package_id.0,
        "MCP update_work_package delete"
    );
    match state.work_package_svc.delete_work_package(package_id).await {
        Ok(()) => {
            state.notify();
            text(
                id,
                format!(
                    "Work package {package_id} deleted; the tasks and epics it linked are unchanged"
                ),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}
