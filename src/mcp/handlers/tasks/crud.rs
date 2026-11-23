use std::collections::HashMap;

use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{Agent, Task, TaskId, TaskStatus};
use crate::service::{
    CreateTaskParams, ListTasksFilter, ServiceError, UpdateTaskParams, UrlUpdate,
};

use super::super::time_filter;
use super::{
    fetch_caller_task, parse_args, reject_alongside, service_err_to_response, tool_error,
    CreateTaskWithEpicArgs, JsonRpcResponse, QueryUsageArgs, StatusFilter, TaskSearchArgs,
    UpdateTaskArgs, INVALID_PARAMS,
};

async fn handle_update_task(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let mut parsed = match parse_args::<UpdateTaskArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let task_id = parsed.task_id;
    tracing::info!(task_id = task_id.0, status = ?parsed.status, "MCP task_update (one task)");
    if let Some(refused) = super::archived::archived_refusal(state, &id, task_id, "changed").await {
        return refused;
    }

    // status="done" is a dedicated close-only path (MarkTaskDoneViaMcp,
    // mcp-task-tools.allium). Checked FIRST, against the raw parsed args —
    // before the url/url_type pairing validation below, which never applies
    // to this path — so a field that would otherwise slip past unnoticed
    // (url_type sent without a url has no slot in UpdateTaskParams at all)
    // or get answered by the wrong error (url without url_type) is instead
    // caught by this call's own "only field set" guard.
    if parsed.status == Some(TaskStatus::Done) {
        return handle_mark_task_done(state, id, identity, parsed).await;
    }

    let mut params = UpdateTaskParams::for_task(task_id);

    // `url`/`url_type` are the two `[manual]` fields of `UpdateTaskArgs`: they
    // validate as a pair rather than mapping to one setter each, so they are
    // taken out here and everything else is folded in by the generated
    // `apply_to_params` below.
    // A `url_type` with no `url` has no slot in `UpdateTaskParams`, so it is
    // refused here, before any branch can drop it unseen (a call that also
    // carries `bookmarked` writes through, and would answer "updated").
    if parsed.url_type.is_some() && parsed.url.is_none() {
        return service_err_to_response(
            id,
            ServiceError::Validation("url_type is only valid together with url".into()),
        );
    }
    match url_update(parsed.url.take(), parsed.url_type.take()) {
        Ok(Some(update)) => params = params.url(update),
        Ok(None) => {}
        Err(e) => return service_err_to_response(id, e),
    }

    // `bookmarked` is not a Task field: it is the board person's own row. It
    // rides in the same `UpdateTaskParams`, so the store writes it with the
    // fields in one transaction (`UpdateTaskViaMcp`).
    let bookmarked = parsed.bookmarked.take();
    // `ready` is the task's ready mark, a second store write after the fields
    // (`UpdateTaskViaMcp`, "ready is a second write"). Everything it can be
    // refused for is checked here, before the first write.
    // `Some(Some(marker))` marks, `Some(None)` clears, `None` leaves the mark.
    let ready_write = match parsed.ready.take() {
        Some(true) => {
            let status_after = match parsed.status {
                Some(status) => status,
                None => match state.task_svc.get_task(task_id).await {
                    Ok(task) => task.status,
                    Err(e) => return service_err_to_response(id, e),
                },
            };
            if status_after != TaskStatus::Backlog {
                return service_err_to_response(
                    id,
                    super::super::orchestration::not_backlog_refusal(),
                );
            }
            match super::super::orchestration::ready_marker(state, identity).await {
                Ok(marker) => Some(Some(marker)),
                Err(e) => return service_err_to_response(id, e),
            }
        }
        Some(false) => Some(None),
        None => None,
    };
    let mut params = parsed.apply_to_params(params);
    if let Some(bookmarked) = bookmarked {
        params = params.bookmarked(bookmarked);
    }
    let mut fields: Vec<String> = params
        .updated_field_names()
        .into_iter()
        .map(str::to_string)
        .collect();

    let mut nudge = "";
    // A call that sets nothing (a lone `url_type` is refused above) reaches
    // `update_task`, which refuses it as it always has.
    // A call that sets only the bookmark writes no task field, so it leaves
    // `updated_at` alone: a person's own mark is not an edit of the shared task
    // (`UpdateTaskViaMcp`). That is `SetBookmark` itself. A call that sets only
    // `ready` writes nothing here.
    let ready_only = fields.is_empty() && ready_write.is_some();
    if !ready_only {
        if let (true, Some(bookmarked)) = (params.is_bookmark_only(), params.bookmarked) {
            if let Err(e) = state.task_svc.set_bookmark(task_id, bookmarked).await {
                return service_err_to_response(id, e);
            }
        } else {
            let result = match state.task_svc.update_task(params).await {
                Ok(result) => result,
                Err(e) => return service_err_to_response(id, e),
            };
            if result.was_pr_finalisation {
                nudge = super::reflection_nudge(&*state.db).await;
            }
        }
    }
    if let Some(write) = ready_write {
        let written =
            super::super::orchestration::write_ready(state, task_id, write.as_ref()).await;
        if let Err(e) = written {
            if fields.is_empty() {
                return service_err_to_response(id, e);
            }
            state.notify_task_changed(task_id);
            let task_ref = super::super::types::task_ref(state, task_id).await;
            return tool_error(
                id,
                format!(
                    "{task_ref}: the fields were written ({}), but the ready mark was not: {e}",
                    fields.join(", ")
                ),
            );
        }
        fields.push("ready".to_string());
    }
    state.notify_task_changed(task_id);
    let task_ref = super::super::types::task_ref(state, task_id).await;
    let text = format!("{task_ref} updated ({}){}", fields.join(", "), nudge);
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// Fold the `url`/`url_type` pair of an update into one [`UrlUpdate`].
/// An empty `url` clears it; a `url` without `url_type` is a validation error.
fn url_update(
    url: Option<String>,
    url_type: Option<crate::models::UrlType>,
) -> Result<Option<UrlUpdate>, ServiceError> {
    let Some(u) = url else {
        return Ok(None);
    };
    // Empty string clears the URL (legacy clear convention).
    if u.is_empty() {
        return Ok(Some(UrlUpdate::Clear));
    }
    let Some(url_type) = url_type else {
        return Err(ServiceError::Validation(
            "url_type is required when url is set (one of: pr, security_alert, issue, other)"
                .into(),
        ));
    };
    Ok(Some(UrlUpdate::Set(crate::models::TaskUrl::new(
        u, url_type,
    ))))
}

// ---------------------------------------------------------------------------
// task_update
// ---------------------------------------------------------------------------

/// `task_update`, the one MCP update tool: [`handle_update_task`] applied to
/// each of `task_ids` in turn, with the same field set. With several ids the
/// reply is per task and the call is not atomic — a failure on one task
/// neither stops nor rolls back the others. With exactly one id the reply is
/// the single-task reply, errors included. See `UpdateTasksViaMcp` in
/// `docs/specs/mcp-task-tools.allium`.
pub(crate) async fn handle_update_tasks(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let Value::Object(mut fields) = args else {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "arguments must be an object".to_string(),
        );
    };
    if fields.contains_key("task_id") {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "task_update takes task_ids, not task_id".to_string(),
        );
    }
    let task_ids = match fields
        .remove("task_ids")
        .map(serde_json::from_value::<Vec<super::FlexibleId<TaskId>>>)
    {
        Some(Ok(ids)) if !ids.is_empty() => ids,
        Some(Ok(_)) => {
            return JsonRpcResponse::err(
                id,
                INVALID_PARAMS,
                "task_ids must not be empty".to_string(),
            )
        }
        Some(Err(e)) => {
            return JsonRpcResponse::err(id, INVALID_PARAMS, format!("Invalid task_ids: {e}"))
        }
        None => {
            return JsonRpcResponse::err(id, INVALID_PARAMS, "task_ids is required".to_string())
        }
    };
    if fields.is_empty() {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "task_update needs at least one field to set besides task_ids".to_string(),
        );
    }

    let mut seen = std::collections::HashSet::new();
    let task_ids: Vec<TaskId> = task_ids
        .into_iter()
        .map(|super::FlexibleId(t)| t)
        .filter(|t| seen.insert(*t))
        .collect();

    // Every task gets the same fields, so a field set the parser rejects
    // would fail each task identically: reject it once, before any write.
    let mut probe = fields.clone();
    probe.insert("task_id".into(), json!(task_ids[0].0));
    if let Err(resp) = parse_args::<UpdateTaskArgs>(&id, Value::Object(probe)) {
        return resp;
    }
    tracing::info!(count = task_ids.len(), "MCP task_update");

    if let [only] = task_ids[..] {
        fields.insert("task_id".into(), json!(only.0));
        return handle_update_task(state, id, identity, Value::Object(fields)).await;
    }

    let mut lines = Vec::with_capacity(task_ids.len() + 1);
    let mut updated = 0;
    for task_id in &task_ids {
        let mut one = fields.clone();
        one.insert("task_id".into(), json!(task_id.0));
        match handle_update_task(state, id.clone(), identity, Value::Object(one))
            .await
            .into_text()
        {
            Ok(text) => {
                updated += 1;
                lines.push(text);
            }
            Err(message) => lines.push(format!(
                "{} failed: {message}",
                super::super::types::task_ref(state, *task_id).await
            )),
        }
    }
    lines.push(format!("Updated {updated} of {} tasks.", task_ids.len()));
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
    )
}

