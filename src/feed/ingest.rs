use std::collections::HashMap;

use crate::db::TaskStore;
use crate::models::{EpicId, FeedItem};
use anyhow::Result;

/// Upsert `items` into `sub_epic_id`, then recalculate its status on success
/// (which propagates upward to the parent). Logs a warning on failure. Shared
/// by both reconciliation paths of `sync_grouped_feed`: the present-group
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
        super::recalculate_epic_status_after_feed(db, sub_epic_id, "sync_grouped_feed").await;
    }
}

/// Group feed items by repo name and upsert each group into its own sub-epic.
/// Clears any flat feed tasks on the parent epic (migration + ongoing hygiene).
/// Returns the IDs of all sub-epics that were found or created (used by the
/// caller to notify the TUI, even when individual upserts partially fail).
pub(super) async fn sync_grouped_feed(
    db: &dyn TaskStore,
    parent_id: EpicId,
    items: &[FeedItem],
    repo_paths: &[String],
    base_branches: &[String],
) -> Vec<EpicId> {
    // Group co-indexed (item, repo_path, base_branch) tuples by repo name.
    // Using zip makes the per-index alignment structural rather than contractual.
    type GroupEntry = (FeedItem, String, String);
    let mut groups: HashMap<String, Vec<GroupEntry>> = HashMap::new();
    for ((item, rp), bb) in items
        .iter()
        .zip(repo_paths.iter())
        .zip(base_branches.iter())
    {
        let name = crate::dispatch::repo_name_from_url(&item.url);
        groups
            .entry(name)
            .or_default()
            .push((item.clone(), rp.clone(), bb.clone()));
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

    for (repo_name, group) in &groups {
        let group_items: Vec<_> = group.iter().map(|(item, _, _)| item.clone()).collect();
        let group_repo_paths: Vec<_> = group.iter().map(|(_, rp, _)| rp.clone()).collect();
        let group_base_branches: Vec<_> = group.iter().map(|(_, _, bb)| bb.clone()).collect();

        let sub_epic_id =
            if let Some(existing) = active_sub_epics.iter().find(|e| e.title == *repo_name) {
                existing.id
            } else {
                match db.create_epic(repo_name, "", Some(parent_id)).await {
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
        .filter(|e| !groups.contains_key(&e.title))
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
        super::recalculate_epic_status_after_feed(db, parent_id, "sync_grouped_feed").await;
    }

    sub_epic_ids
}

/// Upsert feed items using the correct strategy for `epic.group_by_repo`.
///
/// - `group_by_repo = false`: flat upsert directly onto the parent epic.
/// - `group_by_repo = true`: group by repo name, upsert into per-repo sub-epics,
///   then clear flat tasks from the parent.
///
/// Returns `epic_id` plus any sub-epic IDs written to (grouped path only).
/// Callers use this list to send one TUI notification per affected epic.
pub(crate) async fn run_feed_sync(
    db: &dyn TaskStore,
    epic_id: EpicId,
    group_by_repo: bool,
    items: &[FeedItem],
    repo_paths: &[String],
    base_branches: &[String],
) -> Result<Vec<EpicId>> {
    if group_by_repo {
        let sub_ids = sync_grouped_feed(db, epic_id, items, repo_paths, base_branches).await;
        let mut all_ids = vec![epic_id];
        all_ids.extend(sub_ids);
        Ok(all_ids)
    } else {
        db.upsert_feed_tasks(epic_id, items, repo_paths, base_branches)
            .await?;
        Ok(vec![epic_id])
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use super::*;
    use crate::db::{CreateTaskRequest, Database, EpicCrud, EpicPatch, TaskCrud};
    use crate::models::{TaskStatus, TaskTag};

    fn make_item(external_id: &str, url: &str) -> FeedItem {
        FeedItem {
            external_id: external_id.to_string(),
            title: external_id.to_string(),
            description: String::new(),
            url: url.to_string(),
            status: crate::models::TaskStatus::Backlog,
            tag: TaskTag::PrReview,
            labels: vec![],
            sort_order: None,
        }
    }

    /// Regression: archived sub-epics must not be reused when a new cycle runs.
    ///
    /// The lookup must use `active_sub_epics` (status != Archived), not the full
    /// list — otherwise an archived sub-epic with the same repo name is matched
    /// and reused instead of creating a fresh active one.
    #[tokio::test]
    async fn archived_sub_epic_not_reused() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        // Create a sub-epic that is then archived.
        let archived_sub = db.create_epic("repo-a", "", Some(parent.id)).await.unwrap();
        db.patch_epic(
            archived_sub.id,
            &EpicPatch::new().status(TaskStatus::Archived),
        )
        .await
        .unwrap();

        let items = vec![make_item("pr-1", "https://github.com/org/repo-a/pull/1")];
        let repo_paths = vec!["".to_string()];
        let base_branches = vec!["main".to_string()];

        let sub_ids = sync_grouped_feed(&*db, parent.id, &items, &repo_paths, &base_branches).await;

        assert_eq!(sub_ids.len(), 1, "should return exactly one sub-epic ID");
        let new_id = sub_ids[0];
        assert_ne!(
            new_id, archived_sub.id,
            "must create a new sub-epic, not reuse the archived one"
        );

        let all_subs = db.list_sub_epics(parent.id).await.unwrap();
        let active: Vec<_> = all_subs
            .iter()
            .filter(|e| e.status != TaskStatus::Archived)
            .collect();
        assert_eq!(active.len(), 1, "exactly one active sub-epic after sync");
        assert_eq!(active[0].title, "repo-a");
        assert_eq!(active[0].id, new_id);

        let tasks = db.list_tasks_for_epic(new_id).await.unwrap();
        assert_eq!(tasks.len(), 1, "new sub-epic must have the feed task");
        assert_eq!(tasks[0].external_id.as_deref(), Some("pr-1"));
    }

    #[tokio::test]
    async fn items_grouped_by_repo_name() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let items = vec![
            make_item("1", "https://github.com/org/repo-a/pull/1"),
            make_item("2", "https://github.com/org/repo-b/pull/1"),
        ];
        let repo_paths = vec!["".to_string(), "".to_string()];
        let base_branches = vec!["main".to_string(), "main".to_string()];

        sync_grouped_feed(&*db, parent.id, &items, &repo_paths, &base_branches).await;

        let subs = db.list_sub_epics(parent.id).await.unwrap();
        assert_eq!(subs.len(), 2);
        let names: Vec<&str> = subs.iter().map(|e| e.title.as_str()).collect();
        assert!(names.contains(&"repo-a"), "got {names:?}");
        assert!(names.contains(&"repo-b"), "got {names:?}");

        for sub in &subs {
            let tasks = db.list_tasks_for_epic(sub.id).await.unwrap();
            assert_eq!(tasks.len(), 1, "sub-epic {} should have 1 task", sub.title);
        }

        let parent_tasks = db.list_tasks_for_epic(parent.id).await.unwrap();
        assert_eq!(parent_tasks.len(), 0, "parent should have no direct tasks");
    }

    #[tokio::test]
    async fn no_url_groups_as_other() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let items = vec![FeedItem {
            external_id: "x".into(),
            title: "X".into(),
            description: String::new(),
            url: String::new(),
            status: TaskStatus::Backlog,
            tag: TaskTag::Bug,
            labels: vec![],
            sort_order: None,
        }];
        let repo_paths = vec!["".to_string()];
        let base_branches = vec!["main".to_string()];

        sync_grouped_feed(&*db, parent.id, &items, &repo_paths, &base_branches).await;

        let subs = db.list_sub_epics(parent.id).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].title, "other");
    }

    #[tokio::test]
    async fn existing_active_sub_epic_reused() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        // Pre-create the sub-epic as active.
        let pre_existing = db.create_epic("repo-a", "", Some(parent.id)).await.unwrap();

        let items = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
        let repo_paths = vec!["".to_string()];
        let base_branches = vec!["main".to_string()];

        let sub_ids = sync_grouped_feed(&*db, parent.id, &items, &repo_paths, &base_branches).await;

        assert_eq!(
            sub_ids,
            vec![pre_existing.id],
            "should reuse existing active sub-epic"
        );
        let subs = db.list_sub_epics(parent.id).await.unwrap();
        assert_eq!(subs.len(), 1, "no duplicate sub-epic should be created");
    }

    #[tokio::test]
    async fn run_feed_sync_flat_upserts_to_parent_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db.create_epic("Feed", "", None).await.unwrap();
        let items = vec![crate::models::FeedItem {
            external_id: "1".into(),
            title: "T".into(),
            description: String::new(),
            url: String::new(),
            status: crate::models::TaskStatus::Backlog,
            tag: crate::models::TaskTag::Bug,
            labels: vec![],
            sort_order: None,
        }];

        let ids = run_feed_sync(
            &*db,
            epic.id,
            false,
            &items,
            &["".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

        assert_eq!(ids, vec![epic.id]);
        let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].external_id.as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn run_feed_sync_grouped_puts_tasks_in_sub_epics() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db.create_epic("Reviews", "", None).await.unwrap();
        let items = vec![crate::models::FeedItem {
            external_id: "pr-1".into(),
            title: "PR 1".into(),
            description: String::new(),
            url: "https://github.com/org/repo-a/pull/1".into(),
            status: crate::models::TaskStatus::Backlog,
            tag: crate::models::TaskTag::PrReview,
            labels: vec![],
            sort_order: None,
        }];

        let ids = run_feed_sync(
            &*db,
            epic.id,
            true,
            &items,
            &["".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

        assert!(ids.contains(&epic.id));
        assert_eq!(ids.len(), 2, "parent id + 1 sub-epic id");

        let parent_tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
        assert_eq!(parent_tasks.len(), 0, "parent should have no direct tasks");

        let sub_epics = db.list_sub_epics(epic.id).await.unwrap();
        assert_eq!(sub_epics.len(), 1);
        assert_eq!(sub_epics[0].title, "repo-a");
        let sub_tasks = db.list_tasks_for_epic(sub_epics[0].id).await.unwrap();
        assert_eq!(sub_tasks.len(), 1);
    }

    /// An empty emission must clear feed tasks from EVERY active sub-epic —
    /// the feed is the source of truth for the whole grouped subtree, not just
    /// the repos present in the current batch. The sub-epic rows themselves
    /// remain (not auto-deleted).
    #[tokio::test]
    async fn sync_grouped_feed_empty_emission_clears_all_sub_epics() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let items = vec![
            make_item("1", "https://github.com/org/repo-a/pull/1"),
            make_item("2", "https://github.com/org/repo-b/pull/1"),
        ];
        let repo_paths = vec!["".to_string(), "".to_string()];
        let base_branches = vec!["main".to_string(), "main".to_string()];
        sync_grouped_feed(&*db, parent.id, &items, &repo_paths, &base_branches).await;

        assert_eq!(db.list_sub_epics(parent.id).await.unwrap().len(), 2);

        // Second cycle: the feed now returns nothing.
        sync_grouped_feed(&*db, parent.id, &[], &[], &[]).await;

        let subs = db.list_sub_epics(parent.id).await.unwrap();
        assert_eq!(subs.len(), 2, "sub-epic rows remain, only their tasks clear");
        for sub in &subs {
            let tasks = db.list_tasks_for_epic(sub.id).await.unwrap();
            assert_eq!(
                tasks.len(),
                0,
                "sub-epic {} should have no feed tasks after empty emission",
                sub.title
            );
        }
    }

    /// A partial emission clears only the sub-epics whose repo dropped out;
    /// repos still present keep their tasks.
    #[tokio::test]
    async fn sync_grouped_feed_partial_emission_clears_dropped_repo() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let items = vec![
            make_item("1", "https://github.com/org/repo-a/pull/1"),
            make_item("2", "https://github.com/org/repo-b/pull/1"),
        ];
        sync_grouped_feed(
            &*db,
            parent.id,
            &items,
            &["".to_string(), "".to_string()],
            &["main".to_string(), "main".to_string()],
        )
        .await;

        // Second cycle: only repo-a still has an open item.
        let items2 = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
        sync_grouped_feed(&*db, parent.id, &items2, &["".to_string()], &["main".to_string()]).await;

        let subs = db.list_sub_epics(parent.id).await.unwrap();
        let repo_a = subs.iter().find(|e| e.title == "repo-a").unwrap();
        let repo_b = subs.iter().find(|e| e.title == "repo-b").unwrap();
        assert_eq!(
            db.list_tasks_for_epic(repo_a.id).await.unwrap().len(),
            1,
            "repo-a still in feed, task kept"
        );
        assert_eq!(
            db.list_tasks_for_epic(repo_b.id).await.unwrap().len(),
            0,
            "repo-b dropped out, task cleared"
        );
    }

    /// Clearing a dropped sub-epic removes only feed tasks (external_id set);
    /// a manually-added task (external_id = null) in that sub-epic survives.
    #[tokio::test]
    async fn sync_grouped_feed_preserves_manual_task_in_dropped_sub_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let items = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
        sync_grouped_feed(&*db, parent.id, &items, &["".to_string()], &["main".to_string()]).await;

        let subs = db.list_sub_epics(parent.id).await.unwrap();
        let repo_a = subs.iter().find(|e| e.title == "repo-a").unwrap();

        // A manual task the user added under the repo sub-epic (no external_id).
        let manual_id = db
            .create_task(CreateTaskRequest {
                title: "Manual",
                description: "",
                repo_path: "/repo",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: Some(repo_a.id),
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
            })
            .await
            .unwrap();

        // Empty emission clears the feed task but must spare the manual one.
        sync_grouped_feed(&*db, parent.id, &[], &[], &[]).await;

        let tasks = db.list_tasks_for_epic(repo_a.id).await.unwrap();
        assert_eq!(tasks.len(), 1, "only the manual task survives");
        assert_eq!(tasks[0].id, manual_id);
    }
}
