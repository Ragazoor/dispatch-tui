//! Task bookmarks (`core.allium: Bookmark`; `tasks.allium: SetBookmark`).

use super::*;

/// The derived key of a bookmark row. `pub` so `MemoryReducerCaller` calls it
/// instead of carrying a second copy, as it does `unseen_id`.
pub fn bookmark_id(subscriber: &str, task_id: i64) -> String {
    format!("{subscriber}/{task_id}")
}

/// End every person's bookmark on a task: it entered Done or was deleted
/// (`tasks.allium: BookmarkEndsWithTheTaskFinishing`). A move to another epic
/// never calls this.
pub(crate) fn clear_bookmarks_for_task(ctx: &ReducerContext, task_id: i64) {
    ctx.db.bookmarks().task_id().delete(task_id);
}

/// End every person's unseen mark and bookmark on a task: it was deleted.
/// The two per-person marks share their lifetime from here on.
pub(crate) fn clear_task_marks(ctx: &ReducerContext, task_id: i64) {
    clear_unseen_for_task(ctx, task_id);
    clear_bookmarks_for_task(ctx, task_id);
}

/// `UnseenMarkEndsWithTheTaskFinishing` and
/// `BookmarkEndsWithTheTaskFinishing`: a write from `prior` to `next` that
/// enters Done ends both marks. Called by every task-write seam, so whichever
/// writer moved the task (a patch, a session close, a merged PR, a batch move,
/// a feed) ends them.
pub(crate) fn end_marks_on_finish(ctx: &ReducerContext, prior: &Task, next: &Task) {
    if stamps_completion(&prior.status, &next.status) {
        clear_task_marks(ctx, next.id);
    }
}

/// Drop every unseen mark and bookmark whose `(task_id, subscriber)` matches
/// `lost_sight`: the person can no longer see the task (`workspaces.allium:
/// RemoveMember`, `MoveEpicToWorkspace`).
///
/// `subscriber` names the one person the sweep is about, when it is about one
/// (member removal): both tables are then read through their `subscriber`
/// index instead of scanned whole. `None` sweeps everyone's marks.
pub(crate) fn drop_marks_where(
    ctx: &ReducerContext,
    subscriber: Option<&str>,
    lost_sight: impl Fn(i64, &str) -> bool,
) {
    let unseen: Vec<(String, i64, String)> = match subscriber {
        Some(who) => ctx
            .db
            .unseen_tasks()
            .subscriber()
            .filter(who)
            .map(|u| (u.id, u.task_id, u.subscriber))
            .collect(),
        None => ctx
            .db
            .unseen_tasks()
            .iter()
            .map(|u| (u.id, u.task_id, u.subscriber))
            .collect(),
    };
    for (id, task_id, who) in unseen {
        if lost_sight(task_id, &who) {
            ctx.db.unseen_tasks().id().delete(&id);
        }
    }
    let bookmarks: Vec<(String, i64, String)> = match subscriber {
        Some(who) => ctx
            .db
            .bookmarks()
            .subscriber()
            .filter(who)
            .map(|b| (b.id, b.task_id, b.subscriber))
            .collect(),
        None => ctx
            .db
            .bookmarks()
            .iter()
            .map(|b| (b.id, b.task_id, b.subscriber))
            .collect(),
    };
    for (id, task_id, who) in bookmarks {
        if lost_sight(task_id, &who) {
            ctx.db.bookmarks().id().delete(&id);
        }
    }
}

/// `tasks.allium: SetBookmark`: set (`bookmarked`) or remove `subscriber`'s
/// bookmark on `task_id`. Only the person themselves may write it, the
/// operator included: a bookmark is personal. A task the caller cannot see is
/// refused as a missing one, and a Done task cannot be bookmarked. Both
/// directions are idempotent.
#[spacetimedb::reducer]
pub fn set_bookmark(
    ctx: &ReducerContext,
    subscriber: String,
    task_id: i64,
    bookmarked: bool,
) -> Result<(), String> {
    write_bookmark(ctx, subscriber, task_id, bookmarked)
}

/// The body of [`set_bookmark`], shared with `update_task`, which carries a
/// bookmark in the same transaction as the task's fields
/// (`mcp-task-tools.allium: UpdateTaskViaMcp`). It reads the task as it stands
/// now, so a call that also moved the task into Done is refused here.
pub(crate) fn write_bookmark(
    ctx: &ReducerContext,
    subscriber: String,
    task_id: i64,
    bookmarked: bool,
) -> Result<(), String> {
    if subscriber.trim().is_empty() {
        return Err("cannot bookmark without an identity".into());
    }
    if subscriber != caller_identity(ctx) {
        return Err("a bookmark is written only by the person it belongs to".into());
    }
    let Some(task) = visible_task(ctx, task_id) else {
        return Err(format!("task {task_id} does not exist"));
    };
    let id = bookmark_id(&subscriber, task_id);
    if !bookmarked {
        ctx.db.bookmarks().id().delete(id);
        return Ok(());
    }
    if task.status == DONE {
        return Err(format!("Task {task_id} is done and cannot be bookmarked"));
    }
    if ctx.db.bookmarks().id().find(&id).is_none() {
        ctx.db.bookmarks().insert(Bookmark {
            id,
            task_id,
            subscriber,
        });
    }
    Ok(())
}
