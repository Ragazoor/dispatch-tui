//! Feed ingestion: what one parsed emission does to the store.
//!
//! Spec: `docs/specs/feeds.allium` (`IngestFeedEmission`, `FlatFeedReconcile`,
//! `GroupedFeedUpsert`, `RoleRoutedFeedSync`), moved here by store-split M3
//! (task #53256). The board's `src/feed/ingest/` used to decide all of this
//! with a dozen reducer calls, none of them in one transaction; it now runs the
//! feed script, resolves each item's repo path and posts the items with one
//! call to [`ingest_feed`].
//!
//! The strategy comes from the epic's own row, never from the caller, so a
//! board holding a stale copy of `feed_role` or `group_by_repo` cannot ingest
//! by the wrong one. The whole call is one transaction: a refusal anywhere
//! leaves the store as it was.

use super::*;
use std::collections::{HashMap, HashSet};

/// The refusal for an epic whose `feed_role` is one of the three role
/// sub-epic roles but which carries a feed command.
pub const ROLE_SUB_EPIC_FEED_REFUSED: &str =
    "role sub-epic carries a feed command; it is reconciled only via its reviews parent";

/// One parsed feed item as the board posts it: the task fields it resolved
/// (see [`FeedTaskUpsertItem`]) plus the `signals` the routing rules read.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct FeedIngestItem {
    pub item: FeedTaskUpsertItem,
    /// Kebab-case `Signal` names, as the feed script emitted them.
    pub signals: Vec<String>,
}

fn external_ids(items: &[FeedIngestItem]) -> HashSet<&str> {
    items.iter().map(|i| i.item.external_id.as_str()).collect()
}

fn upsert_items(items: Vec<FeedIngestItem>) -> Vec<FeedTaskUpsertItem> {
    items.into_iter().map(|i| i.item).collect()
}

/// The store operations feed ingestion reads and writes through.
///
/// The routing and reconcile decisions below are written ONCE, generic over
/// this trait: the reducer runs them against the live tables
/// ([`ReducerStore`]) and the board's in-memory test double
/// (`src/sync/memory_caller/feed.rs`) against its own rows. Each method is a
/// plain row operation or one of the feed helpers both sides already have; no
/// method makes a routing decision, so the two stores cannot disagree about
/// one (`spacetime-memory-store.allium: ReducerConformance`).
pub trait IngestStore {
    fn epic(&self, id: i64) -> Option<Epic>;
    /// The direct child epics of `parent`.
    fn child_epics(&self, parent: i64) -> Vec<Epic>;
    fn tasks_in_epic(&self, epic_id: i64) -> Vec<Task>;
    /// `root` and every epic under it.
    fn subtree_ids(&self, root: i64) -> HashSet<i64>;
    fn is_retired(&self, feed_epic_id: i64, external_id: &str) -> bool;
    /// The calling identity, as `caller_identity` reads it.
    fn identity(&self) -> String;
    /// The transaction's timestamp, in the store's spelling.
    fn now(&self) -> String;
    /// Insert an epic with a generated id; its id.
    fn insert_epic(&mut self, row: Epic) -> i64;
    /// Overwrite a task row as given, with no rule applied: a feed move.
    fn update_task(&mut self, row: Task);
    fn find_or_create_repo_group(&mut self, root: i64, title: &str, created_by: String) -> i64;
    fn flatten_epic(&mut self, root: i64) -> Result<(), String>;
    fn drop_closed_retired(&mut self, feed_epic_id: i64, keep: &HashSet<&str>);
    fn upsert_feed_tasks(
        &mut self,
        epic_id: i64,
        items: Vec<FeedTaskUpsertItem>,
        created_by: &str,
        delete_absent: bool,
    ) -> Result<(), String>;
    fn delete_stale_feed_tasks_in_epic(&mut self, epic_id: i64, keep: &HashSet<&str>);
    fn recalculate_epic_chain(&mut self, epic_id: i64);
    /// Mark `task_id` unseen for every follower of `epic_id` or an ancestor,
    /// as a signalled create does (`mcp-task-tools.allium:
    /// SignalMarksTheTaskForEveryFollower`).
    fn mark_task_unseen_for_followers(&mut self, task_id: i64, epic_id: i64);
}

