//! An orchestrated epic's memory: its log, its notes and the items that wait
//! for the operator (`core.allium: OrchestratorLogEntry`, `EpicNote`,
//! `WaitingItem`; `orchestrators.allium: OrchestratorStartsATask`,
//! `OrchestratorAnswersAnAgent`, `OrchestratorPassesOnToTheOperator`,
//! `WaitingItemResolved`, `FinishedTaskResolvesItsItems`,
//! `DeletedTaskResolvesItsItems`, `OrchestratorRecordsItsSummary`,
//! `OrchestratorThatKeepsDyingWaitsForTheOperator`, `WriteEpicNote`,
//! `ChangeEpicNote`, `RemoveEpicNote`).
//!
//! Every write but an item's resolve is an orchestrator's own: it names the
//! orchestrator, and the epic and host come from its row. The checks are pure
//! over [`StoreReads`], so `MemoryReducerCaller` calls the same functions.

use super::*;

pub const LOG_STARTED_TASK: &str = "started_task";
pub const LOG_ANSWERED: &str = "answered";
pub const LOG_PASSED_ON: &str = "passed_on";
pub const LOG_ACTED_ON_TROUBLE: &str = "acted_on_trouble";
pub const LOG_SESSION: &str = "session";
pub const LOG_SUMMARY: &str = "summary";

pub const SESSION_STARTED: &str = "session started";
pub const SESSION_RESUMED: &str = "session resumed";
pub const SESSION_ENDED: &str = "session ended";
pub const SESSION_DIED: &str = "session died";

/// `OrchestratorThatKeepsDyingWaitsForTheOperator`'s item.
pub const DOWN_ABOUT: &str = "the orchestrator's session died twice in a row";
pub const DOWN_TRIED: &str = "restarted it once";
pub const DOWN_RECOMMENDATION: &str = "open its window, or start it again from the epic view";

fn required(what: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{what} must not be empty"));
    }
    Ok(value.to_string())
}

/// The orchestrator named `name`, whose write this is.
pub fn orchestrator_writing(r: &impl StoreReads, name: &str) -> Result<Orchestrator, String> {
    r.orchestrator_named(name)
        .ok_or_else(|| format!("no orchestrator is named {name}"))
}

/// A log entry of `o`'s epic and host, its `at` left for the caller.
pub fn log_entry(o: &Orchestrator, kind: &str, task_id: i64, text: &str) -> OrchestratorLogEntry {
    OrchestratorLogEntry {
        id: 0,
        epic_id: o.epic_id,
        host: o.host.clone(),
        at: String::new(),
        kind: kind.to_string(),
        task_id,
        text: text.to_string(),
        question: String::new(),
        trouble: String::new(),
    }
}

/// The session entry `StartOrchestratorSession`, `EndOrchestratorSession`
/// and `OrchestratorWindowGone` write.
pub fn session_entry(o: &Orchestrator, text: &str) -> OrchestratorLogEntry {
    log_entry(o, LOG_SESSION, 0, text)
}

/// A task the orchestrator's epic covers, or the refusal naming it.
fn covered_task(r: &impl StoreReads, o: &Orchestrator, task_id: i64) -> Result<Task, String> {
    let task = r
        .task(task_id)
        .ok_or_else(|| format!("task {task_id} not found"))?;
    match covering_orchestration(r, task.epic_id) {
        Some(on) if on.epic_id == o.epic_id => Ok(task),
        _ => Err(format!(
            "task {task_id} is not in epic {}, which {} orchestrates",
            o.epic_id, o.name
        )),
    }
}

/// `OrchestratorStartsATask`'s entry: a start of `task_id` asked for by
/// `name` on `requester`, with its reason.
pub fn check_orchestrator_start(
    r: &impl StoreReads,
    name: &str,
    requester: &str,
    task_id: i64,
    reason: &str,
) -> Result<OrchestratorLogEntry, String> {
    let o = orchestrator_writing(r, name)?;
    if o.host != requester {
        return Err(format!(
            "{name} runs on {}, so it cannot start a task on {requester}",
            o.host
        ));
    }
    let reason = required("the reason", reason)?;
    Ok(log_entry(&o, LOG_STARTED_TASK, task_id, &reason))
}