/// Every `UpdateTaskArgs` field other than `task_id`/`status` that the caller
/// actually set. Used by [`handle_mark_task_done`] to enforce
/// `MarkTaskDoneViaMcp`'s "status=\"done\" must be the only field set" guard
/// directly against the raw args — deliberately not
/// `UpdateTaskParams::updated_field_names`, which has no slot for `url_type`
/// sent without a `url` (it is `[manual]` and only ever read inside the
/// non-empty-`url` branch above), so that combination would otherwise pass
/// through unnoticed.
///
/// Exhaustive destructuring (no `..`): a new `UpdateTaskArgs` field must be
/// added here explicitly, or this fails to compile.
fn other_update_task_fields_set(parsed: &UpdateTaskArgs) -> Vec<&'static str> {
    let UpdateTaskArgs {
        task_id: _,
        status: _,
        plan_path,
        title,
        description,
        repo_path,
        sort_order,
        url,
        url_type,
        tag,
        sub_status,
        epic_id,
        base_branch,
        wrap_up_mode,
        model,
        auto_run_plan,
        phoenix,
        schedule,
        bookmarked,
        ready,
    } = parsed;

    [
        ("plan_path", plan_path.is_some()),
        ("title", title.is_some()),
        ("description", description.is_some()),
        ("repo_path", repo_path.is_some()),
        ("sort_order", sort_order.is_some()),
        ("url", url.is_some()),
        ("url_type", url_type.is_some()),
        ("tag", tag.is_some()),
        ("sub_status", sub_status.is_some()),
        ("epic_id", epic_id.is_some()),
        ("base_branch", base_branch.is_some()),
        ("wrap_up_mode", wrap_up_mode.is_some()),
        ("model", model.is_some()),
        ("auto_run_plan", auto_run_plan.is_some()),
        ("phoenix", phoenix.is_some()),
        ("schedule", schedule.is_some()),
        ("bookmarked", bookmarked.is_some()),
        ("ready", ready.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, is_set)| is_set.then_some(name))
    .collect()
}

