//! Repo configuration, settings, subscriptions and usage events.

use super::*;

// -- Repo configuration -----------------------------------------------------

/// Record a repo path, or touch the one already there.
///
/// Upsert by path rather than by id, because the path is what the domain calls
/// the same repo. Two rows for one path would give the verify command two
/// answers.
///
/// `host` is stamped on a new row. On an existing row it is stamped only when
/// the row has none (saved before hosts were recorded): a save never takes
/// over a row another host owns. `adopt_repo_path` is the deliberate way to
/// claim an unowned row without touching `last_used`.
#[spacetimedb::reducer]
pub fn save_repo_path(
    ctx: &ReducerContext,
    path: String,
    last_used: String,
    host: String,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("repo path is empty".into());
    }
    match ctx.db.repo_paths().iter().find(|r| r.path == path) {
        Some(existing) => {
            let host = if existing.host.is_empty() {
                host
            } else {
                existing.host.clone()
            };
            ctx.db.repo_paths().id().update(RepoPath {
                last_used,
                host,
                ..existing
            })
        }
        None => ctx.db.repo_paths().insert(RepoPath {
            id: 0,
            path,
            last_used,
            verify_command: String::new(),
            host,
        }),
    };
    Ok(())
}

/// Claim a row no host owns for `host`, keeping its verify command and
/// `last_used`.
///
/// Stamps only when the stored host is empty: a row another host owns, or no
/// row at all, is left as it is (not an error: a stale view is no reason to
/// fail a prune).
#[spacetimedb::reducer]
pub fn adopt_repo_path(ctx: &ReducerContext, path: String, host: String) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is empty".into());
    }
    if let Some(existing) = ctx
        .db
        .repo_paths()
        .iter()
        .find(|r| r.path == path && r.host.is_empty())
    {
        ctx.db
            .repo_paths()
            .id()
            .update(RepoPath { host, ..existing });
    }
    Ok(())
}

/// Remove a repo path, but only the row `host` owns.
///
/// A row another host owns, or one nobody owns, stays: the caller judged it
/// from a view that may be stale, and the row's verify command goes with it.
#[spacetimedb::reducer]
pub fn delete_repo_path(ctx: &ReducerContext, path: String, host: String) -> Result<(), String> {
    for row in ctx
        .db
        .repo_paths()
        .iter()
        .filter(|r| r.path == path && r.host == host)
        .collect::<Vec<_>>()
    {
        ctx.db.repo_paths().id().delete(row.id);
    }
    Ok(())
}

/// Set or clear a repo's verify command.
///
/// `""` clears it. Newlines are refused here rather than at the caller, because
/// this is the one place every caller passes through — the command is run as a
/// single shell line, and a second line would run unannounced.
#[spacetimedb::reducer]
pub fn set_verify_command(
    ctx: &ReducerContext,
    path: String,
    command: String,
) -> Result<(), String> {
    if command.contains('\n') || command.contains('\r') {
        return Err("verify command must be a single line; chain steps with && or ;".into());
    }
    let Some(existing) = ctx.db.repo_paths().iter().find(|r| r.path == path) else {
        return Err(format!("no repo path {path}"));
    };
    ctx.db.repo_paths().id().update(RepoPath {
        verify_command: command,
        ..existing
    });
    Ok(())
}

#[spacetimedb::reducer]
pub fn record_base_branch(
    ctx: &ReducerContext,
    repo_path: String,
    branch: String,
    last_used: String,
) -> Result<(), String> {
    match ctx
        .db
        .repo_base_branches()
        .iter()
        .find(|r| r.repo_path == repo_path && r.branch == branch)
    {
        Some(existing) => ctx.db.repo_base_branches().id().update(RepoBaseBranch {
            last_used,
            ..existing
        }),
        None => ctx.db.repo_base_branches().insert(RepoBaseBranch {
            id: 0,
            repo_path,
            branch,
            last_used,
        }),
    };
    Ok(())
}

// -- Settings (Phase 9) -------------------------------------------------------

/// The derived key of a setting row. Must agree character for
/// character with the SQLite side's equivalent, the same requirement
/// `subscription_id` below states for `Subscription`. `pub` (like
/// `subscription_id`) so `MemoryReducerCaller` (`src/sync/memory_caller.rs`)
/// calls this directly rather than re-deriving it, per
/// `spacetime-memory-store.allium`'s `ReducerConformance` guidance.
pub fn host_scoped_id(host: &str, key: &str) -> String {
    format!("{host}/{key}")
}

