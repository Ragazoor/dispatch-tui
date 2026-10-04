//! Feed ingestion, retired feed items and feed-managed epics.

use super::*;

/// Walk `epic_id`'s ancestry, itself first, for the nearest epic carrying a
/// `feed_command` — the same key `delete_task`, `delete_epic`'s retirement
/// pass and `upsert_feed_tasks_inner` all resolve against. Mirrors
/// `src/db/queries/tasks.rs`'s recursive `nearest_feed_epic` CTE
/// (`core/Epic.nearest_feed_epic`).
pub(crate) fn nearest_feed_epic(ctx: &ReducerContext, epic_id: i64) -> Option<i64> {
    let mut next = epic_id;
    for _ in 0..MAX_EPIC_DEPTH {
        if next == 0 {
            return None;
        }
        let Some(epic) = ctx.db.epics().id().find(next) else {
            return None;
        };
        if !epic.feed_command.is_empty() {
            return Some(epic.id);
        }
        next = epic.parent_epic_id;
    }
    None
}

/// Is `external_id` already retired under `feed_epic_id`? Shared by
/// `retire_feed_item` (whose idempotent-insert check this is) and
/// `upsert_feed_item` (`feeds.allium: IngestSkipsRetiredFeedItems`'s refusal
/// check) — both independently ran this identical query before.
pub(crate) fn is_retired(ctx: &ReducerContext, feed_epic_id: i64, external_id: &str) -> bool {
    ctx.db
        .retired_feed_items()
        .feed_epic_id()
        .filter(&feed_epic_id)
        .any(|r| r.external_id == external_id)
}

/// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed` as an idempotent
/// insert — a second retirement of the same (feed_epic_id, external_id) is a
/// no-op, mirroring SQLite's `INSERT OR IGNORE`.
pub(crate) fn retire_feed_item(ctx: &ReducerContext, feed_epic_id: i64, external_id: &str) {
    if !is_retired(ctx, feed_epic_id, external_id) {
        ctx.db.retired_feed_items().insert(RetiredFeedItem {
            id: 0,
            feed_epic_id,
            external_id: external_id.to_string(),
            retired_at: now(ctx),
        });
    }
}

/// tasks.allium: `DeleteTask`'s retirement clause. A manual task (no
/// `external_id`) or one under no feed epic in its chain retires nothing —
/// there is no cycle to suppress. Shared by `delete_task` and
/// `batch_delete`'s plain-task pass, so a second, hand-copied version of this
/// exact check is not how the two drift apart — see `delete_task_side_effects`'s
/// doc comment for why that drift is a named prior incident here, not a
/// hypothetical one.
pub(crate) fn retire_task_if_feed_backed(ctx: &ReducerContext, epic_id: i64, external_id: &str) {
    if external_id.is_empty() || epic_id == 0 {
        return;
    }
    if let Some(feed_epic_id) = nearest_feed_epic(ctx, epic_id) {
        retire_feed_item(ctx, feed_epic_id, external_id);
    }
}

// -- Feed ingestion -----------------------------------------------------------
//
// Feed SCRIPT EXECUTION stays local — a SpacetimeDB module has no subprocess
// capability (feeds.allium). What lands here is the upsert of the rows the
// script produced: the CLIENT resolves every domain default a feed item needs
// (`sub_status` from `SubStatus::default_for(item.status)`, the inferred
// `url_type`) before sending, the same boundary `create_task`/`create_epic`
// already draw — a reducer takes a fully-formed row, not a partial one it has
// to complete. See [`FeedTaskUpsertItem`].
//
// No internal recalculation: every call site in `src/feed/` already calls the
// already-routed `recalculate_epic_status` explicitly afterward
// (`recalculate_epic_status_after_feed`), so duplicating it here would only be
// a second, harmless, idempotent call with no caller that needs it.