/// `OrchestratorAnswersAnAgent`'s entry.
pub fn check_orchestrator_answer(
    r: &impl StoreReads,
    name: &str,
    task_id: i64,
    question: &str,
    answer: &str,
    reason: &str,
) -> Result<OrchestratorLogEntry, String> {
    let o = orchestrator_writing(r, name)?;
    covered_task(r, &o, task_id)?;
    let question = required("the question", question)?;
    let answer = required("the answer", answer)?;
    let reason = required("the reason", reason)?;
    Ok(OrchestratorLogEntry {
        question,
        ..log_entry(&o, LOG_ANSWERED, task_id, &format!("{answer} — {reason}"))
    })
}

/// `OrchestratorRecordsItsSummary`'s entry.
pub fn check_orchestrator_summary(
    r: &impl StoreReads,
    name: &str,
    text: &str,
) -> Result<OrchestratorLogEntry, String> {
    let o = orchestrator_writing(r, name)?;
    let epic = r
        .epic(o.epic_id)
        .ok_or_else(|| format!("epic {} does not exist", o.epic_id))?;
    if epic.status != DONE {
        return Err(format!(
            "epic {} is not done, so it has no closing summary yet",
            o.epic_id
        ));
    }
    let text = required("the summary", text)?;
    Ok(log_entry(&o, LOG_SUMMARY, 0, &text))
}

/// `OrchestratorPassesOnToTheOperator`: the item and its `passed_on` entry,
/// their stamps left for the caller. `task_id` `0` is the epic as a whole.
pub fn check_raise_waiting_item(
    r: &impl StoreReads,
    name: &str,
    task_id: i64,
    about: &str,
    tried: &str,
    recommendation: &str,
) -> Result<(WaitingItem, OrchestratorLogEntry), String> {
    let o = orchestrator_writing(r, name)?;
    if task_id != 0 {
        covered_task(r, &o, task_id)?;
    }
    let about = required("what the item is about", about)?;
    let tried = required("what was tried", tried)?;
    let recommendation = required("the recommendation", recommendation)?;
    Ok(raised(&o, task_id, &about, &tried, &recommendation))
}

fn raised(
    o: &Orchestrator,
    task_id: i64,
    about: &str,
    tried: &str,
    recommendation: &str,
) -> (WaitingItem, OrchestratorLogEntry) {
    let item = WaitingItem {
        id: 0,
        epic_id: o.epic_id,
        host: o.host.clone(),
        task_id,
        about: about.to_string(),
        tried: tried.to_string(),
        recommendation: recommendation.to_string(),
        raised_at: String::new(),
        resolved_at: String::new(),
    };
    let entry = OrchestratorLogEntry {
        question: about.to_string(),
        ..log_entry(o, LOG_PASSED_ON, task_id, recommendation)
    };
    (item, entry)
}

/// `OrchestratorThatKeepsDyingWaitsForTheOperator`: the item to raise for
/// orchestrator `id` on `host`, or `None` when it is not down after a
/// second death or such an item is already open.
pub fn check_orchestrator_down_item(
    r: &impl StoreReads,
    id: i64,
    host: &str,
) -> Result<Option<(WaitingItem, OrchestratorLogEntry)>, String> {
    let o = r
        .orchestrator(id)
        .ok_or_else(|| format!("orchestrator {id} does not exist"))?;
    if o.host != host {
        return Err(format!(
            "the orchestrator of epic {} runs on {}, not {host}",
            o.epic_id, o.host
        ));
    }
    let down = o.tmux_window.is_empty() && o.restarted_once && o.session_died;
    let open = r
        .items_of_epic(o.epic_id)
        .iter()
        .any(|i| i.host == host && i.task_id == 0 && i.resolved_at.is_empty());
    if !down || open {
        return Ok(None);
    }
    Ok(Some(raised(&o, 0, DOWN_ABOUT, DOWN_TRIED, DOWN_RECOMMENDATION)))
}

/// `WaitingItemResolved`: the item to store, or `None` when it is already
/// resolved. `resolved_at` is left for the caller.
pub fn check_resolve_waiting_item(r: &impl StoreReads, id: i64) -> Result<Option<WaitingItem>, String> {
    let item = r
        .waiting_item(id)
        .ok_or_else(|| format!("waiting item {id} does not exist"))?;
    Ok(item.resolved_at.is_empty().then_some(item))
}

/// `WriteEpicNote`: the note to insert, `written_at` left for the caller.
pub fn check_write_epic_note(r: &impl StoreReads, name: &str, text: &str) -> Result<EpicNote, String> {
    let o = orchestrator_writing(r, name)?;
    let text = required("the note", text)?;
    Ok(EpicNote {
        id: 0,
        epic_id: o.epic_id,
        text,
        done: false,
        written_at: String::new(),
    })
}

