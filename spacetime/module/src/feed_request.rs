//! Feed requests: work one host asks another host to do for a feed epic.
//!
//! Spec: `docs/specs/store-split.allium`: `FeedRequest`, `RequestFeedRefresh`,
//! `HostClaimsFeedRequest`, `HostReportsFeedRequestDone`,
//! `HostReportsFeedRequestFailed`, `StaleDeleteRequestsTeardownOnTheHoldingHost`
//! and `RequestRowsArePruned`; `docs/specs/feeds.allium`: `SerialisedFeedCycle`
//! (scope across hosts).
//!
//! Two kinds share one table. A `refresh` row asks the epic's poll owner to run
//! one feed cycle (a manual "r" pressed on another host). A `teardown` row asks
//! the host that dispatched a feed task to release its worktree and window after
//! the stale delete removed the task's row.
//!
//! Neither kind has a deadline or a backstop timer: a teardown must outlive its
//! host being offline. Every decision is a pure function over rows; the reducers
//! only read the rows, call those and write what they return.

use super::*;

// -- Table --------------------------------------------------------------------

/// One request for a host to do feed work (`store-split.allium: FeedRequest`),
/// open while `requested` or `running`, kept as history once `done` or `failed`.
///
/// Every optional column is `""` for absent and a row reference is `0`, never an
/// `Option` (see "Absence is a sentinel, not a null" in `lib.rs`). Every
/// timestamp is the module clock, in the store's spelling.
#[spacetimedb::table(accessor = feed_requests)]
#[derive(Clone, Debug)]
pub struct FeedRequest {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// `refresh` or `teardown`.
    pub kind: String,
    /// The machine that serves the row. Indexed: a board reads its own host's
    /// rows, and a refresh request asks "does this owner hold an open one?".
    #[index(btree)]
    pub host: String,
    /// On a refresh row: the feed epic to run. `0` on a teardown row.
    pub epic_id: i64,
    /// On a teardown row: the id of the removed task, for logs; the row is gone.
    /// `0` on a refresh row.
    pub task_id: i64,
    /// Teardown snapshots, taken from the task row before it was deleted.
    pub repo_path: String,
    pub worktree: String,
    pub tmux_window: String,
    /// The identity whose call wrote the request, stamped by the module from
    /// `ctx.sender()`, never sent by the client. A host's board serves only rows
    /// its own owner requested.
    pub requested_by: String,
    /// `requested`, `running`, `done` or `failed`.
    pub state: String,
    pub requested_at: String,
    pub claimed_at: String,
    pub finished_at: String,
    pub reason: String,
}

// -- Vocabulary ---------------------------------------------------------------

pub const FEED_REQUEST_REFRESH: &str = "refresh";
pub const FEED_REQUEST_TEARDOWN: &str = "teardown";

pub const FEED_REQUESTED: &str = "requested";
pub const FEED_RUNNING: &str = "running";
pub const FEED_DONE: &str = "done";
pub const FEED_FAILED: &str = "failed";

/// How long a `requested` refresh row waits before it is stale: a refresh the
/// owner never served in this long is no longer what its requester wants.
pub const STALE_REFRESH_REQUEST_SECS: i64 = 60 * 60;

// -- Pure rules ---------------------------------------------------------------

/// `FeedRequest.is_open`.
pub fn feed_request_is_open(state: &str) -> bool {
    state == FEED_REQUESTED || state == FEED_RUNNING
}

/// The one blank feed request row: every optional column absent.
pub fn blank_feed_request() -> FeedRequest {
    FeedRequest {
        id: 0,
        kind: String::new(),
        host: String::new(),
        epic_id: 0,
        task_id: 0,
        repo_path: String::new(),
        worktree: String::new(),
        tmux_window: String::new(),
        requested_by: String::new(),
        state: String::new(),
        requested_at: String::new(),
        claimed_at: String::new(),
        finished_at: String::new(),
        reason: String::new(),
    }
}

/// `RequestFeedRefresh`'s row.
pub fn new_refresh_request(
    epic_id: i64,
    owner_host: &str,
    requested_by: &str,
    now: &str,
) -> FeedRequest {
    FeedRequest {
        kind: FEED_REQUEST_REFRESH.into(),
        host: owner_host.into(),
        epic_id,
        requested_by: requested_by.into(),
        state: FEED_REQUESTED.into(),
        requested_at: now.into(),
        ..blank_feed_request()
    }
}

/// `StaleDeleteRequestsTeardownOnTheHoldingHost`: whether removing `task` owes
/// a teardown request. It does when another host than the one running the
/// ingest holds the task's worktree or window. The ingesting host tears its own
/// rows down in the cycle itself; a row no host holds, or holding nothing, owes
/// nothing.
pub fn teardown_request_owed(task: &Task, ingest_host: &str) -> bool {
    !task.host.is_empty()
        && task.host != ingest_host
        && (!task.worktree.is_empty() || !task.tmux_window.is_empty())
}

