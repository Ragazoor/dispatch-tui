//! Retention for the two request tables.
//!
//! Spec: `docs/specs/store-split.allium` (`RequestRowsArePruned`,
//! `DeletingATaskDeletesItsRequestRows`).
//!
//! Both tables are public, so every board downloads every row on connect, and
//! [`check_exit_token`] scans a task's whole history. Until now only timer rows
//! were ever deleted. Two things bound them: a task's rows go with the task,
//! and a terminal row older than [`REQUEST_RETENTION_DAYS`] is pruned whenever
//! a new request is written.
//!
//! WHY ON A WRITE rather than a scheduled job of its own: a recurring schedule
//! is a new table, new bindings and a timer to keep alive, for a table that can
//! only grow by a write. Pruning on the write that grows it bounds it the same
//! way with nothing to forget to schedule.

use super::*;

/// How long a finished request row is kept for anyone to read.
pub const REQUEST_RETENTION_DAYS: i64 = 14;

/// The timestamp before which a terminal row is old enough to prune, in the
/// store's spelling (it compares as a string against a row's own stamps).
pub fn request_retention_cutoff(now_micros: i64) -> String {
    format_timestamp_micros(now_micros - REQUEST_RETENTION_DAYS * 24 * 60 * 60 * MICROS_PER_SEC)
}

/// The ids [`prunable_requests`] selects.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PrunableRequests {
    pub dispatch_ids: Vec<i64>,
    pub session_ids: Vec<i64>,
}

/// When a row finished; `requested_at` for one that never recorded it.
fn finished_or_requested<'a>(finished_at: &'a str, requested_at: &'a str) -> &'a str {
    if finished_at.is_empty() {
        requested_at
    } else {
        finished_at
    }
}

/// Which terminal rows older than `cutoff` may go, keeping what
/// [`check_exit_token`] still reads:
///
/// - the newest done wrap_up of a task, if no exit_session has consumed it, is
///   a live token and stays (an older one is superseded and no token);
/// - an exit_session row is what spends its token, so it goes only with its
///   wrap_up (pruned alone, it would make the token valid again);
/// - a `dispatched` row newer than a live token is what voids it, and stays.
///
/// Open rows are never selected.
pub fn prunable_requests(
    cutoff: &str,
    dispatches: &[DispatchRequest],
    sessions: &[SessionRequest],
) -> PrunableRequests {
    use std::collections::{HashMap, HashSet};

    let consumed: HashSet<i64> = sessions
        .iter()
        .filter(|r| r.kind == SESSION_KIND_EXIT_SESSION)
        .map(|r| r.wrap_up_request)
        .collect();
    let session_ids: HashSet<i64> = sessions.iter().map(|r| r.id).collect();
    // The newest done wrap_up of each task, by `check_exit_token`'s order. An
    // older one is superseded, so it is no token and prunes like any row.
    let mut newest: HashMap<i64, &SessionRequest> = HashMap::new();
    for r in sessions
        .iter()
        .filter(|r| r.kind == SESSION_KIND_WRAP_UP && r.state == SESSION_DONE)
    {
        let key = |r: &SessionRequest| (r.requested_at.clone(), r.id);
        if newest.get(&r.task_id).is_none_or(|n| key(r) > key(n)) {
            newest.insert(r.task_id, r);
        }
    }
    let live_tokens: Vec<&SessionRequest> = newest
        .into_values()
        .filter(|r| !consumed.contains(&r.id))
        .collect();
    let is_live_token = |id: i64| live_tokens.iter().any(|t| t.id == id);
    let old_terminal = |r: &SessionRequest| {
        !session_request_is_open(&r.state)
            && finished_or_requested(&r.finished_at, &r.requested_at) < cutoff
    };

    let pruned_wrap_ups: HashSet<i64> = sessions
        .iter()
        .filter(|r| r.kind == SESSION_KIND_WRAP_UP && old_terminal(r) && !is_live_token(r.id))
        .map(|r| r.id)
        .collect();
    let session_ids_out = sessions
        .iter()
        .filter(|r| old_terminal(r))
        .filter(|r| match r.kind.as_str() {
            SESSION_KIND_WRAP_UP => pruned_wrap_ups.contains(&r.id),
            SESSION_KIND_EXIT_SESSION => {
                pruned_wrap_ups.contains(&r.wrap_up_request)
                    || !session_ids.contains(&r.wrap_up_request)
            }
            _ => true,
        })
        .map(|r| r.id)
        .collect();

    let dispatch_ids = dispatches
        .iter()
        .filter(|d| {
            !dispatch_request_is_open(&d.state)
                && finished_or_requested(&d.finished_at, &d.requested_at) < cutoff
        })
        .filter(|d| {
            !(d.state == DISPATCH_DISPATCHED
                && live_tokens.iter().any(|t| {
                    t.task_id == d.task_id && d.finished_at.as_str() >= t.finished_at.as_str()
                }))
        })
        .map(|d| d.id)
        .collect();

    PrunableRequests {
        dispatch_ids,
        session_ids: session_ids_out,
    }
}

/// Delete the terminal request rows [`prunable_requests`] selects. Called by
/// the reducers that write a request, before they write theirs.
pub(crate) fn prune_terminal_requests(ctx: &ReducerContext) {
    let cutoff = request_retention_cutoff(ctx.timestamp.to_micros_since_unix_epoch());
    let dispatches: Vec<DispatchRequest> = ctx.db.dispatch_requests().iter().collect();
    let sessions: Vec<SessionRequest> = ctx.db.session_requests().iter().collect();
    let prunable = prunable_requests(&cutoff, &dispatches, &sessions);
    delete_request_ids(ctx, &prunable.dispatch_ids, &prunable.session_ids);
}

fn delete_request_ids(ctx: &ReducerContext, dispatch_ids: &[i64], session_ids: &[i64]) {
    for id in dispatch_ids {
        ctx.db.dispatch_requests().id().delete(*id);
    }
    for id in session_ids {
        ctx.db.session_requests().id().delete(*id);
    }
}

/// `DeletingATaskDeletesItsRequestRows`: every request row of a deleted task,
/// finished or not. Called by [`delete_task_side_effects`] AFTER it has failed
/// the open ones, which cancels their timers.
pub(crate) fn delete_request_rows_of(ctx: &ReducerContext, task_id: i64) {
    let dispatch_ids: Vec<i64> = ctx
        .db
        .dispatch_requests()
        .task_id()
        .filter(&task_id)
        .map(|r| r.id)
        .collect();
    let session_ids: Vec<i64> = ctx
        .db
        .session_requests()
        .task_id()
        .filter(&task_id)
        .map(|r| r.id)
        .collect();
    delete_request_ids(ctx, &dispatch_ids, &session_ids);
}