/// One feed item, already resolved to the fields a task row needs. Not a
/// [`Task`]: a feed item never carries `status`/`sub_status`/`repo_path`/
/// `base_branch`/`wrap_up_mode` on an UPDATE (those are USER-managed fields
/// preserved across a re-poll — see [`upsert_feed_item`]), so shaping this as
/// a full `Task` would invite a caller to believe setting one of those on an
/// existing row does something. It does not, by construction.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct FeedTaskUpsertItem {
    pub external_id: String,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: String,
    pub sub_status: String,
    pub base_branch: String,
    pub tag: String,
    pub labels: String,
    pub sort_order: Option<i64>,
    pub url: String,
    pub url_type: String,
    pub wrap_up_mode: String,
}

/// Insert or update one feed item under `epic_id`, preserving the same fields
/// the SQLite `ON CONFLICT DO UPDATE SET` preserves and updating the same
/// ones it updates.
///
/// **Updated on conflict:** title, description, tag, labels, sort_order, and
/// url/url_type — but only when the existing row has no url yet; an existing
/// non-null url always wins, so a feed cannot blank out or replace a url a
/// user (or an earlier, richer emission) already set.
///
/// **Preserved on conflict:** status, sub_status, repo_path, base_branch,
/// wrap_up_mode, completed_at — user-managed or store-owned fields a re-poll
/// must not disturb. `status`/`sub_status` not moving is what makes a re-poll
/// not a completion; `completed_at` not moving is a direct consequence — see
/// [`write_task`]'s own stamp-on-insert logic, which this reuses for the
/// INSERT branch instead of re-deriving it.
///
/// Looked up by `(epic_id, external_id)` via a scan of `epic_id`'s index —
/// the module carries no partial unique index over the pair the way SQLite's
/// `ON CONFLICT(epic_id, external_id) WHERE external_id IS NOT NULL` target
/// does, and needs none: reducers run one at a time, so there is no second
/// call to race with a check-then-act sequence.
///
/// `feed_epic_id` is `nearest_feed_epic(epic_id)`, resolved once by the caller
/// and reused for every item in the batch (`feeds.allium:
/// IngestSkipsRetiredFeedItems`): a retired `external_id` with no existing
/// survivor row is refused outright; an existing survivor (e.g. the
/// worktree-holding case `ArchivedStatusMigration` left in done) still matches
/// above and is refreshed regardless of retirement.
pub(crate) fn upsert_feed_item(
    ctx: &ReducerContext,
    epic_id: i64,
    feed_epic_id: Option<i64>,
    item: &FeedTaskUpsertItem,
    created_by: &str,
) {
    let existing = ctx
        .db
        .tasks()
        .epic_id()
        .filter(&epic_id)
        .find(|t| t.external_id == item.external_id);

    if existing.is_none() {
        if let Some(feed_epic_id) = feed_epic_id {
            if is_retired(ctx, feed_epic_id, &item.external_id) {
                return;
            }
        }
    }

    let row = match existing {
        Some(existing) => {
            let (url, url_type) = if existing.url.is_empty() {
                (item.url.clone(), item.url_type.clone())
            } else {
                (existing.url.clone(), existing.url_type.clone())
            };
            Task {
                title: item.title.clone(),
                description: item.description.clone(),
                tag: item.tag.clone(),
                labels: item.labels.clone(),
                sort_order: item.sort_order,
                url,
                url_type,
                updated_at: now(ctx),
                ..existing
            }
        }
        None => Task {
            id: 0,
            title: item.title.clone(),
            description: item.description.clone(),
            repo_path: item.repo_path.clone(),
            status: item.status.clone(),
            sub_status: item.sub_status.clone(),
            base_branch: item.base_branch.clone(),
            epic_id,
            external_id: item.external_id.clone(),
            tag: item.tag.clone(),
            labels: item.labels.clone(),
            sort_order: item.sort_order,
            url: item.url.clone(),
            url_type: item.url_type.clone(),
            wrap_up_mode: item.wrap_up_mode.clone(),
            created_at: now(ctx),
            updated_at: now(ctx),
            // Overrides `blank_task()`'s SCRATCH_OWNER: a feed task always
            // has an epic, so `validate_task_ownership` requires an ABSENT
            // owner here, not a non-empty sentinel.
            owner: String::new(),
            // Which host's feed run produced this row (`core.allium:
            // Task.created_by`, `feeds.allium: UpsertFeedTasks`). Insert
            // only, matching `created_by`'s "stamped once, never rewritten"
            // rule elsewhere — the update branch above never touches it.
            created_by: created_by.to_string(),
            ..blank_task()
        },
    };
    // write_task's stamp-on-Done-insert logic covers the INSERT branch; on
    // the UPDATE branch `status`/`completed_at` are carried over unchanged
    // from `existing`, so the condition cannot fire there.
    let _ = write_task(ctx, row);
}

