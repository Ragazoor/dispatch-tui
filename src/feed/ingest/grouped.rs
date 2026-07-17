//! The `group_by_repo` sync strategy: group an emission's items by repo name
//! and upsert each group into its own per-repo sub-epic, reconciling the whole
//! grouped subtree so the feed stays the source of truth.

use std::collections::HashMap;

use super::FeedItemWithTarget;
use crate::db::TaskStore;
use crate::models::{EpicId, FeedItem};

/// Upsert `items` into `sub_epic_id`, then recalculate its status on success
/// (which propagates upward to the parent). Logs a warning on failure. Shared
/// by both reconciliation paths of [`sync_grouped_feed`]: the present-group
/// upsert and the absent-sub-epic clear (called with empty slices).
async fn upsert_sub_epic_and_recalc(
    db: &dyn TaskStore,
    parent_id: EpicId,
    sub_epic_id: EpicId,
    items: &[FeedItem],
    repo_paths: &[String],
    base_branches: &[String],
) {
    if let Err(err) = db
        .upsert_feed_tasks(sub_epic_id, items, repo_paths, base_branches)
        .await
    {
        tracing::warn!(
            epic_id = parent_id.0,
            sub_epic_id = sub_epic_id.0,
            "sync_grouped_feed: upsert_feed_tasks failed: {err:#}"
        );
    } else {
        crate::feed::recalculate_epic_status_after_feed(db, sub_epic_id, "sync_grouped_feed").await;
    }
}

/// Group feed items by repo name and upsert each group into its own sub-epic.
/// Clears any flat feed tasks on the parent epic (migration + ongoing hygiene).
/// Returns the IDs of all sub-epics that were found or created (used by the
/// caller to notify the TUI, even when individual upserts partially fail).
///
/// Takes a single owned `Vec` of paired entries rather than three parallel
/// slices, so per-index alignment is structural — the old length-mismatch
/// guard (and the silent-truncation footgun it papered over) is gone. Taking
/// the `Vec` by value lets the grouping pass move each entry into its group
/// instead of cloning it.
pub(super) async fn sync_grouped_feed(
    db: &dyn TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
) -> Vec<EpicId> {
    // Group entries by repo name, moving each entry into its group (no clone).
    let mut groups: HashMap<String, Vec<FeedItemWithTarget>> = HashMap::new();
    for entry in entries {
        let name = crate::dispatch::repo_name_from_url(&entry.item.url);
        groups.entry(name).or_default().push(entry);
    }

    let existing_sub_epics = match db.list_sub_epics(parent_id).await {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(
                epic_id = parent_id.0,
                "sync_grouped_feed: list_sub_epics failed: {err:#}"
            );
            // list_sub_epics failed: no writes occurred, skip notifications
            return vec![];
        }
    };

    let active_sub_epics: Vec<_> = existing_sub_epics
        .iter()
        .filter(|e| e.status != crate::models::TaskStatus::Archived)
        .collect();

    let mut sub_epic_ids: Vec<EpicId> = Vec::new();
    // Repo names contributing an item this emission, kept for the absent-
    // sub-epic reconciliation below since `groups` is consumed by value here.
    let group_names: std::collections::HashSet<String> = groups.keys().cloned().collect();

    for (repo_name, group) in groups {
        let (group_items, group_repo_paths, group_base_branches) = FeedItemWithTarget::unzip(group);

        let sub_epic_id =
            if let Some(existing) = active_sub_epics.iter().find(|e| e.title == repo_name) {
                existing.id
            } else {
                match db.create_epic(&repo_name, "", Some(parent_id)).await {
                    Ok(e) => e.id,
                    Err(err) => {
                        tracing::warn!(
                            epic_id = parent_id.0,
                            repo = %repo_name,
                            "sync_grouped_feed: create_epic failed: {err:#}"
                        );
                        continue;
                    }
                }
            };

        // Always collect the sub-epic ID so the caller can notify the TUI,
        // even if the upsert below fails (partial writes are still visible).
        sub_epic_ids.push(sub_epic_id);

        // New backlog tasks may regress a done sub-epic; the recalculation
        // inside the helper propagates upward to the parent.
        upsert_sub_epic_and_recalc(
            db,
            parent_id,
            sub_epic_id,
            &group_items,
            &group_repo_paths,
            &group_base_branches,
        )
        .await;
    }

    // Reconcile sub-epics absent from this emission: any active sub-epic whose
    // repo contributed no item has its feed tasks cleared, so feed-as-source-of-
    // truth holds across the whole grouped subtree (not just the present repos).
    // When `groups` is empty (the feed returned nothing) every active sub-epic
    // is cleared. upsert_feed_tasks with an empty item list reuses the
    // external_id-based deletion, so manually-added tasks (external_id = NULL)
    // are preserved and the sub-epic row itself is left in place.
    // Sub-epics already handled above are skipped via the groups membership
    // check; the rest are cleared by upserting an empty list.
    for sub_epic in active_sub_epics
        .iter()
        .filter(|e| !group_names.contains(&e.title))
    {
        // Surface the cleared sub-epic to the caller so the TUI refreshes it.
        sub_epic_ids.push(sub_epic.id);
        upsert_sub_epic_and_recalc(db, parent_id, sub_epic.id, &[], &[], &[]).await;
    }

    // Always clear flat feed tasks from parent, regardless of per-group failures.
    if let Err(err) = db.upsert_feed_tasks(parent_id, &[], &[], &[]).await {
        tracing::warn!(
            epic_id = parent_id.0,
            "sync_grouped_feed: failed to clear parent feed tasks: {err:#}"
        );
    } else {
        // Recalculate the parent's status after its flat tasks are cleared.
        // Sub-epic recalculations above already propagate upward, but this
        // handles the edge case where all sub-epics failed their upserts and
        // the parent's flat task list is now empty.
        crate::feed::recalculate_epic_status_after_feed(db, parent_id, "sync_grouped_feed").await;
    }

    sub_epic_ids
}
