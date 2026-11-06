//! Repo configuration, settings, subscriptions and usage events.

use super::*;

// -- Repo configuration -----------------------------------------------------

/// The verify command of the repository `path`/`github_path` belongs to, read
/// off its rows other than `except` (`0` for a row not stored yet). A row with
/// the same non-empty `github_path` wins, because that is the repository
/// itself; failing one, a sibling by local path (either side has no
/// `github_path`); else empty.
pub fn repository_verify_command<'a>(
    rows: impl IntoIterator<Item = &'a RepoPath>,
    except: i64,
    path: &str,
    github_path: &str,
) -> String {
    let mut by_path = None;
    for r in rows {
        if r.id == except || r.verify_command.is_empty() {
            continue;
        }
        if !github_path.is_empty() && r.github_path == github_path {
            return r.verify_command.clone();
        }
        if by_path.is_none() && is_sibling(&r.path, &r.github_path, path, github_path) {
            by_path = Some(r.verify_command.clone());
        }
    }
    by_path.unwrap_or_default()
}

/// `core.allium: VerifyCommandIsPerRepository`: the command a re-saved row
/// holds. A row that learns (or changes) its GitHub path, or has no command,
/// takes its repository's when the repository has one; any other re-save keeps
/// the row's own. Every save then gives the row's command to its repository's
/// rows that have none (`rows_without_the_repository_command`).
pub fn resaved_verify_command(existing: &RepoPath, github_path: &str, repository: &str) -> String {
    let joins = github_path != existing.github_path || existing.verify_command.is_empty();
    if joins && !repository.is_empty() {
        repository.to_string()
    } else {
        existing.verify_command.clone()
    }
}

/// Every row of the repository at `path`: each row of that path, and each
/// sibling of any of them. Every row of the path is asked, not the first one
/// found, so a row of the path with no GitHub path cannot hide the
/// repository's other machines. Empty exactly when no row has `path`.
pub fn rows_of_the_repository_at<'a>(rows: &'a [RepoPath], path: &str) -> Vec<&'a RepoPath> {
    let named: Vec<&RepoPath> = rows.iter().filter(|r| r.path == path).collect();
    rows.iter()
        .filter(|r| {
            named
                .iter()
                .any(|n| is_sibling(&r.path, &r.github_path, &n.path, &n.github_path))
        })
        .collect()
}

/// Every row of the repository `named` belongs to: its siblings and the whole
/// GitHub group of each, so a row with no GitHub path still reaches the rest of
/// its group (`core.allium: VerifyCommandIsPerRepository`).
pub fn rows_of_the_repository_of<'a>(rows: &'a [RepoPath], named: &RepoPath) -> Vec<&'a RepoPath> {
    let sibling = |r: &RepoPath| is_sibling(&r.path, &r.github_path, &named.path, &named.github_path);
    let groups: Vec<&str> = rows
        .iter()
        .filter(|r| sibling(r))
        .map(|r| r.github_path.as_str())
        .chain(std::iter::once(named.github_path.as_str()))
        .filter(|g| !g.is_empty())
        .collect();
    rows.iter()
        .filter(|r| sibling(r) || (!r.github_path.is_empty() && groups.contains(&r.github_path.as_str())))
        .collect()
}

/// The rows of `row`'s repository that hold no verify command and must take
/// `row`'s, so every row of one repository agrees.
///
/// The repository is `row`'s siblings and the WHOLE GitHub group of each of
/// them: a row with no GitHub path is a sibling of every row at its local
/// path, and stopping at those rows would leave the rest of their groups
/// behind (`core.allium: VerifyCommandIsPerRepository`).
pub fn rows_without_the_repository_command<'a>(
    rows: &'a [RepoPath],
    row: &RepoPath,
) -> Vec<&'a RepoPath> {
    if row.verify_command.is_empty() {
        return Vec::new();
    }
    let sibling = |r: &RepoPath| is_sibling(&r.path, &r.github_path, &row.path, &row.github_path);
    let groups: Vec<&str> = rows
        .iter()
        .filter(|r| sibling(r))
        .map(|r| r.github_path.as_str())
        .chain(std::iter::once(row.github_path.as_str()))
        .filter(|g| !g.is_empty())
        .collect();
    rows.iter()
        .filter(|r| r.id != row.id && r.verify_command.is_empty())
        .filter(|r| sibling(r) || groups.contains(&r.github_path.as_str()))
        .collect()
}

