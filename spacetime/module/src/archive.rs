//! Soft delete: the read-only record a person's delete keeps of a task
//! (`core.allium: ArchivedTask`, `tasks.allium: ArchiveDeletedTask`).
//!
//! The record is BUILT by a pure function over [`StoreReads`], so
//! `MemoryReducerCaller` writes exactly the record the module writes, and READ
//! through two procedures that serve every record.

use super::*;

/// The record [`archive_task_row`] writes for `task`, read as the store stands
/// BEFORE the delete: the task's agents and (in an epic delete) its epic chain
/// are still there. `pub` so `MemoryReducerCaller` builds the same record.
pub fn archive_record(
    r: &impl StoreReads,
    task: &Task,
    archived_at: String,
    archived_by: String,
) -> ArchivedTask {
    let epic = (task.epic_id != 0).then(|| r.epic(task.epic_id)).flatten();
    ArchivedTask {
        id: task.id,
        title: task.title.clone(),
        description: task.description.clone(),
        repo_path: task.repo_path.clone(),
        base_branch: task.base_branch.clone(),
        tag: task.tag.clone(),
        labels: task.labels.clone(),
        url: task.url.clone(),
        url_type: task.url_type.clone(),
        plan_path: task.plan_path.clone(),
        external_id: task.external_id.clone(),
        created_by: task.created_by.clone(),
        owner: task.owner.clone(),
        created_at: task.created_at.clone(),
        completed_at: task.completed_at.clone(),
        epic_id: task.epic_id,
        epic_title: epic.as_ref().map(|e| e.title.clone()).unwrap_or_default(),
        agent_name: display_agent(&r.agents_of(task.id))
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        // Dead columns (`docs/plans/2026-11-23-remove-workspaces.md`, R8):
        // written as their defaults, never read.
        workspace_id: 0,
        private: false,
        private_holder: String::new(),
        archived_at,
        archived_by,
    }
}

/// `tasks.allium: ArchiveDeletedTask`. Called by every person-made task
/// delete — `delete_task`, `batch_delete`, `delete_epic_subtree` — BEFORE the
/// row and its agents go, in the same reducer. Not by the feed stale delete.
/// A record already present for the id is left as it is.
pub(crate) fn archive_task_row(ctx: &ReducerContext, task: &Task) {
    if ctx.db.archived_tasks().id().find(task.id).is_some() {
        return;
    }
    let record = archive_record(ctx, task, now(ctx), caller_identity(ctx));
    ctx.db.archived_tasks().insert(record);
}

/// Every archived task, narrowed by `epic_id` (`0` for any) and
/// `repo_paths` (empty for any), newest archived first, ties by id descending
/// (`mcp-task-tools.allium: ListArchivedTasksViaMcp`). The host applies the
/// remaining `list_tasks` filters.
pub fn archived_tasks_for(
    r: &impl StoreReads,
    epic_id: i64,
    repo_paths: &[String],
) -> Vec<ArchivedTask> {
    let mut rows = r.all_archived_tasks();
    rows.retain(|a| {
        (epic_id == 0 || a.epic_id == epic_id)
            && (repo_paths.is_empty() || repo_paths.contains(&a.repo_path))
    });
    rows.sort_by(|a, b| b.archived_at.cmp(&a.archived_at).then(b.id.cmp(&a.id)));
    rows.dedup_by_key(|a| a.id);
    rows
}
