//! Wrap-up and exit-session as request rows, with a scheduled backstop timer.
//!
//! Spec: `docs/specs/store-split.allium` (step M5, task #53258):
//! `SessionRequest`, `RequestWrapUp`, `HostClaimsSessionRequest`,
//! `HostReportsSessionRequestDone`, `HostRepeatsSessionRequestDone`,
//! `HostReportsSessionRequestFailed`, `RequestExitSession`,
//! `SessionRequestTimesOut`, `HostRestartFailsInterruptedSessionRequests`,
//! `DeletingATaskFailsItsOpenSessionRequest` and the invariants
//! `OneOpenSessionRequestPerTask`, `AnExitTokenIsConsumedOnce` and
//! `EveryExitSessionHadAWrapUp`; the close itself is
//! `pr-workflow.allium: ExitSession` and the chain is
//! `epics.allium: AutoDispatchNextSubtask`.
//!
//! Built exactly as `dispatch_request.rs` is: every DECISION is a pure
//! function over rows, testable without a store (`tests.rs`'s
//! `session_requests`); the reducers below only read the rows, call those
//! functions and write what they return. Every way a request fails goes
//! through [`fail_session_request`], so the row and its timer cannot drift
//! apart between failure paths.

use super::*;

// -- Tables -------------------------------------------------------------------

/// One wrap_up, exit_session or close request for one task
/// (`store-split.allium: SessionRequest`), open while `requested` or `running`,
/// kept as history once `done` or `failed`. A done wrap_up row's id IS the exit
/// token (W3).
///
/// Every optional column is `""` for absent and a row reference is `0`, never
/// an `Option`, so a board can subscribe by it (see "Absence is a sentinel, not
/// a null" in `lib.rs`). Every timestamp is the module clock, in the store's
/// spelling.
#[spacetimedb::table(accessor = session_requests, public)]
#[derive(Clone, Debug)]
pub struct SessionRequest {
    #[primary_key]
    #[auto_inc]
    pub id: i64,
    /// Indexed: `RequestWrapUp`, `RequestExitSession` and `RequestCloseTask` ask "does this task
    /// hold an open request?", the token checks read the task's rows, and every
    /// task delete fails the task's open one.
    #[index(btree)]
    pub task_id: i64,
    /// `wrap_up`, `exit_session` or `close`.
    pub kind: String,
    /// `rebase`, `done` or `pr`: on a wrap_up row the one asked for, on an
    /// exit_session row the one its token was issued for, and always `done` on a
    /// close row.
    pub action: String,
    /// The machine holding the task's worktree and window, whose board does the
    /// host work. Indexed: a board subscribes to its own host's rows, and a
    /// restarting board fails its own host's running ones.
    #[index(btree)]
    pub host: String,
    /// The identity whose call wrote the request (D11, W11), stamped by the
    /// module from `ctx.sender()`, never sent by the client.
    pub requested_by: String,
    /// `requested`, `running`, `done` or `failed`.
    pub state: String,
    pub requested_at: String,
    pub claimed_at: String,
    pub finished_at: String,
    pub reason: String,
    /// On an exit_session row with action pr: the PR url the close recorded.
    pub pr_url: String,
    /// On an exit_session row: the wrap_up row whose id was the token, which
    /// this row consumed. `0` on a wrap_up or close row.
    pub wrap_up_request: i64,
    /// On an exit_session or close row: the window the close cleared from the
    /// task, which the host tears down.
    pub tmux_window: String,
    /// On an exit_session row: the subtask the epic chain requested in the same
    /// transaction, or `0` when the chain stopped.
    pub chained_task: i64,
    /// `requested_at` plus the timeout of the row's kind, computed by the
    /// module when the row is written and stored so the HOST reads the
    /// deadline it enforces off the row (W7). `""` on a row with no host work.
    pub deadline: String,
}

/// The backstop timer: one scheduled row per open request with host work
/// (`store-split.allium: SessionRequestTimesOut`, W7). Due at deadline + grace,
/// deleted by whichever rule finishes the request first.
///
/// Public so a board (and the live-store tests) can read it over SQL; only the
/// module writes it.
#[spacetimedb::table(accessor = session_request_timeouts, public, scheduled(expire_session_request))]
#[derive(Clone, Debug)]
pub struct SessionRequestTimeout {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: spacetimedb::ScheduleAt,
    /// The [`SessionRequest::id`] this timer fails when it fires. Indexed: a
    /// finishing request cancels its timer by this column.
    #[index(btree)]
    pub request_id: i64,
}

// -- Config and vocabulary ----------------------------------------------------

/// `config.wrap_up_rebase_timeout`.
pub const WRAP_UP_REBASE_TIMEOUT_SECS: i64 = 120;

/// `config.exit_session_teardown_timeout`.
pub const EXIT_SESSION_TEARDOWN_TIMEOUT_SECS: i64 = 30;

/// `config.session_request_backstop_grace`: how long after the deadline the
/// module's timer waits, so a live host's own stop lands first.
pub const SESSION_REQUEST_BACKSTOP_GRACE_SECS: i64 = 10;

