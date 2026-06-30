use std::collections::HashMap;

use crate::db::TaskStore;
use crate::feed::route;
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

/// A feed item paired with its resolved repo path and base branch. Assembled
/// once at the [`run_feed_sync`] boundary so the three values travel together
/// as a unit — there is no parallel-slice length invariant left to police.
pub(super) type FeedEntry = (FeedItem, String, String);

/// Group feed items by repo name and upsert each group into its own sub-epic.
/// Clears any flat feed tasks on the parent epic (migration + ongoing hygiene).
/// Returns the IDs of all sub-epics that were found or created (used by the
/// caller to notify the TUI, even when individual upserts partially fail).
///
/// Takes a single owned `Vec` of `(item, repo_path, base_branch)` tuples rather
/// than three parallel slices, so per-index alignment is structural — the old
/// length-mismatch guard (and the silent-truncation footgun it papered over)
/// is gone. Taking the `Vec` by value lets the grouping pass move each tuple
/// into its group instead of cloning it.
pub(super) async fn sync_grouped_feed(
    db: &dyn TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedEntry>,
) -> Vec<EpicId> {
    // Group co-indexed (item, repo_path, base_branch) tuples by repo name,
    // moving each tuple into its group (no clone).
    let mut groups: HashMap<String, Vec<FeedEntry>> = HashMap::new();
    for (item, rp, bb) in entries {
        let name = crate::dispatch::repo_name_from_url(&item.url);
        groups.entry(name).or_default().push((item, rp, bb));
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
        // Assemble the parallel slices into co-indexed tuples once, here at the
        // boundary. repo_paths/base_branches are derived one-per-item upstream
        // (resolve_feed_item_repo_paths / resolve_base_branches), so the zip is
        // lossless — and downstream code no longer has parallel slices to align.
        let entries: Vec<FeedEntry> = items
            .iter()
            .cloned()
            .zip(repo_paths.iter().cloned())
            .zip(base_branches.iter().cloned())
            .map(|((item, rp), bb)| (item, rp, bb))
            .collect();
        let sub_ids = sync_grouped_feed(db, epic_id, entries).await;
        let mut all_ids = vec![epic_id];
        all_ids.extend(sub_ids);
        Ok(all_ids)
    } else {
        db.upsert_feed_tasks(epic_id, items, repo_paths, base_branches)
            .await?;
        Ok(vec![epic_id])
    }
}

/// Display title used when the role sub-epic must be created. The role
/// identity (`feed_role`) is stable; the title is user-editable afterwards.
fn role_sub_epic_title(role: crate::models::FeedRole) -> &'static str {
    use crate::models::FeedRole;
    match role {
        FeedRole::MyReviews => "My Reviews",
        FeedRole::TeamReviews => "Team Reviews",
        FeedRole::Bots => "Bots",
        // Not reachable from `route` (which only yields the three above), but
        // keep the match total without a misleading title.
        _ => "Reviews",
    }
}

/// Find the role sub-epic carrying `role` among `existing_subs`, creating it
/// under `parent_id` if absent. Idempotent and matched by `feed_role` (not
/// title), so a user rename is preserved. Reuses any existing sub-epic with the
/// role — including an archived one — because the partial unique index on
/// `(parent_epic_id, feed_role)` forbids a second sub-epic with the same role.
///
/// `existing_subs` is the parent's sub-epic list, fetched once by the caller
/// and shared across the three role lookups (the roles are distinct, so a
/// create for one role never invalidates the lookup of another).
async fn ensure_role_sub_epic(
    db: &dyn TaskStore,
    parent_id: EpicId,
    existing_subs: &[crate::models::Epic],
    role: crate::models::FeedRole,
) -> Result<EpicId> {
    if let Some(existing) = existing_subs.iter().find(|e| e.feed_role == role) {
        return Ok(existing.id);
    }
    let created = db
        .create_epic(role_sub_epic_title(role), "", Some(parent_id))
        .await?;
    db.patch_epic(created.id, &crate::db::EpicPatch::new().feed_role(role))
        .await?;
    Ok(created.id)
}

