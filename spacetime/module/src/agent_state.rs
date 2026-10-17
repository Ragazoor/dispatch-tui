//! Agent session state: subagents, hooks, poll ownership and the host registry.

use super::*;

/// Drop the live-session rows a task owns.
///
/// Shared by deletion and by the session-clearing paths. Keyed by `task_id`
/// rather than by the row's own identity because neither table has one: they
/// are a set of live things, not entities with a life of their own.
pub(crate) fn delete_agent_state_for(ctx: &ReducerContext, task_id: i64) {
    for row in ctx
        .db
        .task_shells()
        .task_id()
        .filter(&task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_shells().delete(row);
    }
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

// -- Hosts and subscriptions ------------------------------------------------

/// Upsert this machine's row into the shared host registry.
///
/// Decided on task #4907, not assumed: `ensure_host_identity`/`rename_host`/
/// `adopt_user_identity` (`src/db/queries/settings.rs`) keep writing the
/// LOCAL settings row unconditionally, connected or not — that write is the
/// durable identity credential, not a shared table with one copy, so it is
/// deliberately NOT routed through `SharedWriter` the way every other method
/// in this file is. This reducer is the separate MIRROR push that makes the
/// resulting row visible to the rest of the registry, called from
/// `sync.allium: RegisterHostOnConnect` (every identity settle) and
/// `RegisterHostOnRename` (a live rename while connected) — see those rules
/// for when it fires and why re-sending an unchanged row on every reconnect
/// is correct rather than wasteful.
///
/// Upsert-by-id, with `core.allium: LocalHostOwnerIsWrittenOnce` ENFORCED
/// rather than trusted ([`validate_host_registration`]): a caller may only
/// name ITSELF as owner, and an owned host's owner never changes. That is
/// what makes the owner a fact the dispatch requests can check against
/// (`store-split.allium`, D11) — a writable owner would let anyone take over
/// another person's machine and then pass every check. The owner's own
/// re-register (a rename, a reconnect) is unaffected.
#[spacetimedb::reducer]
pub fn register_host(
    ctx: &ReducerContext,
    id: String,
    label: String,
    owner: String,
) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("a host id must not be empty".to_string());
    }
    let existing = ctx.db.hosts().id().find(&id);
    validate_host_registration(existing.as_ref(), &owner, &caller_identity(ctx))
        .map_err(|why| format!("host {id}: {why}"))?;
    match existing {
        Some(existing) => {
            ctx.db.hosts().id().update(Host {
                label,
                owner,
                ..existing
            });
        }
        None => {
            ctx.db.hosts().insert(Host { id, label, owner });
        }
    }
    Ok(())
}

// -- Poll ownership -----------------------------------------------------

/// Find-or-create-or-reassign a [`PollOwner`] row for `(scope, scope_id)`.
///
/// `force = false` (a claim — `pr-workflow.allium: PollPrStatus`,
/// `feeds.allium: FeedTick`) fills an ABSENT row and leaves an existing one
/// alone, whoever it names. `force = true` (an override —
/// `pr-workflow.allium: OverridePrPollOwner`, `feeds.allium:
/// OverrideFeedOwner`) unconditionally reassigns an existing row too — the
/// only way an existing claim ever moves. One function rather than a pair:
/// the two only ever differed in what happens when a row already exists.
///
/// Shared by [`claim_poll_owner`] and [`override_poll_owner`], which are
/// themselves shared by both scopes (task and epic) — `scope` is caller-
/// validated input, not a typed enum, matching how every other
/// module-boundary enum-shaped value here (`Task.status`, `Task.url_type`,
/// `Task.wrap_up_mode`, …) is a plain validated `String` rather than a
/// SATS enum with its own reducer per variant.
pub(crate) fn write_poll_owner_row(
    ctx: &ReducerContext,
    scope: &str,
    scope_id: i64,
    host: String,
    force: bool,
) {
    let existing = ctx
        .db
        .poll_owners()
        .scope_id()
        .filter(&scope_id)
        .find(|p| p.scope == scope);
    match existing {
        Some(row) if force => {
            ctx.db.poll_owners().id().update(PollOwner {
                host,
                claimed_at: now(ctx),
                ..row
            });
        }
        Some(_) => {}
        None => {
            ctx.db.poll_owners().insert(PollOwner {
                id: 0,
                scope: scope.to_string(),
                scope_id,
                host,
                claimed_at: now(ctx),
            });
        }
    }
}

