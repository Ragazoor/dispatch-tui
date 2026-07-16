use std::collections::HashMap;

use crate::db::TaskStore;
use crate::feed::route;
use crate::models::{Epic, EpicId, FeedItem, Task};
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
/// once at the `FeedCommandCompleted` boundary (see [`FeedItemWithTarget::zip`])
/// so the three values travel together as a unit through the rest of the feed
/// pipeline — there is no parallel-slice length invariant left to police.
pub(crate) struct FeedItemWithTarget {
    item: FeedItem,
    repo_path: String,
    base_branch: String,
}

impl FeedItemWithTarget {
    /// Zip co-indexed `items`/`repo_paths`/`base_branches` into paired entries.
    /// `repo_paths` and `base_branches` are derived one-per-item upstream
    /// (`resolve_feed_item_repo_paths` / `resolve_base_branches`), so the zip
    /// is lossless. Called once per emission, at each `FeedCommandCompleted`
    /// call site (`FeedRunner::tick`, `exec_trigger_epic_feed`) — the only
    /// place three parallel collections still exist, immediately before they
    /// collapse into these paired entries for the rest of the pipeline.
    pub(crate) fn zip(
        items: Vec<FeedItem>,
        repo_paths: Vec<String>,
        base_branches: Vec<String>,
    ) -> Vec<Self> {
        items
            .into_iter()
            .zip(repo_paths)
            .zip(base_branches)
            .map(|((item, repo_path), base_branch)| Self {
                item,
                repo_path,
                base_branch,
            })
            .collect()
    }

    /// Split paired entries back into the three slices `TaskStore::upsert_feed_tasks`
    /// still takes (a DB-layer concern, out of scope for this pipeline refactor).
    fn unzip(entries: Vec<Self>) -> (Vec<FeedItem>, Vec<String>, Vec<String>) {
        let mut items = Vec::with_capacity(entries.len());
        let mut repo_paths = Vec::with_capacity(entries.len());
        let mut base_branches = Vec::with_capacity(entries.len());
        for entry in entries {
            items.push(entry.item);
            repo_paths.push(entry.repo_path);
            base_branches.push(entry.base_branch);
        }
        (items, repo_paths, base_branches)
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
        super::recalculate_epic_status_after_feed(db, parent_id, "sync_grouped_feed").await;
    }

    sub_epic_ids
}

/// Upsert feed items using the correct strategy for `epic.group_by_repo`.
///
/// - `group_by_repo = false`: FlatFeedReconcile (feeds.allium) — any active
///   RepoGroup sub-epic left over from a prior grouped state is flattened
///   back onto the parent first (re-homing its tasks, deleting it if it ends
///   up empty), then a flat upsert runs directly on the parent epic. This is
///   the symmetric OFF-side counterpart to the ON-side migration below: it
///   makes toggling group_by_repo off on a feed epic self-healing on the next
///   poll, rather than leaving tasks stranded in their old repo sub-epics.
/// - `group_by_repo = true`: group by repo name, upsert into per-repo sub-epics,
///   then clear flat tasks from the parent.
///
/// Returns `epic_id` plus any sub-epic IDs written to (grouped path only).
/// Callers use this list to send one TUI notification per affected epic.
pub(crate) async fn run_feed_sync(
    db: &dyn TaskStore,
    epic_id: EpicId,
    group_by_repo: bool,
    entries: Vec<FeedItemWithTarget>,
) -> Result<Vec<EpicId>> {
    if group_by_repo {
        let sub_ids = sync_grouped_feed(db, epic_id, entries).await;
        let mut all_ids = vec![epic_id];
        all_ids.extend(sub_ids);
        Ok(all_ids)
    } else {
        // FlatFeedReconcile: reconcile any leftover RepoGroup sub-epics back
        // onto the parent before the flat upsert. Reuses flatten_epic (shared
        // with the manual FlattenEpic path) — idempotent no-op when no
        // RepoGroup sub-epics exist, so this is safe to run on every flat
        // sync, not only the first one after group_by_repo is toggled off.
        // Gated on an active RepoGroup sub-epic actually existing: flatten_epic
        // always ends with a recalculate_epic_status call, which would
        // otherwise duplicate the recalc callers already run right after
        // run_feed_sync returns (feed/mod.rs, runtime/epics.rs) on every poll,
        // not just the one cycle after a toggle.
        let has_repo_group_sub_epic = db.list_sub_epics(epic_id).await?.iter().any(|e| {
            e.origin == crate::models::EpicOrigin::RepoGroup
                && e.status != crate::models::TaskStatus::Archived
        });
        if has_repo_group_sub_epic {
            crate::service::flatten_epic(db, epic_id).await?;
        }
        let (items, repo_paths, base_branches) = FeedItemWithTarget::unzip(entries);
        db.upsert_feed_tasks(epic_id, &items, &repo_paths, &base_branches)
            .await?;
        Ok(vec![epic_id])
    }
}