pub const SESSION_KIND_WRAP_UP: &str = "wrap_up";
pub const SESSION_KIND_EXIT_SESSION: &str = "exit_session";
/// `update_tasks`' done-path close (M7, X1): a teardown with no wrap_up behind it.
pub const SESSION_KIND_CLOSE: &str = "close";

pub const SESSION_REQUESTED: &str = "requested";
pub const SESSION_RUNNING: &str = "running";
pub const SESSION_DONE: &str = "done";
pub const SESSION_FAILED: &str = "failed";

pub const ACTION_REBASE: &str = "rebase";
pub const ACTION_DONE: &str = "done";
pub const ACTION_PR: &str = "pr";
pub const WRAP_UP_ACTIONS: [&str; 3] = [ACTION_REBASE, ACTION_DONE, ACTION_PR];

/// The backstop's reason (`SessionRequestTimesOut`).
pub const SESSION_REASON_TIMED_OUT: &str = "timed out";
/// A restarting board's reason (`HostRestartFailsInterruptedSessionRequests`).
pub const SESSION_REASON_HOST_RESTARTED: &str = "host restarted";
/// A closed task's reason (`RequestCloseTask`).
pub const SESSION_REASON_TASK_CLOSED: &str = "task closed";
/// A deleted task's reason (`DeletingATaskFailsItsOpenSessionRequest`).
pub const SESSION_REASON_TASK_DELETED: &str = "task deleted";

/// `RequestExitSession`'s refusals for a token that is absent or spent.
pub const EXIT_TOKEN_MISSING: &str = "no exit token — call wrap_up first";
/// ...and for one that names another row, is superseded or is voided.
pub const EXIT_TOKEN_INVALID: &str = "invalid exit token";

// -- Pure rules ---------------------------------------------------------------

/// `SessionRequest.is_open`.
pub fn session_request_is_open(state: &str) -> bool {
    state == SESSION_REQUESTED || state == SESSION_RUNNING
}

/// `SessionRequest.has_host_work`: a wrap_up with action done or pr is written
/// already done and never does; an exit_session or close row always does.
pub fn session_request_has_host_work(row: &SessionRequest) -> bool {
    row.kind == SESSION_KIND_EXIT_SESSION
        || row.kind == SESSION_KIND_CLOSE
        || row.action == ACTION_REBASE
}

fn session_timeout_secs(kind: &str) -> i64 {
    if kind == SESSION_KIND_WRAP_UP {
        WRAP_UP_REBASE_TIMEOUT_SECS
    } else {
        EXIT_SESSION_TEARDOWN_TIMEOUT_SECS
    }
}

/// `SessionRequest.deadline`, in the store's timestamp spelling. An
/// unparseable `requested_at` gives `""`, the module's absent.
pub fn session_request_deadline(kind: &str, requested_at: &str) -> String {
    match chrono::NaiveDateTime::parse_from_str(requested_at, SQLITE_TIMESTAMP) {
        Ok(at) => format_timestamp_micros(
            at.and_utc().timestamp_micros() + session_timeout_secs(kind) * MICROS_PER_SEC,
        ),
        Err(_) => String::new(),
    }
}

/// When the backstop timer is due: deadline + grace, from the request's
/// `requested_at` in micros since the epoch.
pub fn session_request_backstop_due_micros(kind: &str, requested_at_micros: i64) -> i64 {
    requested_at_micros
        + (session_timeout_secs(kind) + SESSION_REQUEST_BACKSTOP_GRACE_SECS) * MICROS_PER_SEC
}

/// `core/SubStatus.default_for`, in the store's spelling.
pub fn default_sub_status(status: &str) -> &'static str {
    match status {
        RUNNING => ACTIVE,
        REVIEW => AWAITING_REVIEW,
        _ => "none",
    }
}

/// Refuse an action that is not one of the three wrap-up actions.
pub fn validate_wrap_up_action(action: &str) -> Result<(), String> {
    if WRAP_UP_ACTIONS.contains(&action) {
        Ok(())
    } else {
        Err(format!(
            "unknown wrap-up action {action:?}; expected one of {WRAP_UP_ACTIONS:?}"
        ))
    }
}

/// The task's review tags, which are never wrapped up
/// (`mcp-task-tools.allium: ReviewTasksAreNotWrappedUp`).
const REVIEW_TAGS: [&str; 3] = ["pr-review", "dependabot", "release-please"];

/// `RequestWrapUp`'s task `requires`, in the order a caller is best told about
/// them, with the wording the tool gave before the rule moved here.
/// `has_open_request` is whether the task already holds an open session request
/// (`OneOpenSessionRequestPerTask`).
pub fn validate_wrap_up_request(
    task: &Task,
    host: &str,
    has_open_request: bool,
) -> Result<(), String> {
    let id = task.id;
    if (task.status != RUNNING && task.status != REVIEW) || task.worktree.is_empty() {
        return Err(format!(
            "Task {id} cannot be wrapped up. Requires Running or Review status with a worktree."
        ));
    }
    if REVIEW_TAGS.contains(&task.tag.as_str()) {
        return Err(format!(
            "Task {id} is tagged `{}`, so it is not wrapped up. A review task ends when its PR \
             merges — the feed that created it deletes the card on the next poll — or when you \
             hand it back to the user. If it became real code work, retag it \
             (update_tasks(task_id={id}, tag=\"chore\")) and call wrap_up again.",
            task.tag
        ));
    }
    if task.host != host {
        return Err(format!(
            "task {id}'s worktree is on {}, so {host} cannot wrap it up",
            task.host
        ));
    }
    validate_no_open_session_request(id, has_open_request)
}