fn own_note(r: &impl StoreReads, name: &str, id: i64) -> Result<EpicNote, String> {
    let o = orchestrator_writing(r, name)?;
    let note = r.note(id).ok_or_else(|| format!("note {id} does not exist"))?;
    if note.epic_id != o.epic_id {
        return Err(format!(
            "note {id} belongs to epic {}, not to epic {}, which {name} orchestrates",
            note.epic_id, o.epic_id
        ));
    }
    Ok(note)
}

/// `ChangeEpicNote`: the note as it will be stored.
pub fn check_change_epic_note(
    r: &impl StoreReads,
    name: &str,
    id: i64,
    text: Option<&str>,
    done: Option<bool>,
) -> Result<EpicNote, String> {
    let mut note = own_note(r, name, id)?;
    if text.is_none() && done.is_none() {
        return Err("a note change needs its text, its done tick, or both".into());
    }
    if let Some(text) = text {
        note.text = required("the note", text)?;
    }
    if let Some(done) = done {
        note.done = done;
    }
    Ok(note)
}

/// `RemoveEpicNote`'s refusal.
pub fn check_remove_epic_note(r: &impl StoreReads, name: &str, id: i64) -> Result<(), String> {
    own_note(r, name, id).map(|_| ())
}

/// The open items of `task_id` (`FinishedTaskResolvesItsItems`,
/// `DeletedTaskResolvesItsItems`).
pub fn open_items_of_task(r: &impl StoreReads, task_id: i64) -> Vec<WaitingItem> {
    r.items_of_task(task_id)
        .into_iter()
        .filter(|i| i.resolved_at.is_empty())
        .collect()
}

/// The open items of `epic_id` (`SwitchOrchestratorOff`).
pub fn open_items_of_epic(r: &impl StoreReads, epic_id: i64) -> Vec<WaitingItem> {
    r.items_of_epic(epic_id)
        .into_iter()
        .filter(|i| i.resolved_at.is_empty())
        .collect()
}

/// `FinishedTaskResolvesItsItems`'s trigger.
pub fn finishes(prior_status: &str, status: &str) -> bool {
    prior_status != DONE && status == DONE
}

pub(crate) fn insert_log_entry(ctx: &ReducerContext, entry: OrchestratorLogEntry) {
    ctx.db.orchestrator_log().insert(OrchestratorLogEntry {
        at: now(ctx),
        ..entry
    });
}

fn insert_raised(ctx: &ReducerContext, (item, entry): (WaitingItem, OrchestratorLogEntry)) {
    ctx.db.waiting_items().insert(WaitingItem {
        raised_at: now(ctx),
        ..item
    });
    insert_log_entry(ctx, entry);
}

fn resolve_all(ctx: &ReducerContext, items: Vec<WaitingItem>) {
    let stamp = now(ctx);
    for item in items {
        ctx.db.waiting_items().id().update(WaitingItem {
            resolved_at: stamp.clone(),
            ..item
        });
    }
}

/// Called by every task write that can move a status.
pub(crate) fn resolve_items_on_finish(ctx: &ReducerContext, prior: &Task, next: &Task) {
    if finishes(&prior.status, &next.status) {
        resolve_all(ctx, open_items_of_task(ctx, next.id));
    }
}

/// Called with every delete of a task row.
pub(crate) fn resolve_items_of_deleted_task(ctx: &ReducerContext, task_id: i64) {
    resolve_all(ctx, open_items_of_task(ctx, task_id));
}

/// Called with the switch's delete.
pub(crate) fn resolve_items_of_epic(ctx: &ReducerContext, epic_id: i64) {
    resolve_all(ctx, open_items_of_epic(ctx, epic_id));
}

/// `DeletingAnEpicRemovesItsOrchestratorMemory`.
pub(crate) fn delete_orchestrator_memory(ctx: &ReducerContext, epic_id: i64) {
    ctx.db.orchestrator_log().epic_id().delete(epic_id);
    ctx.db.epic_notes().epic_id().delete(epic_id);
    ctx.db.waiting_items().epic_id().delete(epic_id);
}

