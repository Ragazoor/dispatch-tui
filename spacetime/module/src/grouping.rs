//! Repo grouping: routing the tasks of a `group_by_repo` (non-feed) epic into
//! per-repo `repo-group` sub-epics.
//!
//! Spec: `docs/specs/epics.allium` (`RegroupEpic`, `FlattenEpic`, and the
//! routing note above them), moved here by store-split M2 (task #53255). The
//! board's `service::grouping` used to decide all of this by reading a
//! subscription view and issuing several reducer calls; a view can be a strict
//! subset of the store, and the calls were not one transaction.
//!
//! Operations only ever touch `repo-group` sub-epics, never hand-made ones.
//! Every function that moves a task recalculates the epics it touched, inside
//! the same transaction as the move.

use super::*;

fn auto_groups(epic: &Epic) -> bool {
    can_auto_group(epic.group_by_repo, &epic.feed_command)
}

/// Where a task of `repo_path` assigned to `epic_id` actually lives: the
/// per-repo sub-epic when `epic_id` is a grouped non-feed epic (found, or
/// created here), else `epic_id` unchanged. `0` (no epic) stays `0`.
pub(crate) fn route_to_repo_group(ctx: &ReducerContext, epic_id: i64, repo_path: &str) -> i64 {
    if epic_id == 0 {
        return 0;
    }
    match ctx.db.epics().id().find(epic_id) {
        Some(root) if auto_groups(&root) => find_or_create_repo_group(
            ctx,
            epic_id,
            &repo_name_from_path(repo_path),
            caller_identity(ctx),
        ),
        _ => epic_id,
    }
}

/// The `repo-group` sub-epic of `root` titled `title`, created when absent.
///
/// The one find-or-create for repo groups: manual routing here and feed
/// ingestion's `create_repo_group_sub_epic` both come through it, so the two
/// kinds of grouping cannot disagree about which sub-epic is "the" one.
/// `created_by` is written only on a genuine insert.
pub(crate) fn find_or_create_repo_group(
    ctx: &ReducerContext,
    root: i64,
    title: &str,
    created_by: String,
) -> i64 {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&root)
        .find(|e| e.title == title && e.origin == REPO_GROUP_ORIGIN);
    if let Some(sub) = existing {
        return sub.id;
    }
    ctx.db
        .epics()
        .insert(Epic {
            title: title.to_string(),
            parent_epic_id: root,
            origin: REPO_GROUP_ORIGIN.to_string(),
            created_by,
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        })
        .id
}

/// The grouping root governing `epic_id`, if any: the epic itself when it is a
/// grouped non-feed root, else its parent when `epic_id` is a `repo-group`
/// sub-epic of one.
fn grouping_root_of(ctx: &ReducerContext, epic_id: i64) -> Option<i64> {
    let epic = ctx.db.epics().id().find(epic_id)?;
    if auto_groups(&epic) {
        return Some(epic.id);
    }
    if epic.origin == REPO_GROUP_ORIGIN {
        let parent = ctx.db.epics().id().find(epic.parent_epic_id)?;
        if auto_groups(&parent) {
            return Some(parent.id);
        }
    }
    None
}

/// Delete `epic_id` iff it is a `repo-group` sub-epic with no tasks and no
/// children; otherwise recalculate it. Epic-scoped learnings are re-scoped to
/// the parent first, so none is left with a dangling `scope_ref`
/// (`learnings.allium: ReScopeLearningsOnRepoGroupDelete`).
fn delete_if_empty_repo_group(ctx: &ReducerContext, epic_id: i64) {
    let Some(epic) = ctx.db.epics().id().find(epic_id) else {
        return;
    };
    if epic.origin != REPO_GROUP_ORIGIN {
        return;
    }
    let has_tasks = ctx.db.tasks().epic_id().filter(&epic_id).next().is_some();
    let has_children = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&epic_id)
        .next()
        .is_some();
    if has_tasks || has_children {
        recalculate_epic_chain(ctx, epic_id);
        return;
    }
    if epic.parent_epic_id != 0 {
        rescope_learnings(ctx, epic_id, epic.parent_epic_id);
    }
    // The one the delete reducer runs, over a subtree of just this epic.
    let doomed = collect_epic_subtree_ids(ctx, epic_id);
    delete_epic_subtree_with_feed_cleanup(ctx, epic_id, &doomed);
}