/// `StaleDeleteRequestsTeardownOnTheHoldingHost`'s row, from the task as it was
/// before the delete.
pub fn new_teardown_request(task: &Task, requested_by: &str, now: &str) -> FeedRequest {
    FeedRequest {
        kind: FEED_REQUEST_TEARDOWN.into(),
        host: task.host.clone(),
        task_id: task.id,
        repo_path: task.repo_path.clone(),
        worktree: task.worktree.clone(),
        tmux_window: task.tmux_window.clone(),
        requested_by: requested_by.into(),
        state: FEED_REQUESTED.into(),
        requested_at: now.into(),
        ..blank_feed_request()
    }
}

/// What `RequestFeedRefresh` does once its `requires` hold.
#[derive(Debug, PartialEq, Eq)]
pub enum RefreshDecision {
    /// Write a new request for the owner.
    Write,
    /// An open request for the epic already exists: nothing to write.
    Coalesced,
}

/// `RequestFeedRefresh`'s `requires`, in the order a caller is best told about
/// them. `owner_host` is the epic's poll owner row's host, `owner` the identity
/// that owns that host, `caller` the identity that owns `requester_host`.
pub fn decide_feed_refresh(
    epic: Option<&Epic>,
    requester_host: &str,
    poll_owner_host: Option<&str>,
    poll_owner_host_owner: Option<&str>,
    caller: &str,
    open_for_epic: bool,
) -> Result<RefreshDecision, String> {
    let Some(epic) = epic else {
        return Err("epic does not exist".to_string());
    };
    if epic.feed_command.is_empty() {
        return Err(format!("epic {} has no feed command", epic.id));
    }
    let Some(owner_host) = poll_owner_host else {
        return Err(format!("epic {} has no poll owner yet", epic.id));
    };
    if owner_host == requester_host {
        return Err(format!(
            "host {requester_host} owns the poll of epic {}; it runs the refresh itself",
            epic.id
        ));
    }
    if poll_owner_host_owner != Some(caller) {
        return Err(format!(
            "the poll owner of epic {} is a host of another person, so {caller} may not ask it to refresh",
            epic.id
        ));
    }
    Ok(if open_for_epic {
        RefreshDecision::Coalesced
    } else {
        RefreshDecision::Write
    })
}

/// `HostClaimsFeedRequest`'s `requires`: the row's own host, on a row that is
/// `requested`, or `running` left by a board that died mid-serve (the work is
/// idempotent, so it is served again).
pub fn validate_feed_request_claim(row: &FeedRequest, host: &str) -> Result<(), String> {
    let id = row.id;
    if row.host != host {
        return Err(format!(
            "feed request {id} is for {}, so {host} cannot claim it",
            row.host
        ));
    }
    if !feed_request_is_open(&row.state) {
        return Err(format!(
            "feed request {id} is {}, so it cannot be claimed",
            row.state
        ));
    }
    Ok(())
}

/// `HostClaimsFeedRequest`'s writes.
pub fn claim_feed_request_row(row: FeedRequest, now: &str) -> FeedRequest {
    FeedRequest {
        state: FEED_RUNNING.into(),
        claimed_at: now.into(),
        ..row
    }
}

/// The `requires` both host reports share: the row's own host, on a row it is
/// running.
pub fn validate_feed_request_report(row: &FeedRequest, reporter: &str) -> Result<(), String> {
    let id = row.id;
    if row.host != reporter {
        return Err(format!(
            "feed request {id} is for {}, so {reporter} cannot report on it",
            row.host
        ));
    }
    if row.state != FEED_RUNNING {
        return Err(format!(
            "feed request {id} is {} rather than running, so the report is refused",
            row.state
        ));
    }
    Ok(())
}

/// `HostReportsFeedRequestDone`'s writes.
pub fn done_feed_request_row(row: FeedRequest, now: &str) -> FeedRequest {
    FeedRequest {
        state: FEED_DONE.into(),
        finished_at: now.into(),
        ..row
    }
}

/// `HostReportsFeedRequestFailed`'s writes.
pub fn failed_feed_request_row(row: FeedRequest, reason: &str, now: &str) -> FeedRequest {
    FeedRequest {
        state: FEED_FAILED.into(),
        finished_at: now.into(),
        reason: reason.into(),
        ..row
    }
}

/// `RequestRowsArePruned` for feed requests: a terminal row finished before
/// `cutoff`, and a `requested` refresh row requested before `stale_cutoff`. An
/// open teardown row is never selected: an offline host must still find it.
pub fn prunable_feed_requests(cutoff: &str, stale_cutoff: &str, rows: &[FeedRequest]) -> Vec<i64> {
    rows.iter()
        .filter(|r| {
            if feed_request_is_open(&r.state) {
                r.kind == FEED_REQUEST_REFRESH
                    && r.state == FEED_REQUESTED
                    && r.requested_at.as_str() < stale_cutoff
            } else {
                let at = if r.finished_at.is_empty() {
                    r.requested_at.as_str()
                } else {
                    r.finished_at.as_str()
                };
                at < cutoff
            }
        })
        .map(|r| r.id)
        .collect()
}

