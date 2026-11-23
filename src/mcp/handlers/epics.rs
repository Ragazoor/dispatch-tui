use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{Epic, EpicId, StartRule, TaskStatus};
use crate::service::{CreateEpicParams, ServiceError, UpdateEpicParams};

use super::time_filter;
use super::types::{
    deserialize_flexible_id, deserialize_nullable_flexible_i64, deserialize_nullable_flexible_id,
    deserialize_nullable_string, deserialize_optional_flexible_i64,
    deserialize_optional_flexible_id, deserialize_optional_limit, parse_args, reject_alongside,
    service_err_to_response, JsonRpcResponse,
};

// ---------------------------------------------------------------------------
// Typed argument structs (JSON-RPC layer)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateEpicArgs {
    pub(super) title: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) parent_epic_id: Option<EpicId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListEpicsArgs {
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) epic_id: Option<EpicId>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_id")]
    pub(super) parent_epic_id: Option<EpicId>,
    #[serde(default)]
    pub(super) recursive: Option<bool>,
    #[serde(default)]
    pub(super) query: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    pub(super) limit: Option<std::num::NonZeroUsize>,
    /// Bounds on `updated_at` and `completed_at` (`epics.allium`:
    /// `ListEpicsViaMcp`, `mcp-task-tools.allium`: `TimeFilterInput`).
    #[serde(default)]
    pub(super) updated_after: Option<String>,
    #[serde(default)]
    pub(super) updated_before: Option<String>,
    #[serde(default)]
    pub(super) completed_after: Option<String>,
    #[serde(default)]
    pub(super) completed_before: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateEpicArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub(super) epic_id: EpicId,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) status: Option<TaskStatus>,
    #[serde(default)]
    pub(super) plan_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) sort_order: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    pub(super) feed_command: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_i64")]
    pub(super) feed_interval_secs: Option<Option<i64>>,
    #[serde(default)]
    pub(super) group_by_repo: Option<bool>,
    #[serde(default)]
    pub(super) feed_append_only: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_nullable_flexible_id")]
    pub(super) parent_epic_id: Option<Option<EpicId>>,
    /// The orchestrator arguments (`UpdateEpicViaMcp`), which travel alone.
    #[serde(default)]
    pub(super) orchestrator: Option<bool>,
    #[serde(default)]
    pub(super) start_rule: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    pub(super) max_parallel: Option<i64>,
}

// ---------------------------------------------------------------------------
// Epic tool handlers (thin wrappers over EpicService)
// ---------------------------------------------------------------------------

