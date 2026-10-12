//! Dispatch as request rows, with a scheduled backstop timer.
//!
//! Spec: `docs/specs/store-split.allium` (step M4, task #53257):
//! `DispatchRequest`, `RequestDispatch`, `HostClaimsDispatch`,
//! `HostReportsDispatched`, `HostReportsDispatchFailed`,
//! `DispatchRequestTimesOut`, `HostRestartFailsInterrupted`,
//! `DeletingATaskFailsItsOpenRequest`, `ReleaseClaimOfFailedRequest` and the
//! invariant `OneOpenDispatchRequestPerTask`.
//!
//! Every DECISION is a pure function over rows, testable without a store
//! (`tests.rs`'s `dispatch_requests`); the reducers below only read the rows,
//! call those functions and write what they return. Every way a request fails
//! goes through [`fail_dispatch_request`], so the row, its timer and the
//! conditional claim release cannot drift apart between failure paths.

use super::*;

// -- Tables -------------------------------------------------------------------

/// One request to dispatch one task (`store-split.allium: DispatchRequest`),
/// open while `requested` or `provisioning`, kept as history once `dispatched`
/// or `failed`.
///
/// Every optional column is `""` for absent, never an `Option`, so a board can
/// subscribe by it (see "Absence is a sentinel, not a null" in `lib.rs`). Every
/// timestamp is the module clock, in the store's spelling.
#[spacetimedb::table(accessor = dispatch_requests, public)]
#[derive(Clone, Debug)]
pub struct DispatchRequest {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// Indexed: `RequestDispatch` asks "does this task hold an open request?"
    /// and every task delete fails the task's open one.
    #[index(btree)]
    pub task_id: i64,
    /// [`DISPATCH_REQUEST_KIND`] for now; M5 adds wrap-up and exit-session
    /// requests as further values rather than further columns (D1).
    pub kind: String,
    /// The machine that provisions this request: always the one that asked
    /// (D2). Indexed: a board subscribes to its own host's rows, and a
    /// restarting board fails its own host's provisioning ones.
    #[index(btree)]
    pub host: String,
    /// `dispatch`, `research` or `quick` (D3), fixed when the row is written.
    pub mode: String,
    /// `user`, `mcp`, `epic_chain` or `quick` (D9).
    pub origin: String,
    /// `requested`, `provisioning`, `dispatched` or `failed`.
    pub state: String,
    pub requested_at: String,
    pub claimed_at: String,
    pub finished_at: String,
    pub reason: String,
    pub worktree: String,
    pub tmux_window: String,
    /// `requested_at + config.dispatch_request_timeout`, computed by the module
    /// when the row is written and stored so the HOST reads the deadline it
    /// enforces off the row rather than from a copy of the config (D4).
    pub deadline: String,
    /// The identity whose call wrote the request (D11), stamped by the module
    /// from `ctx.sender()`, never sent by the client. A host's board serves
    /// only rows its own owner requested. Appended last, with a default, so
    /// the table migrates in place.
    #[default("")]
    pub requested_by: String,
    /// Whether provisioning reused a worktree that already existed rather
    /// than creating it, as the host reported it
    /// (`HostReportsDispatched`); `false` until then. Appended last, with a
    /// default, so the table migrates in place.
    #[default(false)]
    pub reused_worktree: bool,
}

/// The backstop timer: one scheduled row per open request
/// (`store-split.allium: DispatchRequestTimesOut`, D4). Due at deadline +
/// grace, deleted by whichever rule finishes the request first.
///
/// Public so a board (and the live-store tests) can read it over SQL; only the
/// module writes it.
#[spacetimedb::table(accessor = dispatch_request_timeouts, public, scheduled(expire_dispatch_request))]
#[derive(Clone, Debug)]
pub struct DispatchRequestTimeout {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: spacetimedb::ScheduleAt,
    /// The [`DispatchRequest::id`] this timer fails when it fires. Indexed: a
    /// finishing request cancels its timer by this column.
    #[index(btree)]
    pub request_id: i64,
}

// -- Config and vocabulary ----------------------------------------------------

/// `config.dispatch_request_timeout`: how long a request may stay open,
/// measured from `requested_at`.
pub const DISPATCH_REQUEST_TIMEOUT_SECS: i64 = 60;