/// `OneOpenSessionRequestPerTask`, as a refusal.
pub fn validate_no_open_session_request(
    task_id: i64,
    has_open_request: bool,
) -> Result<(), String> {
    if has_open_request {
        return Err(format!(
            "task {task_id} already holds an open session request"
        ));
    }
    Ok(())
}

/// `RequestWrapUp`'s `SessionRequest.created(...)`. A rebase row is requested,
/// for its host to claim; done and pr need no host work, so the row is written
/// already done, with no deadline and no timer (W2).
pub fn new_wrap_up_request(task_id: i64, host: &str, action: &str, now: &str) -> SessionRequest {
    let row = SessionRequest {
        task_id,
        kind: SESSION_KIND_WRAP_UP.into(),
        action: action.into(),
        host: host.into(),
        requested_at: now.into(),
        ..blank_session_request()
    };
    if action == ACTION_REBASE {
        SessionRequest {
            state: SESSION_REQUESTED.into(),
            deadline: session_request_deadline(SESSION_KIND_WRAP_UP, now),
            ..row
        }
    } else {
        SessionRequest {
            state: SESSION_DONE.into(),
            finished_at: now.into(),
            ..row
        }
    }
}

/// `HostClaimsSessionRequest`'s `requires`. The host enforces the deadline
/// (W7), so a row at or past it is not started; the backstop fails it.
pub fn validate_session_request_claim(
    row: &SessionRequest,
    host: &str,
    now: &str,
) -> Result<(), String> {
    let id = row.id;
    if row.host != host {
        return Err(format!(
            "session request {id} is for {}, so {host} cannot claim it",
            row.host
        ));
    }
    if row.state != SESSION_REQUESTED {
        return Err(format!(
            "session request {id} is {} rather than requested, so it is already claimed",
            row.state
        ));
    }
    if !row.deadline.is_empty() && now >= row.deadline.as_str() {
        return Err(format!(
            "session request {id}'s deadline {} has passed, so it is not started",
            row.deadline
        ));
    }
    Ok(())
}

/// `HostClaimsSessionRequest`'s writes to the row.
pub fn claim_session_request_row(row: SessionRequest, now: &str) -> SessionRequest {
    SessionRequest {
        state: SESSION_RUNNING.into(),
        claimed_at: now.into(),
        ..row
    }
}

/// `HostClaimsSessionRequest`'s optimistic clear, moved into the claim: the
/// task a wrap_up claim finds in Conflict, with the sub_status the status
/// defaults to, or `None` when there is nothing to clear.
pub fn clear_conflict_on_claim(kind: &str, task: Task, now: &str) -> Option<Task> {
    (kind == SESSION_KIND_WRAP_UP && task.sub_status == "conflict").then(|| Task {
        sub_status: default_sub_status(&task.status).into(),
        updated_at: now.into(),
        ..task
    })
}

/// The `requires` both host reports share: the row's own host, on a row it is
/// running. A late report, after the row failed, is refused here.
pub fn validate_session_report(row: &SessionRequest, reporter: &str) -> Result<(), String> {
    let id = row.id;
    if row.host != reporter {
        return Err(format!(
            "session request {id} is for {}, so {reporter} cannot report on it",
            row.host
        ));
    }
    if row.state != SESSION_RUNNING {
        return Err(format!(
            "session request {id} is {} rather than running, so the report is refused",
            row.state
        ));
    }
    Ok(())
}

/// `HostReportsSessionRequestDone`'s deadline `requires`: a rebase reported
/// done at or after its deadline is too late and issues no token. A teardown
/// is not held to it: the close stands whatever the teardown does (W5).
pub fn validate_session_done_in_time(row: &SessionRequest, now: &str) -> Result<(), String> {
    if row.kind == SESSION_KIND_WRAP_UP && !row.deadline.is_empty() && now >= row.deadline.as_str()
    {
        return Err(format!(
            "session request {}'s deadline {} has passed, so the done report is refused",
            row.id, row.deadline
        ));
    }
    Ok(())
}

/// `HostRepeatsSessionRequestDone`: whether a done report repeats one the row
/// already records — its own host, on a row it claimed and finished. A wrap_up
/// row written done had no report to repeat.
pub fn session_report_is_repeat(row: &SessionRequest, reporter: &str) -> bool {
    row.state == SESSION_DONE && row.host == reporter && !row.claimed_at.is_empty()
}

/// `HostReportsSessionRequestDone`'s writes to the row.
pub fn done_session_request_row(row: SessionRequest, now: &str) -> SessionRequest {
    SessionRequest {
        state: SESSION_DONE.into(),
        finished_at: now.into(),
        ..row
    }
}

