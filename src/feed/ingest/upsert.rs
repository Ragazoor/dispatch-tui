//! Role-routed phase 2: insert/update the present role groups.

use std::collections::HashMap;

use super::FeedItemWithTarget;
use crate::db::TaskStore;
use crate::models::EpicId;

/// Insert/update present roles. Because every cross-role task was already
/// moved out of its losing epic by [`super::routing::route_and_group_entries`],
/// `upsert_feed_tasks`' per-epic delete only ever removes genuinely-stale
/// rows here — never a moved task.
pub(super) async fn upsert_role_groups(
    db: &dyn TaskStore,
    parent_id: EpicId,
    groups: HashMap<EpicId, Vec<FeedItemWithTarget>>,
) {
    for (sub_id, group) in groups {
        let (items, repo_paths, base_branches) = FeedItemWithTarget::unzip(group);
        if let Err(err) = db
            .upsert_feed_tasks(sub_id, &items, &repo_paths, &base_branches)
            .await
        {
            tracing::warn!(
                epic_id = parent_id.0,
                sub_epic_id = sub_id.0,
                "run_role_routed_feed_sync: upsert_feed_tasks failed: {err:#}"
            );
        }
    }
}