/// `task_update(status="done")`: a dedicated close-only call — see
/// `MarkTaskDoneViaMcp` (docs/specs/mcp-task-tools.allium). Reachable for a
/// session caller or a dispatched agent acting on a task other than its own;
/// never for a dispatched agent closing itself (that stays session_wrap-up +
/// session_exit), and never combined with any other field in the same call.
/// The close is the module's: one `close_task` reducer call
/// (`store-split.allium: RequestCloseTask`) applies the Done write, rolls the
/// epic up and, when there is a window or a PR to deal with, writes a `close`
/// row this board's host then serves. It does NOT chain the epic's next
/// subtask: that chain runs in the module, on an session_exit close, because
/// only a close with a `session_wrap-up` behind it can say the work reached
/// `base_branch`.
/// See `MarkTaskDoneViaMcp` in `docs/specs/mcp-task-tools.allium`.
async fn handle_mark_task_done(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    parsed: UpdateTaskArgs,
) -> JsonRpcResponse {
    let task_id = parsed.task_id;

    let other_fields = other_update_task_fields_set(&parsed);
    if !other_fields.is_empty() {
        return service_err_to_response(
            id,
            ServiceError::Validation(format!(
                "status=\"done\" must be the only field set in this call (also set: {}). \
                 Update other fields separately, then close with a status-only call.",
                other_fields.join(", ")
            )),
        );
    }

    if matches!(identity, CallerIdentity::Task(caller_task_id) if *caller_task_id == task_id) {
        return service_err_to_response(
            id,
            ServiceError::Validation(
                "Cannot mark your own task done via task_update — call session_wrap-up then session_exit instead."
                    .into(),
            ),
        );
    }

    // The existence read is the not-found answer: the module refuses a missing
    // task too, but with a message of its own.
    let task = match state.task_svc.get_task(task_id).await {
        Ok(t) => t,
        Err(e) => return service_err_to_response(id, e),
    };

    let closed = state.task_svc.close_task(task_id).await;
    state.notify_task_changed(task_id);
    if let Some(epic_id) = task.epic_id {
        state.notify_epic_changed(epic_id);
    }
    match closed {
        Err(e) => {
            tracing::warn!(
                task_id = task_id.0,
                "task_update done: the close failed: {e}"
            );
            tool_error(
                id,
                format!(
                    "{} could NOT be moved to done — the close did not take effect. \
                     The task is still in its previous status; try again.",
                    task.reference()
                ),
            )
        }
        // No chain runs on this path, so unlike session_exit's, this response
        // never names a next subtask.
        Ok(()) => JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": format!("{} marked done.", task.reference())}]}),
        ),
    }
}

