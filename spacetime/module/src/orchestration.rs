//! The orchestrator switch on an epic and the ready mark on a backlog task
//! (`core.allium: EpicOrchestration`, `ReadyMark`).
//!
//! The checks are pure over [`StoreReads`], so `MemoryReducerCaller` calls
//! the same functions and the two cannot disagree about a verdict or its
//! wording.

use super::*;

pub const START_RULE_MARKED_ONLY: &str = "marked_only";
pub const START_RULE_EVERYTHING: &str = "everything";
pub const READY_MARKER_OPERATOR: &str = "operator";
pub const READY_MARKER_AGENT: &str = "agent";

fn check_start_rule(rule: &str) -> Result<(), String> {
    match rule {
        START_RULE_MARKED_ONLY | START_RULE_EVERYTHING => Ok(()),
        other => Err(format!(
            "unknown start rule {other:?}: expected {START_RULE_MARKED_ONLY} or {START_RULE_EVERYTHING}"
        )),
    }
}

fn check_max_parallel(max_parallel: Option<i64>) -> Result<(), String> {
    match max_parallel {
        Some(n) if n < 1 => Err("the limit must be at least 1".into()),
        _ => Ok(()),
    }
}

/// `orchestrators.allium`'s `covering(epic)`: the switch on `epic_id` or on
/// its nearest ancestor that has one. `0` (no epic) has none.
pub fn covering_orchestration(r: &impl StoreReads, epic_id: i64) -> Option<EpicOrchestration> {
    let mut next = epic_id;
    for _ in 0..=MAX_EPIC_DEPTH {
        if next == 0 {
            return None;
        }
        if let Some(found) = r.orchestration_of(next) {
            return Some(found);
        }
        next = r.epic(next)?.parent_epic_id;
    }
    None
}

/// Every epic id under `root`, `root` itself excluded, ascending.
fn descendant_epic_ids(r: &impl StoreReads, root: i64) -> Vec<i64> {
    let mut seen = std::collections::BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        for child in r.epics_under(id) {
            if child.id != root && seen.insert(child.id) {
                pending.push(child.id);
            }
        }
    }
    seen.into_iter().collect()
}

fn named(r: &impl StoreReads, epic_id: i64) -> String {
    let title = r.epic(epic_id).map(|e| e.title).unwrap_or_default();
    format!("epic {epic_id} ({title})")
}

/// `orchestrators.allium: SwitchOrchestratorOn`'s refusals, and the row it
/// writes, its `switched_on_at` left for the caller to stamp.
pub fn check_switch_orchestrator_on(
    r: &impl StoreReads,
    epic_id: i64,
    start_rule: Option<&str>,
    max_parallel: Option<i64>,
) -> Result<EpicOrchestration, String> {
    let epic = r
        .epic(epic_id)
        .ok_or_else(|| format!("epic {epic_id} does not exist"))?;
    if r.orchestration_of(epic_id).is_some() {
        return Err(format!("epic {epic_id} already has an orchestrator"));
    }
    if let Some(below) = descendant_epic_ids(r, epic_id)
        .into_iter()
        .find(|id| r.orchestration_of(*id).is_some())
    {
        return Err(format!(
            "{} below this one has an orchestrator",
            named(r, below)
        ));
    }
    if let Some(above) = covering_orchestration(r, epic.parent_epic_id) {
        return Err(format!(
            "{} above this one has an orchestrator",
            named(r, above.epic_id)
        ));
    }
    let start_rule = start_rule.unwrap_or(START_RULE_MARKED_ONLY);
    check_start_rule(start_rule)?;
    check_max_parallel(max_parallel)?;
    Ok(EpicOrchestration {
        epic_id,
        start_rule: start_rule.to_string(),
        max_parallel: max_parallel.unwrap_or(0),
        switched_on_at: String::new(),
    })
}

fn require_orchestration(r: &impl StoreReads, epic_id: i64) -> Result<EpicOrchestration, String> {
    r.orchestration_of(epic_id)
        .ok_or_else(|| format!("epic {epic_id} has no orchestrator"))
}

/// `orchestrators.allium: ChangeOrchestratorSettings`: the switch as it will
/// be stored.
pub fn check_change_orchestrator_settings(
    r: &impl StoreReads,
    epic_id: i64,
    start_rule: Option<&str>,
    max_parallel: Option<i64>,
) -> Result<EpicOrchestration, String> {
    let mut row = require_orchestration(r, epic_id)?;
    if let Some(rule) = start_rule {
        check_start_rule(rule)?;
        row.start_rule = rule.to_string();
    }
    check_max_parallel(max_parallel)?;
    if let Some(n) = max_parallel {
        row.max_parallel = n;
    }
    Ok(row)
}

/// `orchestrators.allium: SwitchOrchestratorOff`'s refusal.
pub fn check_switch_orchestrator_off(r: &impl StoreReads, epic_id: i64) -> Result<(), String> {
    require_orchestration(r, epic_id).map(|_| ())
}

/// `epics.allium: ReparentingKeepsOneOrchestratorOnAPath`: the switches in
/// `epic_id`'s subtree, itself included, that go off when it lands under
/// `new_parent`.
pub fn switches_off_on_reparent(r: &impl StoreReads, epic_id: i64, new_parent: i64) -> Vec<i64> {
    if covering_orchestration(r, new_parent).is_none() {
        return Vec::new();
    }
    std::iter::once(epic_id)
        .chain(descendant_epic_ids(r, epic_id))
        .filter(|id| r.orchestration_of(*id).is_some())
        .collect()
}