#[spacetimedb::reducer]
pub fn save_setting(
    ctx: &ReducerContext,
    host: String,
    key: String,
    value: String,
) -> Result<(), String> {
    if host.trim().is_empty() {
        return Err("a host id must not be empty".to_string());
    }
    require_acting_for(ctx, &host)?;
    let id = host_scoped_id(&host, &key);
    match ctx.db.settings().id().find(&id) {
        Some(existing) => ctx.db.settings().id().update(Setting { value, ..existing }),
        None => ctx.db.settings().insert(Setting {
            id,
            host,
            key,
            value,
        }),
    };
    Ok(())
}

#[spacetimedb::reducer]
pub fn clear_setting(ctx: &ReducerContext, host: String, key: String) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let id = host_scoped_id(&host, &key);
    ctx.db.settings().id().delete(&id);
    Ok(())
}

/// Follow an epic. Idempotent, by `sync.allium: SubscribeToEpic`.
#[spacetimedb::reducer]
pub fn subscribe_to_epic(
    ctx: &ReducerContext,
    subscriber: String,
    epic_id: i64,
) -> Result<(), String> {
    if subscriber.trim().is_empty() {
        return Err("cannot subscribe without an identity".into());
    }
    require_own_identity(ctx, &subscriber)?;
    if visible_epic(ctx, epic_id).is_none() {
        return Err(format!("no epic {epic_id}"));
    }
    let id = subscription_id(&subscriber, epic_id);
    // DO NOTHING on a repeat, not an update: every column of this row is part
    // of its own key, so there is nothing a second subscribe could refresh.
    if ctx.db.subscriptions().id().find(&id).is_none() {
        ctx.db.subscriptions().insert(Subscription {
            id,
            subscriber,
            epic_id,
        });
    }
    Ok(())
}

/// The derived key of a subscription row.
///
/// Must agree character for character with the SQLite side's
/// `db::queries::settings::subscription_id`, or the same person subscribing on
/// two backings produces two rows that are the same subscription. `pub` so
/// `MemoryReducerCaller` (src/sync/memory_caller.rs) calls this directly
/// instead of carrying a third copy — the same reuse `derive_epic_status` and
/// friends already get.
pub fn subscription_id(subscriber: &str, epic_id: i64) -> String {
    format!("{subscriber}/{epic_id}")
}

/// Stop following an epic.
///
/// Unfollowing something unfollowed is a refusal rather than a no-op, the
/// asymmetry `sync.allium: UnsubscribeFromEpic` argues for: subscribing twice
/// expresses what the caller wanted, unsubscribing from nothing means they were
/// wrong about the state they were in.
#[spacetimedb::reducer]
pub fn unsubscribe_from_epic(
    ctx: &ReducerContext,
    subscriber: String,
    epic_id: i64,
) -> Result<(), String> {
    require_own_identity(ctx, &subscriber)?;
    let id = subscription_id(&subscriber, epic_id);
    if ctx.db.subscriptions().id().find(&id).is_none() {
        // The sight check (`sync.allium: UnsubscribeFromEpic`): a caller with
        // no row who cannot see the epic is told it is missing, so the answer
        // never confirms that a private epic exists. A caller who still holds
        // a row may always remove it.
        if visible_epic(ctx, epic_id).is_none() {
            return Err(format!("no epic {epic_id}"));
        }
        return Err(format!("not subscribed to epic {epic_id}"));
    }
    ctx.db.subscriptions().id().delete(&id);

    // `sync.allium: UnsubscribeFromEpic` — a poll claim must not outlive the
    // coverage that made it workable. Release the claims on every epic in the
    // unfollowed subtree that this person no longer covers, held by any host
    // they own.
    let remaining: std::collections::HashSet<i64> = ctx
        .db
        .subscriptions()
        .iter()
        .filter(|s| s.subscriber == subscriber)
        .map(|s| s.epic_id)
        .collect();
    let parents: std::collections::HashMap<i64, i64> = ctx
        .db
        .epics()
        .iter()
        .map(|e| (e.id, e.parent_epic_id))
        .collect();
    let released: std::collections::HashSet<i64> =
        epics_losing_coverage(epic_id, &remaining, &parents)
            .into_iter()
            .collect();
    release_poll_claims(ctx, &released, |_, owner| owner == subscriber);
    Ok(())
}

