//! Workspace and super-admin tools (`docs/specs/admin-surfaces.allium`, "the
//! MCP twins"): thin wrappers over `PeopleService`. The store decides who may
//! do what; these only name a person, shape the call and say what happened.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{AdminSnapshot, MemberRole};

use super::types::{
    deserialize_flexible_i64, deserialize_optional_flexible_i64, parse_args,
    service_err_to_response, tool_error, JsonRpcResponse,
};

fn text(id: Option<Value>, text: String) -> JsonRpcResponse {
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListWorkspacesArgs {
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    workspace_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateWorkspaceArgs {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateWorkspaceArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    workspace_id: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    delete: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetWorkspaceMemberArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    workspace_id: i64,
    person: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    remove: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetSuperAdminArgs {
    person: String,
    super_admin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteUnseenItemArgs {
    kind: String,
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    item_id: i64,
}

pub(super) async fn handle_list_workspaces(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListWorkspacesArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(workspace_id = ?parsed.workspace_id, "MCP list_workspaces");
    let snapshot = state.people_svc.admin_snapshot().await;
    match format_workspaces(&snapshot, parsed.workspace_id) {
        Ok(out) => text(id, out),
        Err(why) => tool_error(id, why),
    }
}

/// The workspaces, their members and roles, the super-admins, and (unless one
/// workspace was asked for) the directory of everyone who can be added.
fn format_workspaces(snapshot: &AdminSnapshot, only: Option<i64>) -> Result<String, String> {
    let mut out = String::new();
    let shown: Vec<_> = snapshot
        .workspaces
        .iter()
        .filter(|w| only.is_none_or(|id| w.id == id))
        .collect();
    if let Some(wanted) = only {
        if shown.is_empty() {
            return Err(format!(
                "no workspace {wanted}; call list_workspaces with no workspace_id to see the ones you can see"
            ));
        }
    }
    if shown.is_empty() {
        out.push_str("You belong to no workspace; ask a super-admin to add you.\n");
    }
    for w in shown {
        out.push_str(&format!("Workspace {}: {}\n", w.id, w.name));
        if w.members.is_empty() {
            out.push_str("  (no members)\n");
        }
        for m in &w.members {
            let prefix: String = m.identity.chars().take(8).collect();
            out.push_str(&format!(
                "  - {} ({prefix}) {}, membership {}\n",
                m.shown_name,
                m.role.as_str(),
                m.membership_id
            ));
        }
    }
    let supers: Vec<String> = snapshot.super_admins().map(|p| p.labelled()).collect();
    out.push_str(&format!(
        "\nSuper-admins: {}\n",
        if supers.is_empty() {
            "none".to_string()
        } else {
            supers.join(", ")
        }
    ));
    if snapshot.claimable {
        out.push_str("No super-admin exists yet: claim_super_admin would make you one.\n");
    }
    if only.is_none() {
        out.push_str("\nPeople (the directory; name one by display name or identity):\n");
        for p in &snapshot.directory {
            out.push_str(&format!("  - {}\n", p.labelled()));
        }
    }
    Ok(out.trim_end().to_string())
}

pub(super) async fn handle_create_workspace(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateWorkspaceArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!("MCP create_workspace");
    if let Err(e) = state.people_svc.create_workspace(&parsed.name).await {
        return service_err_to_response(id, e);
    }
    state.notify();
    let name = parsed.name.trim();
    let made = state
        .people_svc
        .admin_snapshot()
        .await
        .workspaces
        .into_iter()
        .find(|w| w.name == name);
    text(
        id,
        match made {
            Some(w) => format!(
                "Created workspace {}: {}. It has no members yet; add some with set_workspace_member.",
                w.id, w.name
            ),
            None => format!("Created workspace {name}."),
        },
    )
}

pub(super) async fn handle_update_workspace(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<UpdateWorkspaceArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(workspace_id = parsed.workspace_id, "MCP update_workspace");
    let result = match (&parsed.name, parsed.delete) {
        (Some(name), false) => state
            .people_svc
            .rename_workspace(parsed.workspace_id, name)
            .await
            .map(|()| {
                format!(
                    "Renamed workspace {} to {}",
                    parsed.workspace_id,
                    name.trim()
                )
            }),
        (None, true) => state
            .people_svc
            .delete_workspace(parsed.workspace_id)
            .await
            .map(|name| format!("Deleted workspace {name}")),
        _ => {
            return tool_error(
                id,
                "give exactly one of name (to rename) and delete=true (to delete)",
            )
        }
    };
    match result {
        Ok(message) => {
            state.notify();
            text(id, message)
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_set_workspace_member(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SetWorkspaceMemberArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(
        workspace_id = parsed.workspace_id,
        "MCP set_workspace_member"
    );
    let role = match (&parsed.role, parsed.remove) {
        (Some(role), false) => match MemberRole::parse(role) {
            Some(role) => Some(role),
            None => return tool_error(id, format!("role is 'admin' or 'member', not '{role}'")),
        },
        (None, true) => None,
        _ => {
            return tool_error(
                id,
                "give exactly one of role (to add the person or change their role) and remove=true",
            )
        }
    };
    match state
        .people_svc
        .set_member(parsed.workspace_id, &parsed.person, role)
        .await
    {
        Ok(message) => {
            state.notify();
            text(id, message)
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_set_super_admin(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<SetSuperAdminArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(super_admin = parsed.super_admin, "MCP set_super_admin");
    match state
        .people_svc
        .set_super_admin(&parsed.person, parsed.super_admin)
        .await
    {
        Ok(name) => {
            state.notify();
            text(
                id,
                if parsed.super_admin {
                    format!("{name} is a super-admin")
                } else {
                    format!("{name} is not a super-admin")
                },
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_claim_super_admin(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    if let Err(resp) = parse_args::<NoArgs>(&id, args) {
        return resp;
    }
    tracing::info!("MCP claim_super_admin");
    match state.people_svc.claim_super_admin().await {
        Ok(()) => {
            state.notify();
            text(
                id,
                "You are now a super-admin. Create workspaces with create_workspace and add people with set_workspace_member.".to_string(),
            )
        }
        Err(e) => service_err_to_response(id, e),
    }
}

pub(super) async fn handle_list_unseen_items(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    if let Err(resp) = parse_args::<NoArgs>(&id, args) {
        return resp;
    }
    tracing::info!("MCP list_unseen_items");
    let snapshot = state.people_svc.admin_snapshot().await;
    if !snapshot.i_am_super_admin {
        return text(
            id,
            "Only a super-admin sees unseen items; you are not one, so there is nothing to list."
                .to_string(),
        );
    }
    if snapshot.unseen.is_empty() {
        return text(id, "No unseen items.".to_string());
    }
    let mut out = String::from(
        "Private items nobody can see (creator no longer in the workspace). No titles are shown. \
         Remove one with delete_unseen_item.\n",
    );
    for i in &snapshot.unseen {
        out.push_str(&format!(
            "- {} {} in {} ({}), by {}, created {}\n",
            i.kind, i.id, i.workspace_name, i.workspace_id, i.creator, i.created_at
        ));
    }
    text(id, out.trim_end().to_string())
}

pub(super) async fn handle_delete_unseen_item(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<DeleteUnseenItemArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(kind = %parsed.kind, item_id = parsed.item_id, "MCP delete_unseen_item");
    match state
        .people_svc
        .delete_unseen_item(&parsed.kind, parsed.item_id)
        .await
    {
        Ok(()) => {
            state.notify();
            text(id, format!("Deleted {} {}", parsed.kind, parsed.item_id))
        }
        Err(e) => service_err_to_response(id, e),
    }
}