/// Every failure's writes to the row.
pub fn failed_session_request_row(row: SessionRequest, reason: &str, now: &str) -> SessionRequest {
    SessionRequest {
        state: SESSION_FAILED.into(),
        finished_at: now.into(),
        reason: reason.into(),
        ..row
    }
}

/// `HostReportsSessionRequestFailed`'s `requires`: only a rebase can conflict.
pub fn validate_session_failure_report(row: &SessionRequest, conflict: bool) -> Result<(), String> {
    if conflict && row.kind != SESSION_KIND_WRAP_UP {
        return Err(format!(
            "session request {} is an {}, which cannot report a rebase conflict",
            row.id, row.kind
        ));
    }
    Ok(())
}

/// `FinishTaskConflict`: the task with its sub_status set to Conflict, or
/// `None` when it is no longer running or in review (Conflict is legal only
/// there), so a task moved on meanwhile is left alone.
pub fn apply_rebase_conflict(task: Task, now: &str) -> Option<Task> {
    (task.status == RUNNING || task.status == REVIEW).then(|| Task {
        sub_status: "conflict".into(),
        updated_at: now.into(),
        ..task
    })
}

/// `SessionRequestTimesOut`'s body: the row failed as timed out, or `None` when
/// it already finished (its own report won).
pub fn expire_session_request_row(row: SessionRequest, now: &str) -> Option<SessionRequest> {
    session_request_is_open(&row.state)
        .then(|| failed_session_request_row(row, SESSION_REASON_TIMED_OUT, now))
}

/// `HostRestartFailsInterruptedSessionRequests`'s selection, over rows already
/// read by the restarting host's index: the running ones.
pub fn interrupted_session_requests(
    rows: impl IntoIterator<Item = SessionRequest>,
) -> Vec<SessionRequest> {
    rows.into_iter()
        .filter(|r| r.state == SESSION_RUNNING)
        .collect()
}

// -- The exit token -------------------------------------------------------------

/// `SessionRequest.is_valid_token` and `RequestExitSession`'s token refusals
/// (W3, W4), over every session row and every dispatch request of one task.
///
/// Absent or spent is "no exit token"; naming another row, being superseded by a
/// newer done wrap_up (newer by `requested_at`, ties broken by row id) or being
/// voided by a later dispatch is "invalid exit token". A task whose done
/// wrap_up rows are all consumed is absent whatever `token_id` names.
pub fn check_exit_token(
    task_id: i64,
    token_id: i64,
    sessions: &[SessionRequest],
    dispatches: &[DispatchRequest],
) -> Result<SessionRequest, String> {
    let consumed = |id: i64| {
        sessions
            .iter()
            .any(|r| r.kind == SESSION_KIND_EXIT_SESSION && r.wrap_up_request == id)
    };
    let done_wrap_ups = || {
        sessions.iter().filter(|r| {
            r.task_id == task_id && r.kind == SESSION_KIND_WRAP_UP && r.state == SESSION_DONE
        })
    };
    if done_wrap_ups().all(|r| consumed(r.id)) {
        return Err(EXIT_TOKEN_MISSING.into());
    }
    let Some(token) = done_wrap_ups().find(|r| r.id == token_id) else {
        return Err(EXIT_TOKEN_INVALID.into());
    };
    if consumed(token.id) {
        return Err(EXIT_TOKEN_MISSING.into());
    }
    let superseded = done_wrap_ups()
        .any(|r| (r.requested_at.as_str(), r.id) > (token.requested_at.as_str(), token.id));
    let voided = dispatches.iter().any(|d| {
        d.task_id == task_id
            && d.state == DISPATCH_DISPATCHED
            && d.finished_at.as_str() >= token.finished_at.as_str()
    });
    if superseded || voided {
        return Err(EXIT_TOKEN_INVALID.into());
    }
    Ok(token.clone())
}

/// `RequestExitSession`'s remaining task `requires`, after the token, in the
/// order the spec lists them: the action matches the token's, the task has a
/// live window, and a pr close names its url.
pub fn validate_exit_session(
    task: &Task,
    window: &str,
    token: &SessionRequest,
    action: &str,
    pr_url: &str,
) -> Result<(), String> {
    if action != token.action {
        return Err(format!(
            "exit token was issued for wrap_up(action=\"{}\"), but exit_session was called with action=\"{action}\"",
            token.action
        ));
    }
    if window.is_empty() {
        return Err(format!("task #{} has no active session", task.id));
    }
    if action == ACTION_PR && pr_url.trim().is_empty() {
        return Err(
            "pr_url is required for action 'pr' — pass the URL returned by `gh pr create`".into(),
        );
    }
    Ok(())
}

// -- The close ------------------------------------------------------------------

/// `ExitSession`'s writes to the task: the window cleared, then pr -> review
/// with a pr-typed url, rebase or done -> done stamped complete. Both leave
/// running, so both clear a deferred Stop.
pub fn apply_session_close(task: Task, action: &str, pr_url: &str, now: &str) -> Task {
    let closed = Task {
        updated_at: now.into(),
        ..task
    };
    if action == ACTION_PR {
        Task {
            status: REVIEW.into(),
            sub_status: default_sub_status(REVIEW).into(),
            url: pr_url.into(),
            url_type: "pr".into(),
            ..closed
        }
    } else {
        Task {
            status: DONE.into(),
            sub_status: default_sub_status(DONE).into(),
            completed_at: now.into(),
            ..closed
        }
    }
}