/// Two rows are one repository when they share a non-empty GitHub path, or
/// share a local path and at least one of them has no GitHub path to say
/// otherwise.
pub fn is_sibling(path_a: &str, gh_a: &str, path_b: &str, gh_b: &str) -> bool {
    if !gh_a.is_empty() && !gh_b.is_empty() {
        gh_a == gh_b
    } else {
        path_a == path_b
    }
}

/// Record a repo path for `host`, or touch the row it already has.
///
/// A path is one row per `(path, host)`: two machines that both have
/// `~/Code/api` each own a row, so each one's feed resolves it. `host` takes
/// the row it already owns; failing that it adopts an unowned row of the path
/// (saved before hosts were recorded); failing that it inserts. `github_path`
/// (`org/repo`, `""` when unknown) is stamped when non-empty. A row that has
/// no verify command, or learns its GitHub path, joins its repository's (see
/// `resaved_verify_command`).
/// `adopt_repo_path` is the way to claim an unowned row without touching
/// `last_used`.
#[spacetimedb::reducer]
pub fn save_repo_path(
    ctx: &ReducerContext,
    path: String,
    last_used: String,
    host: String,
    github_path: String,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("repo path is empty".into());
    }
    require_acting_for(ctx, &host)?;
    // One read of the table serves the lookup, the join and the share.
    let rows: Vec<RepoPath> = ctx.db.repo_paths().iter().collect();
    let own = rows
        .iter()
        .find(|r| r.path == path && r.host == host)
        .or_else(|| rows.iter().find(|r| r.path == path && r.host.is_empty()))
        .cloned();
    let row = match own {
        Some(existing) => {
            let github_path = if github_path.is_empty() {
                existing.github_path.clone()
            } else {
                github_path
            };
            let repository = repository_verify_command(&rows, existing.id, &path, &github_path);
            let verify_command = resaved_verify_command(&existing, &github_path, &repository);
            ctx.db.repo_paths().id().update(RepoPath {
                last_used,
                host,
                github_path,
                verify_command,
                ..existing
            })
        }
        None => {
            let verify_command = repository_verify_command(&rows, 0, &path, &github_path);
            ctx.db.repo_paths().insert(RepoPath {
                id: 0,
                path,
                last_used,
                verify_command,
                host,
                github_path,
            })
        }
    };
    // Give the row's command to the rows of its repository that have none.
    for sibling in rows_without_the_repository_command(&rows, &row) {
        ctx.db.repo_paths().id().update(RepoPath {
            verify_command: row.verify_command.clone(),
            ..sibling.clone()
        });
    }
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
    require_acting_for(ctx, &host)?;
    // `host` may already own a row of this path; a second would break the
    // one-row-per-(path, host) rule, so that case drops the unowned row.
    let owns = ctx
        .db
        .repo_paths()
        .iter()
        .any(|r| r.path == path && r.host == host);
    if let Some(existing) = ctx
        .db
        .repo_paths()
        .iter()
        .find(|r| r.path == path && r.host.is_empty())
    {
        if owns {
            ctx.db.repo_paths().id().delete(existing.id);
        } else {
            ctx.db
                .repo_paths()
                .id()
                .update(RepoPath { host, ..existing });
        }
    }
    Ok(())
}

/// Remove `host`'s row of a repo path, or an unowned one.
///
/// A path only another host has is refused, naming that host: the caller asked
/// to forget something this host does not own, and silently doing nothing made
/// the row come back on reload. A path nobody has is not an error (a stale
/// view). The row's verify command goes with it; sibling rows keep theirs.
#[spacetimedb::reducer]
pub fn delete_repo_path(ctx: &ReducerContext, path: String, host: String) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let rows: Vec<RepoPath> = ctx
        .db
        .repo_paths()
        .iter()
        .filter(|r| r.path == path)
        .collect();
    let mine: Vec<&RepoPath> = rows
        .iter()
        .filter(|r| r.host == host || r.host.is_empty())
        .collect();
    if mine.is_empty() {
        return match rows.first() {
            Some(other) => Err(format!("repo path {path} is owned by host {}", other.host)),
            None => Ok(()),
        };
    }
    for row in mine {
        ctx.db.repo_paths().id().delete(row.id);
    }
    Ok(())
}