/// [`IngestStore`] over a reducer's live tables.
pub(crate) struct ReducerStore<'a>(pub(crate) &'a ReducerContext);

impl IngestStore for ReducerStore<'_> {
    fn epic(&self, id: i64) -> Option<Epic> {
        self.0.db.epics().id().find(id)
    }
    fn child_epics(&self, parent: i64) -> Vec<Epic> {
        self.0.db.epics().parent_epic_id().filter(&parent).collect()
    }
    fn tasks_in_epic(&self, epic_id: i64) -> Vec<Task> {
        self.0.db.tasks().epic_id().filter(&epic_id).collect()
    }
    fn subtree_ids(&self, root: i64) -> HashSet<i64> {
        collect_epic_subtree_ids(self.0, root)
    }
    fn is_retired(&self, feed_epic_id: i64, external_id: &str) -> bool {
        is_retired(self.0, feed_epic_id, external_id)
    }
    fn identity(&self) -> String {
        caller_identity(self.0)
    }
    fn now(&self) -> String {
        now(self.0)
    }
    fn insert_epic(&mut self, row: Epic) -> i64 {
        self.0.db.epics().insert(row).id
    }
    fn update_task(&mut self, row: Task) {
        self.0.db.tasks().id().update(row);
    }
    fn find_or_create_repo_group(&mut self, root: i64, title: &str, created_by: String) -> i64 {
        find_or_create_repo_group(self.0, root, title, created_by)
    }
    fn flatten_epic(&mut self, root: i64) -> Result<(), String> {
        flatten_epic(self.0, root)
    }
    fn drop_closed_retired(&mut self, feed_epic_id: i64, keep: &HashSet<&str>) {
        drop_closed_retired(self.0, feed_epic_id, keep)
    }
    fn upsert_feed_tasks(
        &mut self,
        epic_id: i64,
        items: Vec<FeedTaskUpsertItem>,
        created_by: &str,
        delete_absent: bool,
    ) -> Result<(), String> {
        upsert_feed_tasks_inner(self.0, epic_id, items, created_by, delete_absent)
    }
    fn delete_stale_feed_tasks_in_epic(&mut self, epic_id: i64, keep: &HashSet<&str>) {
        delete_stale_feed_tasks_in_epic(self.0, epic_id, keep)
    }
    fn recalculate_epic_chain(&mut self, epic_id: i64) {
        recalculate_epic_chain(self.0, epic_id)
    }
    fn mark_task_unseen_for_followers(&mut self, task_id: i64, epic_id: i64) {
        mark_task_unseen_for_followers(self.0, task_id, epic_id)
    }
}

/// `feeds.allium: IngestFeedEmission`. Apply one parsed emission to the feed
/// epic `epic_id`.
///
/// `additive` is the board's verdict that this emission may not remove
/// anything (a degraded emission, or an append-only epic): items are
/// inserted, refreshed and moved, and nothing absent from `items` is deleted.
/// The module cannot re-derive it, because what degrades an emission is the
/// script's stderr and exit status. `created_by` is stamped on every task
/// inserted, as `upsert_feed_tasks` documents.
#[spacetimedb::reducer]
pub fn ingest_feed(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: String,
) -> Result<(), String> {
    ingest_into(
        &mut ReducerStore(ctx),
        epic_id,
        items,
        additive,
        &created_by,
    )
}

/// The body of [`ingest_feed`], over any [`IngestStore`]: pick the strategy
/// from the epic's own row and run it.
pub fn ingest_into(
    store: &mut impl IngestStore,
    epic_id: i64,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    let Some(epic) = store.epic(epic_id) else {
        return Err(format!("epic {epic_id} not found for ingest_feed"));
    };
    match epic.feed_role.as_str() {
        ROLE_REVIEWS_PARENT => ingest_role_routed(store, &epic, items, additive, created_by),
        ROLE_MY_REVIEWS | ROLE_TEAM_REVIEWS | ROLE_BOTS => {
            Err(ROLE_SUB_EPIC_FEED_REFUSED.to_string())
        }
        _ if epic.group_by_repo => ingest_grouped(store, &epic, items, additive, created_by),
        _ => ingest_flat(store, &epic, items, additive, created_by),
    }
}