/// Reject anything but `"task"`/`"epic"` — the typo-safety
/// `write_poll_owner_row`'s callers need, at the same input-validation
/// boundary this module already enforces every other caller-supplied enum
/// string at (see e.g. `KNOWN_STATUSES`).
pub(crate) fn require_poll_scope(scope: &str) -> Result<(), String> {
    if scope == POLL_SCOPE_TASK || scope == POLL_SCOPE_EPIC {
        Ok(())
    } else {
        Err(format!(
            "poll scope must be {POLL_SCOPE_TASK:?} or {POLL_SCOPE_EPIC:?}, got {scope:?}"
        ))
    }
}

/// Claim an unowned scope. `core.allium: PollOwner`. Only the host's owner
/// may name it (D11, `store-split.allium: ClaimPollOwner`).
#[spacetimedb::reducer]
pub fn claim_poll_owner(
    ctx: &ReducerContext,
    scope: String,
    scope_id: i64,
    host: String,
) -> Result<(), String> {
    require_poll_scope(&scope)?;
    require_acting_for(ctx, &host)?;
    write_poll_owner_row(ctx, &scope, scope_id, host, false);
    Ok(())
}

/// Reassign a scope's ownership unconditionally. `pr-workflow.allium:
/// OverridePrPollOwner`, `feeds.allium: OverrideFeedOwner`. Only the host's
/// owner may name it (D11, `store-split.allium: OverridePollOwner`).
#[spacetimedb::reducer]
pub fn override_poll_owner(
    ctx: &ReducerContext,
    scope: String,
    scope_id: i64,
    host: String,
) -> Result<(), String> {
    require_poll_scope(&scope)?;
    require_acting_for(ctx, &host)?;
    write_poll_owner_row(ctx, &scope, scope_id, host, true);
    Ok(())
}

//
// The denormalised counters (`live_subagents`, `stop_pending`) and the
// `task_subagents` table that backs them. Mirrors
// `src/db/queries/{subagents,tasks}.rs` — see `docs/specs/
// agent-health.allium` for the guarantees these reproduce; nothing here
// changes what a hook does, only where the counting happens.
//
// EVENT TIME, NOT WRITE TIME. `last_pre_tool_use_at`, `last_notification_at`
// and `stop_pending_at` are the CLIENT's clock — the instant the hook fired —
// passed in as arguments, the same way `created_at` is a client timestamp
// (see `encode.rs`'s note on `ReducerWriter::now`). `updated_at` alone is the
// STORE's clock (`now(ctx)`), because it is bookkeeping about the write, not
// a fact about the agent. Getting this backwards would matter: `agent-health.
// allium`'s `HookUserPromptSubmit` guidance is explicit that the tie-break
// between a deferred `Stop` and the prompt that supersedes it must compare
// EVENT times — "any ordering derived from write order inherits the race the
// rule is trying to resolve" — and a write-time comparison across two
// reducers invoked from two different hook PROCESSES (each with its own
// network latency to the store) is exactly the write-order race that
// guidance warns against.
//
// AMBIGUOUS OUTCOMES ARE REFUSED, NOT GUESSED. A reducer returns no value, so
// the client reads its effect back off the row via `ctx.db` inside the
// `_then` callback (the same mechanism `create_task`'s id read-back uses) —
// but every one of these acts on a row that already exists and whose id the
// caller already has, so there is no id to match: the row is read by primary
// key, not by guessing which one arrived. `try_record_stop` and
// `record_user_prompt_submit` each have a branch (flip vs. defer; resume vs.
// refresh) that is unambiguous to read back ONLY once the precondition
// (`status = Running`, or `status in {Running, Review}`) is known to have
// held — and refusing when it does not turns "the precondition failed" into
// an ordinary `ReducerOutcome::Refused` instead of a state indistinguishable
// from "it just succeeded quietly". See this task's plan doc
// (docs/plans/2026-09-21-phase-6b-agent-session-state-reducers.md), decision 3.

/// Recompute `live_subagents` from `task_subagents` and write it, if the task
/// still exists. Mirrors `src/db/queries/subagents.rs::sync_count`. A missing
/// task is a silent no-op, matching the SQL `UPDATE ... WHERE id = ?` that
/// simply touches zero rows.
pub(crate) fn sync_subagent_count(ctx: &ReducerContext, task_id: i64) -> Result<i64, String> {
    let count = ctx.db.task_subagents().task_id().filter(&task_id).count() as i64;
    // On the task's current agent (`agents.allium`, A4); none, no write.
    update_current_agent(&mut AgentTables(ctx), task_id, |agent| Agent {
        live_subagents: count,
        ..agent
    });
    Ok(count)
}

