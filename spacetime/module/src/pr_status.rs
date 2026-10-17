//! PR state decided in the module.
//!
//! Spec: `docs/specs/store-split.allium` (step M6, task #53259):
//! `ReportPrStatus` and `ReportPrUnreadable`. The host runs `gh` and posts
//! what it read; every task write below is derived here.

use super::*;

const PR_URL_TYPE: &str = "pr";

/// The task written by a PR status report, or `None` when the report changes
/// nothing: the task is no longer in review, carries no pr-typed url, or
/// already shows what the report derives. Assumes the strings passed
/// [`validate_pr_status_report`].
pub fn apply_pr_status(task: Task, state: &str, review_decision: &str, now: &str) -> Option<Task> {
    if task.status != REVIEW || task.url_type != PR_URL_TYPE {
        return None;
    }
    let sub_status = match state.to_uppercase().as_str() {
        "MERGED" => {
            return Some(Task {
                status: DONE.into(),
                sub_status: default_sub_status(DONE).into(),
                completed_at: now.into(),
                updated_at: now.into(),
                ..task
            });
        }
        "CLOSED" => "pr_closed",
        // An open PR never overwrites a conflict the user has to resolve.
        _ if task.sub_status == "conflict" => return None,
        _ => match review_decision.to_uppercase().as_str() {
            "APPROVED" => "approved",
            "CHANGES_REQUESTED" => "changes_requested",
            _ => "awaiting_review",
        },
    };
    (task.sub_status != sub_status).then(|| Task {
        sub_status: sub_status.into(),
        updated_at: now.into(),
        ..task
    })
}

/// Whether the raw strings `gh` printed are ones the module knows. The review
/// decision is empty for a repo without branch protection.
pub fn validate_pr_status_report(state: &str, review_decision: &str) -> Result<(), String> {
    if !matches!(state.to_uppercase().as_str(), "OPEN" | "MERGED" | "CLOSED") {
        return Err(format!("unknown PR state {state:?}"));
    }
    if !matches!(
        review_decision.to_uppercase().as_str(),
        "" | "APPROVED" | "CHANGES_REQUESTED" | "REVIEW_REQUIRED"
    ) {
        return Err(format!("unknown PR review decision {review_decision:?}"));
    }
    Ok(())
}

/// `ReportPrUnreadable`'s write, or `None` when nothing changes.
pub fn apply_pr_unreadable(task: Task, now: &str) -> Option<Task> {
    if task.status != REVIEW || task.url_type != PR_URL_TYPE || task.sub_status == "pr_unreachable"
    {
        return None;
    }
    Some(Task {
        sub_status: "pr_unreachable".into(),
        updated_at: now.into(),
        ..task
    })
}

// -- Reducers -----------------------------------------------------------------

/// `ReportPrStatus`: the host read a PR and posts the raw strings. A task that
/// is gone or no longer in review is accepted and left alone (the read raced a
/// close). A merge also runs the epic rollup and the phoenix respawn, as any
/// route into Done does.
#[spacetimedb::reducer]
pub fn report_pr_status(
    ctx: &ReducerContext,
    task_id: i64,
    host: String,
    state: String,
    review_decision: String,
) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    validate_pr_status_report(&state, &review_decision)?;
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Ok(());
    };
    let prior_status = task.status.clone();
    let epic_id = task.epic_id;
    let stamp = now(ctx);
    let Some(mut next) = apply_pr_status(task, &state, &review_decision, &stamp) else {
        return Ok(());
    };
    if should_respawn_phoenix(&prior_status, &next)
        && respawn_phoenix_successor_of(ctx, &next, &stamp)
    {
        next.phoenix = false;
    }
    let merged = next.status == DONE;
    write_task(ctx, next)?;
    if merged {
        // The merge took the window with it (`agents.allium`, A4: the agent's).
        update_current_agent(&mut AgentTables(ctx), task_id, |agent| Agent {
            tmux_window: String::new(),
            ..agent
        });
        recalculate_epic_chain(ctx, epic_id);
    }
    Ok(())
}

/// `ReportPrUnreadable`: the host gave up reading a PR (its own counter hit the
/// threshold) and says so. The reason is for the host's log, not stored.
#[spacetimedb::reducer]
pub fn report_pr_unreadable(
    ctx: &ReducerContext,
    task_id: i64,
    host: String,
    _reason: String,
) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Ok(());
    };
    if let Some(next) = apply_pr_unreadable(task, &now(ctx)) {
        write_task(ctx, next)?;
    }
    Ok(())
}