/// `OrchestratorStartsATask`: `RequestDispatch` for an orchestrator, with
/// its `started_task` entry in the same transaction.
#[spacetimedb::reducer]
pub fn orchestrator_request_dispatch(
    ctx: &ReducerContext,
    task_id: i64,
    requester: String,
    name: String,
    reason: String,
) -> Result<(), String> {
    let entry = check_orchestrator_start(ctx, &name, &requester, task_id, &reason)?;
    request_dispatch(
        ctx,
        task_id,
        requester,
        "mcp".to_string(),
        String::new(),
    )?;
    insert_log_entry(ctx, entry);
    Ok(())
}

#[spacetimedb::reducer]
pub fn record_orchestrator_answer(
    ctx: &ReducerContext,
    name: String,
    task_id: i64,
    question: String,
    answer: String,
    reason: String,
) -> Result<(), String> {
    let entry = check_orchestrator_answer(ctx, &name, task_id, &question, &answer, &reason)?;
    require_acting_for(ctx, &entry.host)?;
    insert_log_entry(ctx, entry);
    Ok(())
}

#[spacetimedb::reducer]
pub fn record_orchestrator_summary(ctx: &ReducerContext, name: String, text: String) -> Result<(), String> {
    let entry = check_orchestrator_summary(ctx, &name, &text)?;
    require_acting_for(ctx, &entry.host)?;
    insert_log_entry(ctx, entry);
    Ok(())
}

#[spacetimedb::reducer]
pub fn raise_waiting_item(
    ctx: &ReducerContext,
    name: String,
    task_id: i64,
    about: String,
    tried: String,
    recommendation: String,
) -> Result<(), String> {
    let raised = check_raise_waiting_item(ctx, &name, task_id, &about, &tried, &recommendation)?;
    require_acting_for(ctx, &raised.0.host)?;
    insert_raised(ctx, raised);
    Ok(())
}

#[spacetimedb::reducer]
pub fn raise_orchestrator_down_item(ctx: &ReducerContext, id: i64, host: String) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    if let Some(raised) = check_orchestrator_down_item(ctx, id, &host)? {
        insert_raised(ctx, raised);
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn resolve_waiting_item(ctx: &ReducerContext, id: i64) -> Result<(), String> {
    if let Some(item) = check_resolve_waiting_item(ctx, id)? {
        resolve_all(ctx, vec![item]);
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn write_epic_note(ctx: &ReducerContext, name: String, text: String) -> Result<(), String> {
    let note = check_write_epic_note(ctx, &name, &text)?;
    require_acting_for_orchestrator(ctx, &name)?;
    ctx.db.epic_notes().insert(EpicNote {
        written_at: now(ctx),
        ..note
    });
    Ok(())
}

#[spacetimedb::reducer]
pub fn change_epic_note(
    ctx: &ReducerContext,
    name: String,
    id: i64,
    text: Option<String>,
    done: Option<bool>,
) -> Result<(), String> {
    let note = check_change_epic_note(ctx, &name, id, text.as_deref(), done)?;
    require_acting_for_orchestrator(ctx, &name)?;
    ctx.db.epic_notes().id().update(note);
    Ok(())
}

#[spacetimedb::reducer]
pub fn remove_epic_note(ctx: &ReducerContext, name: String, id: i64) -> Result<(), String> {
    check_remove_epic_note(ctx, &name, id)?;
    require_acting_for_orchestrator(ctx, &name)?;
    ctx.db.epic_notes().id().delete(id);
    Ok(())
}

/// The caller acts for the host the orchestrator named `name` runs on.
fn require_acting_for_orchestrator(ctx: &ReducerContext, name: &str) -> Result<(), String> {
    let o = orchestrator_writing(ctx, name)?;
    require_acting_for(ctx, &o.host)
}

pub fn orchestrator_log_of_held(r: &impl StoreReads, who: &str) -> Vec<OrchestratorLogEntry> {
    held_epics_of(r, who)
        .into_iter()
        .flat_map(|e| r.log_of_epic(e.id))
        .collect()
}

pub fn epic_notes_of_held(r: &impl StoreReads, who: &str) -> Vec<EpicNote> {
    held_epics_of(r, who)
        .into_iter()
        .flat_map(|e| r.notes_of_epic(e.id))
        .collect()
}

pub fn waiting_items_of_held(r: &impl StoreReads, who: &str) -> Vec<WaitingItem> {
    held_epics_of(r, who)
        .into_iter()
        .flat_map(|e| r.items_of_epic(e.id))
        .collect()
}