/// Evict `task_subagents` rows for `task_id` whose `session_id` differs from
/// `incoming`. Mirrors `subagents.rs::fence_session` / `agent-health.allium:
/// SubagentSessionFence`.
pub(crate) fn fence_subagent_session(ctx: &ReducerContext, task_id: i64, incoming: &str) {
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .filter(|r| r.session_id != incoming)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// Apply a deferred `Stop` if this write is the one that drained the last
/// subagent. Mirrors `src/db/queries/mod.rs::apply_pending_stop_if_drained`
/// — the SAME shared predicate `subagent_stop` and `subagent_clear` both
/// route through below.
///
/// `last_pre_tool_use_at`/`last_notification_at` are cleared to the module's
/// empty-string sentinel, matching `STOP_FLIP_SET`'s `NULL`.
pub(crate) fn apply_pending_stop_if_drained(
    ctx: &ReducerContext,
    task_id: i64,
) -> Result<bool, String> {
    let Some(row) = ctx.db.tasks().id().find(task_id) else {
        return Ok(false);
    };
    let Some(agent) = current_agent(&AgentTables(ctx), task_id) else {
        return Ok(false);
    };
    if row.status == RUNNING && agent.stop_pending && agent.live_subagents == 0 {
        flip_to_review(ctx, row)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Flip `row` to `Review` — clearing the hook-activity timestamps and the
/// deferred-Stop bit — and recalculate the epic that leaves as a derivation
/// input. Shared by [`apply_pending_stop_if_drained`]'s drain branch and
/// `try_record_stop`'s immediate flip below: the only two places a task ever
/// makes this transition.
pub(crate) fn flip_to_review(ctx: &ReducerContext, row: Task) -> Result<(), String> {
    let epic_id = row.epic_id;
    let id = row.id;
    write_task(
        ctx,
        Task {
            status: REVIEW.into(),
            sub_status: AWAITING_REVIEW.into(),
            ..row
        },
    )?;
    update_current_agent(&mut AgentTables(ctx), id, |agent| Agent {
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        stop_pending: false,
        ..agent
    });
    recalculate_epic_chain(ctx, epic_id);
    Ok(())
}

/// Delete the `task_subagents` row for `(task_id, agent_id)`, if any. Neither
/// table has a primary key, so `subagent_start`'s dedupe-then-insert
/// "replace" is delete-then-insert rather than an update.
pub(crate) fn delete_subagent_entry(ctx: &ReducerContext, task_id: i64, agent_id: &str) {
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .filter(|r| r.agent_id == agent_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// Delete every `task_subagents` row for `task_id`.
pub(crate) fn delete_all_subagents(ctx: &ReducerContext, task_id: i64) {
    for row in ctx
        .db
        .task_subagents()
        .task_id()
        .filter(&task_id)
        .collect::<Vec<_>>()
    {
        ctx.db.task_subagents().delete(row);
    }
}

/// `HookSubagentStart` in `docs/specs/agent-health.allium`. `started_at` is
/// the client's clock, stored verbatim (RFC 3339, matching
/// `subagents.rs::subagent_start`'s `now.to_rfc3339()`) — this column is never
/// compared across rows, so it carries no format requirement of its own.
#[spacetimedb::reducer]
pub fn subagent_start(
    ctx: &ReducerContext,
    task_id: i64,
    agent_id: String,
    session_id: String,
    started_at: String,
) -> Result<(), String> {
    // A hook for a task with no current agent writes no agent fact (A15).
    let Some(owner) = current_agent(&AgentTables(ctx), task_id) else {
        return Ok(());
    };
    fence_subagent_session(ctx, task_id, &session_id);
    // Dedupe first, then insert fresh: an upsert by (task_id, agent_id).
    delete_subagent_entry(ctx, task_id, &agent_id);
    ctx.db.task_subagents().insert(TaskSubagent {
        task_id,
        agent_id,
        session_id,
        started_at,
        agent_row: owner.id,
    });
    sync_subagent_count(ctx, task_id)?;
    Ok(())
}

/// `HookSubagentStop` in `docs/specs/agent-health.allium`. An unrecognised
/// `agent_id` is a no-op, not an underflow — the delete simply matches
/// nothing, and the count is recomputed from the table either way.
#[spacetimedb::reducer]
pub fn subagent_stop(
    ctx: &ReducerContext,
    task_id: i64,
    agent_id: String,
    session_id: String,
) -> Result<(), String> {
    fence_subagent_session(ctx, task_id, &session_id);
    delete_subagent_entry(ctx, task_id, &agent_id);
    sync_subagent_count(ctx, task_id)?;
    apply_pending_stop_if_drained(ctx, task_id)?;
    Ok(())
}

/// Clear every `task_subagents` row for `task_id`, and apply any deferred
/// `Stop` this drains. For `DetachTmux` (`docs/specs/split-pane.allium`), the
/// one draining clear point that owns no status of its own.
#[spacetimedb::reducer]
pub fn subagent_clear(ctx: &ReducerContext, task_id: i64) -> Result<(), String> {
    delete_all_subagents(ctx, task_id);
    sync_subagent_count(ctx, task_id)?;
    apply_pending_stop_if_drained(ctx, task_id)?;
    Ok(())
}

/// [`subagent_clear`] minus the drain, plus voiding `stop_pending`
/// unconditionally. For the three non-draining clear points — `SessionStart`
/// (`ClearSubagentsOnSessionStart`), crash and dispatch-claim — which void a
/// deferred Stop rather than apply it. A missing task is a silent no-op for
/// the `stop_pending` write, matching the SQL's `UPDATE ... WHERE id = ?`.
#[spacetimedb::reducer]
pub fn subagent_clear_and_void_pending_stop(
    ctx: &ReducerContext,
    task_id: i64,
) -> Result<(), String> {
    delete_all_subagents(ctx, task_id);
    sync_subagent_count(ctx, task_id)?;
    update_current_agent(&mut AgentTables(ctx), task_id, |agent| Agent {
        stop_pending: false,
        ..agent
    });
    Ok(())
}

/// `HookStop` in `docs/specs/agent-health.allium`. Refuses when the task is
/// not `Running` (including when it does not exist) rather than a silent
/// no-op: see this file's "Agent session state" section header for why that
/// is what lets the client read `Flipped` vs. `Deferred` back unambiguously.
/// `stop_pending_at` is the client's clock (millisecond precision, matching
/// `tasks.rs::try_record_stop`'s `format_datetime_millis`) — the value
/// `record_user_prompt_submit` later compares its own prompt time against, so
/// it must be an EVENT time, not this write's commit time.
#[spacetimedb::reducer]
pub fn try_record_stop(
    ctx: &ReducerContext,
    id: i64,
    stop_pending_at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} not found"));
    };
    if row.status != RUNNING {
        return Err(format!("task {id} is not running"));
    }
    // No current agent holds no subagents: the Stop flips at once (A15).
    let live = current_agent(&AgentTables(ctx), id).map_or(0, |a| a.live_subagents);
    if live == 0 {
        flip_to_review(ctx, row)?;
    } else {
        update_current_agent(&mut AgentTables(ctx), id, |agent| Agent {
            stop_pending: true,
            stop_pending_at,
            ..agent
        });
    }
    Ok(())
}

/// `HookPreToolUse` in `docs/specs/agent-health.allium`. A missing task, or
/// one that is not `Running`, is a silent no-op — matching the SQL `UPDATE
/// ... WHERE id = ? AND status = ?` that simply touches zero rows. `sub_status`
/// arrives already resolved: the classification (`classify_agent_activity`)
/// runs on the client, against a snapshot it already paid to read — this
/// reducer only applies the decision. `at` is the client's clock (second
/// precision, matching `tasks.rs::record_pre_tool_use`'s `format_datetime`).
#[spacetimedb::reducer]
pub fn record_pre_tool_use(
    ctx: &ReducerContext,
    id: i64,
    sub_status: String,
    at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    if row.status != RUNNING {
        return Ok(());
    }
    update_current_agent(&mut AgentTables(ctx), id, |agent| Agent {
        last_pre_tool_use_at: at,
        ..agent
    });
    write_task(ctx, Task { sub_status, ..row })
}

/// `HookNotification` in `docs/specs/agent-health.allium`. `mode` arrives
/// already resolved from the notification kind — `NotificationWrite::from_kind`
/// runs on the client, same reasoning as `record_pre_tool_use`'s `sub_status`
/// — so this reducer only applies one of four already-decided writes. The
/// live-work predicate for `raise_if_no_own_work_live` is the one thing
/// evaluated HERE rather than on the client: it must read the row's committed
/// `live_subagents` at write time, not a snapshot that could be
/// stale by the time this reducer runs (`agent-health.allium: HookNotification`'s
/// "Evaluation time" guidance). `at` is the client's clock (second precision).
#[spacetimedb::reducer]
pub fn record_notification(
    ctx: &ReducerContext,
    id: i64,
    mode: String,
    at: String,
) -> Result<(), String> {
    if mode == "ignore" {
        return Ok(());
    }
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Ok(());
    };
    if row.status != RUNNING {
        return Ok(());
    }
    let stamp = |ctx: &ReducerContext, at: String| {
        update_current_agent(&mut AgentTables(ctx), id, |agent| Agent {
            last_notification_at: at,
            ..agent
        });
    };
    match mode.as_str() {
        "clear" => {
            stamp(ctx, String::new());
            write_task(
                ctx,
                Task {
                    sub_status: ACTIVE.into(),
                    ..row
                },
            )
        }
        "raise" => {
            stamp(ctx, at);
            write_task(
                ctx,
                Task {
                    sub_status: NEEDS_INPUT.into(),
                    ..row
                },
            )
        }
        "raise_if_no_own_work_live" => {
            // Read at write time, from the agent's committed count.
            let live = current_agent(&AgentTables(ctx), id).map_or(0, |a| a.live_subagents);
            if live == 0 {
                stamp(ctx, at);
                write_task(
                    ctx,
                    Task {
                        sub_status: NEEDS_INPUT.into(),
                        ..row
                    },
                )
            } else {
                Ok(())
            }
        }
        other => Err(format!("unknown notification mode {other:?}")),
    }
}

/// `HookUserPromptSubmit` in `docs/specs/agent-health.allium`. Refuses when the
/// task is neither `Running` nor `Review` (including when it does not exist)
/// — see this file's "Agent session state" section header for why that is
/// what lets the client read `Resumed` vs. `Refreshed` back unambiguously.
///
/// `activity_at` (second precision) is what `last_pre_tool_use_at` takes;
/// `prompt_at` (millisecond precision) is compared against `stop_pending_at`
/// to decide whether to void it — mirroring `tasks.rs::record_user_prompt_submit`'s
/// own two-precision split of a single client `now`. Ties (equal timestamps)
/// preserve the bit; a `stop_pending_at` predating the field (the module's
/// empty-string sentinel) reads as "fired before any prompt" and is voided.
/// Both are EVENT times, not this write's commit time — see this file's
/// section header for why that matters here specifically.
#[spacetimedb::reducer]
pub fn record_user_prompt_submit(
    ctx: &ReducerContext,
    id: i64,
    activity_at: String,
    prompt_at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} not found"));
    };
    if row.status != RUNNING && row.status != REVIEW {
        return Err(format!("task {id} is neither running nor in review"));
    }
    let resumed = row.status == REVIEW;
    let epic_id = row.epic_id;
    update_current_agent(&mut AgentTables(ctx), id, |agent| {
        let void_pending_stop = agent.stop_pending
            && (agent.stop_pending_at.is_empty()
                || agent.stop_pending_at.as_str() < prompt_at.as_str());
        Agent {
            last_pre_tool_use_at: activity_at,
            stop_pending: agent.stop_pending && !void_pending_stop,
            ..agent
        }
    });
    write_task(
        ctx,
        Task {
            status: RUNNING.into(),
            sub_status: ACTIVE.into(),
            ..row
        },
    )?;
    if resumed {
        recalculate_epic_chain(ctx, epic_id);
    }
    Ok(())
}

/// Atomically set `pr_learnings_gate_shown_at` if it is not already set.
/// Refusal carries the "already shown or task missing" answer the same way
/// `request_dispatch` reports a lost race — the client reads `Applied` as
/// `true` ("this call set it, block the PR") and `Refused` as `false` via
/// `ReducerOutcome::won()`, no read-back needed. `at` is the client's clock.
#[spacetimedb::reducer]
pub fn mark_pr_learnings_gate_shown(
    ctx: &ReducerContext,
    id: i64,
    at: String,
) -> Result<(), String> {
    let Some(row) = ctx.db.tasks().id().find(id) else {
        return Err(format!("task {id} not found"));
    };
    if !row.pr_learnings_gate_shown_at.is_empty() {
        return Err(format!("task {id} has already shown the PR learnings gate"));
    }
    write_task(
        ctx,
        Task {
            pr_learnings_gate_shown_at: at,
            ..row
        },
    )
}