/// Give every row of one repository the same verify command: that of the most
/// recently used row that has one. Rows saved before `github_path` existed or
/// before linked rows shared a command can differ; no later save repairs them,
/// because a save only fills an empty command. Run once by
/// `stamp_rules_version_on_connect` (`REPO_PATH_VERIFY_RULES_VERSION`). A row
/// with no `github_path` is its own repository by path, as `is_sibling` says.
pub fn unify_repo_verify_commands(ctx: &ReducerContext) {
    let rows: Vec<RepoPath> = ctx.db.repo_paths().iter().collect();
    let key = |r: &RepoPath| {
        if r.github_path.is_empty() {
            format!("path:{}", r.path)
        } else {
            format!("github:{}", r.github_path)
        }
    };
    let mut groups: std::collections::BTreeMap<String, Vec<&RepoPath>> = Default::default();
    for row in &rows {
        groups.entry(key(row)).or_default().push(row);
    }
    for group in groups.values() {
        let Some(winner) = group
            .iter()
            .filter(|r| !r.verify_command.is_empty())
            .max_by(|a, b| (&a.last_used, a.id).cmp(&(&b.last_used, b.id)))
        else {
            continue;
        };
        for row in group
            .iter()
            .filter(|r| r.verify_command != winner.verify_command)
        {
            ctx.db.repo_paths().id().update(RepoPath {
                verify_command: winner.verify_command.clone(),
                ..(*row).clone()
            });
        }
    }
}

/// Set or clear a repo's verify command, acting through `host`'s row.
///
/// `""` clears it. Newlines are refused here rather than at the caller, because
/// this is the one place every caller passes through — the command is run as a
/// single shell line, and a second line would run unannounced. The caller must
/// own `host`. The row is `host`'s own row of `path`, else an unowned one; a
/// row only another host has is not reachable, so two machines holding
/// different repositories at one path cannot set each other's command. The
/// command then goes to every row of that row's repository.
#[spacetimedb::reducer]
pub fn set_verify_command(
    ctx: &ReducerContext,
    path: String,
    host: String,
    command: String,
) -> Result<(), String> {
    if command.contains('\n') || command.contains('\r') {
        return Err("verify command must be a single line; chain steps with && or ;".into());
    }
    require_acting_for(ctx, &host)?;
    let rows: Vec<RepoPath> = ctx.db.repo_paths().iter().collect();
    let Some(named) = rows
        .iter()
        .find(|r| r.path == path && r.host == host)
        .or_else(|| rows.iter().find(|r| r.path == path && r.host.is_empty()))
    else {
        return Err(format!("no repo path {path} for host {host}"));
    };
    // The command belongs to the repository: every row that is the same one.
    let repository = rows_of_the_repository_of(&rows, named);
    for row in repository {
        ctx.db.repo_paths().id().update(RepoPath {
            verify_command: command.clone(),
            ..row.clone()
        });
    }
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

/// The four managed-feed setting keys (`epics.allium: SetManagedFeedConfig`).
/// Must agree with the board's `store/queries/settings.rs` constants.
pub const MANAGED_FEED_SETTING_KEYS: [&str; 4] = [
    "reviews_feed_command",
    "reviews_feed_interval_secs",
    "cve_feed_command",
    "cve_feed_interval_secs",
];

/// One write of [`set_managed_feed_settings`]: set `key` to `value`, or clear it
/// when `value` is `None`.
#[derive(spacetimedb::SpacetimeType, Clone, Debug)]
pub struct SettingWrite {
    pub key: String,
    pub value: Option<String>,
}

/// Judge every write of [`set_managed_feed_settings`] before any is applied: a
/// managed-feed key, named once, and an interval that is a whole number.
pub fn check_managed_feed_writes(writes: &[SettingWrite]) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for write in writes {
        if !MANAGED_FEED_SETTING_KEYS.contains(&write.key.as_str()) {
            return Err(format!("{:?} is not a managed-feed setting", write.key));
        }
        if seen.contains(&write.key.as_str()) {
            return Err(format!("{:?} is named twice", write.key));
        }
        seen.push(&write.key);
        if write.key.ends_with("_interval_secs") {
            if let Some(v) = &write.value {
                if v.parse::<i64>().is_err() {
                    return Err(format!("{:?} is not a whole number: {v:?}", write.key));
                }
            }
        }
    }
    Ok(())
}

