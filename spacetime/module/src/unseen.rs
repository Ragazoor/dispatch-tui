//! The new-task signal's rows (`core.allium: UnseenTask`).

use super::*;

/// The derived key of an unseen row. `pub` so `MemoryReducerCaller` calls it
/// instead of carrying a second copy, as it does `subscription_id`.
pub fn unseen_id(subscriber: &str, task_id: i64) -> String {
    format!("{subscriber}/{task_id}")
}

/// Every epic from `epic_id` up to its root, `epic_id` first. Bounded by
/// `core.allium: config.max_ancestor_walk` so a corrupt cycle cannot hang a
/// reducer.
fn epic_and_ancestors(ctx: &ReducerContext, epic_id: i64) -> Vec<i64> {
    let mut chain = Vec::new();
    let mut next = epic_id;
    while next != 0 && chain.len() < 1024 && !chain.contains(&next) {
        chain.push(next);
        next = ctx
            .db
            .epics()
            .id()
            .find(next)
            .map_or(0, |epic| epic.parent_epic_id);
    }
    chain
}

/// `mcp-task-tools.allium: SignalMarksTheTaskForEveryFollower`: one row for
/// every person subscribed to `epic_id` or to any ancestor. Later followers are
/// not marked.
pub(crate) fn mark_task_unseen_for_followers(ctx: &ReducerContext, task_id: i64, epic_id: i64) {
    if epic_id == 0 {
        return;
    }
    let chain = epic_and_ancestors(ctx, epic_id);
    let followers: std::collections::BTreeSet<String> = ctx
        .db
        .subscriptions()
        .iter()
        .filter(|s| chain.contains(&s.epic_id))
        .map(|s| s.subscriber)
        .collect();
    for subscriber in followers {
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
/// deleted (`tasks.allium: UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing`).
pub(crate) fn clear_unseen_for_task(ctx: &ReducerContext, task_id: i64) {
    ctx.db.unseen_tasks().task_id().delete(task_id);
}

/// `tasks.allium: SeeTasksByEnteringTheirEpic`: `subscriber` entered `epic_id`,
/// so their marks on the tasks directly in it end. Tasks in child epics stay
/// marked. Nothing to clear is not an error: entering an epic is routine.
#[spacetimedb::reducer]
pub fn mark_epic_seen(
    ctx: &ReducerContext,
    subscriber: String,
    epic_id: i64,
) -> Result<(), String> {
    if subscriber.trim().is_empty() {
        return Err("cannot mark seen without an identity".into());
    }
    if visible_epic(ctx, epic_id).is_none() {
        return Ok(());
    }
    for task in ctx.db.tasks().epic_id().filter(&epic_id) {
        ctx.db
            .unseen_tasks()
            .id()
            .delete(unseen_id(&subscriber, task.id));
    }
    Ok(())
}