/// `FlatFeedReconcile`: heal any repo-group sub-epic left from a grouped past,
/// then upsert straight onto the epic.
fn ingest_flat(
    store: &mut impl IngestStore,
    epic: &Epic,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    let has_repo_group = store
        .child_epics(epic.id)
        .iter()
        .any(|e| e.origin == REPO_GROUP_ORIGIN);
    if has_repo_group {
        store.flatten_epic(epic.id)?;
    }
    if !additive {
        store.drop_closed_retired(epic.id, &external_ids(&items));
    }
    store.upsert_feed_tasks(epic.id, upsert_items(items), created_by, !additive)?;
    store.recalculate_epic_chain(epic.id);
    Ok(())
}

/// `GroupedFeedUpsert`: one sub-epic per repo name, titled by it.
fn ingest_grouped(
    store: &mut impl IngestStore,
    epic: &Epic,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    if !additive {
        store.drop_closed_retired(epic.id, &external_ids(&items));
    }
    let mut groups: Vec<(String, Vec<FeedTaskUpsertItem>)> = Vec::new();
    for entry in items {
        let name = repo_name_from_url(&entry.item.url);
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, group)) => group.push(entry.item),
            None => groups.push((name, vec![entry.item])),
        }
    }

    let subs: Vec<Epic> = store.child_epics(epic.id);
    let group_names: HashSet<&str> = groups.iter().map(|(n, _)| n.as_str()).collect();

    // A repo with no sub-epic yet earns one only if it still has an item the
    // retirement rule does not drop (feeds.allium: "Retired feed items").
    let candidates: Vec<&str> = groups
        .iter()
        .filter(|(name, _)| !subs.iter().any(|s| s.title == *name))
        .flat_map(|(_, group)| group.iter().map(|i| i.external_id.as_str()))
        .collect();
    let dropped = retired_without_task(store, epic.id, &candidates);

    let identity = store.identity();
    for (name, group) in &groups {
        let existing = subs.iter().find(|s| s.title == *name).map(|s| s.id);
        let group: Vec<FeedTaskUpsertItem> = match existing {
            Some(_) => group.clone(),
            None => group
                .iter()
                .filter(|i| !dropped.contains(i.external_id.as_str()))
                .cloned()
                .collect(),
        };
        if group.is_empty() {
            continue;
        }
        let sub_id = match existing {
            Some(id) => id,
            None => {
                let now = store.now();
                store.insert_epic(Epic {
                    title: name.clone(),
                    parent_epic_id: epic.id,
                    origin: REPO_GROUP_ORIGIN.to_string(),
                    created_by: identity.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                    ..blank_epic()
                })
            }
        };
        store.upsert_feed_tasks(sub_id, group, created_by, !additive)?;
        store.recalculate_epic_chain(sub_id);
    }

    if !additive {
        for sub in subs
            .iter()
            .filter(|s| !group_names.contains(s.title.as_str()))
        {
            store.upsert_feed_tasks(sub.id, Vec::new(), created_by, true)?;
            store.recalculate_epic_chain(sub.id);
        }
        store.upsert_feed_tasks(epic.id, Vec::new(), created_by, true)?;
        store.recalculate_epic_chain(epic.id);
    }
    Ok(())
}

/// The retired `external_ids` among `candidates` with no surviving task
/// anywhere under `feed_epic_id` (`feeds.allium`: "Retired feed items").
fn retired_without_task<'a>(
    store: &impl IngestStore,
    feed_epic_id: i64,
    candidates: &[&'a str],
) -> HashSet<&'a str> {
    if candidates.is_empty() {
        return HashSet::new();
    }
    let survivors: HashSet<String> = store
        .subtree_ids(feed_epic_id)
        .into_iter()
        .flat_map(|e| store.tasks_in_epic(e))
        .map(|t| t.external_id)
        .filter(|e| !e.is_empty())
        .collect();
    candidates
        .iter()
        .copied()
        .filter(|c| store.is_retired(feed_epic_id, c) && !survivors.contains(*c))
        .collect()
}

