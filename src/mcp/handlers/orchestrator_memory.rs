//! An orchestrator's memory tools and the reads of it (`orchestrators.allium:
//! OrchestratorMcpTools`; `mcp-task-tools.allium: OrchestratorMemoryViaMcp`,
//! `ResolveWaitingItemViaMcp`, `ReadOrchestratorLogViaMcp`). Every write but
//! a resolve is the calling orchestrator's own: its epic and host come from
//! its row, never from an argument.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{
    item_line, log_line, newest_first, note_line, notes_in_order, open_items, orchestrator_state,
    Epic, Orchestrator, OrchestratorLogEntry, Task, TaskId, ORCHESTRATOR_LOG_SHOWN as LOG_SHOWN,
};

use super::types::{
    deserialize_flexible_i64, deserialize_flexible_id, deserialize_optional_flexible_i64,
    deserialize_optional_limit, parse_args, service_err_to_response, JsonRpcResponse,
    INVALID_PARAMS,
};

/// `orchestrator-log_search`'s default page.
const LOG_PAGE: usize = 50;

fn text(id: Option<Value>, text: String) -> JsonRpcResponse {
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

fn write_err(id: Option<Value>, e: anyhow::Error) -> JsonRpcResponse {
    service_err_to_response(id, e.into())
}

/// The calling orchestrator, or the refusal for any other caller.
async fn calling_orchestrator(
    state: &McpState,
    identity: &CallerIdentity,
    tool: &str,
) -> Result<Orchestrator, String> {
    let CallerIdentity::Orchestrator(name) = identity else {
        return Err(format!(
            "{tool} is an orchestrator's tool: only an epic's orchestrator calls it"
        ));
    };
    state
        .db
        .orchestrator_named(name)
        .await
        .ok_or_else(|| format!("no orchestrator is named {name}"))
}

macro_rules! orchestrator_or_refuse {
    ($state:expr, $id:expr, $identity:expr, $tool:literal) => {
        match calling_orchestrator($state, $identity, $tool).await {
            Ok(o) => o,
            Err(why) => return JsonRpcResponse::err($id, INVALID_PARAMS, why),
        }
    };
}

/// The more-entries line a read ends on when it cut the log.
fn more_line(
    shown: usize,
    total: usize,
    epic: crate::models::EpicId,
    task: Option<TaskId>,
) -> String {
    let task = task
        .map(|t| format!(", task_id={}", t.0))
        .unwrap_or_default();
    format!(
        "\n  ({shown} of {total} shown; orchestrator-log_search(epic_id={}{task}) reads the rest)",
        epic.0
    )
}

/// `GetEpicViaMcp`'s orchestrator lines after its switch line, for an epic
/// whose own switch is on, each with its leading newline.
pub(crate) async fn epic_memory_lines(state: &McpState, epic: &Epic) -> String {
    let (orchestrators, log, notes, items, epics, tasks) = tokio::join!(
        state.db.list_orchestrators(),
        state.db.orchestrator_log_of_epic(epic.id),
        state.db.list_epic_notes(),
        state.db.list_waiting_items(),
        state.db.list_epics(),
        state.db.list_all_tasks_with_epic_id()
    );
    let subtree = crate::models::epics::descendant_epic_ids(epic.id, &epics);
    let subtree_tasks: Vec<&Task> = tasks
        .iter()
        .filter(|t| {
            t.epic_id
                .is_some_and(|e| e == epic.id || subtree.contains(&e))
        })
        .collect();
    let mut out = String::new();
    for o in orchestrators.iter().filter(|o| o.epic_id == epic.id) {
        out.push_str(&format!(
            "\nOrchestrator {} on {}: {}",
            o.name,
            o.host,
            orchestrator_state(o, &subtree_tasks)
        ));
    }
    let open = open_items(&items, epic.id);
    if !open.is_empty() {
        out.push_str("\nWaiting for the operator:");
        for i in open {
            out.push_str(&format!("\n  {}", item_line(i)));
        }
    }
    let notes = notes_in_order(&notes, epic.id);
    if !notes.is_empty() {
        out.push_str("\nNotes:");
        for n in notes {
            out.push_str(&format!("\n  {}", note_line(n)));
        }
    }
    let entries = newest_first(&log, |_| true);
    if !entries.is_empty() {
        out.push_str("\nLog:");
        for e in entries.iter().take(LOG_SHOWN) {
            out.push_str(&format!("\n  {}", log_line(e)));
        }
        if entries.len() > LOG_SHOWN {
            out.push_str(&more_line(LOG_SHOWN, entries.len(), epic.id, None));
        }
    }
    out
}

/// `GetTaskViaMcp`'s `Orchestrator log` lines, with their leading newline,
/// or nothing when no entry names the task.
pub(crate) fn task_log_lines(log: &[OrchestratorLogEntry], task: &Task) -> String {
    let entries = newest_first(log, |e| e.task_id == Some(task.id));
    let Some(first) = entries.first() else {
        return String::new();
    };
    let mut out = String::from("\nOrchestrator log:");
    for e in entries.iter().take(LOG_SHOWN) {
        out.push_str(&format!("\n  {}", log_line(e)));
    }
    if entries.len() > LOG_SHOWN {
        out.push_str(&more_line(
            LOG_SHOWN,
            entries.len(),
            first.epic_id,
            Some(task.id),
        ));
    }
    out
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    task_id: TaskId,
    question: String,
    answer: String,
    reason: String,
}

pub(super) async fn handle_answer_record(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<AnswerArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "orchestrator-answer_record");
    match state
        .db
        .record_orchestrator_answer(&o.name, a.task_id, &a.question, &a.answer, &a.reason)
        .await
    {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!(
                    "Recorded your answer to task #{} in epic {}'s log. Send the answer to its agent as a message if you have not.",
                    a.task_id.0, o.epic_id.0
                ),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TroubleArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    task_id: TaskId,
    trouble: String,
    text: String,
}

pub(super) async fn handle_trouble_record(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<TroubleArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "orchestrator-trouble_record");
    match state
        .db
        .record_orchestrator_trouble(&o.name, a.task_id, &a.trouble, &a.text)
        .await
    {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!(
                    "Recorded your retry for {} on task #{}. This is its one retry: if the trouble comes back, raise a waiting item.",
                    a.trouble, a.task_id.0
                ),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryArgs {
    text: String,
}

pub(super) async fn handle_summary_record(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<SummaryArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "orchestrator-summary_record");
    match state.db.record_orchestrator_summary(&o.name, &a.text).await {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!("Recorded the closing summary of epic {}.", o.epic_id.0),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RaiseArgs {
    #[serde(default)]
    task_id: Option<super::types::FlexibleId<TaskId>>,
    about: String,
    tried: String,
    recommendation: String,
}

pub(super) async fn handle_item_create(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<RaiseArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "waiting-item_create");
    let task = a.task_id.map(|t| t.0);
    match state
        .db
        .raise_waiting_item(&o.name, task, &a.about, &a.tried, &a.recommendation)
        .await
    {
        Ok(item) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!(
                    "Raised waiting item #{item} for the operator in epic {}. Resolve it with waiting-item_resolve(waiting_item_id={item}) once the operator has answered.",
                    o.epic_id.0
                ),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    waiting_item_id: i64,
}

pub(super) async fn handle_item_resolve(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<ResolveArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let before = state
        .db
        .list_waiting_items()
        .await
        .into_iter()
        .find(|i| i.id == a.waiting_item_id);
    match state.db.resolve_waiting_item(a.waiting_item_id).await {
        Ok(()) => {
            let reply = match before {
                Some(item) if !item.is_open() => {
                    format!(
                        "Waiting item #{} was already resolved; nothing changed.",
                        a.waiting_item_id
                    )
                }
                Some(item) => {
                    state.notify_epic_changed(item.epic_id);
                    format!(
                        "Resolved waiting item #{} of epic {}.",
                        a.waiting_item_id, item.epic_id.0
                    )
                }
                None => format!("Resolved waiting item #{}.", a.waiting_item_id),
            };
            text(id, reply)
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    epic_id: i64,
}

/// `mcp-task-tools.allium: RestartOrchestratorViaMcp`.
pub(super) async fn handle_orchestrator_restart(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<RestartArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let epic = crate::models::EpicId(a.epic_id);
    let Some((covering, _)) = crate::store::covering(&*state.db, Some(epic)).await else {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            format!("no orchestrator covers epic {}", a.epic_id),
        );
    };
    let host = crate::store::HostStore::host_id(&*state.db).to_string();
    let Some(o) = state
        .db
        .list_orchestrators()
        .await
        .into_iter()
        .find(|o| o.epic_id == covering.id && o.host == host)
    else {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            format!(
                "epic {} has no orchestrator on this machine yet; the board makes one once the epic has open work",
                covering.id.0
            ),
        );
    };
    if o.is_live() {
        return JsonRpcResponse::err(
            id,
            INVALID_PARAMS,
            format!(
                "orchestrator {} of epic {} is running; there is nothing to restart",
                o.name, covering.id.0
            ),
        );
    }
    match state.db.restart_orchestrator(o.id, &host).await {
        Ok(()) => {
            state.notify_epic_changed(covering.id);
            text(
                id,
                format!(
                    "Restarted orchestrator {} of {}: the board starts its session at its next check.",
                    o.name,
                    covering.reference()
                ),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteCreateArgs {
    text: String,
}

pub(super) async fn handle_note_create(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<NoteCreateArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "epic-note_create");
    match state.db.write_epic_note(&o.name, &a.text).await {
        Ok(note) => {
            state.notify_epic_changed(o.epic_id);
            text(id, format!("Wrote note #{note} for epic {}.", o.epic_id.0))
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteUpdateArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    epic_note_id: i64,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    done: Option<bool>,
}

pub(super) async fn handle_note_update(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<NoteUpdateArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "epic-note_update");
    match state
        .db
        .change_epic_note(&o.name, a.epic_note_id, a.text.as_deref(), a.done)
        .await
    {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!("Updated note #{} of epic {}.", a.epic_note_id, o.epic_id.0),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteDeleteArgs {
    #[serde(deserialize_with = "deserialize_flexible_i64")]
    epic_note_id: i64,
}

pub(super) async fn handle_note_delete(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<NoteDeleteArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let o = orchestrator_or_refuse!(state, id, identity, "epic-note_delete");
    match state.db.remove_epic_note(&o.name, a.epic_note_id).await {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!("Deleted note #{} of epic {}.", a.epic_note_id, o.epic_id.0),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogSearchArgs {
    #[serde(deserialize_with = "deserialize_flexible_id")]
    epic_id: crate::models::EpicId,
    #[serde(default)]
    task_id: Option<super::types::FlexibleId<TaskId>>,
    #[serde(default, deserialize_with = "deserialize_optional_flexible_i64")]
    before: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    limit: Option<std::num::NonZeroUsize>,
}

pub(super) async fn handle_log_search(
    state: &McpState,
    id: Option<Value>,
    _identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let a = match parse_args::<LogSearchArgs>(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let task = a.task_id.map(|t| t.0);
    let log = state.db.orchestrator_log_of_epic(a.epic_id).await;
    let entries = newest_first(&log, |e| {
        task.is_none_or(|t| e.task_id == Some(t)) && a.before.is_none_or(|b| e.id < b)
    });
    if entries.is_empty() {
        return text(
            id,
            format!(
                "No log entries for epic {} with these filters.",
                a.epic_id.0
            ),
        );
    }
    let limit = a.limit.map_or(LOG_PAGE, std::num::NonZeroUsize::get);
    let mut out: Vec<String> = entries.iter().take(limit).map(|e| log_line(e)).collect();
    if entries.len() > limit {
        let last = entries[limit - 1].id;
        out.push(format!(
            "{limit} of {} shown, newest first; pass before={last} for the next page.",
            entries.len()
        ));
    }
    text(id, out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskStatus;
    use crate::models::{EpicId, TaskBuilder};

    fn row(live: bool) -> Orchestrator {
        Orchestrator {
            id: 1,
            epic_id: EpicId(1),
            host: "host-a".into(),
            name: "o".into(),
            tmux_window: live.then(|| crate::models::TmuxWindow::for_orchestrator(EpicId(1))),
            started_at: None,
            restarted_once: false,
            session_died: false,
            last_turn_at: None,
        }
    }

    fn task(status: TaskStatus, host: Option<&str>) -> Task {
        let mut t = TaskBuilder::new(1).epic(Some(1)).status(status).build();
        t.host = host.map(str::to_string);
        t
    }

    /// orchestrators.allium: orchestrator_state.
    #[test]
    fn the_state_follows_the_session_and_the_work() {
        let running_here = task(TaskStatus::Running, Some("host-a"));
        let running_there = task(TaskStatus::Running, Some("host-b"));
        let done = task(TaskStatus::Done, None);

        assert_eq!(orchestrator_state(&row(true), &[&running_here]), "working");
        assert_eq!(orchestrator_state(&row(true), &[&running_there]), "idle");
        assert_eq!(orchestrator_state(&row(false), &[&done]), "resting");
        assert_eq!(orchestrator_state(&row(false), &[&running_there]), "down");
    }
}
