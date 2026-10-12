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
    let Some(epic) = ctx.db.epics().id().find(epic_id) else {
        return Err(format!("epic {epic_id} not found for ingest_feed"));
    };
    match epic.feed_role.as_str() {
        ROLE_REVIEWS_PARENT => ingest_role_routed(ctx, &epic, items, additive, &created_by),
        ROLE_MY_REVIEWS | ROLE_TEAM_REVIEWS | ROLE_BOTS => {
            Err(ROLE_SUB_EPIC_FEED_REFUSED.to_string())
        }
        _ if epic.group_by_repo => ingest_grouped(ctx, &epic, items, additive, &created_by),
        _ => ingest_flat(ctx, &epic, items, additive, &created_by),
    }
}

/// `FlatFeedReconcile`: heal any repo-group sub-epic left from a grouped past,
/// then upsert straight onto the epic.
fn ingest_flat(
    ctx: &ReducerContext,
    epic: &Epic,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    let has_repo_group = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&epic.id)
        .any(|e| e.origin == REPO_GROUP_ORIGIN);
    if has_repo_group {
        flatten_epic(ctx, epic.id)?;
    }
    if !additive {
        drop_closed_retired(ctx, epic.id, &external_ids(&items));
    }
    upsert_feed_tasks_inner(ctx, epic.id, upsert_items(items), created_by, !additive)?;
    recalculate_epic_chain(ctx, epic.id);
    Ok(())
}

/// `GroupedFeedUpsert`: one sub-epic per repo name, titled by it.
fn ingest_grouped(
    ctx: &ReducerContext,
    epic: &Epic,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    if !additive {
        drop_closed_retired(ctx, epic.id, &external_ids(&items));
    }
    let mut groups: Vec<(String, Vec<FeedTaskUpsertItem>)> = Vec::new();
    for entry in items {
        let name = repo_name_from_url(&entry.item.url);
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, group)) => group.push(entry.item),
            None => groups.push((name, vec![entry.item])),
        }
    }

    let subs: Vec<Epic> = ctx.db.epics().parent_epic_id().filter(&epic.id).collect();
    let group_names: HashSet<&str> = groups.iter().map(|(n, _)| n.as_str()).collect();

    // A repo with no sub-epic yet earns one only if it still has an item the
    // retirement rule does not drop (feeds.allium: "Retired feed items").
    let candidates: Vec<&str> = groups
        .iter()
        .filter(|(name, _)| !subs.iter().any(|s| s.title == *name))
        .flat_map(|(_, group)| group.iter().map(|i| i.external_id.as_str()))
        .collect();
    let dropped = retired_without_task(ctx, epic.id, &candidates);

    let identity = caller_identity(ctx);
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
        let sub_id = existing.unwrap_or_else(|| {
            ctx.db
                .epics()
                .insert(Epic {
                    title: name.clone(),
                    parent_epic_id: epic.id,
                    created_by: identity.clone(),
                    created_at: now(ctx),
                    updated_at: now(ctx),
                    ..blank_epic()
                })
                .id
        });
        upsert_feed_tasks_inner(ctx, sub_id, group, created_by, !additive)?;
        recalculate_epic_chain(ctx, sub_id);
    }

    if !additive {
        for sub in subs
            .iter()
            .filter(|s| !group_names.contains(s.title.as_str()))
        {
            upsert_feed_tasks_inner(ctx, sub.id, Vec::new(), created_by, true)?;
            recalculate_epic_chain(ctx, sub.id);
        }
        upsert_feed_tasks_inner(ctx, epic.id, Vec::new(), created_by, true)?;
        recalculate_epic_chain(ctx, epic.id);
    }
    Ok(())
}

/// The retired `external_ids` among `candidates` with no surviving task
/// anywhere under `feed_epic_id` (`feeds.allium`: "Retired feed items").
fn retired_without_task<'a>(
    ctx: &ReducerContext,
    feed_epic_id: i64,
    candidates: &[&'a str],
) -> HashSet<&'a str> {
    if candidates.is_empty() {
        return HashSet::new();
    }
    let subtree = collect_epic_subtree_ids(ctx, feed_epic_id);
    let survivors: HashSet<String> = subtree
        .iter()
        .flat_map(|&e| ctx.db.tasks().epic_id().filter(&e).collect::<Vec<_>>())
        .map(|t| t.external_id)
        .filter(|e| !e.is_empty())
        .collect();
    candidates
        .iter()
        .copied()
        .filter(|c| is_retired(ctx, feed_epic_id, c) && !survivors.contains(*c))
        .collect()
}

/// The role sub-epic of `parent` carrying `role`, created when absent.
fn ensure_role_sub_epic(
    ctx: &ReducerContext,
    parent: i64,
    role: &str,
    title: &str,
    identity: &str,
) -> i64 {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent)
        .find(|e| e.feed_role == role);
    if let Some(sub) = existing {
        return sub.id;
    }
    ctx.db
        .epics()
        .insert(Epic {
            title: title.to_string(),
            parent_epic_id: parent,
            feed_role: role.to_string(),
            created_by: identity.to_string(),
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        })
        .id
}