/// Delete stale feed tasks in every direct child epic of `parent`, keeping
/// only `keep`. Manual tasks (`external_id` empty) are always preserved.
fn delete_stale_in_children(store: &mut impl IngestStore, parent: i64, keep: &HashSet<&str>) {
    for child in store.child_epics(parent) {
        store.delete_stale_feed_tasks_in_epic(child.id, keep);
    }
}

/// The role sub-epic of `parent` carrying `role`, created when absent.
fn ensure_role_sub_epic(
    store: &mut impl IngestStore,
    parent: i64,
    role: &str,
    title: &str,
    identity: &str,
) -> i64 {
    let existing = store
        .child_epics(parent)
        .into_iter()
        .find(|e| e.feed_role == role);
    if let Some(sub) = existing {
        return sub.id;
    }
    let now = store.now();
    store.insert_epic(Epic {
        title: title.to_string(),
        parent_epic_id: parent,
        feed_role: role.to_string(),
        created_by: identity.to_string(),
        created_at: now.clone(),
        updated_at: now,
        ..blank_epic()
    })
}

/// Index `epic_id`'s feed tasks by `external_id` into `existing`.
fn remember_feed_tasks(
    store: &impl IngestStore,
    existing: &mut HashMap<String, Task>,
    epic_id: i64,
) {
    for task in store.tasks_in_epic(epic_id) {
        if !task.external_id.is_empty() {
            existing.insert(task.external_id.clone(), task);
        }
    }
}

/// What `route_and_move` decided: the items to upsert per target epic, the
/// external ids that stay, the repo groups it reached, and the new My Reviews
/// PRs to signal once inserted, by target epic.
struct RoutedItems {
    groups: HashMap<i64, Vec<FeedTaskUpsertItem>>,
    all_external_ids: HashSet<String>,
    repo_group_cache: HashMap<(i64, String), i64>,
    new_in_my_reviews: HashMap<i64, HashSet<String>>,
}

/// Index every feed task under the reviews `parent` by `external_id`, and
/// list the repo groups that already exist under the role sub-epics.
///
/// Task identity is global by external_id across the parent, the role
/// sub-epics and their repo-group children.
fn index_existing(
    store: &impl IngestStore,
    parent_id: i64,
    roles: &[i64; 3],
) -> (HashMap<String, Task>, Vec<i64>) {
    let mut existing: HashMap<String, Task> = HashMap::new();
    let mut pre_existing_repo_groups: Vec<i64> = Vec::new();
    remember_feed_tasks(store, &mut existing, parent_id);
    for &sub in roles {
        for child in store.child_epics(sub) {
            remember_feed_tasks(store, &mut existing, child.id);
            pre_existing_repo_groups.push(child.id);
        }
    }
    for &sub in roles {
        remember_feed_tasks(store, &mut existing, sub);
    }
    (existing, pre_existing_repo_groups)
}

/// Route each item to its role epic (and repo group), and move any already
/// known task there. Nothing is upserted or deleted here.
fn route_and_move(
    store: &mut impl IngestStore,
    items: &[FeedIngestItem],
    dropped: &HashSet<&str>,
    existing: &HashMap<String, Task>,
    roles: [i64; 3],
    identity: &str,
) -> RoutedItems {
    let [my, team, bots] = roles;
    let mut groups: HashMap<i64, Vec<FeedTaskUpsertItem>> = HashMap::new();
    let mut all_external_ids: HashSet<String> = HashSet::new();
    let mut repo_group_cache: HashMap<(i64, String), i64> = HashMap::new();
    let mut new_in_my_reviews: HashMap<i64, HashSet<String>> = HashMap::new();
    for entry in items {
        let item = &entry.item;
        if dropped.contains(item.external_id.as_str()) {
            continue;
        }
        let role_target = match route_signals(&entry.signals) {
            ROLE_TEAM_REVIEWS => team,
            ROLE_BOTS => bots,
            _ => my,
        };
        let groups_by_repo = store
            .epic(role_target)
            .is_some_and(|e| can_auto_group(e.group_by_repo, &e.feed_command));
        let target = if groups_by_repo {
            let name = repo_name_from_url(&item.url);
            *repo_group_cache
                .entry((role_target, name.clone()))
                .or_insert_with(|| {
                    store.find_or_create_repo_group(role_target, &name, identity.to_string())
                })
        } else {
            role_target
        };

        all_external_ids.insert(item.external_id.clone());

        // A move touches only epic_id and updated_at, so status, sub_status,
        // worktree, tmux_window and sort_order survive; the upsert that
        // follows refreshes the feed-owned fields.
        if let Some(task) = existing.get(&item.external_id) {
            if task.epic_id != target {
                let updated_at = store.now();
                store.update_task(Task {
                    epic_id: target,
                    updated_at,
                    ..task.clone()
                });
            }
        } else if role_target == my {
            new_in_my_reviews
                .entry(target)
                .or_default()
                .insert(item.external_id.clone());
        }
        groups.entry(target).or_default().push(item.clone());
    }
    RoutedItems {
        groups,
        all_external_ids,
        repo_group_cache,
        new_in_my_reviews,
    }
}