/// Dispatch a feed emission to the correct sync strategy for `feed_role`. This
/// is the SINGLE authoritative role→sync-path mapping, shared by both the
/// auto-poll ([`crate::feed::FeedRunner`] tick) and the manual "r" refresh
/// (`exec_trigger_epic_feed`) so the two paths cannot drift — a `reviews_parent`
/// epic ALWAYS routes through [`run_role_routed_feed_sync`], never a flat upsert
/// onto the parent (feeds.allium: FeedSync dispatch). Callers that must reject
/// role sub-epics carrying a feed_command (the tick's provisioning guard) do so
/// BEFORE calling this; every reachable role here is safe to sync.
pub(crate) async fn run_feed_sync_by_role(
    db: &dyn TaskStore,
    epic_id: EpicId,
    feed_role: crate::models::FeedRole,
    group_by_repo: bool,
    entries: Vec<FeedItemWithTarget>,
) -> Result<Vec<EpicId>> {
    match feed_role {
        crate::models::FeedRole::ReviewsParent => {
            run_role_routed_feed_sync(db, epic_id, entries).await
        }
        _ => run_feed_sync(db, epic_id, group_by_repo, entries).await,
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

/// The three role sub-epics of a `reviews_parent` epic, plus the parent's
/// sub-epic list fetched once and reused for `can_auto_group` lookups (the
/// roles are distinct, so creating one never affects another).
struct RoleSubEpics {
    my: EpicId,
    team: EpicId,
    bots: EpicId,
    existing_subs: Vec<Epic>,
}

impl RoleSubEpics {
    fn ids(&self) -> [EpicId; 3] {
        [self.my, self.team, self.bots]
    }

    /// `route` only ever yields My/Team/Bots; My is also the safe fallback.
    fn target_for(&self, role: crate::models::FeedRole) -> EpicId {
        use crate::models::FeedRole;
        match role {
            FeedRole::TeamReviews => self.team,
            FeedRole::Bots => self.bots,
            _ => self.my,
        }
    }

    /// Extract can_auto_group from `existing_subs` (already loaded), avoiding
    /// an extra get_epic round-trip. A newly-created role sub-epic is not in
    /// the list and defaults to false (group_by_repo is false on creation).
    fn can_auto_group(&self, id: EpicId) -> bool {
        self.existing_subs
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.can_auto_group())
            .unwrap_or(false)
    }
}

/// Ensure the three role sub-epics exist (idempotent, matched by feed_role).
async fn ensure_role_sub_epics(db: &dyn TaskStore, parent_id: EpicId) -> Result<RoleSubEpics> {
    use crate::models::FeedRole;
    let existing_subs = db.list_sub_epics(parent_id).await?;
    let my = ensure_role_sub_epic(db, parent_id, &existing_subs, FeedRole::MyReviews).await?;
    let team = ensure_role_sub_epic(db, parent_id, &existing_subs, FeedRole::TeamReviews).await?;
    let bots = ensure_role_sub_epic(db, parent_id, &existing_subs, FeedRole::Bots).await?;
    Ok(RoleSubEpics {
        my,
        team,
        bots,
        existing_subs,
    })
}

/// Index existing subtree feed tasks by external_id (global identity across
/// the role sub-epics). Scan repo-group sub-epics FIRST so that role sub-epic
/// copies overwrite them below — ensuring role sub-epic copies win when both
/// exist. This prevents duplicate-insert constraint violations on the MOVE
/// path when group_by_repo is off but orphaned repo-group tasks still exist.
///
/// Also returns the pre-existing repo-group child IDs so the caller can
/// recalculate them after the stale-deletion pass — even for sub-epics not
/// written this cycle.
///
/// Scans the PARENT itself FIRST (lowest priority): a feed task stranded flat
/// on the reviews_parent epic — e.g. from an out-of-band flat upsert — is
/// folded into the index so the routing loop MOVES it down into its role
/// sub-epic (never a duplicate insert, so no subtree-uniqueness collision).
/// Scanned before the sub-epics so a genuine sub-epic copy wins the identity
/// if both somehow exist. A parent-stranded task absent from this emission is
/// instead removed by the parent clear in [`clear_parent_stranded_tasks`].
async fn build_existing_task_index(
    db: &dyn TaskStore,
    parent_id: EpicId,
    roles: &RoleSubEpics,
) -> Result<(HashMap<String, Task>, Vec<EpicId>)> {
    let mut existing: HashMap<String, Task> = HashMap::new();
    let mut pre_existing_repo_group_ids: Vec<EpicId> = Vec::new();

    for task in db.list_tasks_for_epic(parent_id).await? {
        if let Some(ext) = task.external_id.clone() {
            existing.insert(ext, task);
        }
    }
    for sub in roles.ids() {
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
    for sub in roles.ids() {
        for task in db.list_tasks_for_epic(sub).await? {
            if let Some(ext) = task.external_id.clone() {
                existing.insert(ext, task);
            }
        }
    }

    Ok((existing, pre_existing_repo_group_ids))
}

/// Result of [`route_and_group_entries`]: present entries grouped by target
/// sub-epic (for the insert/update pass), the union of all emitted
/// external_ids (for the stale-delete pass), and the repo-group sub-epics
/// created/looked-up while routing (for the final recalculation pass).
struct RoutedEntries {
    groups: HashMap<EpicId, Vec<FeedItemWithTarget>>,
    all_external_ids: Vec<String>,
    repo_group_cache: HashMap<(EpicId, String), EpicId>,
}

/// Route each entry to its target sub-epic (resolving into a per-repo
/// sub-epic when the role has `group_by_repo`), moving any cross-role or
/// parent-stranded task in place as it goes — `set_task_epic_id` touches only
/// epic_id/updated_at, so status/sub_status/worktree/tmux_window/sort_order
/// survive, and the field update that follows applies the latest feed
/// metadata.
async fn route_and_group_entries(
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

/// Insert/update present roles. Because every cross-role task was already
/// moved out of its losing epic by [`route_and_group_entries`],
/// `upsert_feed_tasks`' per-epic delete only ever removes genuinely-stale
/// rows here — never a moved task.
async fn upsert_role_groups(
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

/// Subtree-scoped delete: removes merged/closed PRs from flat role sub-epics
/// and clears role sub-epics absent from this emission (moved tasks are in
/// the keep-set, so they are never deleted here), then a second pass at the
/// role level to cover repo-group grandchildren — always run, not only for
/// grouped roles, so orphaned repo-group tasks are cleaned up when
/// group_by_repo is off. The SQL is one level deep, so calling it with the
/// role sub-epic as root reaches its repo-group children — exactly the
/// grandchild level relative to the parent.
async fn delete_stale_subtree(
    db: &dyn TaskStore,
    parent_id: EpicId,
    roles: &RoleSubEpics,
    all_external_ids: &[String],
) {
    if let Err(err) = db
        .delete_stale_subtree_feed_tasks(parent_id, all_external_ids)
        .await
    {
        tracing::warn!(
            epic_id = parent_id.0,
            "run_role_routed_feed_sync: delete_stale_subtree_feed_tasks failed: {err:#}"
        );
    }

    for sub in roles.ids() {
        if let Err(err) = db
            .delete_stale_subtree_feed_tasks(sub, all_external_ids)
            .await
        {
            tracing::warn!(
                epic_id = parent_id.0,
                sub_epic_id = sub.0,
                "run_role_routed_feed_sync: delete_stale_subtree_feed_tasks (role level) failed: {err:#}"
            );
        }
    }
}

/// Parent sweep: a reviews_parent epic must hold NO feed-managed task
/// directly (NoFlatFeedTasksOnReviewsParent). Present parent-stranded tasks
/// were already MOVED down by [`route_and_group_entries`], so anything left
/// here is either a merged/closed stray or a legacy duplicate whose routed
/// copy already lives in a sub-epic — both must go. An empty upsert reuses
/// `upsert_feed_tasks`' per-epic stale-delete (external_id NOT IN {} deletes
/// every feed task on the epic) in ONE statement, preserving manual tasks
/// (external_id IS NULL). Same idiom [`sync_grouped_feed`] uses to clear the
/// parent on the grouped path.
async fn clear_parent_stranded_tasks(db: &dyn TaskStore, parent_id: EpicId) {
    if let Err(err) = db.upsert_feed_tasks(parent_id, &[], &[], &[]).await {
        tracing::warn!(
            epic_id = parent_id.0,
            "run_role_routed_feed_sync: failed to clear parent-stranded feed tasks: {err:#}"
        );
    }
}

/// Recalculate: repo-group sub-epics first (they propagate upward to role
/// sub-epics), then role sub-epics, then the parent.
async fn recalculate_subtree(
    db: &dyn TaskStore,
    parent_id: EpicId,
    roles: &RoleSubEpics,
    repo_group_ids: &std::collections::HashSet<EpicId>,
) {
    for id in repo_group_ids {
        super::recalculate_epic_status_after_feed(db, *id, "run_role_routed_feed_sync").await;
    }
    for sub in roles.ids() {
        super::recalculate_epic_status_after_feed(db, sub, "run_role_routed_feed_sync").await;
    }
    super::recalculate_epic_status_after_feed(db, parent_id, "run_role_routed_feed_sync").await;
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
///
/// Takes a single owned `Vec<FeedItemWithTarget>` rather than three parallel
/// slices, so per-index alignment with `repo_path`/`base_branch` is
/// structural — there is no length-mismatch guard here, unlike the earlier
/// version of this function (per-item pairing cannot come apart).
pub(crate) async fn run_role_routed_feed_sync(
    db: &dyn TaskStore,
    parent_id: EpicId,
    entries: Vec<FeedItemWithTarget>,
) -> Result<Vec<EpicId>> {
    let roles = ensure_role_sub_epics(db, parent_id).await?;
    let (existing, pre_existing_repo_group_ids) =
        build_existing_task_index(db, parent_id, &roles).await?;
    let routed = route_and_group_entries(db, parent_id, entries, &existing, &roles).await?;

    upsert_role_groups(db, parent_id, routed.groups).await;
    delete_stale_subtree(db, parent_id, &roles, &routed.all_external_ids).await;
    clear_parent_stranded_tasks(db, parent_id).await;

    let repo_group_ids: std::collections::HashSet<EpicId> = routed
        .repo_group_cache
        .into_values()
        .chain(pre_existing_repo_group_ids)
        .collect();
    recalculate_subtree(db, parent_id, &roles, &repo_group_ids).await;

    let mut all_ids = vec![parent_id, roles.my, roles.team, roles.bots];
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
            wrap_up_mode: None,
        }
    }

    fn make_signal_item(external_id: &str, url: &str, signals: Vec<Signal>) -> FeedItem {
        FeedItem {
            signals,
            ..make_item(external_id, url)
        }
    }

    /// Zip three parallel test slices into paired [`FeedItemWithTarget`]
    /// entries. Mirrors the assembly `FeedItemWithTarget::zip` performs at
    /// the feed boundary.
    fn entries(
        items: &[FeedItem],
        repo_paths: &[&str],
        base_branches: &[&str],
    ) -> Vec<FeedItemWithTarget> {
        items
            .iter()
            .zip(repo_paths.iter())
            .zip(base_branches.iter())
            .map(|((i, rp), bb)| FeedItemWithTarget {
                item: i.clone(),
                repo_path: rp.to_string(),
                base_branch: bb.to_string(),
            })
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

        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&cycle1, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&cycle1, &[""], &["main"]))
            .await
            .unwrap();

        let cycle2 = vec![make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::Reviewed],
        )];
        run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
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

    /// Count feed-managed tasks (external_id set) sitting DIRECTLY on an epic.
    async fn flat_feed_task_count(db: &Database, epic: EpicId) -> usize {
        db.list_tasks_for_epic(epic)
            .await
            .unwrap()
            .into_iter()
            .filter(|t| t.external_id.is_some())
            .count()
    }

    /// Bug B (parent-stranded rescue): a feed task sitting flat on the
    /// reviews_parent epic itself is MOVED down into its routed role sub-epic —
    /// same row, in-flight state preserved — not left to deadlock the
    /// subtree-uniqueness trigger. Enforces NoFlatFeedTasksOnReviewsParent.
    #[tokio::test]
    async fn route_routed_rescues_flat_task_stranded_on_parent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        // Strand a flat feed task directly on the parent — exactly what an
        // out-of-band flat upsert (the manual-trigger bug, or an older binary)
        // produces. Inserting into the reviews_parent epic does not fire the
        // subtree-uniqueness trigger, so this is a valid starting state.
        let item = make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::DirectRequest],
        );
        db.upsert_feed_tasks(
            parent.id,
            std::slice::from_ref(&item),
            &["".into()],
            &["main".into()],
        )
        .await
        .unwrap();
        let stranded = db.list_tasks_for_epic(parent.id).await.unwrap().remove(0);

        // Simulate in-flight dispatched work on the stranded task.
        db.patch_task(
            stranded.id,
            &TaskPatch::new()
                .status(TaskStatus::Running)
                .sub_status(crate::models::SubStatus::Active)
                .worktree(Some("/tmp/wt-pr-1"))
                .tmux_window(Some("dispatch:7")),
        )
        .await
        .unwrap();

        // Reconcile with the same PR present in the emission.
        run_role_routed_feed_sync(&*db, parent.id, entries(&[item], &[""], &["main"]))
            .await
            .unwrap();

        assert_eq!(
            flat_feed_task_count(&db, parent.id).await,
            0,
            "reviews_parent must hold no flat feed task after reconcile"
        );

        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        let my_tasks = db.list_tasks_for_epic(my).await.unwrap();
        assert_eq!(my_tasks.len(), 1, "the rescued PR lands in My Reviews once");
        let moved = &my_tasks[0];
        assert_eq!(moved.id, stranded.id, "same task row, not delete+recreate");
        assert_eq!(moved.status, TaskStatus::Running, "status preserved");
        assert_eq!(
            moved.sub_status,
            crate::models::SubStatus::Active,
            "sub_status preserved"
        );
        assert_eq!(moved.worktree.as_deref(), Some("/tmp/wt-pr-1"));
        assert_eq!(moved.tmux_window.as_deref(), Some("dispatch:7"));
    }

    /// Bug B (parent-stranded stale delete): a feed task stranded on the parent
    /// that no current item names is removed as stale by the subtree delete,
    /// whose scope must include the parent epic itself.
    #[tokio::test]
    async fn route_routed_deletes_stale_flat_task_on_parent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let gone = make_item("pr-gone", "https://github.com/org/repo/pull/9");
        db.upsert_feed_tasks(parent.id, &[gone], &["".into()], &["main".into()])
            .await
            .unwrap();
        assert_eq!(flat_feed_task_count(&db, parent.id).await, 1);

        // Emission no longer contains pr-gone (merged/closed).
        run_role_routed_feed_sync(&*db, parent.id, entries(&[], &[], &[]))
            .await
            .unwrap();

        assert_eq!(
            flat_feed_task_count(&db, parent.id).await,
            0,
            "stale parent-stranded feed task must be deleted"
        );
    }

    /// Bug B guard: a MANUAL task (external_id = null) on the parent is never
    /// touched by the parent-inclusive reconcile — only feed-managed tasks are.
    #[tokio::test]
    async fn route_routed_preserves_manual_task_on_parent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let manual_id = db
            .create_task(CreateTaskRequest {
                title: "Manual note on parent",
                description: "",
                repo_path: "/repo",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: Some(parent.id),
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
            })
            .await
            .unwrap();

        run_role_routed_feed_sync(&*db, parent.id, entries(&[], &[], &[]))
            .await
            .unwrap();

        let survivors = db.list_tasks_for_epic(parent.id).await.unwrap();
        assert_eq!(survivors.len(), 1, "manual task must survive");
        assert_eq!(survivors[0].id, manual_id);
        assert!(
            survivors[0].external_id.is_none(),
            "the survivor is the manual (non-feed) task"
        );
    }

    /// Bug B (legacy duplicate convergence): the corrupt state the old
    /// flat-upsert bug produced — the SAME PR present BOTH flat on the parent
    /// AND routed in a role sub-epic. The reconcile must converge to a single
    /// copy in the sub-epic and clear the parent duplicate.
    #[tokio::test]
    async fn route_routed_clears_parent_duplicate_when_canonical_in_sub_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let item = make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::DirectRequest],
        );

        // Cycle 1: route the PR into My Reviews (the canonical copy).
        run_role_routed_feed_sync(
            &*db,
            parent.id,
            entries(std::slice::from_ref(&item), &[""], &["main"]),
        )
        .await
        .unwrap();
        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        assert_eq!(db.list_tasks_for_epic(my).await.unwrap().len(), 1);

        // Corrupt the state: plant a duplicate flat copy on the parent, as the
        // old manual-trigger flat upsert did (inserting onto a reviews_parent
        // epic does not fire the subtree-uniqueness trigger).
        db.upsert_feed_tasks(
            parent.id,
            std::slice::from_ref(&item),
            &["".into()],
            &["main".into()],
        )
        .await
        .unwrap();
        assert_eq!(
            flat_feed_task_count(&db, parent.id).await,
            1,
            "duplicate planted"
        );

        // Cycle 2: reconcile with the same PR present. Must converge.
        run_role_routed_feed_sync(
            &*db,
            parent.id,
            entries(std::slice::from_ref(&item), &[""], &["main"]),
        )
        .await
        .unwrap();

        assert_eq!(
            flat_feed_task_count(&db, parent.id).await,
            0,
            "parent duplicate cleared"
        );
        assert_eq!(
            db.list_tasks_for_epic(my).await.unwrap().len(),
            1,
            "exactly one canonical copy remains in My Reviews"
        );
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
            entries(&cycle1, &["", ""], &["main", "main"]),
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
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

    /// WP2 regression: each item must land with ITS OWN repo_path/base_branch,
    /// never a neighbour's. Three items across three different roles (so they
    /// land in three different sub-epics) each carry a distinct repo_path and
    /// base_branch; a mis-paired zip (the footgun the old parallel-slice
    /// length guard only detected after the fact) would surface here as a
    /// task holding the wrong branch or repo_path.
    #[tokio::test]
    async fn route_routed_preserves_per_item_repo_path_and_base_branch() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("Reviews", "", None).await.unwrap();
        db.patch_epic(
            parent.id,
            &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
        )
        .await
        .unwrap();

        let items = vec![
            make_signal_item(
                "pr-my",
                "https://github.com/org/repo-my/pull/1",
                vec![Signal::DirectRequest],
            ),
            make_signal_item(
                "pr-team",
                "https://github.com/org/repo-team/pull/2",
                vec![Signal::TeamRequest],
            ),
            make_signal_item(
                "pr-bots",
                "https://github.com/org/repo-bots/pull/3",
                vec![Signal::AuthorBot],
            ),
        ];
        let entries = entries(
            &items,
            &["/repo-my", "/repo-team", "/repo-bots"],
            &["my-branch", "team-branch", "bots-branch"],
        );
        run_role_routed_feed_sync(&*db, parent.id, entries)
            .await
            .unwrap();

        let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        let bots = role_sub_epic(&db, parent.id, FeedRole::Bots).await;

        let task_by_ext = |tasks: &[crate::models::Task], ext: &str| {
            tasks
                .iter()
                .find(|t| t.external_id.as_deref() == Some(ext))
                .unwrap()
                .clone()
        };

        let my_tasks = db.list_tasks_for_epic(my).await.unwrap();
        let my_task = task_by_ext(&my_tasks, "pr-my");
        assert_eq!(my_task.repo_path, "/repo-my");
        assert_eq!(my_task.base_branch, "my-branch");

        let team_tasks = db.list_tasks_for_epic(team).await.unwrap();
        let team_task = task_by_ext(&team_tasks, "pr-team");
        assert_eq!(team_task.repo_path, "/repo-team");
        assert_eq!(team_task.base_branch, "team-branch");

        let bots_tasks = db.list_tasks_for_epic(bots).await.unwrap();
        let bots_task = task_by_ext(&bots_tasks, "pr-bots");
        assert_eq!(bots_task.repo_path, "/repo-bots");
        assert_eq!(bots_task.base_branch, "bots-branch");
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&items1, &[""], &["main"]))
            .await
            .unwrap();

        // Enable group_by_repo on Team Reviews.
        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();

        // Second cycle — same PR. Should now land in a repo-group sub-epic.
        run_role_routed_feed_sync(&*db, parent.id, entries(&items1, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();

        // Second cycle — lands in repo-group sub-epic.
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        // Third cycle — must not duplicate.
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();

        // Second cycle — PR lands in repo-group sub-epic.
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        let repo_subs = db.list_sub_epics(team).await.unwrap();
        assert_eq!(
            db.list_tasks_for_epic(repo_subs[0].id).await.unwrap().len(),
            1,
            "task present before stale deletion cycle"
        );

        // Third cycle — PR absent (merged/closed).
        run_role_routed_feed_sync(&*db, parent.id, entries(&[], &[], &[]))
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

        sync_grouped_feed(
            &*db,
            parent.id,
            entries(&items, &["", ""], &["main", "main"]),
        )
        .await;

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
            wrap_up_mode: None,
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
            wrap_up_mode: None,
        }];

        let ids = run_feed_sync(&*db, epic.id, false, entries(&items, &[""], &["main"]))
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
            wrap_up_mode: None,
        }];

        let ids = run_feed_sync(&*db, epic.id, true, entries(&items, &[""], &["main"]))
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
        sync_grouped_feed(
            &*db,
            parent.id,
            entries(&items, &["", ""], &["main", "main"]),
        )
        .await;

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
        sync_grouped_feed(
            &*db,
            parent.id,
            entries(&items, &["", ""], &["main", "main"]),
        )
        .await;

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
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;

        // Enable group_by_repo → cycle 2 moves task into repo sub-epic.
        db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
            .await
            .unwrap();
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
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
        run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
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

    // --- FlatFeedReconcile (feeds.allium) ---
    //
    // Toggling group_by_repo OFF on a feed epic only flips the flag; these
    // tests cover the flat sync path's reconciliation of pre-existing
    // RepoGroup sub-epics (docs/specs/feeds.allium: FlatFeedReconcile).

    /// A feed epic with group_by_repo=false and an existing active RepoGroup
    /// sub-epic: the flat sync path re-homes the sub-epic's task back to the
    /// parent (as the SAME row, not a delete+recreate) and deletes the
    /// now-empty sub-epic, then upserts the current emission onto the parent.
    #[tokio::test]
    async fn flat_sync_rehomes_tasks_from_existing_repo_group_subepic_and_deletes_it() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("CVE", "", None).await.unwrap();

        // Simulate a pre-existing grouped state: a RepoGroup sub-epic holding
        // a feed task, as if group_by_repo had been on for a prior poll.
        let sub = db
            .create_repo_group_sub_epic(parent.id, "myrepo")
            .await
            .unwrap();
        let seed = vec![make_item("cve-1", "https://github.com/org/myrepo/pull/1")];
        db.upsert_feed_tasks(sub, &seed, &["".into()], &["main".into()])
            .await
            .unwrap();
        let pre_existing = db.list_tasks_for_epic(sub).await.unwrap().remove(0);

        // Flat sync (group_by_repo=false) with the same item still emitted.
        let items = vec![make_item("cve-1", "https://github.com/org/myrepo/pull/1")];
        run_feed_sync(&*db, parent.id, false, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        let parent_tasks = db.list_tasks_for_epic(parent.id).await.unwrap();
        assert_eq!(parent_tasks.len(), 1, "task re-homed onto the parent");
        assert_eq!(
            parent_tasks[0].id, pre_existing.id,
            "re-home is a move (same task row), not a delete+recreate"
        );
        assert_eq!(parent_tasks[0].external_id.as_deref(), Some("cve-1"));

        assert!(
            db.get_epic(sub).await.unwrap().is_none(),
            "emptied RepoGroup sub-epic is deleted"
        );
    }

    /// Regression: a feed epic with group_by_repo=false and NO existing
    /// RepoGroup sub-epics behaves exactly as a plain flat upsert (no-op
    /// reconciliation, not vacuous — the emission still lands on the parent).
    #[tokio::test]
    async fn flat_sync_with_no_repo_group_subepics_is_unaffected() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("CVE", "", None).await.unwrap();

        let items = vec![make_item("cve-2", "https://github.com/org/other/pull/2")];
        run_feed_sync(&*db, parent.id, false, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        let parent_tasks = db.list_tasks_for_epic(parent.id).await.unwrap();
        assert_eq!(parent_tasks.len(), 1, "flat upsert still lands on parent");
        assert_eq!(parent_tasks[0].external_id.as_deref(), Some("cve-2"));
        assert!(
            db.list_sub_epics(parent.id).await.unwrap().is_empty(),
            "no sub-epics created or left behind"
        );
    }

    /// A manually-created (non-RepoGroup) sub-epic under a feed epic is never
    /// touched by flat-path reconciliation, mirroring
    /// `flatten_preserves_manual_sub_epics` in src/service/grouping.rs.
    #[tokio::test]
    async fn flat_sync_preserves_manual_sub_epic() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let parent = db.create_epic("CVE", "", None).await.unwrap();
        let manual = db.create_epic("notes", "", Some(parent.id)).await.unwrap();
        let manual_task = db
            .create_task(CreateTaskRequest {
                title: "Manual note",
                description: "",
                repo_path: "/repo",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: Some(manual.id),
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
            })
            .await
            .unwrap();

        let items = vec![make_item("cve-3", "https://github.com/org/other/pull/3")];
        run_feed_sync(&*db, parent.id, false, entries(&items, &[""], &["main"]))
            .await
            .unwrap();

        assert!(
            db.get_epic(manual.id).await.unwrap().is_some(),
            "manual sub-epic survives flat-path reconciliation"
        );
        let manual_tasks = db.list_tasks_for_epic(manual.id).await.unwrap();
        assert_eq!(manual_tasks.len(), 1, "manual task stays put");
        assert_eq!(manual_tasks[0].id, manual_task);
    }

    // A `reviews_parent` epic's exclusion from flat-path reconciliation
    // (docs/specs/feeds.allium: FlatFeedReconcile requires feed_role !=
    // reviews_parent) is structural, not a runtime branch inside
    // FlatFeedReconcile itself: `run_feed_sync_by_role`'s match arm (above)
    // routes `FeedRole::ReviewsParent` to `run_role_routed_feed_sync`
    // exclusively, so `run_feed_sync`'s flat branch is never reached for a
    // reviews_parent epic. That dispatch is already covered by
    // `route_routed_inserts_into_role_sub_epic` and the other
    // `route_routed_*` / `role_routed_*` tests above, which exercise
    // `run_role_routed_feed_sync` directly — no separate test needed here.
}