/// `config.dispatch_request_backstop_grace`: how long after the deadline the
/// module's timer waits, so a live host's own stop lands first.
pub const DISPATCH_REQUEST_BACKSTOP_GRACE_SECS: i64 = 10;

pub const DISPATCH_REQUEST_KIND: &str = "dispatch";

pub const DISPATCH_REQUESTED: &str = "requested";
pub const DISPATCH_PROVISIONING: &str = "provisioning";
pub const DISPATCH_DISPATCHED: &str = "dispatched";
pub const DISPATCH_FAILED: &str = "failed";

pub const DISPATCH_MODES: [&str; 3] = ["dispatch", "research", "quick"];
pub const DISPATCH_ORIGIN_EPIC_CHAIN: &str = "epic_chain";
pub const DISPATCH_ORIGINS: [&str; 4] = ["user", "mcp", DISPATCH_ORIGIN_EPIC_CHAIN, "quick"];

/// The backstop's reason (`DispatchRequestTimesOut`).
pub const DISPATCH_REASON_TIMED_OUT: &str = "timed out";
/// A restarting board's reason (`HostRestartFailsInterrupted`).
pub const DISPATCH_REASON_HOST_RESTARTED: &str = "host restarted";
/// A deleted task's reason (`DeletingATaskFailsItsOpenRequest`).
pub const DISPATCH_REASON_TASK_DELETED: &str = "task deleted";

const MICROS_PER_SEC: i64 = 1_000_000;

// -- Pure rules ---------------------------------------------------------------

/// `DispatchRequest.is_open`.
pub fn dispatch_request_is_open(state: &str) -> bool {
    state == DISPATCH_REQUESTED || state == DISPATCH_PROVISIONING
}

/// `DispatchRequest.deadline`, in the store's timestamp spelling. An
/// unparseable `requested_at` gives `""`, the module's absent, rather than a
/// panic inside a reducer; a module-clock stamp always parses.
pub fn dispatch_request_deadline(requested_at: &str) -> String {
    match chrono::NaiveDateTime::parse_from_str(requested_at, SQLITE_TIMESTAMP) {
        Ok(at) => format_timestamp_micros(
            at.and_utc().timestamp_micros() + DISPATCH_REQUEST_TIMEOUT_SECS * MICROS_PER_SEC,
        ),
        Err(_) => String::new(),
    }
}

/// When the backstop timer is due: deadline + grace, from the request's
/// `requested_at` in micros since the epoch.
pub fn dispatch_request_backstop_due_micros(requested_at_micros: i64) -> i64 {
    requested_at_micros
        + (DISPATCH_REQUEST_TIMEOUT_SECS + DISPATCH_REQUEST_BACKSTOP_GRACE_SECS) * MICROS_PER_SEC
}

/// `RequestDispatch`'s `DispatchRequest.created(...)`.
pub fn new_dispatch_request(
    task_id: i64,
    host: &str,
    mode: &str,
    origin: &str,
    now: &str,
) -> DispatchRequest {
    DispatchRequest {
        task_id,
        kind: DISPATCH_REQUEST_KIND.into(),
        host: host.into(),
        mode: mode.into(),
        origin: origin.into(),
        state: DISPATCH_REQUESTED.into(),
        requested_at: now.into(),
        deadline: dispatch_request_deadline(now),
        ..blank_dispatch_request()
    }
}

/// Refuse an origin that is not one of the four launch paths (D9).
pub fn validate_dispatch_origin(origin: &str) -> Result<(), String> {
    if DISPATCH_ORIGINS.contains(&origin) {
        Ok(())
    } else {
        Err(format!(
            "unknown dispatch origin {origin:?}; expected one of {DISPATCH_ORIGINS:?}"
        ))
    }
}

/// `RequestDispatch`'s task `requires`, in the order a caller is best told
/// about them. `has_open_request` is whether the task already holds an open
/// request (`OneOpenDispatchRequestPerTask`).
///
/// `epics.allium: PhoenixIsNeverChained` lives here, keyed on the origin: the
/// epic chain's request for a phoenix task is refused like any other refusal,
/// and the chain moves on to its next candidate. Every other origin may take
/// a phoenix task.
pub fn validate_dispatch_request(
    task: &Task,
    requester: &str,
    origin: &str,
    has_open_request: bool,
) -> Result<(), String> {
    validate_claim_acceptance(task, requester)?;
    if has_open_request {
        return Err(format!(
            "task {} already holds an open dispatch request",
            task.id
        ));
    }
    if origin == DISPATCH_ORIGIN_EPIC_CHAIN && task.phoenix {
        return Err(format!(
            "task {} is a phoenix task, which the epic chain never dispatches",
            task.id
        ));
    }
    Ok(())
}

