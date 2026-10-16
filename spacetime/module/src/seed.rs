//! Seed reducers: the restore path, one batched upsert per table.

use super::*;

/// Write tasks with the ids they already have.
///
/// **A snapshot taken before `owner` existed is refused here, by design.** Every
/// epic-less row in it fails the check, loudly and before anything is written.
/// Backfilling the seeding user onto those rows is the seeding client's job,
/// named in the migration design as one of the two seed-time backfills — not
/// something this reducer should guess at, because the answer is *which person*
/// and the module has no way to know.
#[spacetimedb::reducer]
pub fn seed_tasks(ctx: &ReducerContext, rows: Vec<Task>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_tasks needs each task's real id, not a generated one".into());
        }
        write_task(ctx, row)?;
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_epics(ctx: &ReducerContext, rows: Vec<Epic>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_epics needs each epic's real id".into());
        }
        validate_status("epic", row.id, &row.status)?;
        if ctx.db.epics().id().find(row.id).is_some() {
            ctx.db.epics().id().update(row);
        } else {
            ctx.db.epics().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_todos(ctx: &ReducerContext, rows: Vec<Todo>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_todos needs each todo's real id".into());
        }
        if ctx.db.todos().id().find(row.id).is_some() {
            ctx.db.todos().id().update(row);
        } else {
            ctx.db.todos().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_task_watchers(ctx: &ReducerContext, rows: Vec<TaskWatcher>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_task_watchers needs each watcher's real id".into());
        }
        if ctx.db.task_watchers().id().find(row.id).is_some() {
            ctx.db.task_watchers().id().update(row);
        } else {
            ctx.db.task_watchers().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_work_packages(ctx: &ReducerContext, rows: Vec<WorkPackage>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_work_packages needs each package's real id".into());
        }
        if ctx.db.work_packages().id().find(row.id).is_some() {
            ctx.db.work_packages().id().update(row);
        } else {
            ctx.db.work_packages().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_work_package_links(
    ctx: &ReducerContext,
    rows: Vec<WorkPackageLink>,
) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a work package link needs its derived id".into());
        }
        if ctx
            .db
            .work_package_links()
            .id()
            .find(row.id.clone())
            .is_some()
        {
            ctx.db.work_package_links().id().update(row);
        } else {
            ctx.db.work_package_links().insert(row);
        }
    }
    Ok(())
}

/// Agents keep their ids, like every seeded row; a name already held by a
/// different agent is refused rather than overwritten, and every seeded name
/// is written to the ledger if it is not there yet, so the picker can never
/// hand it out again (`AgentNameIsNeverReused`).
#[spacetimedb::reducer]
pub fn seed_agents(ctx: &ReducerContext, rows: Vec<Agent>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_agents needs each agent's real id".into());
        }
        if let Some(holder) = ctx.db.agents().name().find(&row.name) {
            if holder.id != row.id {
                return Err(format!(
                    "agent {}: name {:?} already belongs to agent {}",
                    row.id, row.name, holder.id
                ));
            }
        }
        validate_seeded_agent(&row)?;
        let other_current = ctx
            .db
            .agents()
            .task_id()
            .filter(&row.task_id)
            .any(|a| a.id != row.id && a.state == AGENT_CURRENT);
        if row.state == AGENT_CURRENT && other_current {
            return Err(format!(
                "agent {}: task {} already has a current agent",
                row.id, row.task_id
            ));
        }
        if ctx
            .db
            .handed_out_agent_names()
            .name()
            .find(&row.name)
            .is_none()
        {
            ctx.db.handed_out_agent_names().insert(HandedOutAgentName {
                name: row.name.clone(),
                handed_out_at: row.started_at.clone(),
            });
        }
        if ctx.db.agents().id().find(row.id).is_some() {
            ctx.db.agents().id().update(row);
        } else {
            ctx.db.agents().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_handed_out_agent_names(
    ctx: &ReducerContext,
    rows: Vec<HandedOutAgentName>,
) -> Result<(), String> {
    for row in rows {
        if row.name.is_empty() {
            return Err("a handed-out agent name cannot be empty".into());
        }
        if ctx
            .db
            .handed_out_agent_names()
            .name()
            .find(&row.name)
            .is_some()
        {
            ctx.db.handed_out_agent_names().name().update(row);
        } else {
            ctx.db.handed_out_agent_names().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_repo_paths(ctx: &ReducerContext, rows: Vec<RepoPath>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_repo_paths needs each row's real id".into());
        }
        if ctx.db.repo_paths().id().find(row.id).is_some() {
            ctx.db.repo_paths().id().update(row);
        } else {
            ctx.db.repo_paths().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_repo_base_branches(
    ctx: &ReducerContext,
    rows: Vec<RepoBaseBranch>,
) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_repo_base_branches needs each row's real id".into());
        }
        if ctx.db.repo_base_branches().id().find(row.id).is_some() {
            ctx.db.repo_base_branches().id().update(row);
        } else {
            ctx.db.repo_base_branches().insert(row);
        }
    }
    Ok(())
}

/// Always inserts zero rows in production — a fresh local dump never has any
/// `PollOwner` claims to seed (see the table's own doc comment) — but kept as
/// a genuine find-or-update-or-insert, not a stub, so a future dump-from-a-
/// live-shared-store path is not blocked on this reducer being rewritten.
#[spacetimedb::reducer]
pub fn seed_poll_owners(ctx: &ReducerContext, rows: Vec<PollOwner>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_poll_owners needs each row's real id".into());
        }
        if ctx.db.poll_owners().id().find(row.id).is_some() {
            ctx.db.poll_owners().id().update(row);
        } else {
            ctx.db.poll_owners().insert(row);
        }
    }
    Ok(())
}

// Tables with no generated id. Identity comes from the row's own columns, and
// there is no index to look it up by, so a re-seed scans and removes the old
// row before writing the new one. Linear per row and acceptable: these tables
// hold live agent state, which is small and short-lived, not history.

#[spacetimedb::reducer]
pub fn seed_task_shells(ctx: &ReducerContext, rows: Vec<TaskShell>) -> Result<(), String> {
    for row in rows {
        if let Some(existing) = ctx
            .db
            .task_shells()
            .iter()
            .find(|e| e.task_id == row.task_id && e.shell_id == row.shell_id)
        {
            ctx.db.task_shells().delete(existing);
        }
        ctx.db.task_shells().insert(row);
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_task_subagents(ctx: &ReducerContext, rows: Vec<TaskSubagent>) -> Result<(), String> {
    for row in rows {
        if let Some(existing) = ctx
            .db
            .task_subagents()
            .iter()
            .find(|e| e.task_id == row.task_id && e.agent_id == row.agent_id)
        {
            ctx.db.task_subagents().delete(existing);
        }
        ctx.db.task_subagents().insert(row);
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_hosts(ctx: &ReducerContext, rows: Vec<Host>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a host needs its minted id".into());
        }
        if ctx.db.hosts().id().find(row.id.clone()).is_some() {
            ctx.db.hosts().id().update(row);
        } else {
            ctx.db.hosts().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_subscriptions(ctx: &ReducerContext, rows: Vec<Subscription>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a subscription needs an id".into());
        }
        if row.subscriber.trim().is_empty() {
            return Err(format!("subscription {} needs a subscriber", row.id));
        }
        if ctx.db.subscriptions().id().find(row.id.clone()).is_some() {
            ctx.db.subscriptions().id().update(row);
        } else {
            ctx.db.subscriptions().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_settings(ctx: &ReducerContext, rows: Vec<Setting>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a setting needs its derived id".into());
        }
        if ctx.db.settings().id().find(row.id.clone()).is_some() {
            ctx.db.settings().id().update(row);
        } else {
            ctx.db.settings().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_filter_presets(ctx: &ReducerContext, rows: Vec<FilterPreset>) -> Result<(), String> {
    for row in rows {
        if row.id.is_empty() {
            return Err("a filter preset needs its derived id".into());
        }
        if ctx.db.filter_presets().id().find(row.id.clone()).is_some() {
            ctx.db.filter_presets().id().update(row);
        } else {
            ctx.db.filter_presets().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_learnings(ctx: &ReducerContext, rows: Vec<Learning>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_learnings needs each learning's real id".into());
        }
        if ctx.db.learnings().id().find(row.id).is_some() {
            ctx.db.learnings().id().update(row);
        } else {
            ctx.db.learnings().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_learning_retrievals(
    ctx: &ReducerContext,
    rows: Vec<LearningRetrieval>,
) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_learning_retrievals needs each row's real id".into());
        }
        if ctx.db.learning_retrievals().id().find(row.id).is_some() {
            ctx.db.learning_retrievals().id().update(row);
        } else {
            ctx.db.learning_retrievals().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_usage_events(ctx: &ReducerContext, rows: Vec<UsageEvent>) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_usage_events needs each row's real id".into());
        }
        if ctx.db.usage_events().id().find(row.id).is_some() {
            ctx.db.usage_events().id().update(row);
        } else {
            ctx.db.usage_events().insert(row);
        }
    }
    Ok(())
}

#[spacetimedb::reducer]
pub fn seed_retired_feed_items(
    ctx: &ReducerContext,
    rows: Vec<RetiredFeedItem>,
) -> Result<(), String> {
    for row in rows {
        if row.id == 0 {
            return Err("seed_retired_feed_items needs each row's real id".into());
        }
        if ctx.db.retired_feed_items().id().find(row.id).is_some() {
            ctx.db.retired_feed_items().id().update(row);
        } else {
            ctx.db.retired_feed_items().insert(row);
        }
    }
    Ok(())
}