/// Delete every task in `epic_id` whose `external_id` is set and not in
/// `keep`. Shared by [`upsert_feed_tasks_inner`]'s single-epic pass and
/// [`delete_stale_subtree_feed_tasks`]'s per-child-epic loop.
///
/// Owes each removed row [`delete_task_side_effects`] the same as any other
/// task removal — see that function's doc comment.
pub(crate) fn delete_stale_feed_tasks_in_epic(
    ctx: &ReducerContext,
    epic_id: i64,
    keep: &std::collections::HashSet<&str>,
) {
    let stale: Vec<i64> = ctx
        .db
        .tasks()
        .epic_id()
        .filter(&epic_id)
        .filter(|t| !t.external_id.is_empty() && !keep.contains(t.external_id.as_str()))
        .map(|t| t.id)
        .collect();
    for id in stale {
        ctx.db.tasks().id().delete(id);
        delete_task_side_effects(ctx, id);
    }
}

/// Shared body of [`upsert_feed_tasks`] and [`upsert_feed_tasks_additive`].
/// `delete_absent` selects the stale-delete pass the same way
/// `src/db/queries/tasks.rs::upsert_feed_tasks_inner` does.
pub(crate) fn upsert_feed_tasks_inner(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedTaskUpsertItem>,
    created_by: &str,
    delete_absent: bool,
) -> Result<(), String> {
    if ctx.db.epics().id().find(epic_id).is_none() {
        return Err(format!("epic {epic_id} not found for upsert_feed_tasks"));
    }
    // feeds.allium: IngestSkipsRetiredFeedItems. One resolution, reused for
    // every item — mirrors src/db/queries/tasks.rs::upsert_feed_tasks_inner's
    // single `nearest_feed_epic` query.
    let feed_epic_id = nearest_feed_epic(ctx, epic_id);
    for item in &items {
        upsert_feed_item(ctx, epic_id, feed_epic_id, item, created_by);
    }
    if delete_absent {
        let keep: std::collections::HashSet<&str> =
            items.iter().map(|i| i.external_id.as_str()).collect();
        delete_stale_feed_tasks_in_epic(ctx, epic_id, &keep);
    }
    Ok(())
}

/// Upsert tasks from a feed, reconciling: every stale feed task in `epic_id`
/// absent from `items` is removed. `feeds.allium: UpsertFeedTasks`.
///
/// `created_by` names the calling host's own owner identity
/// (`local_host().owner`) — stamped on every newly INSERTED task, never on an
/// update. May be empty: an install that has never connected to a shared
/// store has no identity to stamp, and that is a real, honest state rather
/// than a call this reducer refuses (`core.allium: Task.created_by`).
#[spacetimedb::reducer]
pub fn upsert_feed_tasks(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedTaskUpsertItem>,
    created_by: String,
) -> Result<(), String> {
    upsert_feed_tasks_inner(ctx, epic_id, items, &created_by, true)
}

/// The insert/update half of [`upsert_feed_tasks`] WITHOUT its stale-delete
/// pass — items absent from `items` are left alone. For a partially degraded
/// emission whose omissions are not trustworthy evidence a task is gone
/// (`feeds.allium: DegradedNonEmptyEmission`). `created_by` as
/// [`upsert_feed_tasks`] documents.
#[spacetimedb::reducer]
pub fn upsert_feed_tasks_additive(
    ctx: &ReducerContext,
    epic_id: i64,
    items: Vec<FeedTaskUpsertItem>,
    created_by: String,
) -> Result<(), String> {
    upsert_feed_tasks_inner(ctx, epic_id, items, &created_by, false)
}