/// The refusal text for a `task_create` call that omitted `epic_id`
/// (`TheRefusalNamesTheLikelyAnswer`, docs/specs/mcp-task-tools.allium).
///
/// Actionable rather than a bare "missing field": when the caller is a
/// dispatched agent whose own task has an epic, that epic is named as the
/// likely answer. With no epic to name, the message says what is required and
/// invents nothing — an id the caller did not supply and the system does not
/// know is exactly the guess this rule exists to stop, so the no-epic wording
/// deliberately carries no number at all.
///
/// This is why the requirement is enforced here rather than by making the
/// field non-`Option` at the deserialization boundary: serde's own
/// missing-field error cannot see the caller's identity.
fn missing_epic_id_message(caller_epic: Option<String>) -> String {
    let base = "epic_id is required. Pass the id of the epic this task belongs to, \
                or null if it is deliberately standalone.";
    match caller_epic {
        Some(epic) => {
            format!("{base} Your own task is in {epic}, which is the likely answer here.")
        }
        None => base.to_string(),
    }
}

pub(crate) async fn handle_create_task(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<CreateTaskWithEpicArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(
        title = %parsed.title,
        epic_id = ?parsed.epic_id,
        identity = ?identity,
        "MCP task_create"
    );

    // A Task-kind caller must exist, whatever epic_id says — the check is
    // unconditional (CreateTaskViaMcp's first `requires`), not a side effect of
    // resolving the epic. The caller's epic is fetched only to name it in the
    // refusal below; it is never used as a value (EveryTaskNamesItsEpicOrNull).
    let caller_epic = match identity {
        CallerIdentity::Task(caller_id) => {
            match fetch_caller_task(&*state.db, &id, *caller_id).await {
                Ok(t) => match t.epic_id {
                    Some(epic) => Some(super::super::types::epic_ref(state, epic).await),
                    None => None,
                },
                Err(resp) => return resp,
            }
        }
        CallerIdentity::Session => None,
    };

    let Some(epic_id) = parsed.epic_id else {
        return service_err_to_response(
            id,
            ServiceError::Validation(missing_epic_id_message(caller_epic)),
        );
    };

    let signal = parsed.signal;
    // CreateTaskViaMcp: the ready mark is a second write after the creation;
    // who it names is settled before anything is created.
    let marker = if parsed.ready {
        match super::super::orchestration::ready_marker(state, identity).await {
            Ok(marker) => Some(marker),
            Err(e) => return service_err_to_response(id, e),
        }
    } else {
        None
    };
    let params = CreateTaskParams {
        title: parsed.title,
        description: parsed.description,
        repo_path: parsed.repo_path,
        plan_path: parsed.plan_path,
        epic_id,
        sort_order: parsed.sort_order,
        tag: parsed.tag,
        base_branch: parsed.base_branch,
        wrap_up_mode: parsed.wrap_up_mode,
        model: parsed.model,
        auto_run_plan: parsed.auto_run_plan,
        phoenix: parsed.phoenix,
        schedule: parsed.schedule.filter(|s| !s.is_empty()),
    };
    let created = if signal {
        state.task_svc.create_task_signalled(params).await
    } else {
        state.task_svc.create_task(params).await
    };
    let task_id = match created {
        Ok(task_id) => task_id,
        Err(e) => return service_err_to_response(id, e),
    };
    if let Some(marker) = &marker {
        if let Err(e) = super::super::orchestration::write_ready(state, task_id, Some(marker)).await
        {
            state.notify_task_changed(task_id);
            return tool_error(
                id,
                format!(
                    "{} was created, but not marked ready: {e}",
                    super::super::types::task_ref(state, task_id).await
                ),
            );
        }
    }
    state.notify_task_changed(task_id);
    let text = format!(
        "{} created{}",
        super::super::types::task_ref(state, task_id).await,
        if marker.is_some() {
            " and marked ready"
        } else {
            ""
        }
    );
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// Rows `task_search` returns when the caller names no `limit`
/// (`ListTasksViaMcp`, mcp-task-tools.allium).
const DEFAULT_TASK_LIMIT: usize = 50;

/// `task_search`: the `task_id` form is the detail view
/// (`GetTaskViaMcp`), every other call the list (`ListTasksViaMcp`).
pub(crate) async fn handle_task_search(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let parsed = match parse_args::<TaskSearchArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    match parsed.task_id {
        Some(task_id) => {
            if let Some(refusal) = reject_alongside(
                &id,
                "task_id",
                &[
                    ("epic_id", parsed.epic_id.is_some()),
                    ("repo_paths", parsed.repo_paths.is_some()),
                    ("status", parsed.status.is_some()),
                    ("recursive", parsed.recursive),
                    ("base_branch", parsed.base_branch.is_some()),
                    ("query", parsed.query.is_some()),
                    ("limit", parsed.limit.is_some()),
                    ("bookmarked", parsed.bookmarked.is_some()),
                    ("archived", parsed.archived),
                    ("updated_after", parsed.updated_after.is_some()),
                    ("updated_before", parsed.updated_before.is_some()),
                    ("completed_after", parsed.completed_after.is_some()),
                    ("completed_before", parsed.completed_before.is_some()),
                ],
            ) {
                return refusal;
            }
            get_task_detail(state, id, task_id).await
        }
        None => list_tasks(state, id, parsed).await,
    }
}

async fn get_task_detail(state: &McpState, id: Option<Value>, task_id: TaskId) -> JsonRpcResponse {
    tracing::info!(task_id = task_id.0, "MCP task_search one task");

    match state.task_svc.get_task(task_id).await {
        Ok(task) => {
            let text = super::task_detail_text(state, &task).await;
            JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
        }
        // GetArchivedTaskViaMcp: an id with no live task may be archived.
        Err(e @ crate::service::ServiceError::NotFound(_)) => {
            match state.db.archived_task(task_id).await {
                Ok(Some(record)) => {
                    let text = super::archived::archived_detail_text(&record);
                    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
                }
                Ok(None) => service_err_to_response(id, e),
                Err(read) => JsonRpcResponse::err(id, super::INTERNAL_ERROR, read.to_string()),
            }
        }
        Err(e) => service_err_to_response(id, e),
    }
}

/// Read each unique plan file once to avoid repeated I/O per task.
async fn build_plan_goal_cache(tasks: &[Task]) -> HashMap<String, String> {
    let mut cache = HashMap::new();
    for t in tasks {
        if let Some(path) = t.plan_path.as_deref() {
            if !cache.contains_key(path) {
                let goal = super::plan_goal(path).await.unwrap_or_default();
                cache.insert(path.to_owned(), goal);
            }
        }
    }
    cache
}

/// The filters a list call applied, as `name=value` in the order
/// `ListTasksViaMcp` fixes, for the empty reply. `None` when it applied none.
fn applied_filters(p: &TaskSearchArgs) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(e) = p.epic_id {
        parts.push(format!("epic_id={}", e.0));
    }
    if p.recursive {
        parts.push("recursive=true".to_string());
    }
    if let Some(paths) = &p.repo_paths {
        parts.push(format!("repo_paths=[{}]", paths.join(", ")));
    }
    if let Some(statuses) = &p.status {
        let names: Vec<&str> = statuses.0.iter().map(|s| s.as_str()).collect();
        parts.push(format!("status=[{}]", names.join(", ")));
    }
    if let Some(b) = &p.base_branch {
        parts.push(format!("base_branch={b}"));
    }
    if let Some(q) = &p.query {
        parts.push(format!("query={q:?}"));
    }
    if let Some(b) = p.bookmarked {
        parts.push(format!("bookmarked={b}"));
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

pub(super) use crate::mcp::handlers::types::empty_reply;

/// Keep the first `limit` rows (50 when `limit` is `None`). Returns the line
/// to append when rows were cut: it says how many, in what order, and how to
/// get the rest.
pub(super) fn cut_rows<T>(
    rows: &mut Vec<T>,
    limit: Option<std::num::NonZeroUsize>,
    order: &str,
    narrow_with: &str,
) -> Option<String> {
    let total = rows.len();
    let n = limit.map_or(DEFAULT_TASK_LIMIT, std::num::NonZeroUsize::get);
    if total <= n {
        return None;
    }
    rows.truncate(n);
    Some(format!(
        "Showing {n} of {total}, {order}. Narrow with {narrow_with}, or raise limit."
    ))
}

async fn list_tasks(
    state: &McpState,
    id: Option<Value>,
    parsed: TaskSearchArgs,
) -> JsonRpcResponse {
    tracing::info!(status = ?parsed.status, "MCP task_search list");

    if parsed.archived
        && (parsed.status.is_some() || parsed.recursive || parsed.bookmarked.is_some())
    {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "status, recursive and bookmarked do not apply to archived tasks".to_string(),
        );
    }
    let time_filters_set = parsed.updated_after.is_some()
        || parsed.updated_before.is_some()
        || parsed.completed_after.is_some()
        || parsed.completed_before.is_some();
    if parsed.archived && time_filters_set {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "updated_after, updated_before, completed_after and completed_before \
             do not apply to archived tasks"
                .to_string(),
        );
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

    let filters = applied_filters(&parsed);

    if parsed.archived {
        return super::archived::list_archived(
            state,
            id,
            super::archived::ArchivedFilter {
                epic_id: parsed.epic_id,
                repo_paths: parsed.repo_paths.unwrap_or_default(),
                base_branch: parsed.base_branch,
                query: parsed.query,
                limit: parsed.limit,
                filters,
            },
        )
        .await;
    }

    let status_filter: Option<Vec<TaskStatus>> = parsed.status.map(StatusFilter::into_vec);
    let epic_id = parsed.epic_id;
    if parsed.recursive && epic_id.is_none() {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            "recursive needs an epic scope: pass epic_id".to_string(),
        );
    }

    let mut filtered = match state
        .task_svc
        .list_tasks(ListTasksFilter {
            statuses: status_filter,
            epic_id,
            recursive: parsed.recursive,
            repo_paths: parsed.repo_paths,
            base_branch: parsed.base_branch,
            query: parsed.query,
        })
        .await
    {
        Ok(filtered) => filtered,
        Err(e) => return service_err_to_response(id, e),
    };

    filtered.retain(|t| updated.contains(Some(t.updated_at)) && completed.contains(t.completed_at));
    let bookmarks: std::collections::HashSet<TaskId> =
        state.db.bookmarked_task_ids().await.into_iter().collect();
    if let Some(wanted) = parsed.bookmarked {
        filtered.retain(|t| bookmarks.contains(&t.id) == wanted);
    }
    if filtered.is_empty() {
        return JsonRpcResponse::ok(
            id,
            json!({"content": [{"type": "text", "text": empty_reply("tasks", filters)}]}),
        );
    }
    // ListTasksViaMcp: most recently updated first, ties by id descending.
    filtered.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.0.cmp(&a.id.0))
    });
    let cut_hint = cut_rows(
        &mut filtered,
        parsed.limit,
        "most recently updated first",
        "epic_id, status, query or the time filters",
    );
    let (epic_titles, plan_goals, agents, ready) = tokio::join!(
        super::build_epic_titles(state),
        build_plan_goal_cache(&filtered),
        state.db.list_agents(),
        state.db.ready_task_ids()
    );
    // The display agent (agents.allium: A18): not ended, else the last to end.
    let mut by_task: HashMap<TaskId, Vec<&Agent>> = HashMap::new();
    for a in &agents {
        by_task.entry(a.task_id).or_default().push(a);
    }
    let current_agents: HashMap<TaskId, &Agent> = by_task
        .into_iter()
        .filter_map(|(task, of_task)| {
            crate::models::display_agent(of_task.iter().copied()).map(|a| (task, a))
        })
        .collect();
    let mut lines: Vec<String> = filtered
        .iter()
        .map(|t| {
            let goal = match t.plan_path.as_deref().and_then(|p| plan_goals.get(p)) {
                Some(g) if !g.is_empty() => g.clone(),
                _ => t.description.clone(),
            };
            super::format_task_line(
                t,
                bookmarks.contains(&t.id),
                ready.contains(&t.id),
                current_agents.get(&t.id).copied(),
                &epic_titles,
                &super::goal_line(&goal),
            )
        })
        .collect();
    lines.extend(cut_hint);
    JsonRpcResponse::ok(
        id,
        json!({"content": [{"type": "text", "text": lines.join("\n")}]}),
    )
}