/// `ExitSession`'s `exit_session` row (`SessionRequest.created(...)`): names the
/// task's host, the window to tear down, the token it consumes and the
/// subtask the chain requested (`0` for none). `task` is the task as it was
/// BEFORE the close, so the window is the one the close cleared.
pub fn new_exit_session_request(
    task: &Task,
    window: &str,
    token_id: i64,
    action: &str,
    pr_url: &str,
    chained_task: i64,
    now: &str,
) -> SessionRequest {
    SessionRequest {
        task_id: task.id,
        kind: SESSION_KIND_EXIT_SESSION.into(),
        action: action.into(),
        host: task.host.clone(),
        state: SESSION_REQUESTED.into(),
        requested_at: now.into(),
        pr_url: if action == ACTION_PR {
            pr_url.into()
        } else {
            String::new()
        },
        wrap_up_request: token_id,
        tmux_window: window.into(),
        chained_task,
        deadline: session_request_deadline(SESSION_KIND_EXIT_SESSION, now),
        ..blank_session_request()
    }
}

/// `RequestCloseTask`'s terminal write: ExitSession's fields for action done,
/// except that a done -> done re-close is no transition and keeps the task's
/// `completed_at`.
pub fn apply_done_close(task: Task, now: &str) -> Task {
    let was_already_done = task.status == DONE;
    let prior_status = task.status.clone();
    let kept = task.completed_at.clone();
    // schedules.allium: LeavingBacklogCancelsThePendingRun.
    let closed = cancel_pending_run_on_leaving_backlog(
        &prior_status,
        apply_session_close(task, ACTION_DONE, "", now),
    );
    if was_already_done {
        Task {
            completed_at: kept,
            ..closed
        }
    } else {
        closed
    }
}

/// Whether a close has host work: a live window to kill or a pr-typed url to
/// close (`RequestCloseTask`'s `needs_teardown`).
pub fn close_has_host_work(task: &Task, window: &str) -> bool {
    !window.is_empty() || (!task.url.is_empty() && task.url_type == "pr")
}

/// `RequestCloseTask`'s close row. `task` is the task as it was BEFORE the
/// close, so the window is the one the close cleared; the host is the task's
/// own, else the caller's.
pub fn new_close_request(
    task: &Task,
    window: &str,
    caller_host: &str,
    now: &str,
) -> SessionRequest {
    SessionRequest {
        task_id: task.id,
        kind: SESSION_KIND_CLOSE.into(),
        action: ACTION_DONE.into(),
        host: if task.host.is_empty() {
            caller_host.into()
        } else {
            task.host.clone()
        },
        state: SESSION_REQUESTED.into(),
        requested_at: now.into(),
        tmux_window: window.into(),
        deadline: session_request_deadline(SESSION_KIND_CLOSE, now),
        ..blank_session_request()
    }
}

/// `patch_task`'s transition rule that is not the patch's own fields (M7,
/// X2): leaving running clears the current agent's deferred Stop
/// (`core.allium: PendingStopOnlyWhileRunning`) unless the patch names
/// `stop_pending` itself.
pub fn patch_voids_pending_stop(
    prior_status: &str,
    new_status: &str,
    patch_named_stop_pending: bool,
) -> bool {
    prior_status == RUNNING && new_status != RUNNING && !patch_named_stop_pending
}

/// `PhoenixRespawn`'s trigger for a close: the task entered Done from another
/// status, carries the flag and is not feed-owned.
pub fn should_respawn_phoenix(prior_status: &str, closed: &Task) -> bool {
    closed.status == DONE && prior_status != DONE && closed.phoenix && closed.external_id.is_empty()
}

/// `PhoenixRespawn`'s successor: everything the operator configured and nothing
/// the previous run produced. `epic_id` is where the successor lands (already
/// routed into a repo group), and a task on a user board keeps its owner.
pub fn phoenix_successor(predecessor: &Task, epic_id: i64, now: &str, created_by: &str) -> Task {
    Task {
        id: 0,
        title: predecessor.title.clone(),
        description: predecessor.description.clone(),
        repo_path: predecessor.repo_path.clone(),
        plan_path: predecessor.plan_path.clone(),
        epic_id,
        tag: predecessor.tag.clone(),
        labels: predecessor.labels.clone(),
        base_branch: predecessor.base_branch.clone(),
        wrap_up_mode: predecessor.wrap_up_mode.clone(),
        model: predecessor.model.clone(),
        auto_run_plan: predecessor.auto_run_plan,
        phoenix: true,
        // schedules.allium (S3-S5): the schedule repeats with the flag. The
        // write that inserts the successor arms it from the respawn's moment
        // (`store_task`, `arm_created_task`).
        schedule: predecessor.schedule.clone(),
        created_at: now.into(),
        updated_at: now.into(),
        created_by: created_by.into(),
        owner: if epic_id == 0 {
            predecessor.owner.clone()
        } else {
            String::new()
        },
        ..blank_task()
    }
}