/// Delete stale feed tasks across the WHOLE subtree of `parent_id` (every
/// direct child epic), keeping only `keep_external_ids`. Manual tasks
/// (`external_id` empty) are always preserved.
#[spacetimedb::reducer]
pub fn delete_stale_subtree_feed_tasks(
    ctx: &ReducerContext,
    parent_id: i64,
    keep_external_ids: Vec<String>,
) -> Result<(), String> {
    let keep: std::collections::HashSet<&str> =
        keep_external_ids.iter().map(String::as_str).collect();
    let child_epics: Vec<i64> = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_id)
        .map(|e| e.id)
        .collect();
    for epic_id in child_epics {
        delete_stale_feed_tasks_in_epic(ctx, epic_id, &keep);
    }
    Ok(())
}

/// feeds.allium: `DropClosedRetiredFeedItems`. Drop every `retired_feed_items`
/// row keyed on `feed_epic_id` whose `external_id` is absent from
/// `keep_external_ids` — called only after a TRUSTED (mirroring, non-additive)
/// cycle whose parsed emission no longer carries the id, so the upstream item
/// closed and a later reopen shows up as new.
#[spacetimedb::reducer]
pub fn drop_closed_retired_feed_items(
    ctx: &ReducerContext,
    feed_epic_id: i64,
    keep_external_ids: Vec<String>,
) -> Result<(), String> {
    let keep: std::collections::HashSet<&str> =
        keep_external_ids.iter().map(String::as_str).collect();
    let stale: Vec<i64> = ctx
        .db
        .retired_feed_items()
        .feed_epic_id()
        .filter(&feed_epic_id)
        .filter(|r| !keep.contains(r.external_id.as_str()))
        .map(|r| r.id)
        .collect();
    for id in stale {
        ctx.db.retired_feed_items().id().delete(id);
    }
    Ok(())
}

/// epics.allium: `DeleteEpic`'s retirement clause. For every feed task
/// anywhere in `doomed`'s subtree, write a `retired_feed_items` row keyed on
/// its `nearest_feed_epic` — UNLESS that epic is itself part of `doomed`, in
/// which case there is nothing surviving to retire under (the feed epic's own
/// delete is a reset, and its existing records are dropped by [`delete_epic`]
/// instead). Must run before the subtree's tasks/epics are actually deleted.
pub(crate) fn retire_feed_tasks_before_epic_delete(
    ctx: &ReducerContext,
    doomed: &std::collections::HashSet<i64>,
) {
    let tasks: Vec<(i64, String)> = doomed
        .iter()
        .flat_map(|&epic_id| {
            ctx.db
                .tasks()
                .epic_id()
                .filter(&epic_id)
                .filter(|t| !t.external_id.is_empty())
                .map(|t| (t.epic_id, t.external_id))
                .collect::<Vec<_>>()
        })
        .collect();
    // Many doomed tasks share the same epic_id, so cache nearest_feed_epic per
    // distinct epic_id rather than re-resolving it once per task — mirrors
    // src/db/queries/epics.rs::retire_feed_tasks_before_epic_delete's cache.
    let mut nearest_feed_epic_cache: std::collections::HashMap<i64, Option<i64>> =
        std::collections::HashMap::new();
    for (epic_id, external_id) in tasks {
        let feed_epic_id = *nearest_feed_epic_cache
            .entry(epic_id)
            .or_insert_with(|| nearest_feed_epic(ctx, epic_id));
        let Some(feed_epic_id) = feed_epic_id else {
            continue;
        };
        if doomed.contains(&feed_epic_id) {
            continue;
        }
        retire_feed_item(ctx, feed_epic_id, &external_id);
    }
}