/// `workspaces.allium: RemoveMember`, `MoveEpicToWorkspace` (task #98746, V6):
/// release every epic-scoped poll claim on an epic in `epics` held by a host
/// whose owner `loses_sight(epic_id, owner)` of it, so another host can claim
/// the feed instead of the epic stalling on a host that can no longer read it.
/// The twin of the release in [`unsubscribe_from_epic`].
pub(crate) fn release_poll_claims(
    ctx: &ReducerContext,
    epics: &std::collections::HashSet<i64>,
    loses_sight: impl Fn(i64, &str) -> bool,
) {
    let stale: Vec<i64> = epics
        .iter()
        .flat_map(|epic_id| ctx.db.poll_owners().scope_id().filter(epic_id))
        .filter(|p| p.scope == POLL_SCOPE_EPIC)
        .filter(|p| {
            ctx.db
                .hosts()
                .id()
                .find(&p.host)
                .is_some_and(|h| loses_sight(p.scope_id, &h.owner))
        })
        .map(|p| p.id)
        .collect();
    for row_id in stale {
        ctx.db.poll_owners().id().delete(row_id);
    }
}

/// `feeds.allium: FeedCommandRunsOnSettersHost` (task #98911, M9): the one-time
/// sweep for claims held from before the setter rule. Releases every
/// epic-scoped claim on a feed epic held by a host whose owner did not set its
/// command (or by a host no longer registered), so the setter's host can claim
/// it. Run once by `stamp_rules_version_on_connect` on the first connection
/// after the store moves onto [`FEED_SETTER_CLAIMS_RULES_VERSION`]; the claim
/// reducers refuse such a claim from then on. Rows are collected before any is
/// deleted, so none is skipped.
pub(crate) fn release_claims_of_non_setter_hosts(ctx: &ReducerContext) {
    let stale: Vec<i64> = ctx
        .db
        .poll_owners()
        .iter()
        .filter(|p| p.scope == POLL_SCOPE_EPIC)
        .filter(|p| {
            let owner = ctx
                .db
                .hosts()
                .id()
                .find(&p.host)
                .map(|h| h.owner)
                .unwrap_or_default();
            ctx.db
                .epics()
                .id()
                .find(p.scope_id)
                .is_some_and(|epic| check_feed_claim_by(&epic, &owner).is_err())
        })
        .map(|p| p.id)
        .collect();
    for row_id in stale {
        ctx.db.poll_owners().id().delete(row_id);
    }
}

/// `workspaces.allium: ClaimsFollowSight` (task #98911, M10): the task-scoped
/// twin of [`release_poll_claims`]. Releases every task-scoped claim on a task
/// of an epic in `epics` held by a host whose owner `loses_sight(epic_id,
/// owner)` of that epic. A task's sight is its epic's, so the epic id is what
/// the closure judges. Epic-less tasks are their owner's alone and never move
/// with an epic, so they are not walked.
pub(crate) fn release_task_poll_claims(
    ctx: &ReducerContext,
    epics: &std::collections::HashSet<i64>,
    loses_sight: impl Fn(i64, &str) -> bool,
) {
    let stale: Vec<i64> = epics
        .iter()
        .flat_map(|epic_id| ctx.db.tasks().epic_id().filter(epic_id))
        .flat_map(|task| {
            ctx.db
                .poll_owners()
                .scope_id()
                .filter(&task.id)
                .filter(|p| p.scope == POLL_SCOPE_TASK)
                .filter(|p| {
                    ctx.db
                        .hosts()
                        .id()
                        .find(&p.host)
                        .is_some_and(|h| loses_sight(task.epic_id, &h.owner))
                })
                .map(|p| p.id)
                .collect::<Vec<_>>()
        })
        .collect();
    for row_id in stale {
        ctx.db.poll_owners().id().delete(row_id);
    }
}

/// `workspaces.allium: ClaimsFollowSight` (task #98851, M4; task-scoped claims
/// #98911, M10): the one helper every loss-of-sight path calls once the
/// change is written. Releases the epic-scoped claims on `epics`, and the
/// task-scoped claims on their tasks, held by a host whose owner no longer
/// sees that epic, judged on the store as it is now. A path that can take
/// sight away MUST call this (or, for the two paths that judge by membership,
/// both release functions);
/// `every_loss_of_sight_path_releases_the_claims_it_strands`
/// (`src/sync/memory_caller/tests/feed_authority.rs`) runs each one.
pub(crate) fn release_claims_without_sight(
    ctx: &ReducerContext,
    epics: &std::collections::HashSet<i64>,
) {
    let loses_sight = |epic_id: i64, owner: &str| {
        ctx.db
            .epics()
            .id()
            .find(epic_id)
            .is_none_or(|epic| !epic_visible_to(ctx, &epic, owner))
    };
    release_poll_claims(ctx, epics, loses_sight);
    release_task_poll_claims(ctx, epics, loses_sight);
}

