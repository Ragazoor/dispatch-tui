//! Repo configuration, settings, subscriptions and usage events.

use super::*;

// -- Repo configuration -----------------------------------------------------

/// Record a repo path, or touch the one already there.
///
/// Upsert by path rather than by id, because the path is what the domain calls
/// the same repo. Two rows for one path would give the verify command two
/// answers.
#[spacetimedb::reducer]
pub fn save_repo_path(ctx: &ReducerContext, path: String, last_used: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("repo path is empty".into());
    }
    match ctx.db.repo_paths().iter().find(|r| r.path == path) {
        Some(existing) => ctx.db.repo_paths().id().update(RepoPath {
            last_used,
            ..existing
        }),
        None => ctx.db.repo_paths().insert(RepoPath {
            id: 0,
            path,
            last_used,
            verify_command: String::new(),
        }),
    };
    Ok(())
}

#[spacetimedb::reducer]
pub fn delete_repo_path(ctx: &ReducerContext, path: String) -> Result<(), String> {
    for row in ctx
        .db
        .repo_paths()
        .iter()
        .filter(|r| r.path == path)
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
    if ctx.db.epics().id().find(epic_id).is_none() {
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
    let id = subscription_id(&subscriber, epic_id);
    if ctx.db.subscriptions().id().find(&id).is_none() {
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
    for released in epics_losing_coverage(epic_id, &remaining, &parents) {
        let stale: Vec<i64> = ctx
            .db
            .poll_owners()
            .scope_id()
            .filter(&released)
            .filter(|p| p.scope == POLL_SCOPE_EPIC)
            .filter(|p| {
                ctx.db
                    .hosts()
                    .id()
                    .find(&p.host)
                    .is_some_and(|h| h.owner == subscriber)
            })
            .map(|p| p.id)
            .collect();
        for row_id in stale {
            ctx.db.poll_owners().id().delete(row_id);
        }
    }
    Ok(())
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
/// no read-back and nothing for `own_creations` to cover.
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