/// D3: the caller's mode when it names one, else today's derivation (a plan
/// -> dispatch, else the research tag -> research, else dispatch). `quick` is
/// never derived.
pub fn choose_dispatch_mode(task: &Task, mode: &str) -> Result<&'static str, String> {
    if mode.is_empty() {
        return Ok(if task.plan_path.is_empty() && task.tag == "research" {
            "research"
        } else {
            "dispatch"
        });
    }
    DISPATCH_MODES
        .iter()
        .copied()
        .find(|m| *m == mode)
        .ok_or_else(|| {
            format!("unknown dispatch mode {mode:?}; expected one of {DISPATCH_MODES:?}")
        })
}

/// The claim's writes as `RequestDispatch` ensures them: today's claim plus
/// the dispatch-claim subagent clear point (`agent-health.allium`).
pub fn apply_dispatch_claim(task: Task, now: &str) -> Task {
    Task {
        live_subagents: 0,
        stop_pending: false,
        ..claimed_task(task, now)
    }
}

/// `core.allium: LocalHostOwnerIsWrittenOnce`, as `register_host` enforces
/// it: `owner` may name only the caller itself, and an owned host's owner is
/// never rewritten — not to another identity, not to empty. A re-register
/// with the same owner (a rename) passes.
///
/// A NEW row must name its owner (`store-split.allium`, D11). Host ids are
/// public, so an ownerless row would go to whichever caller registered the id
/// next with itself as owner — a teammate could take a new machine's host
/// before its own board did, and lock it out. A legacy row written ownerless
/// before this was enforced may still be adopted by the first caller naming
/// itself, and an ownerless re-register of it changes no ownership.
pub fn validate_host_registration(
    existing: Option<&Host>,
    owner: &str,
    caller: &str,
) -> Result<(), String> {
    if !owner.is_empty() && owner != caller {
        return Err(format!(
            "{caller} may not register a host as owned by {owner}; only as its own"
        ));
    }
    if existing.is_none() && owner.is_empty() {
        return Err(
            "a new host must be registered with its owner, the caller's own identity".to_string(),
        );
    }
    if let Some(existing) = existing {
        if !existing.owner.is_empty() && existing.owner != owner {
            return Err(format!(
                "the host is owned by {}, and a host's owner is written once",
                existing.owner
            ));
        }
    }
    Ok(())
}

/// D11: whether `caller` may act for `host_id` — name it as a request's
/// requester, claim its rows, report on them, sweep them. Only the host's
/// registered owner may; a host with no owner yet, or none registered, is
/// refused for everyone. The scheduled backstop is the module's own call and
/// does not come through here.
pub fn validate_host_acting_for(
    host: Option<&Host>,
    host_id: &str,
    caller: &str,
) -> Result<(), String> {
    let Some(host) = host else {
        return Err(format!(
            "host {host_id} is not registered, so no one may act for it"
        ));
    };
    if host.owner.is_empty() {
        return Err(format!(
            "host {host_id} has no owner yet, so no one may act for it"
        ));
    }
    if host.owner != caller {
        return Err(format!(
            "host {host_id} belongs to {}, so {caller} may not act for it",
            host.owner
        ));
    }
    Ok(())
}

/// `HostClaimsDispatch`'s `requires`.
pub fn validate_dispatch_request_claim(row: &DispatchRequest, host: &str) -> Result<(), String> {
    let id = row.id;
    if row.host != host {
        return Err(format!(
            "dispatch request {id} is for {}, so {host} cannot claim it",
            row.host
        ));
    }
    if row.state != DISPATCH_REQUESTED {
        return Err(format!(
            "dispatch request {id} is {} rather than requested, so it is already claimed",
            row.state
        ));
    }
    Ok(())
}

/// `HostClaimsDispatch`'s writes.
pub fn claim_dispatch_request_row(row: DispatchRequest, now: &str) -> DispatchRequest {
    DispatchRequest {
        state: DISPATCH_PROVISIONING.into(),
        claimed_at: now.into(),
        ..row
    }
}