/// `AutoDispatchNextSubtask`'s candidates: the epic's backlog subtasks in board
/// order, `sort_order` (nulls fall back to the task id), then id. Phoenix
/// subtasks are refused by the request itself, not filtered here.
pub fn chain_candidates(tasks: Vec<Task>) -> Vec<Task> {
    let mut candidates: Vec<Task> = tasks.into_iter().filter(|t| t.status == BACKLOG).collect();
    candidates.sort_by_key(|t| (t.sort_order.unwrap_or(t.id), t.id));
    candidates
}

// -- Store helpers ------------------------------------------------------------

fn find_session_request(ctx: &ReducerContext, request_id: i64) -> Result<SessionRequest, String> {
    ctx.db
        .session_requests()
        .id()
        .find(request_id)
        .ok_or_else(|| format!("session request {request_id} does not exist"))
}

fn task_has_open_session_request(ctx: &ReducerContext, task_id: i64) -> bool {
    ctx.db
        .session_requests()
        .task_id()
        .filter(&task_id)
        .any(|r| session_request_is_open(&r.state))
}

/// Delete every timer row naming `request_id`.
fn cancel_session_request_timer(ctx: &ReducerContext, request_id: i64) {
    for timer in ctx
        .db
        .session_request_timeouts()
        .request_id()
        .filter(&request_id)
        .collect::<Vec<_>>()
    {
        ctx.db
            .session_request_timeouts()
            .scheduled_id()
            .delete(timer.scheduled_id);
    }
}

/// Schedule the backstop for a row with host work.
fn schedule_session_request_timer(ctx: &ReducerContext, row: &SessionRequest) {
    let due =
        session_request_backstop_due_micros(&row.kind, ctx.timestamp.to_micros_since_unix_epoch());
    ctx.db
        .session_request_timeouts()
        .insert(SessionRequestTimeout {
            scheduled_id: 0,
            scheduled_at: spacetimedb::Timestamp::from_micros_since_unix_epoch(due).into(),
            request_id: row.id,
        });
}

/// THE one failure path: fail the row and cancel its timer. A reported failure,
/// the backstop, a host restart and a task delete all come through here. A
/// failed session request releases no claim and moves no task (a conflict
/// report sets the sub_status in its own reducer).
pub(crate) fn fail_session_request(ctx: &ReducerContext, row: SessionRequest, reason: &str) {
    let request_id = row.id;
    ctx.db
        .session_requests()
        .id()
        .update(failed_session_request_row(row, reason, &now(ctx)));
    cancel_session_request_timer(ctx, request_id);
}

/// `RequestCloseTask`'s first write: the task's open session request, if any,
/// failed "task closed" (its timer cancelled).
fn fail_open_session_requests_for_close(ctx: &ReducerContext, task_id: i64) {
    let open: Vec<SessionRequest> = ctx
        .db
        .session_requests()
        .task_id()
        .filter(&task_id)
        .filter(|r| session_request_is_open(&r.state))
        .collect();
    for row in open {
        fail_session_request(ctx, row, SESSION_REASON_TASK_CLOSED);
    }
}

/// `DeletingATaskFailsItsOpenSessionRequest`: called by every path that deletes
/// a task row (through `delete_task_side_effects`).
pub(crate) fn fail_open_session_requests_of(ctx: &ReducerContext, task_id: i64) {
    let open: Vec<SessionRequest> = ctx
        .db
        .session_requests()
        .task_id()
        .filter(&task_id)
        .filter(|r| session_request_is_open(&r.state))
        .collect();
    for row in open {
        fail_session_request(ctx, row, SESSION_REASON_TASK_DELETED);
    }
}

/// `PhoenixRespawn` inside the close: the successor row, and whether it landed
/// (`TheFlagIsTheReceipt`). A failure is swallowed: Done outranks the respawn.
pub(crate) fn respawn_phoenix_successor_of(
    ctx: &ReducerContext,
    closed: &Task,
    stamp: &str,
) -> bool {
    let epic_id = route_to_repo_group(ctx, closed.epic_id, &closed.repo_path);
    let successor = phoenix_successor(closed, epic_id, stamp, &caller_identity(ctx));
    write_task(ctx, successor).is_ok()
}

/// `AutoDispatchNextSubtask`: the first backlog subtask of the closed task's
/// epic whose `epic_chain` request is accepted, requested on the closed task's
/// host through the shared dispatch path. `0` when the chain stops. A refused
/// candidate is ordinary and the chain moves on; nothing here fails the close.
fn chain_next_subtask(ctx: &ReducerContext, closed: &Task, host: &str) -> i64 {
    if closed.epic_id == 0 {
        return 0;
    }
    match ctx.db.epics().id().find(closed.epic_id) {
        Some(epic) if epic.auto_dispatch => {}
        _ => return 0,
    }
    let subtasks: Vec<Task> = ctx.db.tasks().epic_id().filter(&closed.epic_id).collect();
    let stamp = now(ctx);
    for candidate in chain_candidates(subtasks) {
        let has_open = ctx
            .db
            .dispatch_requests()
            .task_id()
            .filter(&candidate.id)
            .any(|r| dispatch_request_is_open(&r.state));
        if validate_dispatch_request(
            &candidate,
            host,
            DISPATCH_ORIGIN_EPIC_CHAIN,
            has_open,
            &stamp,
        )
        .is_err()
        {
            continue;
        }
        let Ok(mode) = choose_dispatch_mode(&candidate, "") else {
            continue;
        };
        let id = candidate.id;
        if create_dispatch_request(ctx, candidate, host, mode, DISPATCH_ORIGIN_EPIC_CHAIN).is_ok() {
            return id;
        }
    }
    0
}

