//! Work-package tools (`docs/specs/work-packages.allium`, "MCP tools"): thin
//! wrappers over `WorkPackageServiceApi`.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{EpicId, TaskId, WorkPackageId, WorkPackageItem};
use crate::service::{CreateWorkPackageParams, UpdateWorkPackageParams, WorkPackageView};

use super::types::{
    deserialize_flexible_id, deserialize_optional_flexible_i64, deserialize_optional_flexible_id,
    parse_args, reject_alongside, service_err_to_response, FlexibleId, JsonRpcResponse,
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
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    workspace_id: Option<i64>,
    #[serde(default)]
    visibility: Option<crate::models::Visibility>,
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
    sort_order: Option<i64>,
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
    #[serde(default)]
    visibility: Option<crate::models::Visibility>,
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
    let Some(workspace_id) = parsed.workspace_id else {
        return super::types::tool_error(
            id,
            format!(
                "a work package needs a workspace_id. {}",
                super::epics::workspace_choices(state).await
            ),
        );
    };
    let visibility = parsed.visibility.unwrap_or_default();
    let link = items(parsed.link_task_ids, parsed.link_epic_ids);
    let linked = link.len();
    let title = parsed.title.clone();
    match state
        .work_package_svc
        .create_work_package(CreateWorkPackageParams {
            title: parsed.title,
            description: parsed.description,
            link,
            workspace_id,
            visibility,
        })
        .await
    {
        Ok(package_id) => {
            state.notify();
            text(
                id,
                format!(
                    "Work package {package_id} (\"{title}\") created in workspace {} ({}) with {linked} linked item(s)",
                    workspace_id, visibility
                ),
            )
        }
        Err(e) => super::epics::workspace_refusal(state, id, workspace_id, e).await,
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
        Ok(views) => {
            // Task items show the task's display name (`McpRepliesNameWhatTheyTouch`).
            let mut names = std::collections::HashMap::new();
            for view in &views {
                for i in &view.items {
                    if let WorkPackageItem::Task(task_id) = i.item {
                        if let Some(name) = state
                            .task_svc
                            .get_task(task_id)
                            .await
                            .ok()
                            .and_then(|t| t.agent_name)
                            .filter(|n| !n.is_empty())
                        {
                            names.insert(task_id, name);
                        }
                    }
                }
            }
            text(
                id,
                views
                    .iter()
                    .map(|v| format_view(v, &names))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

fn format_view(
    view: &WorkPackageView,
    names: &std::collections::HashMap<TaskId, String>,
) -> String {
    let p = &view.package;
    let mut out = format!("Work package {}: {}", p.id, p.title);
    if !p.description.is_empty() {
        out.push_str(&format!("\nDescription: {}", p.description));
    }
    if let Some(order) = p.sort_order {
        out.push_str(&format!("\nSort order: {order}"));
    }
    if let Some(creator) = &p.created_by {
        out.push_str(&format!("\nCreated by: {creator}"));
    }
    out.push_str(&format!(
        "\nWorkspace: {}\nVisibility: {}",
        p.workspace_id, p.visibility
    ));
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
            let lead = match i.item {
                WorkPackageItem::Task(task_id) => names
                    .get(&task_id)
                    .map_or(String::new(), |name| format!("{name}: ")),
                WorkPackageItem::Epic(_) => String::new(),
            };
            out.push_str(&format!(
                "\n- {} {} [{}] {lead}{}",
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
        ("sort_order", parsed.sort_order.is_some()),
        ("visibility", parsed.visibility.is_some()),
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
        sort_order: parsed.sort_order,
        link: items(parsed.link_task_ids, parsed.link_epic_ids),
        unlink: items(parsed.unlink_task_ids, parsed.unlink_epic_ids),
        visibility: parsed.visibility,
    };
    match state.work_package_svc.update_work_package(params).await {
        Ok(not_linked) => {
            state.notify();
            let package = package_ref(state, package_id).await;
            let mut named = Vec::with_capacity(not_linked.len());
            for item in &not_linked {
                named.push(item_ref(state, *item).await);
            }
            text(id, update_reply(&package, &fields, &named))
        }
        Err(e) => service_err_to_response(id, e),
    }
}

/// `Work package 5 ("Title")`, or the bare id when the store does not hold
/// the package (`McpRepliesNameWhatTheyTouch`).
async fn package_ref(state: &McpState, package_id: WorkPackageId) -> String {
    match state
        .work_package_svc
        .list_work_packages(Some(package_id))
        .await
    {
        Ok(views) => match views.first() {
            Some(view) => format!("Work package {package_id} (\"{}\")", view.package.title),
            None => format!("Work package {package_id}"),
        },
        Err(_) => format!("Work package {package_id}"),
    }
}

/// A linked item as a reply names it: a task or an epic reference, or
/// `task 4` / `epic 7` when the store does not hold it.
async fn item_ref(state: &McpState, item: WorkPackageItem) -> String {
    match item {
        WorkPackageItem::Task(id) => match state.task_svc.get_task(id).await {
            Ok(task) => task.reference(),
            Err(_) => format!("task {id}"),
        },
        WorkPackageItem::Epic(id) => match state.epic_svc.get_epic(id).await {
            Ok(epic) => epic.reference(),
            Err(_) => format!("epic {id}"),
        },
    }
}

/// The reply to an update: what was written, and any requested unlink that
/// named an item the package never held (it changed nothing).
fn update_reply(package: &str, fields: &[&str], not_linked: &[String]) -> String {
    let mut reply = format!("{package} updated ({})", fields.join(", "));
    if !not_linked.is_empty() {
        reply.push_str(&format!("; not linked: {}", not_linked.join(", ")));
    }
    reply
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
    // Named before the delete: afterwards there is no title left to read.
    let package = package_ref(state, package_id).await;
    match state.work_package_svc.delete_work_package(package_id).await {
        Ok(()) => {
            state.notify();
            text(
                id,
                format!("{package} deleted; the tasks and epics it linked are unchanged"),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}