/// The epics in `unfollowed`'s subtree (itself included) that a person whose
/// remaining follows are `followed` no longer covers: an epic is covered when
/// it, or any ancestor, is followed. `parents` maps every epic id to its
/// parent id (`0` for none). Pure, and `pub`, so `MemoryReducerCaller` shares
/// it instead of carrying a second copy of the walk.
pub fn epics_losing_coverage(
    unfollowed: i64,
    followed: &std::collections::HashSet<i64>,
    parents: &std::collections::HashMap<i64, i64>,
) -> Vec<i64> {
    let covered = |start: i64| {
        let mut cur = start;
        let mut hops = 0;
        while cur != 0 && hops <= parents.len() {
            if followed.contains(&cur) {
                return true;
            }
            cur = parents.get(&cur).copied().unwrap_or(0);
            hops += 1;
        }
        false
    };
    let mut out = Vec::new();
    let mut stack = vec![unfollowed];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if !covered(id) {
            out.push(id);
        }
        stack.extend(parents.iter().filter(|(_, p)| **p == id).map(|(c, _)| *c));
    }
    out
}

// -- Usage events (Phase 11, task #4915) -------------------------------------

/// Record one usage event, then drop the oldest rows beyond `cap`.
///
/// The prune is the store's half of the cap the SQLite path enforced with a
/// `DELETE ... WHERE id <= MAX(id) - cap` in the same transaction as the
/// insert: here it runs in the same reducer, so no reader ever sees the table
/// over the cap. Ids are monotonic under `#[auto_inc]`, so "oldest" is
/// "lowest id", exactly as it was in SQLite.
///
/// No caller needs the new row's id — recording returns nothing — so there is
/// no read-back and nothing for an own-creations read-back to cover.
#[spacetimedb::reducer]
pub fn record_usage_event(ctx: &ReducerContext, row: UsageEvent, cap: i64) -> Result<(), String> {
    if cap <= 0 {
        return Err(format!("usage cap must be positive, got {cap}"));
    }
    // The inserted row's own id IS the highest id, because `#[auto_inc]`
    // never hands out anything but the next value — no need to scan the
    // table for a max that insert already told us.
    let inserted = ctx.db.usage_events().insert(UsageEvent {
        // Never trust an incoming id on a create — see `create_task`.
        id: 0,
        recorded_by: caller_identity(ctx),
        ..row
    });
    prune_usage_events(ctx, inserted.id, cap);
    Ok(())
}

pub(crate) fn prune_usage_events(ctx: &ReducerContext, highest: i64, cap: i64) {
    let threshold = highest - cap;
    if threshold <= 0 {
        return;
    }
    let stale: Vec<i64> = ctx
        .db
        .usage_events()
        .iter()
        .filter(|e| e.id <= threshold)
        .map(|e| e.id)
        .collect();
    for id in stale {
        ctx.db.usage_events().id().delete(id);
    }
}

/// The caller's usage, grouped, on demand (task #98756). `query_usage` calls
/// it; no view or subscription serves usage rows, because a view over them
/// would re-run over a person's whole history on every recorded event.
///
/// Built for wasm only. A procedure's transaction calls host imports
/// (`procedure_start_mut_tx` and friends) that the native `test-utils` store
/// does not provide, so compiling it into `spacetime/module-tests` fails the
/// link of every test binary there. Its body is `usage_summary_for`, which the
/// module's own unit tests cover natively.
#[cfg(target_arch = "wasm32")]
#[spacetimedb::procedure]
pub fn own_usage_summary(
    ctx: &mut spacetimedb::ProcedureContext,
    category: Option<String>,
    actor: Option<String>,
    since: Option<String>,
    limit: i64,
) -> Vec<UsageSummaryRow> {
    let who = ctx.sender().to_hex().to_string();
    let filter = UsageFilter {
        category,
        actor,
        since,
        limit,
    };
    ctx.with_tx(|tx| usage_summary_for(tx, &who, &filter))
}