/// Write `task` into `target`, with no recalculation: the callers move many
/// tasks, or clean up after the move, and recalculate once at the end.
///
/// Ownership follows `set_task_epic`: a task joining an epic loses its owner.
fn relink_task(ctx: &ReducerContext, task: Task, target: i64) -> Result<(), String> {
    write_task(
        ctx,
        Task {
            epic_id: target,
            owner: String::new(),
            updated_at: now(ctx),
            ..task
        },
    )
}

/// After a task's repo changed: move it to the sub-epic for the new repo and
/// clean up the sub-epic it left. A no-op when the task is not in a grouped
/// subtree (it sits in a hand-made sub-epic, or in no epic at all).
pub(crate) fn reroute_on_repo_change(ctx: &ReducerContext, task_id: i64) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Ok(());
    };
    let Some(root) = grouping_root_of(ctx, task.epic_id) else {
        return Ok(());
    };
    let current = task.epic_id;
    let target = route_to_repo_group(ctx, root, &task.repo_path);
    if target == current {
        return Ok(());
    }
    relink_task(ctx, task, target)?;
    if current != root {
        // Deletes it when empty, else recalculates it.
        delete_if_empty_repo_group(ctx, current);
    }
    // `target` is a sub-epic of `root`, so this chain covers the root too.
    recalculate_epic_chain(ctx, target);
    Ok(())
}

/// Migrate every direct task of `root_id` into its repo sub-epic.
///
/// `epics.allium: RegroupEpic`. Idempotent: a task already in the right
/// sub-epic is left alone. The caller establishes that `root_id` is grouped
/// and not a feed; for any other epic `route_to_repo_group` answers the epic
/// itself and nothing moves. Each sub-epic a task joined is recalculated once,
/// then the root's chain once, rather than per moved task.
#[spacetimedb::reducer]
pub fn regroup_epic(ctx: &ReducerContext, root_id: i64) -> Result<(), String> {
    let tasks: Vec<Task> = ctx.db.tasks().epic_id().filter(&root_id).collect();
    let mut joined = std::collections::BTreeSet::new();
    for task in tasks {
        let target = route_to_repo_group(ctx, root_id, &task.repo_path);
        if target != root_id {
            relink_task(ctx, task, target)?;
            joined.insert(target);
        }
    }
    for sub in joined {
        if let Some(epic) = ctx.db.epics().id().find(sub) {
            recalculate_one(ctx, &epic);
        }
    }
    recalculate_epic_chain(ctx, root_id);
    Ok(())
}

/// Re-home the tasks of every `repo-group` sub-epic of `root_id` back onto it,
/// then delete the sub-epics that are left empty.
///
/// `epics.allium: FlattenEpic`. Hand-made sub-epics are never touched. Tasks
/// move FIRST: deleting an epic cascades to its tasks, so the order is
/// load-bearing.
#[spacetimedb::reducer]
pub fn flatten_epic(ctx: &ReducerContext, root_id: i64) -> Result<(), String> {
    let subs: Vec<Epic> = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&root_id)
        .filter(|e| e.origin == REPO_GROUP_ORIGIN)
        .collect();
    for sub in subs {
        let tasks: Vec<Task> = ctx.db.tasks().epic_id().filter(&sub.id).collect();
        for task in tasks {
            // The root is recalculated once below, and the sub-epic by
            // `delete_if_empty_repo_group`.
            relink_task(ctx, task, root_id)?;
        }
        delete_if_empty_repo_group(ctx, sub.id);
    }
    recalculate_epic_chain(ctx, root_id);
    Ok(())
}
