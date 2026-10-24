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
        let epic = ctx.db.epics().id().find(next)?;
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
// `ingest_feed` recalculates the epic chain itself, in the same transaction.

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
/// wrap_up_mode, model, completed_at — user-managed or store-owned fields a re-poll
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
/// `feed_ingest.rs`'s per-child-epic loop.
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

/// The upsert body `ingest_feed` runs for each feed epic.
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

/// feeds.allium: `DropClosedRetiredFeedItems`. Drop every `retired_feed_items`
/// row keyed on `feed_epic_id` whose `external_id` is absent from `keep` —
/// run by `ingest_feed` only after a TRUSTED (mirroring, non-additive) cycle
/// whose parsed emission no longer carries the id, so the upstream item
/// closed and a later reopen shows up as new.
pub(crate) fn drop_closed_retired(
    ctx: &ReducerContext,
    feed_epic_id: i64,
    keep: &std::collections::HashSet<&str>,
) {
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

/// `ProvisionManagedEpics` (M7, X4): the managed feed-epic tree in ONE
/// transaction, so two boards starting together cannot create duplicates.
///
/// Each subtree only when its command is non-empty (`""` is the module's
/// absent: that feed is disabled, nothing is created or removed). An epic is
/// found by `(feed_role, parent)`, never by title, so a user's rename
/// survives; a found command-carrying epic has only its `feed_command` and
/// `feed_interval_secs` reconciled (`0` is no interval), a missing one is
/// created with `feed_role` set from the start. The three review sub-epics
/// carry no command. Ports `src/service/managed_feeds.rs::ensure_managed_epics`.
#[spacetimedb::reducer]
pub fn ensure_managed_epics(
    ctx: &ReducerContext,
    reviews_command: String,
    reviews_interval_secs: i64,
    cve_command: String,
    cve_interval_secs: i64,
) -> Result<(), String> {
    let identity = caller_identity(ctx);
    if !reviews_command.is_empty() {
        let parent = ensure_role_epic(
            ctx,
            ROLE_REVIEWS_PARENT,
            0,
            Some((&reviews_command, reviews_interval_secs)),
            &identity,
        );
        for role in [ROLE_MY_REVIEWS, ROLE_TEAM_REVIEWS, ROLE_BOTS] {
            ensure_role_epic(ctx, role, parent, None, &identity);
        }
    }
    if !cve_command.is_empty() {
        ensure_role_epic(
            ctx,
            ROLE_CVE,
            0,
            Some((&cve_command, cve_interval_secs)),
            &identity,
        );
    }
    Ok(())
}

/// Default display title of a freshly created managed epic, consulted only on
/// creation. Agrees with `ensure_role_sub_epic`'s titles in `feed_ingest.rs`.
fn managed_role_title(role: &str) -> &'static str {
    match role {
        ROLE_REVIEWS_PARENT => "PR Reviews",
        ROLE_MY_REVIEWS => "My Reviews",
        ROLE_TEAM_REVIEWS => "Team Reviews",
        ROLE_BOTS => "Bots",
        ROLE_CVE => "CVE",
        _ => "Reviews",
    }
}

/// One managed epic: found by `(role, parent)` and left alone but for the
/// command and interval of a command-carrying role, else created. Its id.
fn ensure_role_epic(
    ctx: &ReducerContext,
    role: &str,
    parent_epic_id: i64,
    command: Option<(&str, i64)>,
    created_by: &str,
) -> i64 {
    let existing = ctx
        .db
        .epics()
        .parent_epic_id()
        .filter(&parent_epic_id)
        .find(|e| e.feed_role == role);
    if let Some(epic) = existing {
        if let Some((cmd, interval)) = command {
            if epic.feed_command != cmd || epic.feed_interval_secs != interval {
                let id = epic.id;
                ctx.db.epics().id().update(Epic {
                    feed_command: cmd.to_string(),
                    feed_interval_secs: interval,
                    updated_at: now(ctx),
                    ..epic
                });
                return id;
            }
        }
        return epic.id;
    }
    let (feed_command, feed_interval_secs) =
        command.map(|(c, i)| (c.to_string(), i)).unwrap_or_default();
    ctx.db
        .epics()
        .insert(Epic {
            title: managed_role_title(role).to_string(),
            parent_epic_id,
            feed_role: role.to_string(),
            feed_command,
            feed_interval_secs,
            created_by: created_by.to_string(),
            created_at: now(ctx),
            updated_at: now(ctx),
            ..blank_epic()
        })
        .id
}
