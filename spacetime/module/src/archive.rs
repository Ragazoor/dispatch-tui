//! Soft delete: the read-only record a person's delete keeps of a task
//! (`core.allium: ArchivedTask`, `tasks.allium: ArchiveDeletedTask`).
//!
//! The record is BUILT by a pure function over [`StoreReads`], so
//! `MemoryReducerCaller` writes exactly the record the module writes, and READ
//! through two procedures that apply [`archived_task_visible_to`].

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
    let holder = epic.as_ref().and_then(|e| private_holder(r, e));
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
        agent_name: display_agent_name(&r.agents_of(task.id)),
        workspace_id: epic.as_ref().map_or(0, |e| e.workspace_id),
        private: holder.is_some(),
        private_holder: holder.unwrap_or_default(),
        archived_at,
        archived_by,
    }
}

/// The task's display agent (`agents.allium`: A18): the one not ended, else
/// the last to end. `""` for a task with no agent.
fn display_agent_name(agents: &[Agent]) -> String {
    agents
        .iter()
        .find(|a| a.state != AGENT_ENDED)
        .or_else(|| agents.iter().max_by(|a, b| a.ended_at.cmp(&b.ended_at)))
        .map(|a| a.name.clone())
        .unwrap_or_default()
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

/// `ArchivedTask`'s frozen visibility: an epic-less record is its owner's;
/// an epic record is seen by members of its workspace, and only by its holder
/// when the epic chain was private.
pub fn archived_task_visible_to(r: &impl StoreReads, a: &ArchivedTask, who: &str) -> bool {
    if a.epic_id == 0 {
        return !a.owner.is_empty() && a.owner == who;
    }
    if !workspace_has_member(r, a.workspace_id, who) {
        return false;
    }
    !a.private || (!a.private_holder.is_empty() && a.private_holder == who)
}

/// The archived task `id` when the caller may see it (`operator` sees every
/// record). `None` is both "missing" and "unseen".
pub fn archived_task_for(
    r: &impl StoreReads,
    operator: bool,
    who: &str,
    id: i64,
) -> Option<ArchivedTask> {
    r.archived_task(id)
        .filter(|a| operator || archived_task_visible_to(r, a, who))
}

/// Every archived task the caller may see, narrowed by `epic_id` (`0` for
/// any) and `repo_paths` (empty for any), in id order. The host applies the
/// remaining `list_tasks` filters and the newest-first order.
///
/// Read through the indexes a caller can reach: their own epic-less records
/// and the records of each workspace they belong to. The operator also reads
/// every listed workspace's records.
pub fn archived_tasks_for(
    r: &impl StoreReads,
    operator: bool,
    who: &str,
    epic_id: i64,
    repo_paths: &[String],
) -> Vec<ArchivedTask> {
    let mut workspaces: Vec<i64> = r.memberships_of(who).iter().map(|m| m.workspace_id).collect();
    if operator {
        workspaces.extend(r.listed_workspaces().iter().map(|w| w.id));
    }
    workspaces.sort_unstable();
    workspaces.dedup();
    let mut rows: Vec<ArchivedTask> = r
        .archived_tasks_owned_by(who)
        .into_iter()
        .chain(
            workspaces
                .into_iter()
                .flat_map(|ws| r.archived_tasks_in_workspace(ws)),
        )
        .filter(|a| operator || archived_task_visible_to(r, a, who))
        .filter(|a| epic_id == 0 || a.epic_id == epic_id)
        .filter(|a| repo_paths.is_empty() || repo_paths.contains(&a.repo_path))
        .collect();
    rows.sort_by_key(|a| a.id);
    rows.dedup_by_key(|a| a.id);
    rows
}
