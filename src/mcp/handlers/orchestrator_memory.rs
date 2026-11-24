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
    Epic, EpicNote, Orchestrator, OrchestratorLogEntry, Task, TaskId, TaskStatus, WaitingItem,
};
use crate::sync::writes::StoreRefusal;

use super::types::{
    deserialize_flexible_id, parse_args, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS,
};

/// `orchestrators.allium: config.orchestrator_log_shown`: how many log
/// entries a read and the prompt show, newest first.
pub(crate) const LOG_SHOWN: usize = 10;

/// `orchestrator-log_search`'s default page.
const LOG_PAGE: usize = 50;

fn text(id: Option<Value>, text: String) -> JsonRpcResponse {
    JsonRpcResponse::ok(id, json!({"content": [{"type": "text", "text": text}]}))
}

/// The store's refusal as invalid params, in its own words; anything else
/// as an internal error.
fn write_err(id: Option<Value>, e: anyhow::Error) -> JsonRpcResponse {
    match e.downcast_ref::<StoreRefusal>() {
        Some(refusal) => JsonRpcResponse::err(id, INVALID_PARAMS, refusal.0.clone()),
        None => JsonRpcResponse::err(id, INTERNAL_ERROR, format!("{e:#}")),
    }
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

/// One log entry as every read renders it.
pub(crate) fn log_line(e: &OrchestratorLogEntry) -> String {
    let mut line = format!(
        "#{} {} {} {}",
        e.id,
        e.at.format("%Y-%m-%dT%H:%M:%SZ"),
        e.host,
        e.kind
    );
    if let Some(task) = e.task_id {
        line.push_str(&format!(" task #{}", task.0));
    }
    if let Some(trouble) = &e.trouble {
        line.push_str(&format!(" ({trouble})"));
    }
    if let Some(q) = &e.question {
        line.push_str(&format!(" — asked: {q}"));
    }
    line.push_str(&format!(" — {}", e.text));
    line
}

/// One open item as the epic read and the prompt render it.
pub(crate) fn item_line(i: &WaitingItem) -> String {
    let about = match i.task_id {
        Some(task) => format!("task #{}: {}", task.0, i.about),
        None => format!("the epic: {}", i.about),
    };
    format!(
        "#{} {about} — tried: {} — recommends: {} (raised {}, {})",
        i.id,
        i.tried,
        i.recommendation,
        i.raised_at.format("%Y-%m-%dT%H:%M:%SZ"),
        i.host
    )
}

/// One note as the epic read and the prompt render it.
pub(crate) fn note_line(n: &EpicNote) -> String {
    let done = if n.done { " [done]" } else { "" };
    format!("#{}{done} {}", n.id, n.text)
}

/// The notes of `epic`, open ones first, each group by id.
pub(crate) fn notes_in_order(notes: &[EpicNote], epic: crate::models::EpicId) -> Vec<&EpicNote> {
    let mut mine: Vec<&EpicNote> = notes.iter().filter(|n| n.epic_id == epic).collect();
    mine.sort_by_key(|n| (n.done, n.id));
    mine
}

/// The open items of `epic`, oldest first.
pub(crate) fn open_items(items: &[WaitingItem], epic: crate::models::EpicId) -> Vec<&WaitingItem> {
    let mut open: Vec<&WaitingItem> = items
        .iter()
        .filter(|i| i.epic_id == epic && i.is_open())
        .collect();
    open.sort_by_key(|i| i.id);
    open
}

/// The log entries that pass `keep`, newest first.
pub(crate) fn newest_first(
    log: &[OrchestratorLogEntry],
    keep: impl Fn(&OrchestratorLogEntry) -> bool,
) -> Vec<&OrchestratorLogEntry> {
    let mut kept: Vec<&OrchestratorLogEntry> = log.iter().filter(|e| keep(e)).collect();
    kept.sort_by_key(|e| std::cmp::Reverse(e.id));
    kept
}

/// `orchestrators.allium: orchestrator_state`.
pub(crate) fn orchestrator_state(o: &Orchestrator, subtree_tasks: &[&Task]) -> &'static str {
    let open = subtree_tasks.iter().any(|t| t.status != TaskStatus::Done);
    let running_here = subtree_tasks
        .iter()
        .any(|t| t.status == TaskStatus::Running && t.host.as_deref() == Some(o.host.as_str()));
    match (o.is_live(), open, running_here) {
        (true, _, true) => "working",
        (true, _, false) => "idle",
        (false, false, _) => "resting",
        (false, true, _) => "down",
    }
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
        state.db.list_orchestrator_log(),
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
    let entries = newest_first(&log, |e| e.epic_id == epic.id);
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
                    "Raised waiting item #{item} for the operator in epic {}. Resolve it with waiting-item_resolve(item_id={item}) once the operator has answered.",
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
    item_id: i64,
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
        .find(|i| i.id == a.item_id);
    match state.db.resolve_waiting_item(a.item_id).await {
        Ok(()) => {
            let reply = match before {
                Some(item) if !item.is_open() => {
                    format!(
                        "Waiting item #{} was already resolved; nothing changed.",
                        a.item_id
                    )
                }
                Some(item) => {
                    state.notify_epic_changed(item.epic_id);
                    format!(
                        "Resolved waiting item #{} of epic {}.",
                        a.item_id, item.epic_id.0
                    )
                }
                None => format!("Resolved waiting item #{}.", a.item_id),
            };
            text(id, reply)
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
    note_id: i64,
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
        .change_epic_note(&o.name, a.note_id, a.text.as_deref(), a.done)
        .await
    {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!("Updated note #{} of epic {}.", a.note_id, o.epic_id.0),
            )
        }
        Err(e) => write_err(id, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteDeleteArgs {
    note_id: i64,
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
    match state.db.remove_epic_note(&o.name, a.note_id).await {
        Ok(()) => {
            state.notify_epic_changed(o.epic_id);
            text(
                id,
                format!("Deleted note #{} of epic {}.", a.note_id, o.epic_id.0),
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
    #[serde(default)]
    before: Option<i64>,
    #[serde(default)]
    limit: Option<usize>,
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
    if a.limit == Some(0) {
        return JsonRpcResponse::err(id, INVALID_PARAMS, "limit must be at least 1".to_string());
    }
    let task = a.task_id.map(|t| t.0);
    let log = state.db.list_orchestrator_log().await;
    let entries = newest_first(&log, |e| {
        e.epic_id == a.epic_id
            && task.is_none_or(|t| e.task_id == Some(t))
            && a.before.is_none_or(|b| e.id < b)
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
    let limit = a.limit.unwrap_or(LOG_PAGE);
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