pub(super) async fn handle_epic_create(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(title = %parsed.title, "MCP epic_create");

    let created = state
        .epic_svc
        .create_epic(CreateEpicParams {
            title: parsed.title,
            description: parsed.description,
            sort_order: parsed.sort_order,
            parent_epic_id: parsed.parent_epic_id,
            feed_command: None,
            feed_interval_secs: None,
        })
        .await;
    let epic = match created {
        Ok(epic) => epic,
        Err(e) => return service_err_to_response(id, e),
    };
    state.notify_epic_changed(epic.id);
    let text = format!("Epic {} created: {}", epic.id, epic.title);
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// epics.allium: GetEpicViaMcp, the `epic_id` form of epic_search.
async fn get_epic(state: &McpState, id: Option<Value>, epic_id: EpicId) -> JsonRpcResponse {
    tracing::info!(epic_id = epic_id.0, "MCP epic_search one epic");

    let (epic, done_count, total) = match state.epic_svc.get_epic_with_progress(epic_id).await {
        Ok(found) => found,
        Err(e) => return service_err_to_response(id, e),
    };
    let mut text = format!(
        "Epic {id}: {title}\nDescription: {desc}\nStatus: {status}",
        id = epic.id,
        title = epic.title,
        desc = epic.description,
        status = epic.status.as_str(),
    );
    if let Some(parent_id) = epic.parent_epic_id {
        text.push_str(&format!("\nParent: {parent_id}"));
        if let Ok(parent) = state.epic_svc.get_epic(parent_id).await {
            text.push_str(&format!(" {}", parent.title));
        }
    }
    if let Some(ref p) = epic.plan_path {
        text.push_str(&format!("\nPlan: {p}"));
    }
    if let Some(sort_order) = epic.sort_order {
        text.push_str(&format!("\nSort order: {sort_order}"));
    }
    if let Some(ref fc) = epic.feed_command {
        text.push_str(&format!("\nFeed command: {fc}"));
    }
    if let Some(fi) = epic.feed_interval_secs {
        text.push_str(&format!("\nFeed interval: {fi}s"));
    }
    text.push_str(&format!(
        "\nCreated: {}",
        epic.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    text.push_str(&format!(
        "\nUpdated: {}",
        epic.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    text.push_str(&format!("\nSubtasks: {done_count}/{total} done"));
    text.push_str(&super::orchestration::epic_line(state, &epic).await);
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

pub(super) async fn handle_epic_search(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<ListEpicsArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if let Some(epic_id) = parsed.epic_id {
        if let Some(refusal) = reject_alongside(
            &id,
            "epic_id",
            &[
                ("parent_epic_id", parsed.parent_epic_id.is_some()),
                ("recursive", parsed.recursive.is_some()),
                ("query", parsed.query.is_some()),
                ("limit", parsed.limit.is_some()),
                ("updated_after", parsed.updated_after.is_some()),
                ("updated_before", parsed.updated_before.is_some()),
                ("completed_after", parsed.completed_after.is_some()),
                ("completed_before", parsed.completed_before.is_some()),
            ],
        ) {
            return refusal;
        }
        return get_epic(state, id, epic_id).await;
    }
    let (updated, completed) = match time_filter::parse_windows(
        &id,
        (
            parsed.updated_after.as_deref(),
            parsed.updated_before.as_deref(),
        ),
        (
            parsed.completed_after.as_deref(),
            parsed.completed_before.as_deref(),
        ),
    ) {
        Ok(windows) => windows,
        Err(resp) => return resp,
    };
    let recursive = parsed.recursive.unwrap_or(false);
    tracing::info!(parent_epic_id = ?parsed.parent_epic_id, recursive, "MCP epic_search");

    let epics = match state
        .epic_svc
        .list_epics_with_progress_under(parsed.parent_epic_id, recursive)
        .await
    {
        Ok(epics) => epics,
        Err(e) => return service_err_to_response(id, e),
    };
    let mut epics: Vec<_> = match parsed.query.as_deref() {
        Some(q) => epics
            .into_iter()
            .filter(|(e, _, _)| crate::models::text_query_matches(q, &e.title, &e.description))
            .collect(),
        None => epics,
    };
    epics.retain(|(e, _, _)| {
        updated.contains(Some(e.updated_at)) && completed.contains(e.completed_at)
    });
    if epics.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": super::types::empty_reply("epics", applied_filters(&parsed))}]}),
        );
    }
    // ListEpicsViaMcp: most recently updated first, ties by id descending.
    epics.sort_by(|(a, _, _), (b, _, _)| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.0.cmp(&a.id.0))
    });
    let cut_hint = cut_rows(&mut epics, parsed.limit);
    // A parent's title comes from the listing when the parent is in it, else
    // from the store (`McpRepliesNameWhatTheyTouch`).
    let mut parent_titles: std::collections::HashMap<crate::models::EpicId, String> = epics
        .iter()
        .map(|(e, _, _)| (e.id, e.title.clone()))
        .collect();
    for (e, _, _) in &epics {
        if let Some(parent) = e.parent_epic_id {
            if let std::collections::hash_map::Entry::Vacant(slot) = parent_titles.entry(parent) {
                if let Ok(found) = state.epic_svc.get_epic(parent).await {
                    slot.insert(found.title);
                }
            }
        }
    }
    let mut lines: Vec<String> = epics
        .iter()
        .map(|(e, done, total)| {
            let parent_title = e
                .parent_epic_id
                .and_then(|p| parent_titles.get(&p))
                .map(String::as_str);
            format_epic_list_line(e, *done, *total, parent_title)
        })
        .collect();
    lines.extend(cut_hint);
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
    )
}

/// The default `limit` of `epic_search` (`ListEpicsViaMcp`).
const DEFAULT_EPIC_LIMIT: usize = 50;

/// Keep the first `limit` rows (50 when `limit` is `None`). Returns the line to
/// append when rows were cut: it says how many, in what order, and how to get
/// the rest.
fn cut_rows<T>(rows: &mut Vec<T>, limit: Option<std::num::NonZeroUsize>) -> Option<String> {
    let total = rows.len();
    let n = limit.map_or(DEFAULT_EPIC_LIMIT, std::num::NonZeroUsize::get);
    if total <= n {
        return None;
    }
    rows.truncate(n);
    Some(format!(
        "Showing {n} of {total}, newest update first. Narrow with parent_epic_id, query or the \
         time filters, or raise limit."
    ))
}

/// The filters a search applied, as `name=value` in the order `ListEpicsViaMcp`
/// fixes, for the empty reply. `None` when it applied none.
fn applied_filters(p: &ListEpicsArgs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(e) = p.parent_epic_id {
        parts.push(format!("parent_epic_id={}", e.0));
    }
    if p.recursive == Some(true) {
        parts.push("recursive=true".to_string());
    }
    if let Some(q) = &p.query {
        parts.push(format!("query={q:?}"));
    }
    for (name, value) in [
        ("updated_after", &p.updated_after),
        ("updated_before", &p.updated_before),
        ("completed_after", &p.completed_after),
        ("completed_before", &p.completed_before),
    ] {
        if let Some(v) = value {
            parts.push(format!("{name}={v}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn format_epic_list_line(
    e: &Epic,
    done: usize,
    total: usize,
    parent_title: Option<&str>,
) -> String {
    let plan_indicator = if e.plan_path.is_some() { " [plan]" } else { "" };
    let status_indicator = if e.status != TaskStatus::Backlog {
        format!(" [{}]", e.status.as_str())
    } else {
        String::new()
    };
    let parent_indicator = match e.parent_epic_id {
        Some(p) => match parent_title {
            Some(title) => format!(" (parent:{p} {title})"),
            None => format!(" (parent:{p})"),
        },
        None => String::new(),
    };
    format!(
        "- [{}] {} ({}/{} done){}{}{} | Updated: {} | Completed: {}: {}",
        e.id,
        e.title,
        done,
        total,
        plan_indicator,
        status_indicator,
        parent_indicator,
        time_filter::iso(e.updated_at),
        time_filter::iso_or_dash(e.completed_at),
        e.description
    )
}

pub(super) async fn handle_epic_update(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<UpdateEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(epic_id = parsed.epic_id.0, "MCP epic_update");

    if parsed.orchestrator.is_some() || parsed.start_rule.is_some() || parsed.max_parallel.is_some()
    {
        return update_orchestration(state, id, parsed).await;
    }
    let moved = match parsed.parent_epic_id {
        Some(new_parent) => switches_a_move_turns_off(state, parsed.epic_id, new_parent).await,
        None => None,
    };

    let params = UpdateEpicParams {
        epic_id: parsed.epic_id,
        title: parsed.title,
        description: parsed.description,
        status: parsed.status,
        plan_path: parsed.plan_path,
        sort_order: parsed.sort_order,
        completed_at: None,
        auto_dispatch: None,
        feed_command: parsed.feed_command.map(|v| match v {
            Some(s) => crate::service::FieldUpdate::Set(s),
            None => crate::service::FieldUpdate::Clear,
        }),
        feed_interval_secs: parsed.feed_interval_secs,
        group_by_repo: parsed.group_by_repo,
        feed_append_only: parsed.feed_append_only,
        parent_epic_id: parsed.parent_epic_id,
    };
    let field_names: Vec<String> = params
        .updated_field_names()
        .into_iter()
        .map(String::from)
        .collect();

    let result = match state.epic_svc.update_epic(params).await {
        Ok(result) => result,
        Err(e) => return service_err_to_response(id, e),
    };
    let epic_id = result.epic_id;
    state.notify_epic_changed(epic_id);
    let epic_ref = super::types::epic_ref(state, epic_id).await;
    let mut text = format!("{epic_ref} updated ({})", field_names.join(", "));
    if let Some((above, off)) = moved {
        for (off_id, off_title) in off {
            text.push_str(&format!(
                "\nSwitched off the orchestrator of epic {} ({off_title}): epic {} above it has one.",
                off_id.0, above.0
            ));
        }
    }
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// `epics.allium: ReparentingKeepsOneOrchestratorOnAPath`, read before the
/// move: the epic whose switch covers `new_parent`, with the switched-on
/// epics of the moved subtree (`epic_id` included), which the move turns
/// off. `None` when nothing covers `new_parent`.
async fn switches_a_move_turns_off(
    state: &McpState,
    epic_id: EpicId,
    new_parent: Option<EpicId>,
) -> Option<(EpicId, Vec<(EpicId, String)>)> {
    let (above, _) = super::orchestration::covering(state, new_parent).await?;
    let epics = state.db.list_epics().await;
    let subtree = crate::models::descendant_epic_ids(epic_id, &epics);
    let mut off = Vec::new();
    for e in epics.iter().filter(|e| subtree.contains(&e.id)) {
        if state.db.epic_orchestration(e.id).await.is_some() {
            off.push((e.id, e.title.clone()));
        }
    }
    off.sort_unstable_by_key(|(id, _)| *id);
    Some((above.id, off))
}

/// `UpdateEpicViaMcp`'s orchestrator arguments: switch on, switch off, or
/// change the settings, each one store write.
async fn update_orchestration(
    state: &McpState,
    id: Option<Value>,
    parsed: UpdateEpicArgs,
) -> JsonRpcResponse {
    let also_set: Vec<&str> = [
        ("title", parsed.title.is_some()),
        ("description", parsed.description.is_some()),
        ("status", parsed.status.is_some()),
        ("plan_path", parsed.plan_path.is_some()),
        ("sort_order", parsed.sort_order.is_some()),
        ("feed_command", parsed.feed_command.is_some()),
        ("feed_interval_secs", parsed.feed_interval_secs.is_some()),
        ("group_by_repo", parsed.group_by_repo.is_some()),
        ("feed_append_only", parsed.feed_append_only.is_some()),
        ("parent_epic_id", parsed.parent_epic_id.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, set)| set.then_some(name))
    .collect();
    if !also_set.is_empty() {
        return service_err_to_response(
            id,
            ServiceError::Validation(format!(
                "the orchestrator arguments go in a call of their own (also set: {})",
                also_set.join(", ")
            )),
        );
    }
    let start_rule = match parsed.start_rule.as_deref().map(StartRule::parse) {
        None => None,
        Some(Some(rule)) => Some(rule),
        Some(None) => {
            return service_err_to_response(
                id,
                ServiceError::Validation("start_rule must be marked_only or everything".into()),
            )
        }
    };
    let epic_id = parsed.epic_id;
    let (result, done) = match parsed.orchestrator {
        Some(false) if start_rule.is_some() || parsed.max_parallel.is_some() => {
            return service_err_to_response(
                id,
                ServiceError::Validation(
                    "switching the orchestrator off takes no start_rule or max_parallel".into(),
                ),
            )
        }
        Some(false) => (
            state.epic_svc.switch_orchestrator_off(epic_id).await,
            "orchestrator switched off",
        ),
        Some(true) => (
            state
                .epic_svc
                .switch_orchestrator_on(epic_id, start_rule, parsed.max_parallel)
                .await,
            "orchestrator switched on",
        ),
        None => (
            state
                .epic_svc
                .change_orchestrator_settings(epic_id, start_rule, parsed.max_parallel)
                .await,
            "orchestrator settings changed",
        ),
    };
    if let Err(e) = result {
        return service_err_to_response(id, e);
    }
    state.notify_epic_changed(epic_id);
    let epic_ref = super::types::epic_ref(state, epic_id).await;
    let text = format!("{epic_ref}: {done}");
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}