/// Reconcile a `reviews_parent` epic's whole role-sub-epic subtree from one
/// emission, with **global `external_id` identity** across the role sub-epics:
///
/// - A PR whose `route(signals)` differs from its current role is **moved**
///   (`set_task_epic_id` + `patch_task`), preserving status / sub_status /
///   worktree / tmux_window / sort_order — an in-flight review agent keeps its
///   session.
/// - A PR seen for the first time is inserted into its target role sub-epic.
/// - A PR absent from the emission (merged/closed) is removed by a **single**
///   subtree-scoped delete, run once with the union of all emitted ids so a
///   just-moved task is never deleted. Manual tasks (`external_id IS NULL`) are
///   preserved.
///
/// Steps run as one non-interleaved unit per parent tick. Returns the parent id
/// plus the three role sub-epic ids (for TUI notification), mirroring
/// [`sync_grouped_feed`]'s return contract.
pub(crate) async fn run_role_routed_feed_sync(
    db: &dyn TaskStore,
    parent_id: EpicId,
    items: &[FeedItem],
    repo_paths: &[String],
    base_branches: &[String],
) -> Result<Vec<EpicId>> {
    use crate::models::FeedRole;

    // repo_paths/base_branches are parallel-to-items by contract. A mismatch
    // would let the zip below silently truncate and drop PRs, so refuse to
    // write and surface the parent for a refresh (mirrors sync_grouped_feed).
    if items.len() != repo_paths.len() || items.len() != base_branches.len() {
        tracing::warn!(
            epic_id = parent_id.0,
            items = items.len(),
            repo_paths = repo_paths.len(),
            base_branches = base_branches.len(),
            "run_role_routed_feed_sync: slice length mismatch, skipping (no writes)"
        );
        return Ok(vec![parent_id]);
    }

    // Ensure the three role sub-epics exist (idempotent, matched by feed_role).
    // Fetch the parent's sub-epics once and share the list across all three
    // lookups — the roles are distinct, so creating one never affects another.
    let existing_subs = db.list_sub_epics(parent_id).await?;
    let my = ensure_role_sub_epic(db, parent_id, &existing_subs, FeedRole::MyReviews).await?;
    let team = ensure_role_sub_epic(db, parent_id, &existing_subs, FeedRole::TeamReviews).await?;
    let bots = ensure_role_sub_epic(db, parent_id, &existing_subs, FeedRole::Bots).await?;
    let target_for = |role: FeedRole| match role {
        FeedRole::TeamReviews => team,
        FeedRole::Bots => bots,
        // `route` only ever yields My/Team/Bots; My is also the safe fallback.
        _ => my,
    };

    // Extract can_auto_group flags from existing_subs (already loaded), avoiding
    // extra get_epic round-trips. Newly-created role sub-epics are not in the
    // list and default to false (group_by_repo is false on creation).
    let role_can_group = |id: EpicId| -> bool {
        existing_subs
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.can_auto_group())
            .unwrap_or(false)
    };

    // Index existing subtree feed tasks by external_id (global identity across
    // the role sub-epics). Scan repo-group sub-epics FIRST so that role
    // sub-epic copies overwrite them below — ensuring role sub-epic copies win
    // when both exist. This prevents duplicate-insert constraint violations on
    // the MOVE path when group_by_repo is off but orphaned repo-group tasks
    // still exist.
    // Collect pre-existing repo-group child IDs so we recalculate them after
    // the stale-deletion pass — even for sub-epics not written this cycle.
    let mut existing: HashMap<String, crate::models::Task> = HashMap::new();
    let mut pre_existing_repo_group_ids: Vec<EpicId> = Vec::new();
    for sub in [my, team, bots] {
        let children = db.list_sub_epics(sub).await?;
        for child in &children {
            for task in db.list_tasks_for_epic(child.id).await? {
                if let Some(ext) = task.external_id.clone() {
                    existing.insert(ext, task);
                }
            }
        }
        pre_existing_repo_group_ids.extend(children.iter().map(|e| e.id));
    }
    // Role sub-epics scanned last — their copies overwrite repo-group copies.
    for sub in [my, team, bots] {
        for task in db.list_tasks_for_epic(sub).await? {
            if let Some(ext) = task.external_id.clone() {
                existing.insert(ext, task);
            }
        }
    }

    // Route each item; move cross-role tasks (preserving state); group present
    // items by their target sub-epic for the insert/update pass. Reuses the
    // shared `FeedEntry` tuple alias rather than a duplicate local type. (This
    // path still takes three parallel slices and zips them here; converting it
    // to a `FeedEntry` slice end-to-end is a larger, separately-scoped change.)
    let mut groups: HashMap<EpicId, Vec<FeedEntry>> = HashMap::new();
    let mut all_external_ids: Vec<String> = Vec::with_capacity(items.len());
    // Cache (role_sub_epic, repo_name) → repo_group_id so multiple items sharing
    // the same repo only call create_repo_group_sub_epic once.
    let mut repo_group_cache: HashMap<(EpicId, String), EpicId> = HashMap::new();

    for ((item, rp), bb) in items
        .iter()
        .zip(repo_paths.iter())
        .zip(base_branches.iter())
    {
        let role_target = target_for(route(&item.signals));

        // When the role sub-epic has group_by_repo, resolve the final target
        // to the appropriate per-repo sub-epic instead.
        let target = if role_can_group(role_target) {
            let repo_name = crate::dispatch::repo_name_from_url(&item.url);
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

        all_external_ids.push(item.external_id.clone());

        if let Some(task) = existing.get(&item.external_id) {
            if task.epic_id != Some(target) {
                // Move: set_task_epic_id touches only epic_id/updated_at, so
                // status/sub_status/worktree/tmux_window/sort_order survive.
                // Field updates then apply the latest feed metadata in place.
                db.set_task_epic_id(task.id, Some(target)).await?;
                db.patch_task(
                    task.id,
                    &crate::db::TaskPatch::new()
                        .title(&item.title)
                        .description(&item.description)
                        .tag(Some(item.tag))
                        .labels(&item.labels)
                        .sort_order(item.sort_order),
                )
                .await?;
            }
        }

        groups
            .entry(target)
            .or_default()
            .push((item.clone(), rp.clone(), bb.clone()));
    }

    // Insert/update present roles. Because every cross-role task was already
    // moved out of its losing epic above, upsert_feed_tasks' per-epic delete
    // only ever removes genuinely-stale rows here — never a moved task.
    for (sub_id, group) in &groups {
        let gi: Vec<FeedItem> = group.iter().map(|(i, _, _)| i.clone()).collect();
        let grp: Vec<String> = group.iter().map(|(_, p, _)| p.clone()).collect();
        let gbb: Vec<String> = group.iter().map(|(_, _, b)| b.clone()).collect();
        if let Err(err) = db.upsert_feed_tasks(*sub_id, &gi, &grp, &gbb).await {
            tracing::warn!(
                epic_id = parent_id.0,
                sub_epic_id = sub_id.0,
                "run_role_routed_feed_sync: upsert_feed_tasks failed: {err:#}"
            );
        }
    }

    // Subtree-scoped delete at the ReviewsParent level: removes merged/closed
    // PRs from flat role sub-epics and clears role sub-epics absent from this
    // emission. Moved tasks are in the keep-set so they are never deleted here.
    if let Err(err) = db
        .delete_stale_subtree_feed_tasks(parent_id, &all_external_ids)
        .await
    {
        tracing::warn!(
            epic_id = parent_id.0,
            "run_role_routed_feed_sync: delete_stale_subtree_feed_tasks failed: {err:#}"
        );
    }

    // Second stale-deletion pass at the role sub-epic level: covers repo-group
    // grandchildren. Always run for every role sub-epic — not just grouped ones
    // — so orphaned repo-group tasks are cleaned up when group_by_repo is off.
    // The SQL is one level deep, so calling it with the role sub-epic as root
    // reaches its repo-group children — exactly the grandchild level relative
    // to the parent.
    for sub in [my, team, bots] {
        if let Err(err) = db
            .delete_stale_subtree_feed_tasks(sub, &all_external_ids)
            .await
        {
            tracing::warn!(
                epic_id = parent_id.0,
                sub_epic_id = sub.0,
                "run_role_routed_feed_sync: delete_stale_subtree_feed_tasks (role level) failed: {err:#}"
            );
        }
    }

    // Recalculate: repo-group sub-epics first (they propagate upward to role
    // sub-epics), then role sub-epics, then the parent.
    // Union newly created repo-group sub-epics (repo_group_cache) with
    // pre-existing ones so stale-deleted sub-epics also get recalculated.
    let repo_group_ids: std::collections::HashSet<EpicId> = repo_group_cache
        .values()
        .copied()
        .chain(pre_existing_repo_group_ids)
        .collect();
    for id in &repo_group_ids {
        super::recalculate_epic_status_after_feed(db, *id, "run_role_routed_feed_sync").await;
    }
    for sub in [my, team, bots] {
        super::recalculate_epic_status_after_feed(db, sub, "run_role_routed_feed_sync").await;
    }
    super::recalculate_epic_status_after_feed(db, parent_id, "run_role_routed_feed_sync").await;

    let mut all_ids = vec![parent_id, my, team, bots];
    all_ids.extend(repo_group_ids);
    Ok(all_ids)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use super::*;
    use crate::db::{
        CreateTaskRequest, Database, EpicCrud, EpicPatch, EpicRead, TaskCrud, TaskPatch,
    };
    use crate::models::{FeedRole, Signal, TaskStatus, TaskTag};

    fn make_item(external_id: &str, url: &str) -> FeedItem {
        FeedItem {
            external_id: external_id.to_string(),
            title: external_id.to_string(),
            description: String::new(),
            url: url.to_string(),
            url_type: None,
            status: crate::models::TaskStatus::Backlog,
            tag: TaskTag::PrReview,
            labels: vec![],
            sort_order: None,
            signals: vec![],
        }
    }

    fn make_signal_item(external_id: &str, url: &str, signals: Vec<Signal>) -> FeedItem {
        FeedItem {
            signals,
            ..make_item(external_id, url)
        }
    }

    /// Zip three parallel test slices into the `(item, repo_path, base_branch)`
    /// tuple slice `sync_grouped_feed` now takes. Mirrors the assembly that
    /// `run_feed_sync` performs at the boundary.
    fn entries(
        items: &[FeedItem],
        repo_paths: &[&str],
        base_branches: &[&str],
    ) -> Vec<FeedEntry> {
        items
            .iter()
            .zip(repo_paths.iter())
            .zip(base_branches.iter())
            .map(|((i, rp), bb)| (i.clone(), rp.to_string(), bb.to_string()))
            .collect()
    }

    /// Find the sub-epic of `parent` carrying `role`, asserting exactly one.
    async fn role_sub_epic(db: &Database, parent: EpicId, role: FeedRole) -> EpicId {
        let subs = db.list_sub_epics(parent).await.unwrap();
        let matching: Vec<_> = subs.iter().filter(|e| e.feed_role == role).collect();
        assert_eq!(
            matching.len(),
            1,
            "expected exactly one {role:?} sub-epic, got {subs:?}"
        );
        matching[0].id
    }

    // --- run_role_routed_feed_sync (WP3) ---

    /// Task 2 (B0): an emitted PR routes into the sub-epic for its role and the
    /// other role sub-epics stay empty.
    #[tokio::test]
    async fn route_routed_inserts_into_role_sub_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let items = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::DirectRequest],
        )];

        run_role_routed_feed_sync(
            &*db,
            parent.id,
            &items,
            &["".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        let bots = role_sub_epic(&db, parent.id, FeedRole::Bots).await;

        let my_tasks = db.list_tasks_for_epic(my).await.unwrap();
        assert_eq!(my_tasks.len(), 1, "direct-request PR lands in My Reviews");
        assert_eq!(my_tasks[0].external_id.as_deref(), Some("pr-1"));
        assert!(db.list_tasks_for_epic(team).await.unwrap().is_empty());
        assert!(db.list_tasks_for_epic(bots).await.unwrap().is_empty());
    }

    /// Task 3 (B2): a PR whose role changes is MOVED, preserving its in-flight
    /// status, sub_status, worktree, and tmux_window (agent session survives).
    #[tokio::test]
    async fn route_routed_moves_task_preserving_state() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        // Cycle 1: a team-requested PR lands in Team Reviews.
        let cycle1 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::TeamRequest],
        )];
        run_role_routed_feed_sync(
            &*db,
            parent.id,
            &cycle1,
            &["".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        let task = db.list_tasks_for_epic(team).await.unwrap().remove(0);

        // Simulate in-flight dispatched work on the task.
        db.patch_task(
            task.id,
            &TaskPatch::new()
                .status(TaskStatus::Running)
                .sub_status(crate::models::SubStatus::Active)
                .worktree(Some("/tmp/wt-pr-1"))
                .tmux_window(Some("dispatch:7")),
        )
        .await
        .unwrap();

        // Cycle 2: the same PR is now also reviewed -> routes to My Reviews.
        let cycle2 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::TeamRequest, Signal::Reviewed],
        )];
        run_role_routed_feed_sync(
            &*db,
            parent.id,
            &cycle2,
            &["".to_string()],
            &["main".to_string()],
        )
        .await
        .unwrap();

        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        let my_tasks = db.list_tasks_for_epic(my).await.unwrap();
        assert_eq!(my_tasks.len(), 1, "exactly one task, moved into My Reviews");
        let moved = &my_tasks[0];
        assert_eq!(moved.id, task.id, "same task row, not a recreate");
        assert_eq!(moved.external_id.as_deref(), Some("pr-1"));
        assert_eq!(moved.status, TaskStatus::Running, "status preserved");
        assert_eq!(
            moved.sub_status,
            crate::models::SubStatus::Active,
            "sub_status preserved"
        );
        assert_eq!(moved.worktree.as_deref(), Some("/tmp/wt-pr-1"));
        assert_eq!(moved.tmux_window.as_deref(), Some("dispatch:7"));

        assert!(
            db.list_tasks_for_epic(team).await.unwrap().is_empty(),
            "old role sub-epic no longer holds the moved task"
        );
    }

    /// Task 4 (B1): the moved task is NOT deleted by the same cycle even though
    /// it is absent from its losing role's group.
    #[tokio::test]
    async fn route_routed_move_not_deleted_same_cycle() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let cycle1 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::TeamRequest],
        )];
        run_role_routed_feed_sync(&*db, parent.id, &cycle1, &["".into()], &["main".into()])
            .await
            .unwrap();

        let cycle2 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::Reviewed],
        )];
        run_role_routed_feed_sync(&*db, parent.id, &cycle2, &["".into()], &["main".into()])
            .await
            .unwrap();

        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        assert_eq!(
            db.list_tasks_for_epic(my).await.unwrap().len(),
            1,
            "moved PR survives in its new role"
        );
        assert!(db.list_tasks_for_epic(team).await.unwrap().is_empty());
    }

    /// Task 4: a PR present in cycle 1 but absent from cycle 2 (merged/closed)
    /// is removed from the subtree; a manual task (external_id NULL) survives.
    #[tokio::test]
    async fn route_routed_removes_merged_pr_keeps_manual() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let cycle1 = vec![
            make_signal_item(
                "pr-1",
                "https://github.com/org/repo/pull/1",
                vec![Signal::DirectRequest],
            ),
            make_signal_item(
                "pr-2",
                "https://github.com/org/repo/pull/2",
                vec![Signal::TeamRequest],
            ),
        ];
        run_role_routed_feed_sync(
            &*db,
            parent.id,
            &cycle1,
            &["".into(), "".into()],
            &["main".into(), "main".into()],
        )
        .await
        .unwrap();

        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        // A manual task the user added under a role sub-epic.
        let manual_id = db
            .create_task(CreateTaskRequest {
                title: "Manual",
                description: "",
                repo_path: "/repo",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: Some(my),
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
            })
            .await
            .unwrap();

        // Cycle 2: pr-2 merged/closed (absent). pr-1 still direct-requested.
        let cycle2 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::DirectRequest],
        )];
        run_role_routed_feed_sync(&*db, parent.id, &cycle2, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        assert!(
            db.list_tasks_for_epic(team).await.unwrap().is_empty(),
            "merged pr-2 removed from Team Reviews"
        );

        let my_tasks = db.list_tasks_for_epic(my).await.unwrap();
        assert!(
            my_tasks.iter().any(|t| t.id == manual_id),
            "manual task survives reconcile"
        );
        assert!(
            my_tasks
                .iter()
                .any(|t| t.external_id.as_deref() == Some("pr-1")),
            "still-open pr-1 retained"
        );
    }

    // --- group_by_repo on role sub-epics ---

    /// When a role sub-epic has `group_by_repo = true`, feed items must be
    /// routed into per-repo sub-epics rather than into the role sub-epic directly.
    #[tokio::test]
    async fn role_routed_group_by_repo_routes_into_repo_sub_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        // First cycle — creates role sub-epics.
        let items1 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/myrepo/pull/1",
            vec![Signal::TeamRequest],
        )];
        run_role_routed_feed_sync(&*db, parent.id, &items1, &["".into()], &["main".into()])
            .await
            .unwrap();

        // Enable group_by_repo on Team Reviews.
        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();

        // Second cycle — same PR. Should now land in a repo-group sub-epic.
        run_role_routed_feed_sync(&*db, parent.id, &items1, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team_direct = db.list_tasks_for_epic(team).await.unwrap();
        assert!(
            team_direct.is_empty(),
            "Team Reviews must have no direct tasks when group_by_repo is active"
        );

        let repo_subs = db.list_sub_epics(team).await.unwrap();
        assert_eq!(
            repo_subs.len(),
            1,
            "one repo-group sub-epic under Team Reviews"
        );
        assert_eq!(repo_subs[0].title, "myrepo");
        let repo_tasks = db.list_tasks_for_epic(repo_subs[0].id).await.unwrap();
        assert_eq!(repo_tasks.len(), 1, "PR landed in the repo-group sub-epic");
        assert_eq!(repo_tasks[0].external_id.as_deref(), Some("pr-1"));
    }

    /// Re-running the feed when group_by_repo is active must not create
    /// duplicate tasks in the role sub-epic — the `existing` map must reach
    /// into repo-group sub-epics so the PR is recognised as already present.
    #[tokio::test]
    async fn role_routed_group_by_repo_no_duplicate_on_resync() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let items = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/myrepo/pull/1",
            vec![Signal::TeamRequest],
        )];

        // First cycle — creates role sub-epics.
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();

        // Second cycle — lands in repo-group sub-epic.
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        // Third cycle — must not duplicate.
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let repo_subs = db.list_sub_epics(team).await.unwrap();
        assert_eq!(repo_subs.len(), 1);
        let tasks = db.list_tasks_for_epic(repo_subs[0].id).await.unwrap();
        assert_eq!(tasks.len(), 1, "exactly one task after three cycles");
        assert!(
            db.list_tasks_for_epic(team).await.unwrap().is_empty(),
            "no duplicate in role sub-epic"
        );
    }

    /// When a PR disappears from the feed and group_by_repo is active, the
    /// stale deletion must reach into the repo-group sub-epic grandchildren
    /// and remove the task.
    #[tokio::test]
    async fn role_routed_group_by_repo_stale_deletion_reaches_grandchildren() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let items = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/myrepo/pull/1",
            vec![Signal::TeamRequest],
        )];

        // First cycle — creates role sub-epics.
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();

        // Second cycle — PR lands in repo-group sub-epic.
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let repo_subs = db.list_sub_epics(team).await.unwrap();
        assert_eq!(
            db.list_tasks_for_epic(repo_subs[0].id).await.unwrap().len(),
            1,
            "task present before stale deletion cycle"
        );

        // Third cycle — PR absent (merged/closed).
        run_role_routed_feed_sync(&*db, parent.id, &[], &[], &[])
            .await
            .unwrap();

        let tasks_after = db.list_tasks_for_epic(repo_subs[0].id).await.unwrap();
        assert!(
            tasks_after.is_empty(),
            "stale PR must be removed from repo-group sub-epic"
        );
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

        let sub_ids = sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;

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

        sync_grouped_feed(&*db, parent.id, entries(&items, &["", ""], &["main", "main"])).await;

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
            url_type: None,
            status: TaskStatus::Backlog,
            tag: TaskTag::Bug,
            labels: vec![],
            sort_order: None,
            signals: vec![],
        }];

        sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;

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

        let sub_ids = sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;

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
            url_type: None,
            status: crate::models::TaskStatus::Backlog,
            tag: crate::models::TaskTag::Bug,
            labels: vec![],
            sort_order: None,
            signals: vec![],
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
            url_type: None,
            status: crate::models::TaskStatus::Backlog,
            tag: crate::models::TaskTag::PrReview,
            labels: vec![],
            sort_order: None,
            signals: vec![],
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
        sync_grouped_feed(&*db, parent.id, entries(&items, &["", ""], &["main", "main"])).await;

        assert_eq!(db.list_sub_epics(parent.id).await.unwrap().len(), 2);

        // Second cycle: the feed now returns nothing.
        sync_grouped_feed(&*db, parent.id, vec![]).await;

        let subs = db.list_sub_epics(parent.id).await.unwrap();
        assert_eq!(
            subs.len(),
            2,
            "sub-epic rows remain, only their tasks clear"
        );
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
        sync_grouped_feed(&*db, parent.id, entries(&items, &["", ""], &["main", "main"])).await;

        // Second cycle: only repo-a still has an open item.
        let items2 = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
        sync_grouped_feed(&*db, parent.id, entries(&items2, &[""], &["main"])).await;

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

    /// When group_by_repo is toggled OFF, the next feed cycle must move tasks
    /// from the orphaned repo-group sub-epic onto the role sub-epic — no duplicate.
    #[tokio::test]
    async fn role_routed_group_by_repo_off_rehomes_repo_tasks_no_duplicate() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let items = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/myrepo/pull/1",
            vec![Signal::TeamRequest],
        )];

        // Cycle 1 — team_reviews has group_by_repo OFF, task lands flat.
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;

        // Enable group_by_repo → cycle 2 moves task into repo sub-epic.
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let repo_subs = db.list_sub_epics(team).await.unwrap();
        assert_eq!(repo_subs.len(), 1);
        assert_eq!(
            db.list_tasks_for_epic(repo_subs[0].id).await.unwrap().len(),
            1
        );
        assert!(db.list_tasks_for_epic(team).await.unwrap().is_empty());

        // Disable group_by_repo → cycle 3 must re-home task to role sub-epic, no duplicate.
        db.patch_epic(team, &EpicPatch::new().group_by_repo(false))
            .await
            .unwrap();
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team_tasks = db.list_tasks_for_epic(team).await.unwrap();
        assert_eq!(
            team_tasks.len(),
            1,
            "exactly one task on role sub-epic after toggle-off cycle"
        );
        assert_eq!(team_tasks[0].external_id.as_deref(), Some("pr-1"));

        // No tasks remain in any repo-group sub-epic.
        for sub in db.list_sub_epics(team).await.unwrap() {
            assert!(
                db.list_tasks_for_epic(sub.id).await.unwrap().is_empty(),
                "repo-group sub-epic {} must be empty after group_by_repo turned off",
                sub.title
            );
        }
    }

    /// When orphaned repo-group sub-epic tasks pre-exist (simulating a state from
    /// before the fix), the next feed cycle must re-home them without duplicating.
    #[tokio::test]
    async fn role_routed_orphaned_repo_tasks_rehosted_on_next_sync() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        // Manually create the role sub-epic and an orphaned repo-group sub-epic
        // with a task, simulating the pre-fix state.
        //
        // We use raw SQL because:
        //   - create_epic sets origin='manual', not 'repo-group'; the feed code
        //     identifies repo sub-epics by origin='repo-group'.
        //   - CreateTaskRequest has no external_id field (always NULL); tasks must
        //     have a non-NULL external_id to be visible to the existing-task index.
        //   - Insert the task with external_id=NULL then UPDATE to avoid the v72
        //     BEFORE INSERT trigger (same pattern as the v71 test).
        let (team_id, repo_sub_id) = db.db_call(|conn| {
            conn.execute_batch(
                "INSERT INTO epics (title, description, status, feed_role, origin, parent_epic_id)
                 VALUES ('Team Reviews', '', 'backlog', 'team-reviews', 'manual', 1);
                 INSERT INTO epics (title, description, status, feed_role, origin, parent_epic_id, group_by_repo)
                 VALUES ('myrepo', '', 'backlog', 'none', 'repo-group',
                         (SELECT id FROM epics WHERE feed_role = 'team-reviews'), 0);",
            )
            .map_err(anyhow::Error::from)?;
            let team_id: i64 = conn.query_row(
                "SELECT id FROM epics WHERE feed_role = 'team-reviews'",
                [],
                |r| r.get(0),
            )?;
            let repo_sub_id: i64 = conn.query_row(
                "SELECT id FROM epics WHERE origin = 'repo-group'",
                [],
                |r| r.get(0),
            )?;
            Ok::<_, anyhow::Error>((team_id, repo_sub_id))
        })
        .await
        .unwrap();
        let team = EpicId(team_id);
        let repo_sub_id = EpicId(repo_sub_id);

        db.db_call(move |conn| {
            conn.execute_batch(&format!(
                "INSERT INTO tasks (title, description, repo_path, status, base_branch, epic_id)
                 VALUES ('PR #1', '', '/r', 'backlog', 'main', {repo});
                 UPDATE tasks SET external_id = 'pr-1' WHERE epic_id = {repo};",
                repo = repo_sub_id.0
            ))
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();

        // Feed cycle with group_by_repo=false — should find the orphaned task
        // and re-home it, producing exactly one task on the role sub-epic.
        let items = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/myrepo/pull/1",
            vec![Signal::TeamRequest],
        )];
        run_role_routed_feed_sync(&*db, parent.id, &items, &["".into()], &["main".into()])
            .await
            .unwrap();

        let team_tasks = db.list_tasks_for_epic(team).await.unwrap();
        assert_eq!(team_tasks.len(), 1, "task re-homed to role sub-epic");
        assert_eq!(team_tasks[0].external_id.as_deref(), Some("pr-1"));
        assert!(
            db.list_tasks_for_epic(repo_sub_id)
                .await
                .unwrap()
                .is_empty(),
            "orphaned repo-group sub-epic now empty"
        );
    }

    /// Clearing a dropped sub-epic removes only feed tasks (external_id set);
    /// a manually-added task (external_id = null) in that sub-epic survives.
    #[tokio::test]
    async fn sync_grouped_feed_preserves_manual_task_in_dropped_sub_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();

        let items = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
        sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;

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
        sync_grouped_feed(&*db, parent.id, vec![]).await;

        let tasks = db.list_tasks_for_epic(repo_a.id).await.unwrap();
        assert_eq!(tasks.len(), 1, "only the manual task survives");
        assert_eq!(tasks[0].id, manual_id);
    }
}