// -- Reducers -----------------------------------------------------------------

/// `RequestWrapUp`: the host-local checks have already passed in the board.
/// Writes a `requested` row with its backstop timer for a rebase, or a row
/// already `done` for done and pr, in one transaction. Writes nothing to the
/// task: the Conflict clear waits for the claim.
#[spacetimedb::reducer]
pub fn request_wrap_up(
    ctx: &ReducerContext,
    task_id: i64,
    host: String,
    action: String,
) -> Result<(), String> {
    if host.is_empty() {
        return Err("a wrap-up request must name the requesting host".into());
    }
    require_acting_for(ctx, &host)?;
    validate_wrap_up_action(&action)?;
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Err(format!("task {task_id} no longer exists"));
    };
    validate_wrap_up_request(&task, &host, task_has_open_session_request(ctx, task_id))?;

    prune_terminal_requests(ctx);
    let row = ctx.db.session_requests().insert(SessionRequest {
        requested_by: caller_identity(ctx),
        ..new_wrap_up_request(task_id, &host, &action, &now(ctx))
    });
    if session_request_has_host_work(&row) {
        schedule_session_request_timer(ctx, &row);
    }
    Ok(())
}

/// `HostClaimsSessionRequest`: the row's host takes it, requested -> running,
/// clearing a Conflict sub_status in the same transaction. Of two boards, or a
/// row delivered twice, exactly one claim is accepted.
#[spacetimedb::reducer]
pub fn claim_session_request(
    ctx: &ReducerContext,
    request_id: i64,
    host: String,
) -> Result<(), String> {
    let row = find_session_request(ctx, request_id)?;
    let stamp = now(ctx);
    validate_session_request_claim(&row, &host, &stamp)?;
    require_acting_for(ctx, &host)?;
    if let Some(task) = ctx.db.tasks().id().find(row.task_id) {
        if let Some(cleared) = clear_conflict_on_claim(&row.kind, task, &stamp) {
            write_task(ctx, cleared)?;
        }
    }
    ctx.db
        .session_requests()
        .id()
        .update(claim_session_request_row(row, &stamp));
    Ok(())
}

/// `HostReportsSessionRequestDone`: the host finished the rebase (a valid exit
/// token from now on) or the teardown. A late rebase report is refused; a
/// repeat of a report the row already records is accepted and writes nothing
/// (`HostRepeatsSessionRequestDone`).
#[spacetimedb::reducer]
pub fn report_session_request_done(
    ctx: &ReducerContext,
    request_id: i64,
    reporter: String,
) -> Result<(), String> {
    let row = find_session_request(ctx, request_id)?;
    if session_report_is_repeat(&row, &reporter) {
        return require_acting_for(ctx, &reporter);
    }
    validate_session_report(&row, &reporter)?;
    require_acting_for(ctx, &reporter)?;
    validate_session_done_in_time(&row, &now(ctx))?;
    cancel_session_request_timer(ctx, row.id);
    ctx.db
        .session_requests()
        .id()
        .update(done_session_request_row(row, &now(ctx)));
    Ok(())
}

/// `HostReportsSessionRequestFailed`: the host says why, and whether it was a
/// rebase conflict, which makes the module set the Conflict sub_status
/// (`FinishTaskConflict`) in this transaction.
#[spacetimedb::reducer]
pub fn report_session_request_failed(
    ctx: &ReducerContext,
    request_id: i64,
    reporter: String,
    reason: String,
    conflict: bool,
) -> Result<(), String> {
    let row = find_session_request(ctx, request_id)?;
    validate_session_report(&row, &reporter)?;
    validate_session_failure_report(&row, conflict)?;
    require_acting_for(ctx, &reporter)?;
    let task_id = row.task_id;
    fail_session_request(ctx, row, &reason);
    if conflict {
        if let Some(task) = ctx.db.tasks().id().find(task_id) {
            if let Some(conflicted) = apply_rebase_conflict(task, &now(ctx)) {
                write_task(ctx, conflicted)?;
            }
        }
    }
    Ok(())
}