/// `tasks.allium: MarkTaskReady`: the mark to write, or `None` when the task
/// is already ready. `agent_name` is `""` for none; `marked_at` is left for
/// the caller to stamp.
pub fn check_mark_task_ready(
    r: &impl StoreReads,
    task_id: i64,
    marked_by: &str,
    agent_name: &str,
) -> Result<Option<ReadyMark>, String> {
    let task = r
        .task(task_id)
        .ok_or_else(|| format!("task {task_id} not found"))?;
    if task.status != BACKLOG {
        return Err("only a backlog task can be marked ready".into());
    }
    let named_agent = !agent_name.trim().is_empty();
    match marked_by {
        READY_MARKER_AGENT if !named_agent => {
            return Err("an agent's ready mark names the agent".into())
        }
        READY_MARKER_OPERATOR if named_agent => {
            return Err("the operator's ready mark names no agent".into())
        }
        READY_MARKER_AGENT | READY_MARKER_OPERATOR => {}
        other => {
            return Err(format!(
                "unknown marker {other:?}: expected {READY_MARKER_OPERATOR} or {READY_MARKER_AGENT}"
            ))
        }
    }
    if r.ready_mark_of(task_id).is_some() {
        return Ok(None);
    }
    Ok(Some(ReadyMark {
        task_id,
        marked_by: marked_by.to_string(),
        agent_name: agent_name.trim().to_string(),
        marked_at: String::new(),
    }))
}

/// `tasks.allium: ClearReadyMark`'s refusal.
pub fn check_clear_ready_mark(r: &impl StoreReads, task_id: i64) -> Result<(), String> {
    r.task(task_id)
        .map(|_| ())
        .ok_or_else(|| format!("task {task_id} not found"))
}

/// `tasks.allium: LeavingBacklogDropsTheReadyMark`'s trigger.
pub fn leaves_backlog(prior_status: &str, status: &str) -> bool {
    prior_status == BACKLOG && status != BACKLOG
}

#[spacetimedb::reducer]
pub fn switch_orchestrator_on(
    ctx: &ReducerContext,
    epic_id: i64,
    start_rule: Option<String>,
    max_parallel: Option<i64>,
) -> Result<(), String> {
    let row = check_switch_orchestrator_on(ctx, epic_id, start_rule.as_deref(), max_parallel)?;
    ctx.db.epic_orchestrations().insert(EpicOrchestration {
        switched_on_at: now(ctx),
        ..row
    });
    Ok(())
}

#[spacetimedb::reducer]
pub fn change_orchestrator_settings(
    ctx: &ReducerContext,
    epic_id: i64,
    start_rule: Option<String>,
    max_parallel: Option<i64>,
) -> Result<(), String> {
    let row =
        check_change_orchestrator_settings(ctx, epic_id, start_rule.as_deref(), max_parallel)?;
    ctx.db.epic_orchestrations().epic_id().update(row);
    Ok(())
}

#[spacetimedb::reducer]
pub fn switch_orchestrator_off(ctx: &ReducerContext, epic_id: i64) -> Result<(), String> {
    check_switch_orchestrator_off(ctx, epic_id)?;
    ctx.db.epic_orchestrations().epic_id().delete(epic_id);
    Ok(())
}

#[spacetimedb::reducer]
pub fn mark_task_ready(
    ctx: &ReducerContext,
    task_id: i64,
    marked_by: String,
    agent_name: String,
) -> Result<(), String> {
    if let Some(row) = check_mark_task_ready(ctx, task_id, &marked_by, &agent_name)? {
        ctx.db.ready_marks().insert(ReadyMark {
            marked_at: now(ctx),
            ..row
        });
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn clear_ready_mark(ctx: &ReducerContext, task_id: i64) -> Result<(), String> {
    check_clear_ready_mark(ctx, task_id)?;
    ctx.db.ready_marks().task_id().delete(task_id);
    Ok(())
}

/// Called by every task write that can move a status.
pub(crate) fn drop_ready_mark_on_leaving_backlog(ctx: &ReducerContext, prior: &Task, next: &Task) {
    if leaves_backlog(&prior.status, &next.status) {
        ctx.db.ready_marks().task_id().delete(next.id);
    }
}

/// Called after an epic's parent changed.
pub(crate) fn switch_off_on_reparent(ctx: &ReducerContext, epic_id: i64, new_parent: i64) {
    for id in switches_off_on_reparent(ctx, epic_id, new_parent) {
        ctx.db.epic_orchestrations().epic_id().delete(id);
    }
}

pub fn orchestrations_of_held(r: &impl StoreReads, who: &str) -> Vec<EpicOrchestration> {
    held_epics_of(r, who)
        .into_iter()
        .filter_map(|e| r.orchestration_of(e.id))
        .collect()
}

pub fn ready_marks_of_held(r: &impl StoreReads, who: &str) -> Vec<ReadyMark> {
    held_tasks_of(r, who)
        .into_iter()
        .filter_map(|t| r.ready_mark_of(t.id))
        .collect()
}