/// The `requires` both host reports share: the row's own host, on a row it is
/// provisioning. A late report, after the row failed, is refused here.
pub fn validate_dispatch_report(row: &DispatchRequest, reporter: &str) -> Result<(), String> {
    let id = row.id;
    if row.host != reporter {
        return Err(format!(
            "dispatch request {id} is for {}, so {reporter} cannot report on it",
            row.host
        ));
    }
    if row.state != DISPATCH_PROVISIONING {
        return Err(format!(
            "dispatch request {id} is {} rather than provisioning, so the report is refused",
            row.state
        ));
    }
    Ok(())
}

/// `HostReportsDispatched`'s D4 `requires`: a dispatched report is in time
/// only while the module clock is before the row's deadline. Both are the
/// store's fixed-width timestamp spelling, so they compare as strings. A row
/// with no deadline (an unparseable `requested_at`) is never too late.
pub fn validate_dispatched_report_in_time(row: &DispatchRequest, now: &str) -> Result<(), String> {
    if !row.deadline.is_empty() && now >= row.deadline.as_str() {
        return Err(format!(
            "dispatch request {}'s deadline {} has passed, so the dispatched report is refused",
            row.id, row.deadline
        ));
    }
    Ok(())
}

/// `HostRepeatsDispatchedReport`: whether a dispatched report repeats the one
/// the row already records — its own host, on a row dispatched with the same
/// worktree and window. Such a report is accepted and changes nothing.
pub fn dispatch_report_is_repeat(
    row: &DispatchRequest,
    reporter: &str,
    worktree: &str,
    tmux_window: &str,
) -> bool {
    row.state == DISPATCH_DISPATCHED
        && row.host == reporter
        && row.worktree == worktree
        && row.tmux_window == tmux_window
}

/// `HostReportsDispatched`'s writes to the row.
pub fn dispatched_request_row(
    row: DispatchRequest,
    worktree: &str,
    tmux_window: &str,
    reused_worktree: bool,
    now: &str,
) -> DispatchRequest {
    DispatchRequest {
        state: DISPATCH_DISPATCHED.into(),
        finished_at: now.into(),
        worktree: worktree.into(),
        tmux_window: tmux_window.into(),
        reused_worktree,
        ..row
    }
}

/// `HostReportsDispatched`'s writes to the task: the worktree, the window and
/// the host paired with the worktree (`HostTracksWorktree`).
pub fn apply_dispatched_to_task(task: Task, row: &DispatchRequest) -> Task {
    Task {
        worktree: row.worktree.clone(),
        tmux_window: row.tmux_window.clone(),
        host: row.host.clone(),
        updated_at: row.finished_at.clone(),
        ..task
    }
}

/// Every failure's writes to the row.
pub fn failed_request_row(row: DispatchRequest, reason: &str, now: &str) -> DispatchRequest {
    DispatchRequest {
        state: DISPATCH_FAILED.into(),
        finished_at: now.into(),
        reason: reason.into(),
        ..row
    }
}

/// `ReleaseClaimOfFailedRequest`: the released task, or `None` when the task
/// is no longer the bare claim (it got a worktree, or it moved on) and must be
/// left alone. `updated_at` is the caller's to stamp.
pub fn release_claim_after_failure(task: Task) -> Option<Task> {
    is_bare_claim(&task).then(|| released_task(task))
}

/// `HostRestartFailsInterrupted`'s selection, over rows already read by the
/// restarting host's index: the provisioning ones.
pub fn interrupted_dispatch_requests(
    rows: impl IntoIterator<Item = DispatchRequest>,
) -> Vec<DispatchRequest> {
    rows.into_iter()
        .filter(|r| r.state == DISPATCH_PROVISIONING)
        .collect()
}

// -- Store helpers ------------------------------------------------------------

/// [`validate_host_acting_for`] against the registry.
fn require_acting_for(ctx: &ReducerContext, host_id: &str) -> Result<(), String> {
    let host = ctx.db.hosts().id().find(&host_id.to_string());
    validate_host_acting_for(host.as_ref(), host_id, &caller_identity(ctx))
}

fn find_request(ctx: &ReducerContext, request_id: i64) -> Result<DispatchRequest, String> {
    ctx.db
        .dispatch_requests()
        .id()
        .find(request_id)
        .ok_or_else(|| format!("dispatch request {request_id} does not exist"))
}