/// `RequestExitSession`, with the close it asks for (`ExitSession`) and the
/// epic chain (`AutoDispatchNextSubtask`): ONE transaction, so the close is
/// whole or absent. A refusal writes nothing and leaves the token valid.
#[spacetimedb::reducer]
pub fn request_exit_session(
    ctx: &ReducerContext,
    task_id: i64,
    token: i64,
    action: String,
    pr_url: String,
) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Err(format!("task {task_id} no longer exists"));
    };
    let sessions: Vec<SessionRequest> = ctx
        .db
        .session_requests()
        .task_id()
        .filter(&task_id)
        .collect();
    let dispatches: Vec<DispatchRequest> = ctx
        .db
        .dispatch_requests()
        .task_id()
        .filter(&task_id)
        .collect();
    let token_row = check_exit_token(task_id, token, &sessions, &dispatches)?;
    let window = session_window(&AgentTables(ctx), task_id);
    validate_exit_session(&task, &window, &token_row, &action, &pr_url)?;
    // D11 (W11): the row names the task's host, whose board tears the window
    // down, so only that host's owner may ask.
    require_acting_for(ctx, &task.host)?;
    validate_no_open_session_request(
        task_id,
        sessions.iter().any(|r| session_request_is_open(&r.state)),
    )?;

    let stamp = now(ctx);
    let host = task.host.clone();
    let epic_id = task.epic_id;
    let mut closed = apply_session_close(task.clone(), &action, &pr_url, &stamp);
    // The respawn follows the Done write in spirit and lands in the same
    // transaction; where it sits relative to the chain cannot matter (the
    // chain skips phoenix subtasks).
    if should_respawn_phoenix(&task.status, &closed)
        && respawn_phoenix_successor_of(ctx, &closed, &stamp)
    {
        closed.phoenix = false;
    }
    write_task(ctx, closed.clone())?;
    close_agent_session(&mut AgentTables(ctx), task_id);
    recalculate_epic_chain(ctx, epic_id);

    let chained = chain_next_subtask(ctx, &closed, &host);
    let row = ctx.db.session_requests().insert(SessionRequest {
        requested_by: caller_identity(ctx),
        ..new_exit_session_request(
            &task,
            &window,
            token_row.id,
            &action,
            &pr_url,
            chained,
            &stamp,
        )
    });
    schedule_session_request_timer(ctx, &row);
    Ok(())
}

/// `RequestCloseTask` (`update_tasks`' done path): ONE transaction that fails
/// the task's open session request ("task closed"), applies the Done close,
/// respawns a phoenix task that enters Done, rolls up the epic chain and, when
/// there is host work and the teardown host is the caller's, writes a `close`
/// row with its backstop timer. No token is issued, checked or consumed and no
/// epic chain runs. A missing task is refused.
#[spacetimedb::reducer]
pub fn close_task(ctx: &ReducerContext, task_id: i64, host: String) -> Result<(), String> {
    let Some(task) = ctx.db.tasks().id().find(task_id) else {
        return Err(format!("task {task_id} no longer exists"));
    };
    let stamp = now(ctx);
    fail_open_session_requests_for_close(ctx, task_id);
    let window = session_window(&AgentTables(ctx), task_id);

    let epic_id = task.epic_id;
    let mut closed = apply_done_close(task.clone(), &stamp);
    if should_respawn_phoenix(&task.status, &closed)
        && respawn_phoenix_successor_of(ctx, &closed, &stamp)
    {
        closed.phoenix = false;
    }
    write_task(ctx, closed)?;
    // store-split.allium: CloseFailsOpenRequests, for dispatch requests too.
    fail_open_dispatch_requests_of(ctx, task_id, DISPATCH_REASON_TASK_CLOSED);
    close_agent_session(&mut AgentTables(ctx), task_id);
    recalculate_epic_chain(ctx, epic_id);

    let row = new_close_request(&task, &window, &host, &stamp);
    // D11, without erroring: a row naming a host makes that host's board act,
    // so only its owner may write one; a non-owner just gets no row.
    if close_has_host_work(&task, &window) && require_acting_for(ctx, &row.host).is_ok() {
        let row = ctx.db.session_requests().insert(SessionRequest {
            requested_by: caller_identity(ctx),
            ..row
        });
        schedule_session_request_timer(ctx, &row);
    }
    Ok(())
}

/// `HostRestartFailsInterruptedSessionRequests`: a starting board fails its own
/// host's running rows ("host restarted").
#[spacetimedb::reducer]
pub fn fail_interrupted_session_requests(ctx: &ReducerContext, host: String) -> Result<(), String> {
    require_acting_for(ctx, &host)?;
    for row in interrupted_session_requests(ctx.db.session_requests().host().filter(&host)) {
        fail_session_request(ctx, row, SESSION_REASON_HOST_RESTARTED);
    }
    Ok(())
}

/// `SessionRequestTimesOut`, the scheduled backstop. Refuses any caller but the
/// module's own scheduler: a client must not be able to time a request out
/// early.
#[spacetimedb::reducer]
pub fn expire_session_request(
    ctx: &ReducerContext,
    timer: SessionRequestTimeout,
) -> Result<(), String> {
    if ctx.sender() != ctx.database_identity() {
        return Err(
            "expire_session_request is run by the module's scheduler, not by clients".into(),
        );
    }
    if let Some(row) = ctx.db.session_requests().id().find(timer.request_id) {
        if session_request_is_open(&row.state) {
            fail_session_request(ctx, row, SESSION_REASON_TIMED_OUT);
        }
    }
    Ok(())
}