// ---------------------------------------------------------------------------
// usage_read
// ---------------------------------------------------------------------------

fn parse_usage_since(s: &str) -> std::result::Result<chrono::DateTime<chrono::Utc>, String> {
    use chrono::TimeZone;
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .map(|ndt| chrono::Utc.from_utc_datetime(&ndt))
        })
        .map_err(|_| format!("invalid `since` datetime: {s}"))
}

pub(crate) async fn handle_usage_read(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let args: QueryUsageArgs = match parse_args(&id, args) {
        Ok(a) => a,
        Err(e) => return e,
    };

    // Reject unknown enum strings up front rather than silently returning an
    // empty result set when the caller mistypes a filter.
    if let Some(ref c) = args.category {
        if crate::models::UsageCategory::parse(c).is_none() {
            return JsonRpcResponse::err(id, INVALID_PARAMS, format!("unknown category: {c}"));
        }
    }
    if let Some(ref a) = args.actor {
        if crate::models::UsageActor::parse(a).is_none() {
            return JsonRpcResponse::err(id, INVALID_PARAMS, format!("unknown actor: {a}"));
        }
    }

    let since = match args.since.as_deref().map(parse_usage_since) {
        Some(Ok(dt)) => Some(dt),
        Some(Err(msg)) => return JsonRpcResponse::err(id, INVALID_PARAMS, msg),
        None => None,
    };

    let query = crate::store::UsageQuery {
        category: args.category,
        actor: args.actor,
        since,
        limit: args.limit.map(std::num::NonZeroUsize::get),
    };

    match state.db.query_usage(&query).await {
        Ok(summaries) => {
            let json_rows: Vec<Value> = summaries
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "category": s.category,
                        "action": s.action,
                        "detail": s.detail,
                        "actor": s.actor,
                        "count": s.count,
                        "last_used": s.last_used.to_rfc3339(),
                    })
                })
                .collect();
            let text =
                serde_json::to_string_pretty(&json_rows).unwrap_or_else(|_| "[]".to_string());
            JsonRpcResponse::ok(
                id,
                serde_json::json!({
                    "content": [{ "type": "text", "text": text }]
                }),
            )
        }
        Err(e) => service_err_to_response(id, ServiceError::Internal(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> UpdateTaskArgs {
        UpdateTaskArgs {
            task_id: TaskId(1),
            status: Some(TaskStatus::Done),
            plan_path: None,
            title: None,
            description: None,
            repo_path: None,
            sort_order: None,
            url: None,
            url_type: None,
            tag: None,
            sub_status: None,
            epic_id: None,
            base_branch: None,
            wrap_up_mode: None,
            model: None,
            auto_run_plan: None,
            phoenix: None,
            schedule: None,
            bookmarked: None,
            ready: None,
        }
    }

    /// Each field set individually must be reported by name. The exhaustive
    /// destructuring in `other_update_task_fields_set` already makes an
    /// unhandled new field a compile error (or an unused-binding warning
    /// under `-D warnings`); what this test uniquely covers is the *name* —
    /// a copy-paste that reports "title" for the `description` field
    /// compiles fine and is caught only here. Mirrors
    /// `update_task_params_every_field_covered` in
    /// `src/service/tasks/params.rs`, the same convention applied to the
    /// sibling struct.
    #[test]
    fn other_update_task_fields_set_every_field_covered() {
        let cases: Vec<(&str, UpdateTaskArgs)> = vec![
            (
                "plan_path",
                UpdateTaskArgs {
                    plan_path: Some("p".to_string()),
                    ..base_args()
                },
            ),
            (
                "title",
                UpdateTaskArgs {
                    title: Some("t".to_string()),
                    ..base_args()
                },
            ),
            (
                "description",
                UpdateTaskArgs {
                    description: Some("d".to_string()),
                    ..base_args()
                },
            ),
            (
                "repo_path",
                UpdateTaskArgs {
                    repo_path: Some("r".to_string()),
                    ..base_args()
                },
            ),
            (
                "sort_order",
                UpdateTaskArgs {
                    sort_order: Some(0),
                    ..base_args()
                },
            ),
            (
                "url",
                UpdateTaskArgs {
                    url: Some("https://example.com".to_string()),
                    ..base_args()
                },
            ),
            (
                "url_type",
                UpdateTaskArgs {
                    url_type: Some(crate::models::UrlType::Pr),
                    ..base_args()
                },
            ),
            (
                "tag",
                UpdateTaskArgs {
                    tag: Some(crate::models::TaskTag::Bug),
                    ..base_args()
                },
            ),
            (
                "sub_status",
                UpdateTaskArgs {
                    sub_status: Some(crate::models::SubStatus::Active),
                    ..base_args()
                },
            ),
            (
                "epic_id",
                UpdateTaskArgs {
                    epic_id: Some(crate::models::EpicId(1)),
                    ..base_args()
                },
            ),
            (
                "base_branch",
                UpdateTaskArgs {
                    base_branch: Some("main".to_string()),
                    ..base_args()
                },
            ),
            (
                "wrap_up_mode",
                UpdateTaskArgs {
                    wrap_up_mode: Some(Some(crate::models::WrapUpMode::Rebase)),
                    ..base_args()
                },
            ),
            (
                "model",
                UpdateTaskArgs {
                    model: Some(Some(crate::models::TaskModel::Opus)),
                    ..base_args()
                },
            ),
            (
                "auto_run_plan",
                UpdateTaskArgs {
                    auto_run_plan: Some(true),
                    ..base_args()
                },
            ),
            (
                "phoenix",
                UpdateTaskArgs {
                    phoenix: Some(true),
                    ..base_args()
                },
            ),
            (
                "schedule",
                UpdateTaskArgs {
                    schedule: Some("0 9 * * 1".into()),
                    ..base_args()
                },
            ),
            (
                "bookmarked",
                UpdateTaskArgs {
                    bookmarked: Some(true),
                    ..base_args()
                },
            ),
            (
                "ready",
                UpdateTaskArgs {
                    ready: Some(true),
                    ..base_args()
                },
            ),
        ];
        for (expected, args) in &cases {
            assert_eq!(
                other_update_task_fields_set(args),
                vec![*expected],
                "setting {expected} should report exactly that field name"
            );
        }
    }

    #[test]
    fn other_update_task_fields_set_empty_when_only_status_set() {
        assert!(other_update_task_fields_set(&base_args()).is_empty());
    }
}
