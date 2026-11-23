//! The new-task signal's rows (`core.allium: UnseenTask`).

use super::*;

/// The derived key of an unseen row. `pub` so `MemoryReducerCaller` calls it
/// instead of carrying a second copy, as it does `subscription_id`.
pub fn unseen_id(subscriber: &str, task_id: i64) -> String {
    format!("{subscriber}/{task_id}")
}

/// `mcp-task-tools.allium: SignalMarksTheTaskForEveryone`: one row for every
/// known person (every `UserProfile`) when the task is in an epic. A
/// subscription plays no part, and a person registered later is not marked.
pub(crate) fn mark_task_unseen_for_everyone(ctx: &ReducerContext, task_id: i64, epic_id: i64) {
    if epic_id == 0 {
        return;
    }
    let people: Vec<String> = ctx.db.user_profiles().iter().map(|p| p.identity).collect();
    for subscriber in people {
        let id = unseen_id(&subscriber, task_id);
        if ctx.db.unseen_tasks().id().find(&id).is_none() {
            ctx.db.unseen_tasks().insert(UnseenTask {
                id,
                task_id,
                subscriber,
            });
        }
    }
}

/// End every person's mark on a task: it left its epic, finished or was
/// deleted (`tasks.allium: UnseenMarkEndsWithTheTaskLeavingItsEpic`,
/// `UnseenMarkEndsWithTheTaskFinishing`).
pub(crate) fn clear_unseen_for_task(ctx: &ReducerContext, task_id: i64) {
    ctx.db.unseen_tasks().task_id().delete(task_id);
}

/// `tasks.allium: SeeTaskByLandingOnIt`: `subscriber`'s board cursor rested on
/// `task_id`'s card, so their mark on that one task ends. Every other task,
/// and everyone else's mark, stays. Nothing to clear is not an error: the
/// cursor lands on cards all the time, and a missing task is the same no-op.
#[spacetimedb::reducer]
pub fn mark_task_seen(
    ctx: &ReducerContext,
    subscriber: String,
    task_id: i64,
) -> Result<(), String> {
    if subscriber.trim().is_empty() {
        return Err("cannot mark seen without an identity".into());
    }
    require_own_identity(ctx, &subscriber)?;
    if ctx.db.tasks().id().find(task_id).is_none() {
        return Ok(());
    }
    ctx.db
        .unseen_tasks()
        .id()
        .delete(unseen_id(&subscriber, task_id));
    Ok(())
}