/// `RoleRoutedFeedSync`: reconcile the whole `reviews-parent` subtree from one
/// emission. Phase order matters and is the board's old order: every
/// cross-role move lands before any delete, so a moved task is never deleted.
fn ingest_role_routed(
    store: &mut impl IngestStore,
    parent: &Epic,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    let identity = store.identity();
    let my = ensure_role_sub_epic(store, parent.id, ROLE_MY_REVIEWS, "My Reviews", &identity);
    let team = ensure_role_sub_epic(
        store,
        parent.id,
        ROLE_TEAM_REVIEWS,
        "Team Reviews",
        &identity,
    );
    let bots = ensure_role_sub_epic(store, parent.id, ROLE_BOTS, "Bots", &identity);
    let roles = [my, team, bots];

    let (existing, pre_existing_repo_groups) = index_existing(store, parent.id, &roles);

    // Read from the FULL emission, before the exclusion filter: an excluded
    // but still open PR keeps its retired record.
    if !additive {
        store.drop_closed_retired(parent.id, &external_ids(&items));
    }

    let items: Vec<FeedIngestItem> = items
        .into_iter()
        .filter(|i| !excluded_from_reviews(&i.signals))
        .collect();

    let candidates: Vec<&str> = items.iter().map(|i| i.item.external_id.as_str()).collect();
    let dropped = retired_without_task(store, parent.id, &candidates);

    let routed = route_and_move(store, &items, &dropped, &existing, roles, &identity);
    let RoutedItems {
        groups,
        all_external_ids,
        repo_group_cache,
        new_in_my_reviews,
    } = routed;

    for (sub, group) in groups {
        store.upsert_feed_tasks(sub, group, created_by, !additive)?;
    }
    // `feeds.allium: NewMyReviewsTaskIsSignalled`: an insert into My Reviews
    // is a signalled create. A refresh or a move into it is not.
    // A retired PR is never inserted, so it finds no row here and no mark.
    for (target, new_ids) in new_in_my_reviews {
        for task in store.tasks_in_epic(target) {
            if new_ids.contains(&task.external_id) {
                store.mark_task_unseen_for_followers(task.id, target);
            }
        }
    }
    if !additive {
        let keep: HashSet<&str> = all_external_ids.iter().map(String::as_str).collect();
        delete_stale_in_children(store, parent.id, &keep);
        for &sub in &roles {
            delete_stale_in_children(store, sub, &keep);
        }
        // A reviews parent holds no feed task directly: whatever is left on it
        // is a merged stray or a duplicate of a routed copy.
        store.delete_stale_feed_tasks_in_epic(parent.id, &HashSet::new());
    }

    let repo_groups: HashSet<i64> = repo_group_cache
        .into_values()
        .chain(pre_existing_repo_groups)
        .collect();
    for id in repo_groups {
        store.recalculate_epic_chain(id);
    }
    for &sub in &roles {
        store.recalculate_epic_chain(sub);
    }
    store.recalculate_epic_chain(parent.id);
    Ok(())
}