/// The cutoff before which a `requested` refresh row is stale.
pub fn stale_refresh_cutoff(now_micros: i64) -> String {
    format_timestamp_micros(now_micros - STALE_REFRESH_REQUEST_SECS * MICROS_PER_SEC)
}

// -- Reducers -----------------------------------------------------------------

fn find_feed_request(ctx: &ReducerContext, id: i64) -> Result<FeedRequest, String> {
    ctx.db
        .feed_requests()
        .id()
        .find(id)
        .ok_or_else(|| format!("feed request {id} does not exist"))
}

/// Delete the rows [`prunable_feed_requests`] selects. Called before a request
/// is written.
pub(crate) fn prune_feed_requests(ctx: &ReducerContext) {
    let micros = ctx.timestamp.to_micros_since_unix_epoch();
    let rows: Vec<FeedRequest> = ctx.db.feed_requests().iter().collect();
    for id in prunable_feed_requests(
        &request_retention_cutoff(micros),
        &stale_refresh_cutoff(micros),
        &rows,
    ) {
        ctx.db.feed_requests().id().delete(id);
    }
}

/// Write one teardown request for `task` if the stale delete owes one. Called
/// with the task row as it was BEFORE the delete, and AFTER the task's own
/// request rows were cleaned up, so the new row is not swept with them.
pub(crate) fn write_teardown_request_if_owed(ctx: &ReducerContext, task: &Task, ingest_host: &str) {
    if !teardown_request_owed(task, ingest_host) {
        return;
    }
    // A board serves only rows its own owner requested, so a row for a host of
    // another person would stay open for ever: write none.
    let caller = caller_identity(ctx);
    if ctx
        .db
        .hosts()
        .id()
        .find(&task.host)
        .is_none_or(|h| h.owner != caller)
    {
        return;
    }
    prune_feed_requests(ctx);
    ctx.db
        .feed_requests()
        .insert(new_teardown_request(task, &caller_identity(ctx), &now(ctx)));
}

/// `RequestFeedRefresh`: a host that is not the epic's poll owner asks the
/// owner to run one feed cycle.
#[spacetimedb::reducer]
pub fn request_feed_refresh(
    ctx: &ReducerContext,
    epic_id: i64,
    host: String,
) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    let epic = crate::visibility::visible_epic(ctx, epic_id);
    let owner_row = ctx
        .db
        .poll_owners()
        .scope_id()
        .filter(&epic_id)
        .find(|p| p.scope == POLL_SCOPE_EPIC);
    let owner_host_row = owner_row
        .as_ref()
        .and_then(|p| ctx.db.hosts().id().find(&p.host));
    let open_for_epic = owner_row.as_ref().is_some_and(|p| {
        ctx.db.feed_requests().host().filter(&p.host).any(|r| {
            r.kind == FEED_REQUEST_REFRESH && r.epic_id == epic_id && feed_request_is_open(&r.state)
        })
    });
    let decision = decide_feed_refresh(
        epic.as_ref(),
        &host,
        owner_row.as_ref().map(|p| p.host.as_str()),
        owner_host_row.as_ref().map(|h| h.owner.as_str()),
        &caller_identity(ctx),
        open_for_epic,
    )?;
    if decision == RefreshDecision::Write {
        prune_feed_requests(ctx);
        let owner = owner_row.map(|p| p.host).unwrap_or_default();
        ctx.db.feed_requests().insert(new_refresh_request(
            epic_id,
            &owner,
            &caller_identity(ctx),
            &now(ctx),
        ));
    }
    Ok(())
}

/// `HostClaimsFeedRequest`.
#[spacetimedb::reducer]
pub fn claim_feed_request(
    ctx: &ReducerContext,
    request_id: i64,
    host: String,
) -> Result<(), String> {
    let row = find_feed_request(ctx, request_id)?;
    validate_feed_request_claim(&row, &host)?;
    require_acting_for(ctx, &host)?;
    ctx.db
        .feed_requests()
        .id()
        .update(claim_feed_request_row(row, &now(ctx)));
    Ok(())
}

/// `HostReportsFeedRequestDone`.
#[spacetimedb::reducer]
pub fn report_feed_request_done(
    ctx: &ReducerContext,
    request_id: i64,
    reporter: String,
) -> Result<(), String> {
    let row = find_feed_request(ctx, request_id)?;
    validate_feed_request_report(&row, &reporter)?;
    require_acting_for(ctx, &reporter)?;
    ctx.db
        .feed_requests()
        .id()
        .update(done_feed_request_row(row, &now(ctx)));
    Ok(())
}

/// `HostReportsFeedRequestFailed`.
#[spacetimedb::reducer]
pub fn report_feed_request_failed(
    ctx: &ReducerContext,
    request_id: i64,
    reporter: String,
    reason: String,
) -> Result<(), String> {
    let row = find_feed_request(ctx, request_id)?;
    validate_feed_request_report(&row, &reporter)?;
    require_acting_for(ctx, &reporter)?;
    ctx.db
        .feed_requests()
        .id()
        .update(failed_feed_request_row(row, &reason, &now(ctx)));
    Ok(())
}
