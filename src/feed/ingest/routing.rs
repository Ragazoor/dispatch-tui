//! Role-routed phase 1: route each entry to its target sub-epic and group the
//! present entries for the upsert pass.

use std::collections::HashMap;

use super::role_routed::RoleSubEpics;
use super::FeedItemWithTarget;
use crate::feed::route;
use crate::models::{EpicId, Task, TaskId};
use crate::store::TaskStore;
use anyhow::Result;

/// Result of [`route_and_group_entries`]: present entries grouped by target
/// sub-epic (for the insert/update pass), the union of all emitted
/// external_ids (for the stale-delete pass), and the repo-group sub-epics
/// created/looked-up while routing (for the final recalculation pass).
pub(super) struct RoutedEntries {
    pub(super) groups: HashMap<EpicId, Vec<FeedItemWithTarget>>,
    pub(super) all_external_ids: Vec<String>,
    pub(super) repo_group_cache: HashMap<(EpicId, String), EpicId>,
}

/// Pure decision: does the existing task (if any) for `external_id` need to
/// move to `target`? Returns the task id to move, or `None` when there is no
/// existing task for this `external_id` or it is already in `target`. Carries
/// no I/O, so it is unit-testable without a DB — isolates the routing
/// *decision* from the `apply_move` side effect that acts on it.
fn decide_move(
    existing: &HashMap<String, Task>,
    external_id: &str,
    target: EpicId,
) -> Option<TaskId> {
    existing.get(external_id).and_then(|task| {
        if task.epic_id == Some(target) {
            None
        } else {
            Some(task.id)
        }
    })
}

/// Apply a previously-decided move: `set_task_epic_id` touches only
/// epic_id/updated_at, so status/sub_status/worktree/tmux_window/sort_order
/// survive, and the field update that follows applies the latest feed
/// metadata.
async fn apply_move(
    db: &dyn TaskStore,
    task_id: TaskId,
    target: EpicId,
    item: &crate::models::FeedItem,
) -> Result<()> {
    db.set_task_epic_id(task_id, Some(target)).await?;
    db.patch_task(
        task_id,
        &crate::store::TaskPatch::new()
            .title(&item.title)
            .description(&item.description)
            .tag(Some(item.tag))
            .labels(&item.labels)
            .sort_order(item.sort_order),
    )
    .await?;
    Ok(())
}

/// Route each entry to its target sub-epic (resolving into a per-repo
/// sub-epic when the role has `group_by_repo`), moving any cross-role or
/// parent-stranded task in place as it goes.
pub(super) async fn route_and_group_entries(
    db: &dyn TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
    existing: &HashMap<String, Task>,
    roles: &RoleSubEpics,
) -> Result<RoutedEntries> {
    let mut groups: HashMap<EpicId, Vec<FeedItemWithTarget>> = HashMap::new();
    let mut all_external_ids: Vec<String> = Vec::with_capacity(entries.len());
    // Cache (role_sub_epic, repo_name) → repo_group_id so multiple items sharing
    // the same repo only call create_repo_group_sub_epic once.
    let mut repo_group_cache: HashMap<(EpicId, String), EpicId> = HashMap::new();

    // feeds.allium: "Retired feed items" — drop retired-and-taskless items
    // before routing decides anything for them, so one never earns a move or
    // a freshly auto-grouped repo sub-epic (`create_repo_group_sub_epic`
    // below). `retired_without_task` itself already checks the whole feed
    // epic's subtree, so an item with a surviving task (e.g. the
    // ArchivedStatusMigration's worktree-holding case) is never in this set
    // and is routed normally.
    let candidate_ids: Vec<String> = entries.iter().map(|e| e.item.external_id.clone()).collect();
    let drop_ids: std::collections::HashSet<String> = db
        .retired_without_task(parent_id, &candidate_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    for entry in entries {
        if drop_ids.contains(&entry.item.external_id) {
            continue;
        }
        let role_target = roles.target_for(route(&entry.item.signals));

        let target = if roles.can_auto_group(role_target) {
            let repo_name = crate::models::repo_name_from_url(&entry.item.url);
            let key = (role_target, repo_name.clone());
            if let Some(&cached) = repo_group_cache.get(&key) {
                cached
            } else {
                match db.create_repo_group_sub_epic(role_target, &repo_name).await {
                    Ok(id) => {
                        repo_group_cache.insert(key, id);
                        id
                    }
                    Err(err) => {
                        tracing::warn!(
                            epic_id = parent_id.0,
                            role_sub_epic_id = role_target.0,
                            "run_role_routed_feed_sync: create_repo_group_sub_epic failed: {err:#}"
                        );
                        role_target
                    }
                }
            }
        } else {
            role_target
        };

        all_external_ids.push(entry.item.external_id.clone());

        if let Some(task_id) = decide_move(existing, &entry.item.external_id, target) {
            apply_move(db, task_id, target, &entry.item).await?;
        }

        groups.entry(target).or_default().push(entry);
    }

    Ok(RoutedEntries {
        groups,
        all_external_ids,
        repo_group_cache,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TaskBuilder, TaskTag};

    fn make_task(id: i64, epic_id: Option<i64>) -> Task {
        TaskBuilder::new(id)
            .epic(epic_id)
            .tag(Some(TaskTag::PrReview))
            .build()
    }

    #[test]
    fn decide_move_none_when_no_existing_task() {
        let existing = HashMap::new();
        assert_eq!(decide_move(&existing, "pr-1", EpicId(1)), None);
    }

    #[test]
    fn decide_move_none_when_already_at_target() {
        let mut existing = HashMap::new();
        existing.insert("pr-1".to_string(), make_task(7, Some(1)));
        assert_eq!(decide_move(&existing, "pr-1", EpicId(1)), None);
    }

    #[test]
    fn decide_move_some_when_task_in_different_epic() {
        let mut existing = HashMap::new();
        existing.insert("pr-1".to_string(), make_task(7, Some(2)));
        assert_eq!(decide_move(&existing, "pr-1", EpicId(1)), Some(TaskId(7)));
    }

    #[test]
    fn decide_move_some_when_task_has_no_epic() {
        let mut existing = HashMap::new();
        existing.insert("pr-1".to_string(), make_task(7, None));
        assert_eq!(decide_move(&existing, "pr-1", EpicId(1)), Some(TaskId(7)));
    }
}