/// Delete every timer row naming `request_id`.
fn cancel_dispatch_request_timer(ctx: &ReducerContext, request_id: i64) {
    for timer in ctx
        .db
        .dispatch_request_timeouts()
        .request_id()
        .filter(&request_id)
        .collect::<Vec<_>>()
    {
        ctx.db
            .dispatch_request_timeouts()
            .scheduled_id()
            .delete(timer.scheduled_id);
    }
}

/// THE one failure path: fail the row, cancel its timer, release the task's
/// claim when it is still the bare claim, and run the epic status rollup for
/// the task's epic chain (`ReleaseClaimOfFailedRequest`), so no board has to
/// recalculate after the module moved its subtask. A reported failure, the
/// backstop, a host restart and a task delete all come through here. A task
/// already deleted has no chain to roll up; its delete rolled it up.
pub(crate) fn fail_dispatch_request(
    ctx: &ReducerContext,
    row: DispatchRequest,
    reason: &str,
) -> Result<(), String> {
    let failed = failed_request_row(row, reason, &now(ctx));
    let stamp = failed.finished_at.clone();
    let task_id = failed.task_id;
    let request_id = failed.id;
    ctx.db.dispatch_requests().id().update(failed);
    cancel_dispatch_request_timer(ctx, request_id);
    if let Some(task) = ctx.db.tasks().id().find(task_id) {
        let epic_id = task.epic_id;
        if let Some(released) = release_claim_after_failure(task) {
            write_task(
                ctx,
                Task {
                    updated_at: stamp,
                    ..released
                },
            )?;
        }
        recalculate_epic_chain(ctx, epic_id);
    }
    Ok(())
}

/// `DeletingATaskFailsItsOpenRequest`: called by every path that deletes a
/// task row, after the row is gone, so the release finds no task.
pub(crate) fn fail_open_dispatch_requests_of(ctx: &ReducerContext, task_id: i64) {
    let open: Vec<DispatchRequest> = ctx
        .db
        .dispatch_requests()
        .task_id()
        .filter(&task_id)
        .filter(|r| dispatch_request_is_open(&r.state))
        .collect();
    for row in open {
        // The task row is already deleted, so the release (the only fallible
        // write) has nothing to write.
        let _ = fail_dispatch_request(ctx, row, DISPATCH_REASON_TASK_DELETED);
    }
}

// -- Reducers -----------------------------------------------------------------

/// `RequestDispatch`: claim the task and write a `requested` row on the
/// requester's host, with its backstop timer, in one transaction. `mode` `""`
/// derives it (D3).
///
/// A refusal is ORDINARY, as with [`claim_backlog_task`]: it is how a caller
/// learns another entry point got there first, and it writes nothing.
#[spacetimedb::reducer]
pub fn request_dispatch(
    ctx: &ReducerContext,
    task_id: i64,
    requester: String,
    origin: String,
    mode: String,
) -> Result<(), String> {
    if requester.is_empty() {
        return Err("a dispatch request must name the requesting host".into());
    }
    require_acting_for(ctx, &requester)?;
    validate_dispatch_origin(&origin)?;
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Err(format!("task {task_id} no longer exists"));
    };
    let has_open = ctx
        .db
        .dispatch_requests()
        .task_id()
        .filter(&task_id)
        .any(|r| dispatch_request_is_open(&r.state));
    validate_dispatch_request(&task, &requester, &origin, has_open)?;
    let mode = choose_dispatch_mode(&task, &mode)?;

    let stamp = now(ctx);
    let epic_id = task.epic_id;
    delete_all_subagents(ctx, task_id);
    write_task(ctx, apply_dispatch_claim(task, &stamp))?;
    // The epic status rollup for the claimed task's epic chain, in this
    // reducer (`RequestDispatch`), like every later move of the request.
    recalculate_epic_chain(ctx, epic_id);

    let row = ctx.db.dispatch_requests().insert(DispatchRequest {
        requested_by: caller_identity(ctx),
        ..new_dispatch_request(task_id, &requester, mode, &origin, &stamp)
    });
    let due = dispatch_request_backstop_due_micros(ctx.timestamp.to_micros_since_unix_epoch());
    ctx.db
        .dispatch_request_timeouts()
        .insert(DispatchRequestTimeout {
            scheduled_id: 0,
            scheduled_at: spacetimedb::Timestamp::from_micros_since_unix_epoch(due).into(),
            request_id: row.id,
        });
    Ok(())
}

