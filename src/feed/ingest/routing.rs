//! Role-routed phase 1: route each entry to its target sub-epic and group the
//! present entries for the upsert pass.

use std::collections::HashMap;

use super::role_routed::RoleSubEpics;
use super::FeedItemWithTarget;
use crate::db::TaskStore;
use crate::feed::route;
use crate::models::{EpicId, Task};
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

/// Route each entry to its target sub-epic (resolving into a per-repo
/// sub-epic when the role has `group_by_repo`), moving any cross-role or
/// parent-stranded task in place as it goes — `set_task_epic_id` touches only
/// epic_id/updated_at, so status/sub_status/worktree/tmux_window/sort_order
/// survive, and the field update that follows applies the latest feed
/// metadata.
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

    for entry in entries {
        let role_target = roles.target_for(route(&entry.item.signals));

        let target = if roles.can_auto_group(role_target) {
            let repo_name = crate::dispatch::repo_name_from_url(&entry.item.url);
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

        if let Some(task) = existing.get(&entry.item.external_id) {
            if task.epic_id != Some(target) {
                db.set_task_epic_id(task.id, Some(target)).await?;
                db.patch_task(
                    task.id,
                    &crate::db::TaskPatch::new()
                        .title(&entry.item.title)
                        .description(&entry.item.description)
                        .tag(Some(entry.item.tag))
                        .labels(&entry.item.labels)
                        .sort_order(entry.item.sort_order),
                )
                .await?;
            }
        }

        groups.entry(target).or_default().push(entry);
    }

    Ok(RoutedEntries {
        groups,
        all_external_ids,
        repo_group_cache,
    })
}