/// Write the managed-feed settings as ONE change (task #98916, N5): the command
/// and interval of a feed are coupled, so a failure between two `save_setting`
/// calls must not leave one without the other. Every write is judged first
/// ([`check_managed_feed_writes`]); none lands unless all pass.
#[spacetimedb::reducer]
pub fn set_managed_feed_settings(
    ctx: &ReducerContext,
    host: String,
    writes: Vec<SettingWrite>,
) -> Result<(), String> {
    if host.trim().is_empty() {
        return Err("a host id must not be empty".to_string());
    }
    require_acting_for(ctx, &host)?;
    check_managed_feed_writes(&writes)?;
    for SettingWrite { key, value } in writes {
        let id = host_scoped_id(&host, &key);
        match value {
            Some(value) => match ctx.db.settings().id().find(&id) {
                Some(existing) => ctx.db.settings().id().update(Setting { value, ..existing }),
                None => ctx.db.settings().insert(Setting {
                    id,
                    host: host.clone(),
                    key,
                    value,
                }),
            },
            None => {
                ctx.db.settings().id().delete(&id);
                continue;
            }
        };
    }
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

/// `workspaces.allium: ClaimsFollowSight`, B1 (task #98992): the one helper an
/// EPIC-level loss-of-sight path calls once its change is written. Releases
/// the claims ([`release_claims_without_sight`]) and ends every unseen mark
/// and bookmark on a task of `epics` whose person no longer sees that task,
/// judged on the store as it is now. A path that can take sight away MUST call
/// this, or [`on_task_sight_lost`] for a single task;
/// `every_loss_of_sight_path_releases_the_claims_it_strands` runs each one.
pub(crate) fn on_sight_lost(ctx: &ReducerContext, epics: &std::collections::HashSet<i64>) {
    release_claims_without_sight(ctx, epics);
    let tasks: std::collections::HashSet<i64> = epics
        .iter()
        .flat_map(|epic_id| ctx.db.tasks().epic_id().filter(epic_id))
        .map(|t| t.id)
        .collect();
    drop_marks_of_those_who_cannot_see(ctx, &tasks);
}

/// [`on_sight_lost`] for one task whose own sight changed: a move into a
/// private epic, or a detach onto a user board.
pub(crate) fn on_task_sight_lost(ctx: &ReducerContext, task_id: i64) {
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return;
    };
    let stale: Vec<i64> = ctx
        .db
        .poll_owners()
        .scope_id()
        .filter(&task_id)
        .filter(|p| p.scope == POLL_SCOPE_TASK)
        .filter(|p| {
            ctx.db
                .hosts()
                .id()
                .find(&p.host)
                .is_some_and(|h| !task_visible_to(ctx, &task, &h.owner))
        })
        .map(|p| p.id)
        .collect();
    for row_id in stale {
        ctx.db.poll_owners().id().delete(row_id);
    }
    drop_marks_of_those_who_cannot_see(ctx, &std::collections::HashSet::from([task_id]));
}

fn drop_marks_of_those_who_cannot_see(
    ctx: &ReducerContext,
    tasks: &std::collections::HashSet<i64>,
) {
    if tasks.is_empty() {
        return;
    }
    crate::bookmark::drop_marks_where(ctx, None, |task_id, subscriber| {
        tasks.contains(&task_id)
            && ctx
                .db
                .tasks()
                .id()
                .find(task_id)
                .is_some_and(|t| !task_visible_to(ctx, &t, subscriber))
    });
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