/// `HostClaimsDispatch`: the row's host takes it, requested -> provisioning.
/// Of two boards, or a row delivered twice, exactly one claim is accepted.
#[spacetimedb::reducer]
pub fn claim_dispatch_request(
    ctx: &ReducerContext,
    request_id: i64,
    host: String,
) -> Result<(), String> {
    let row = find_request(ctx, request_id)?;
    validate_dispatch_request_claim(&row, &host)?;
    require_acting_for(ctx, &host)?;
    ctx.db
        .dispatch_requests()
        .id()
        .update(claim_dispatch_request_row(row, &now(ctx)));
    Ok(())
}

/// `HostReportsDispatched`: record the result on the row and the task, roll up
/// the task's epic chain, and cancel the timer. A late report — on a row that
/// already failed, or past the row's deadline — is refused. A repeat of the
/// report the row already records is accepted and writes nothing
/// (`HostRepeatsDispatchedReport`), so a host whose report got no answer can
/// simply send it again.
#[spacetimedb::reducer]
pub fn report_dispatch_provisioned(
    ctx: &ReducerContext,
    request_id: i64,
    reporter: String,
    worktree: String,
    tmux_window: String,
    reused_worktree: bool,
) -> Result<(), String> {
    let row = find_request(ctx, request_id)?;
    if dispatch_report_is_repeat(&row, &reporter, &worktree, &tmux_window) {
        return require_acting_for(ctx, &reporter);
    }
    validate_dispatch_report(&row, &reporter)?;
    require_acting_for(ctx, &reporter)?;
    let stamp = now(ctx);
    validate_dispatched_report_in_time(&row, &stamp)?;
    if worktree.is_empty() {
        return Err(format!(
            "dispatch request {request_id}: a dispatched report must name the worktree"
        ));
    }
    let Some(task) = ctx.db.tasks().id().find(row.task_id) else {
        return Err(format!("task {} no longer exists", row.task_id));
    };
    let row = dispatched_request_row(row, &worktree, &tmux_window, reused_worktree, &stamp);
    let epic_id = task.epic_id;
    write_task(ctx, apply_dispatched_to_task(task, &row))?;
    // The epic status rollup for the task's epic chain, in this reducer
    // (`HostReportsDispatched`).
    recalculate_epic_chain(ctx, epic_id);
    cancel_dispatch_request_timer(ctx, row.id);
    ctx.db.dispatch_requests().id().update(row);
    Ok(())
}

/// `HostReportsDispatchFailed`: the host rolled back and says why.
#[spacetimedb::reducer]
pub fn report_dispatch_failed(
    ctx: &ReducerContext,
    request_id: i64,
    reporter: String,
    reason: String,
) -> Result<(), String> {
    let row = find_request(ctx, request_id)?;
    validate_dispatch_report(&row, &reporter)?;
    require_acting_for(ctx, &reporter)?;
    fail_dispatch_request(ctx, row, &reason)
}

/// `HostRestartFailsInterrupted`: a starting board fails its own host's
/// provisioning rows ("host restarted"), releasing their claims at once.
#[spacetimedb::reducer]
pub fn fail_interrupted_dispatch_requests(
    ctx: &ReducerContext,
    host: String,
) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    for row in interrupted_dispatch_requests(ctx.db.dispatch_requests().host().filter(&host)) {
        fail_dispatch_request(ctx, row, DISPATCH_REASON_HOST_RESTARTED)?;
    }
    Ok(())
}

/// `DispatchRequestTimesOut`, the scheduled backstop. Refuses any caller but
/// the module's own scheduler: a client must not be able to time a request
/// out early.
#[spacetimedb::reducer]
pub fn expire_dispatch_request(
    ctx: &ReducerContext,
    timer: DispatchRequestTimeout,
) -> Result<(), String> {
    if ctx.sender() != ctx.database_identity() {
        return Err(
            "expire_dispatch_request is run by the module's scheduler, not by clients".into(),
        );
    }
    match ctx.db.dispatch_requests().id().find(timer.request_id) {
        // Still open: its host never finished it. Otherwise its own report won.
        Some(row) if dispatch_request_is_open(&row.state) => {
            fail_dispatch_request(ctx, row, DISPATCH_REASON_TIMED_OUT)
        }
        _ => Ok(()),
    }
}
