//! An orchestrated epic's orchestrator on one machine (`core.allium:
//! Orchestrator`; `orchestrators.allium: BoardRunsItsOrchestrators`,
//! `OrchestratorWindowGone`, `StartOrchestratorSession`,
//! `OrchestratorTurnClearsTheRestartFlag`, `EndOrchestratorSession`).
//!
//! The store keeps the row and decides every change to it; the board on the
//! row's host opens and closes the window. The checks are pure over
//! [`StoreReads`], so `MemoryReducerCaller` calls the same functions.

use super::*;

/// `orchestrator_window(o)` in `orchestrators.allium`.
pub fn orchestrator_window_name(epic_id: i64) -> String {
    format!("orch-{epic_id}")
}

/// Whether an agent or an orchestrator holds `name`
/// (`OrchestratorNamesAreUnique`, `AgentNamesAreUnique`).
pub fn name_taken(r: &impl StoreReads, name: &str) -> bool {
    r.agent_named(name) || r.orchestrator_named(name).is_some()
}

/// `BoardRunsItsOrchestrators`'s create: the row to insert for `epic_id` on
/// `host`, its name picked from `seed`, or `None` when the pair has one
/// (`OneOrchestratorPerEpicAndHost`). Its id is left for the store.
pub fn check_ensure_orchestrator(
    r: &impl StoreReads,
    epic_id: i64,
    host: &str,
    seed: u64,
) -> Result<Option<Orchestrator>, String> {
    if r.orchestration_of(epic_id).is_none() {
        return Err(format!("epic {epic_id} has no orchestrator"));
    }
    if host.trim().is_empty() {
        return Err("an orchestrator runs on a named host".into());
    }
    if r.orchestrators_of_epic(epic_id).iter().any(|o| o.host == host) {
        return Ok(None);
    }
    Ok(Some(Orchestrator {
        id: 0,
        epic_id,
        host: host.to_string(),
        name: pick_agent_name(seed, |n| name_taken(r, n)),
        tmux_window: String::new(),
        started_at: String::new(),
        restarted_once: false,
        session_died: false,
        last_turn_at: String::new(),
    }))
}

fn own_orchestrator(r: &impl StoreReads, id: i64, host: &str) -> Result<Orchestrator, String> {
    let row = r
        .orchestrator(id)
        .ok_or_else(|| format!("orchestrator {id} does not exist"))?;
    if row.host != host {
        return Err(format!(
            "the orchestrator of epic {} runs on {}, so {host} cannot start or end it",
            row.epic_id, row.host
        ));
    }
    Ok(row)
}

/// `StartOrchestratorSession`: the row once its session has started in
/// `orchestrator_window_name`, its `started_at` left for the caller.
pub fn check_start_orchestrator_session(
    r: &impl StoreReads,
    id: i64,
    host: &str,
) -> Result<Orchestrator, String> {
    let row = own_orchestrator(r, id, host)?;
    if !row.tmux_window.is_empty() {
        return Err(format!("the orchestrator of epic {} is already running", row.epic_id));
    }
    if row.restarted_once && row.session_died {
        return Err(format!(
            "the orchestrator of epic {} died twice in a row; it waits for the operator",
            row.epic_id
        ));
    }
    Ok(Orchestrator {
        tmux_window: orchestrator_window_name(row.epic_id),
        restarted_once: row.restarted_once || row.session_died,
        session_died: false,
        ..row
    })
}

/// The text of a session start's log entry.
pub fn start_entry_text(resumed: bool) -> &'static str {
    if resumed {
        SESSION_RESUMED
    } else {
        SESSION_STARTED
    }
}

/// The text of a session end's log entry.
pub fn end_entry_text(died: bool) -> &'static str {
    if died {
        SESSION_DIED
    } else {
        SESSION_ENDED
    }
}

/// `EndOrchestratorSession` (`died` false: the board ended it) and
/// `OrchestratorWindowGone` (`died` true).
pub fn check_end_orchestrator_session(
    r: &impl StoreReads,
    id: i64,
    host: &str,
    died: bool,
) -> Result<Orchestrator, String> {
    let row = own_orchestrator(r, id, host)?;
    if row.tmux_window.is_empty() {
        return Err(format!("the orchestrator of epic {} is not running", row.epic_id));
    }
    Ok(Orchestrator {
        tmux_window: String::new(),
        session_died: died,
        ..row
    })
}

/// `OrchestratorTurnClearsTheRestartFlag`: the row once its session has
/// completed a turn, its `last_turn_at` left for the caller.
pub fn check_orchestrator_turn(r: &impl StoreReads, name: &str) -> Result<Orchestrator, String> {
    let row = r
        .orchestrator_named(name)
        .ok_or_else(|| format!("no orchestrator is named {name}"))?;
    Ok(Orchestrator {
        restarted_once: false,
        ..row
    })
}

/// The orchestrators of the epics `who` holds, which include every one on
/// a host they own.
pub fn orchestrators_of_held(r: &impl StoreReads, who: &str) -> Vec<Orchestrator> {
    orchestrations_of_held(r, who)
        .into_iter()
        .flat_map(|o| r.orchestrators_of_epic(o.epic_id))
        .collect()
}

#[spacetimedb::reducer]
pub fn ensure_orchestrator(ctx: &ReducerContext, epic_id: i64, host: String) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let seed = ctx.timestamp.to_micros_since_unix_epoch() as u64;
    if let Some(row) = check_ensure_orchestrator(ctx, epic_id, &host, seed)? {
        ctx.db.orchestrators().insert(row);
    }
    Ok(())
}

/// `resumed`: the board continues the conversation it last held for the
/// epic, which the start's log entry says.
#[spacetimedb::reducer]
pub fn start_orchestrator_session(
    ctx: &ReducerContext,
    id: i64,
    host: String,
    resumed: bool,
) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let row = check_start_orchestrator_session(ctx, id, &host)?;
    insert_log_entry(ctx, session_entry(&row, start_entry_text(resumed)));
    ctx.db.orchestrators().id().update(Orchestrator {
        started_at: now(ctx),
        ..row
    });
    Ok(())
}

#[spacetimedb::reducer]
pub fn end_orchestrator_session(
    ctx: &ReducerContext,
    id: i64,
    host: String,
    died: bool,
) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let row = check_end_orchestrator_session(ctx, id, &host, died)?;
    insert_log_entry(ctx, session_entry(&row, end_entry_text(died)));
    ctx.db.orchestrators().id().update(row);
    Ok(())
}

#[spacetimedb::reducer]
pub fn orchestrator_turn_completed(ctx: &ReducerContext, name: String) -> Result<(), String> {
    let row = check_orchestrator_turn(ctx, &name)?;
    require_acting_for(ctx, &row.host)?;
    ctx.db.orchestrators().id().update(Orchestrator {
        last_turn_at: now(ctx),
        ..row
    });
    Ok(())
}

/// Called with the switch's delete (`SwitchOrchestratorOff`).
pub(crate) fn delete_orchestrators_of(ctx: &ReducerContext, epic_id: i64) {
    ctx.db.orchestrators().epic_id().delete(epic_id);
}
