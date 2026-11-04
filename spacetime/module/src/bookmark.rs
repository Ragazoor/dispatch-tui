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

/// `tasks.allium: SetBookmark`: set (`bookmarked`) or remove `subscriber`'s
/// bookmark on `task_id`. Only the person themselves may write it. A task the
/// caller cannot see is refused as a missing one, and a Done task cannot be
/// bookmarked. Both directions are idempotent.
#[spacetimedb::reducer]
pub fn set_bookmark(
    ctx: &ReducerContext,
    subscriber: String,
    task_id: i64,
    bookmarked: bool,
) -> Result<(), String> {
    if subscriber.trim().is_empty() {
        return Err("cannot bookmark without an identity".into());
    }
    require_own_identity(ctx, &subscriber)?;
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