/// `RoleRoutedFeedSync`: reconcile the whole `reviews-parent` subtree from one
/// emission. Phase order matters and is the board's old order: every
/// cross-role move lands before any delete, so a moved task is never deleted.
fn ingest_role_routed(
    ctx: &ReducerContext,
    parent: &Epic,
    items: Vec<FeedIngestItem>,
    additive: bool,
    created_by: &str,
) -> Result<(), String> {
    let identity = caller_identity(ctx);
    let my = ensure_role_sub_epic(ctx, parent.id, ROLE_MY_REVIEWS, "My Reviews", &identity);
    let team = ensure_role_sub_epic(ctx, parent.id, ROLE_TEAM_REVIEWS, "Team Reviews", &identity);
    let bots = ensure_role_sub_epic(ctx, parent.id, ROLE_BOTS, "Bots", &identity);
    let roles = [my, team, bots];

    // Task identity is global by external_id across the parent, the role
    // sub-epics and their repo-group children.
    let mut existing: HashMap<String, Task> = HashMap::new();
    let mut pre_existing_repo_groups: Vec<i64> = Vec::new();
    let mut remember = |ctx: &ReducerContext, epic_id: i64| {
        for task in ctx.db.tasks().epic_id().filter(&epic_id) {
            if !task.external_id.is_empty() {
                existing.insert(task.external_id.clone(), task);
            }
        }
    };
    remember(ctx, parent.id);
    for &sub in &roles {
        for child in ctx.db.epics().parent_epic_id().filter(&sub) {
            remember(ctx, child.id);
            pre_existing_repo_groups.push(child.id);
        }
    }
    for &sub in &roles {
        remember(ctx, sub);
    }

    // Read from the FULL emission, before the exclusion filter: an excluded
    // but still open PR keeps its retired record.
    if !additive {
        drop_closed_retired(ctx, parent.id, &external_ids(&items));
    }

    let items: Vec<FeedIngestItem> = items
        .into_iter()
        .filter(|i| !excluded_from_reviews(&i.signals))
        .collect();

    let candidates: Vec<&str> = items.iter().map(|i| i.item.external_id.as_str()).collect();
    let dropped = retired_without_task(ctx, parent.id, &candidates);

    let mut groups: HashMap<i64, Vec<FeedTaskUpsertItem>> = HashMap::new();
    let mut all_external_ids: HashSet<String> = HashSet::new();
    let mut repo_group_cache: HashMap<(i64, String), i64> = HashMap::new();
    for entry in &items {
        let item = &entry.item;
        if dropped.contains(item.external_id.as_str()) {
            continue;
        }
        let role_target = match route_signals(&entry.signals) {
            ROLE_TEAM_REVIEWS => team,
            ROLE_BOTS => bots,
            _ => my,
        };
        let groups_by_repo = ctx
            .db
            .epics()
            .id()
            .find(role_target)
            .is_some_and(|e| can_auto_group(e.group_by_repo, &e.feed_command));
        let target = if groups_by_repo {
            let name = repo_name_from_url(&item.url);
            *repo_group_cache
                .entry((role_target, name.clone()))
                .or_insert_with(|| {
                    find_or_create_repo_group(ctx, role_target, &name, identity.clone())
                })
        } else {
            role_target
        };

        all_external_ids.insert(item.external_id.clone());

        // A move touches only epic_id and updated_at, so status, sub_status,
        // worktree, tmux_window and sort_order survive; the upsert below then
        // refreshes the feed-owned fields.
        if let Some(task) = existing.get(&item.external_id) {
            if task.epic_id != target {
                ctx.db.tasks().id().update(Task {
                    epic_id: target,
                    updated_at: now(ctx),
                    ..task.clone()
                });
            }
        }
        groups.entry(target).or_default().push(item.clone());
    }

    for (sub, group) in groups {
        upsert_feed_tasks_inner(ctx, sub, group, created_by, !additive)?;
    }
    if !additive {
        let keep: HashSet<&str> = all_external_ids.iter().map(String::as_str).collect();
        delete_stale_in_children(ctx, parent.id, &keep);
        for &sub in &roles {
            delete_stale_in_children(ctx, sub, &keep);
        }
        // A reviews parent holds no feed task directly: whatever is left on it
        // is a merged stray or a duplicate of a routed copy.
        delete_stale_feed_tasks_in_epic(ctx, parent.id, &HashSet::new());
    }

    let repo_groups: HashSet<i64> = repo_group_cache
        .into_values()
        .chain(pre_existing_repo_groups)
        .collect();
    for id in repo_groups {
        recalculate_epic_chain(ctx, id);
    }
    for &sub in &roles {
        recalculate_epic_chain(ctx, sub);
    }
    recalculate_epic_chain(ctx, parent.id);
    Ok(())
}