/// Drop every stale `retired_feed_items` row for each epic id in `doomed`:
/// deleting a feed epic is a reset, not a prune, and SQLite's `ON DELETE
/// CASCADE` has no module-side equivalent to do this for free. Shared by
/// [`delete_epic`] and `batch_delete`'s epic pass.
pub(crate) fn drop_retired_feed_items_for_epics(
    ctx: &ReducerContext,
    doomed: &std::collections::HashSet<i64>,
) {
    for &doomed_id in doomed {
        let stale: Vec<i64> = ctx
            .db
            .retired_feed_items()
            .feed_epic_id()
            .filter(&doomed_id)
            .map(|r| r.id)
            .collect();
        for retired_id in stale {
            ctx.db.retired_feed_items().id().delete(retired_id);
        }
    }
}

/// Find-or-create the `RepoGroup` sub-epic of `parent_id` titled `title`.
/// Mirrors `src/db/queries/epics.rs::create_repo_group_sub_epic`, minus the
/// unique-constraint retry arm that mirror needs and this does not: SQLite
/// has concurrent writers and a partial unique index to referee them; a
/// SpacetimeDB reducer runs to completion before the next one starts, so the
/// look-up this function does IS the whole safety argument — there is no
/// window for a second caller's insert to land between it and this one's.
///
/// **`created_by` is not optional here.** An epic has no `owner` field at
/// all, and the per-epic subscription (`WHERE id = {epic}`) covers only that
/// epic's own row and its tasks — never a NEW CHILD epic's row, whatever the
/// child's parent. `own_creations` (`WHERE created_by = {identity}`) is the
/// only subscription that can ever make a freshly created or freshly found
/// sub-epic visible to the caller that needs its id back — see
/// `create_epic`'s `matches_created_epic` precedent, which this follows.
/// `created_by` is written only on a genuine insert; the found-existing arm
/// leaves a prior creator's stamp untouched, same as
/// `LocalHostOwnerIsWrittenOnce`-shaped fields elsewhere in this module never
/// get silently reassigned to whoever asked most recently.
///
/// There is no archived state for a found epic to be unarchived out of any
/// more, so a match is simply left alone and the only other case is a fresh
/// insert. Unlike a managed epic (`epics.allium`'s `ProvisionManagedEpics`
/// guidance on rename-stability), this reducer is NOT rename-stable: it
/// matches on `(parent_id, title)`, same as
/// `src/db/queries/epics.rs::create_repo_group_sub_epic`, so a user rename of
/// a repo-group sub-epic makes the next grouped cycle create a new one.
#[spacetimedb::reducer]
pub fn create_repo_group_sub_epic(
    ctx: &ReducerContext,
    parent_id: i64,
    title: String,
    created_by: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_id)
        .find(|e| e.title == title && e.origin == "repo-group");
    if existing.is_none() {
        ctx.db.epics().insert(Epic {
            title,
            parent_epic_id: parent_id,
            origin: "repo-group".to_string(),
            created_by,
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        });
    }
    Ok(())
}

/// Create-or-return a managed-feed-role epic, `feed_role` set from the start.
/// Mirrors `src/db/queries/epics.rs::create_managed_role_epic` on the same
/// terms [`create_repo_group_sub_epic`] documents: race-free by construction,
/// no unique-index retry arm needed, `created_by` required for the same
/// `own_creations` reason.
#[spacetimedb::reducer]
pub fn create_managed_role_epic(
    ctx: &ReducerContext,
    title: String,
    parent_epic_id: i64,
    role: String,
    feed_command: String,
    feed_interval_secs: i64,
    created_by: String,
) -> Result<(), String> {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_epic_id)
        .find(|e| e.feed_role == role);
    if existing.is_none() {
        ctx.db.epics().insert(Epic {
            title,
            parent_epic_id,
            feed_role: role,
            feed_command,
            feed_interval_secs,
            // `origin` stays at `blank_epic()`'s "manual" default, unlisted
            // here on purpose: the SQLite INSERT this mirrors
            // (`src/db/queries/epics.rs::create_managed_role_epic`) never sets
            // it either, so a managed-role epic's origin is "manual" today —
            // only `create_repo_group_sub_epic` above writes "repo-group",
            // and nothing anywhere reads "managed" as an origin value.
            created_by,
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        });
    }
    Ok(())
}
