//! The in-memory twin of the M5 session-request reducers.
//!
//! Spec: `docs/specs/store-split.allium` — `RequestWrapUp`,
//! `HostClaimsSessionRequest`, `HostReportsSessionRequestDone`,
//! `HostRepeatsSessionRequestDone`, `HostReportsSessionRequestFailed`,
//! `RequestExitSession`, `SessionRequestTimesOut`,
//! `HostRestartFailsInterruptedSessionRequests`,
//! `DeletingATaskFailsItsOpenSessionRequest` and the invariants
//! `OneOpenSessionRequestPerTask`, `AnExitTokenIsConsumedOnce` and
//! `EveryExitSessionHadAWrapUp`; the close is `pr-workflow.allium:
//! ExitSession`. The board's tests drive wrap-up and exit-session through this
//! double, so it must refuse, close and chain exactly where the module does;
//! `tests/memory_caller_conformance.rs` diffs the two.
//!
//! `expire_session_request` is the double's stand-in for the module's
//! scheduler, as `expire_dispatch_request` is for M4's.

use super::*;
use crate::models::{
    DispatchOrigin, DispatchRequestState, SessionRequestId, SessionRequestKind,
    SessionRequestState, SubStatus, TaskStatus, WrapUpMode,
};

const HOST: &str = "host-a";
const OTHER_HOST: &str = "host-b";
/// The identity this double's calls are sent as, and the owner of both hosts.
const ME: &str = "c0ffee";

fn caller() -> (MemoryReducerCaller, Arc<SharedRows>) {
    let (caller, rows) = super::caller();
    let caller = caller
        .with_sender(ME)
        .with_registered_host(HOST, ME)
        .with_registered_host(OTHER_HOST, ME);
    (caller, rows)
}

/// [`caller`] on a clock the test can move, for the deadline.
fn caller_with_clock() -> (MemoryReducerCaller, Arc<SharedRows>, Arc<FixedClock>) {
    let rows = Arc::new(SharedRows::new());
    let clock = Arc::new(FixedClock::new(
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ));
    let caller = MemoryReducerCaller::new(rows.clone(), clock.clone())
        .with_sender(ME)
        .with_registered_host(HOST, ME)
        .with_registered_host(OTHER_HOST, ME);
    (caller, rows, clock)
}

fn applied_id(outcome: ReducerOutcome) -> SessionRequestId {
    match outcome {
        ReducerOutcome::Applied(ids) => {
            assert_eq!(ids.len(), 1, "a request reads back one id: {ids:?}");
            SessionRequestId(ids[0])
        }
        ReducerOutcome::Refused(why) => panic!("expected the request to apply, refused: {why}"),
    }
}

fn refusal(outcome: ReducerOutcome) -> String {
    match outcome {
        ReducerOutcome::Refused(why) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A task running on [`HOST`] with a worktree and a window, the state every
/// wrap-up starts from. Patched in directly rather than dispatched: this
/// double's fixed clock stamps a dispatch and the wrap-up that follows alike,
/// and a dispatch finishing at or after the token voids it (W3).
async fn dispatched_task(caller: &MemoryReducerCaller) -> TaskId {
    dispatched_task_in(caller, blank_task()).await
}

async fn dispatched_task_in(caller: &MemoryReducerCaller, row: bindings::Task) -> TaskId {
    let task = caller.create_task(row).await.unwrap();
    caller
        .patch_task(
            task,
            bindings::TaskPatch {
                status: Some("running".into()),
                sub_status: Some("active".into()),
                worktree: Some(format!("/repo/.worktrees/{}-t", task.0)),
                tmux_window: Some(format!("task-{}", task.0)),
                host: Some(HOST.into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap()
        .applied()
        .unwrap();
    task
}

async fn wrap_up(caller: &MemoryReducerCaller, task: TaskId, action: &str) -> ReducerOutcome {
    caller
        .request_wrap_up(task, HOST.into(), action.into())
        .await
        .unwrap()
}

/// A done wrap_up row for `task`: its id is the exit token.
async fn token_for(caller: &MemoryReducerCaller, task: TaskId, action: &str) -> SessionRequestId {
    let id = applied_id(wrap_up(caller, task, action).await);
    if action == "rebase" {
        assert!(caller
            .claim_session_request(id, HOST.into())
            .await
            .unwrap()
            .won());
        assert!(caller
            .report_session_request_done(id, HOST.into())
            .await
            .unwrap()
            .won());
    }
    id
}

async fn exit(
    caller: &MemoryReducerCaller,
    task: TaskId,
    token: SessionRequestId,
    action: &str,
    pr_url: &str,
) -> ReducerOutcome {
    caller
        .request_exit_session(task, token, action.into(), pr_url.into())
        .await
        .unwrap()
}

// -- RequestWrapUp --------------------------------------------------------------

/// A rebase request is a `requested` row for its host with a deadline 120s
/// after `requested_at` (W7), and it writes NOTHING to the task: the Conflict
/// clear waits for the claim.
#[tokio::test]
async fn a_rebase_wrap_up_writes_a_requested_row_and_leaves_the_task_alone() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let before = rows.task(task).unwrap();

    let id = applied_id(wrap_up(&caller, task, "rebase").await);

    let row = rows
        .session_request(id)
        .expect("the row reaches SharedRows");
    assert_eq!(row.task_id, task);
    assert_eq!(row.kind, SessionRequestKind::WrapUp);
    assert_eq!(row.action, WrapUpMode::Rebase);
    assert_eq!(row.host, HOST);
    assert_eq!(row.requested_by, ME, "stamped from the call, D11");
    assert_eq!(row.state, SessionRequestState::Requested);
    assert!(row.is_open());
    assert_eq!(
        row.deadline.unwrap() - row.requested_at,
        chrono::Duration::seconds(120),
        "config.wrap_up_rebase_timeout, counted from requested_at"
    );
    assert!(row.claimed_at.is_none());
    assert_eq!(rows.task(task).unwrap(), before);
}

/// done and pr need no host work: the row is written already done, with no
/// deadline, and is a valid exit token at once (W2, W3).
#[tokio::test]
async fn a_done_or_pr_wrap_up_is_written_already_done() {
    let (caller, rows) = caller();
    for action in ["done", "pr"] {
        let task = dispatched_task(&caller).await;

        let id = applied_id(wrap_up(&caller, task, action).await);

        let row = rows.session_request(id).unwrap();
        assert_eq!(row.state, SessionRequestState::Done, "{action}");
        assert_eq!(row.action.as_str(), action);
        assert!(row.finished_at.is_some());
        assert!(row.deadline.is_none(), "no host work, no deadline");
        assert!(row.claimed_at.is_none());
        assert!(!row.is_open());
    }
}

/// Every `requires` of `RequestWrapUp` refuses and writes nothing.
#[tokio::test]
async fn a_refused_wrap_up_request_writes_nothing() {
    let (caller, rows) = caller();

    // Not running or review.
    let backlog = caller.create_task(blank_task()).await.unwrap();
    let why = refusal(wrap_up(&caller, backlog, "rebase").await);
    assert!(why.contains("cannot be wrapped up"), "{why}");

    // A review-tagged task has no wrap-up (ReviewTasksAreNotWrappedUp).
    let review = dispatched_task_in(
        &caller,
        bindings::Task {
            tag: "pr-review".into(),
            ..blank_task()
        },
    )
    .await;
    let why = refusal(wrap_up(&caller, review, "done").await);
    assert!(why.contains("pr-review"), "{why}");
    assert!(
        why.contains("retag"),
        "the refusal names the way out: {why}"
    );

    // The worktree is on another machine.
    let foreign = dispatched_task(&caller).await;
    let why = refusal(
        caller
            .request_wrap_up(foreign, OTHER_HOST.into(), "rebase".into())
            .await
            .unwrap(),
    );
    assert!(why.contains(HOST), "{why}");

    // An action nobody knows, and no host at all.
    let task = dispatched_task(&caller).await;
    refusal(wrap_up(&caller, task, "squash").await);
    refusal(
        caller
            .request_wrap_up(task, String::new(), "rebase".into())
            .await
            .unwrap(),
    );
    refusal(
        caller
            .request_wrap_up(TaskId(9999), HOST.into(), "rebase".into())
            .await
            .unwrap(),
    );

    assert!(rows.session_requests().is_empty());
}

/// W1: a task holds at most one open session request; a finished one does not
/// count.
#[tokio::test]
async fn a_second_wrap_up_while_one_is_open_is_refused() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let first = applied_id(wrap_up(&caller, task, "rebase").await);

    let why = refusal(wrap_up(&caller, task, "rebase").await);
    assert!(why.contains("open session request"), "{why}");
    assert_eq!(rows.session_requests().len(), 1);

    caller
        .claim_session_request(first, HOST.into())
        .await
        .unwrap();
    caller
        .report_session_request_failed(first, HOST.into(), "boom".into(), false)
        .await
        .unwrap();
    applied_id(wrap_up(&caller, task, "rebase").await);
}

// -- HostClaimsSessionRequest ----------------------------------------------------

/// Only the row's own host claims it, and only once.
#[tokio::test]
async fn claim_session_request_is_the_hosts_and_happens_once() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);

    assert!(!caller
        .claim_session_request(id, OTHER_HOST.into())
        .await
        .unwrap()
        .won());
    assert!(caller
        .claim_session_request(id, HOST.into())
        .await
        .unwrap()
        .won());
    let row = rows.session_request(id).unwrap();
    assert_eq!(row.state, SessionRequestState::Running);
    assert!(row.claimed_at.is_some());

    assert!(
        !caller
            .claim_session_request(id, HOST.into())
            .await
            .unwrap()
            .won(),
        "a redelivered row is claimed exactly once"
    );
    assert!(!caller
        .claim_session_request(SessionRequestId(9999), HOST.into())
        .await
        .unwrap()
        .won());
}

/// The claim carries today's optimistic Conflict clear: a task in Conflict
/// goes back to its status's default sub-status when the rebase starts, and
/// nothing else about the task changes.
#[tokio::test]
async fn the_claim_clears_a_conflict_sub_status() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    caller
        .patch_task(
            task,
            bindings::TaskPatch {
                sub_status: Some("conflict".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    assert_eq!(
        rows.task(task).unwrap().sub_status,
        SubStatus::Conflict,
        "the request alone leaves the flag"
    );

    caller.claim_session_request(id, HOST.into()).await.unwrap();

    assert_eq!(
        rows.task(task).unwrap().sub_status,
        SubStatus::default_for(TaskStatus::Running)
    );
}

/// W7: the host enforces the deadline, so a row at or past it is not started.
#[tokio::test]
async fn a_claim_at_or_past_the_deadline_is_refused() {
    let (caller, rows, clock) = caller_with_clock();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);

    clock.advance(chrono::Duration::seconds(120));
    let late = caller.claim_session_request(id, HOST.into()).await.unwrap();

    assert!(refusal(late).contains("deadline"));
    assert_eq!(
        rows.session_request(id).unwrap().state,
        SessionRequestState::Requested
    );
}

// -- HostReportsSessionRequestDone -------------------------------------------------

/// A done report finishes the row and is the token from then on.
#[tokio::test]
async fn a_done_report_finishes_the_row() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);

    let unclaimed = caller
        .report_session_request_done(id, HOST.into())
        .await
        .unwrap();
    assert!(!unclaimed.won(), "a requested row has not been claimed yet");

    caller.claim_session_request(id, HOST.into()).await.unwrap();
    assert!(!caller
        .report_session_request_done(id, OTHER_HOST.into())
        .await
        .unwrap()
        .won());
    assert!(caller
        .report_session_request_done(id, HOST.into())
        .await
        .unwrap()
        .won());

    let row = rows.session_request(id).unwrap();
    assert_eq!(row.state, SessionRequestState::Done);
    assert!(row.finished_at.is_some());
    assert!(!row.is_open());
}

/// A rebase reported done at or after its deadline is too late and issues no
/// token; the row stays running for the host's failure report.
#[tokio::test]
async fn a_done_report_past_the_deadline_is_refused() {
    let (caller, rows, clock) = caller_with_clock();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    caller.claim_session_request(id, HOST.into()).await.unwrap();

    clock.advance(chrono::Duration::seconds(120));
    let late = caller
        .report_session_request_done(id, HOST.into())
        .await
        .unwrap();

    assert!(refusal(late).contains("deadline"));
    assert_eq!(
        rows.session_request(id).unwrap().state,
        SessionRequestState::Running
    );
}

/// `HostRepeatsSessionRequestDone`: a repeat of a report the row already
/// records is accepted and changes nothing, whatever the clock says; a done
/// report on a failed row is still refused.
#[tokio::test]
async fn a_repeated_done_report_is_accepted_and_changes_nothing() {
    let (caller, rows, clock) = caller_with_clock();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    caller.claim_session_request(id, HOST.into()).await.unwrap();
    caller
        .report_session_request_done(id, HOST.into())
        .await
        .unwrap();
    let row = rows.session_request(id).unwrap();

    clock.advance(chrono::Duration::seconds(600));
    let again = caller
        .report_session_request_done(id, HOST.into())
        .await
        .unwrap();
    assert!(again.won(), "{again:?}");
    assert_eq!(rows.session_request(id).unwrap(), row);

    let other = dispatched_task(&caller).await;
    let failed = applied_id(wrap_up(&caller, other, "rebase").await);
    caller
        .claim_session_request(failed, HOST.into())
        .await
        .unwrap();
    caller
        .report_session_request_failed(failed, HOST.into(), "x".into(), false)
        .await
        .unwrap();
    assert!(!caller
        .report_session_request_done(failed, HOST.into())
        .await
        .unwrap()
        .won());
}

// -- HostReportsSessionRequestFailed ------------------------------------------------

/// A plain failure finishes the row with its reason and leaves the task alone.
#[tokio::test]
async fn a_failure_report_records_the_reason_and_leaves_the_task() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let before = rows.task(task).unwrap();
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    caller.claim_session_request(id, HOST.into()).await.unwrap();

    assert!(caller
        .report_session_request_failed(id, HOST.into(), "deadline passed".into(), false)
        .await
        .unwrap()
        .won());

    let row = rows.session_request(id).unwrap();
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("deadline passed"));
    assert!(row.finished_at.is_some());
    assert_eq!(rows.task(task).unwrap(), before);
}

/// THE MODULE sets the Conflict sub_status on a conflict report
/// (FinishTaskConflict), only while the task is running or review.
#[tokio::test]
async fn a_conflict_report_sets_the_conflict_sub_status() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    caller.claim_session_request(id, HOST.into()).await.unwrap();

    assert!(caller
        .report_session_request_failed(id, HOST.into(), "Rebase conflict on b in a.rs".into(), true)
        .await
        .unwrap()
        .won());

    assert_eq!(rows.task(task).unwrap().sub_status, SubStatus::Conflict);

    // A task that moved on meanwhile is left alone.
    let moved = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, moved, "rebase").await);
    caller.claim_session_request(id, HOST.into()).await.unwrap();
    caller
        .patch_task(
            moved,
            bindings::TaskPatch {
                status: Some("done".into()),
                sub_status: Some("none".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    caller
        .report_session_request_failed(id, HOST.into(), "conflict".into(), true)
        .await
        .unwrap();
    assert_eq!(rows.task(moved).unwrap().sub_status, SubStatus::None);
}

/// Only a rebase can conflict, and only the row's host, on a running row,
/// reports.
#[tokio::test]
async fn a_conflict_cannot_be_reported_on_an_exit_session_or_by_another_host() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "done").await;
    let close = applied_id(exit(&caller, task, token, "done", "").await);
    caller
        .claim_session_request(close, HOST.into())
        .await
        .unwrap();

    assert!(!caller
        .report_session_request_failed(close, HOST.into(), "x".into(), true)
        .await
        .unwrap()
        .won());
    assert!(!caller
        .report_session_request_failed(close, OTHER_HOST.into(), "x".into(), false)
        .await
        .unwrap()
        .won());
    assert_eq!(
        rows.session_request(close).unwrap().state,
        SessionRequestState::Running
    );
}

// -- RequestExitSession + ExitSession -----------------------------------------------

/// The close, the token and the teardown row are ONE transaction: the task
/// goes Done with its window cleared and a completion stamp, and an
/// exit_session row for its host names the window and the token it consumed.
#[tokio::test]
async fn an_exit_session_applies_the_close_and_writes_the_teardown_row() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "rebase").await;

    let close = applied_id(exit(&caller, task, token, "rebase", "").await);

    let t = rows.task(task).unwrap();
    assert_eq!(t.status, TaskStatus::Done);
    assert_eq!(t.sub_status, SubStatus::default_for(TaskStatus::Done));
    assert!(t.tmux_window.is_none(), "the close clears the window");
    assert!(t.completed_at.is_some());

    let row = rows.session_request(close).unwrap();
    assert_eq!(row.kind, SessionRequestKind::ExitSession);
    assert_eq!(row.action, WrapUpMode::Rebase);
    assert_eq!(row.host, HOST);
    assert_eq!(row.state, SessionRequestState::Requested);
    assert_eq!(
        row.tmux_window.as_ref().map(|w| w.as_str()),
        Some(format!("task-{}", task.0).as_str()),
        "the window the close cleared, for the host to kill"
    );
    assert_eq!(row.wrap_up_request, Some(token));
    assert_eq!(
        row.deadline.unwrap() - row.requested_at,
        chrono::Duration::seconds(30),
        "config.exit_session_teardown_timeout"
    );
    assert!(row.chained_task.is_none());
}

/// action pr takes the task to Review with the pr url and leaves
/// completed_at alone.
#[tokio::test]
async fn a_pr_exit_session_takes_the_task_to_review_with_the_url() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "pr").await;

    let close = applied_id(exit(&caller, task, token, "pr", "https://github.com/o/r/pull/9").await);

    let t = rows.task(task).unwrap();
    assert_eq!(t.status, TaskStatus::Review);
    assert_eq!(
        t.url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/o/r/pull/9")
    );
    assert!(t.url.as_ref().unwrap().is_pr());
    assert!(t.completed_at.is_none());
    assert_eq!(
        rows.session_request(close).unwrap().pr_url.as_deref(),
        Some("https://github.com/o/r/pull/9")
    );
}

/// A token is consumed by the exit_session row that names it (W3), and a spent
/// one reads "no exit token — call wrap_up first".
#[tokio::test]
async fn an_exit_token_is_consumed_by_the_close() {
    let (caller, _rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "done").await;
    applied_id(exit(&caller, task, token, "done", "").await);

    let again = refusal(exit(&caller, task, token, "done", "").await);

    assert_eq!(again, "no exit token — call wrap_up first");
}

/// A refused close writes NOTHING and leaves the token valid for a retry: the
/// state the old board-side close had, "token spent, close not landed", no
/// longer exists.
#[tokio::test]
async fn a_refused_exit_session_leaves_the_task_and_the_token_untouched() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "pr").await;
    let before = rows.task(task).unwrap();
    let rows_before = rows.session_requests().len();

    // The action differs from the token's, and a pr close without a url.
    let why = refusal(exit(&caller, task, token, "done", "").await);
    assert!(why.contains("pr") && why.contains("done"), "{why}");
    let why = refusal(exit(&caller, task, token, "pr", "").await);
    assert!(why.contains("pr_url is required"), "{why}");

    assert_eq!(rows.task(task).unwrap(), before);
    assert_eq!(rows.session_requests().len(), rows_before);
    applied_id(exit(&caller, task, token, "pr", "https://github.com/o/r/pull/9").await);
}

/// No done wrap_up, another row's id, and a task with no window.
#[tokio::test]
async fn an_exit_session_without_a_token_or_a_window_is_refused() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;

    assert_eq!(
        refusal(exit(&caller, task, SessionRequestId(1), "done", "").await),
        "no exit token — call wrap_up first",
        "no wrap_up has finished"
    );

    let token = token_for(&caller, task, "done").await;
    assert_eq!(
        refusal(exit(&caller, task, SessionRequestId(token.0 + 40), "done", "").await),
        "invalid exit token"
    );

    caller
        .patch_task(
            task,
            bindings::TaskPatch {
                tmux_window: Some(String::new()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let why = refusal(exit(&caller, task, token, "done", "").await);
    assert_eq!(why, format!("task #{} has no active session", task.0));
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Running);
}

/// Only the task's most recent done wrap_up is a token (W3).
#[tokio::test]
async fn a_newer_wrap_up_supersedes_the_older_token() {
    let (caller, _rows, clock) = caller_with_clock();
    let task = dispatched_task(&caller).await;
    let older = token_for(&caller, task, "done").await;
    clock.advance(chrono::Duration::seconds(5));
    let newer = token_for(&caller, task, "done").await;

    assert_eq!(
        refusal(exit(&caller, task, older, "done", "").await),
        "invalid exit token"
    );
    applied_id(exit(&caller, task, newer, "done", "").await);
}

/// W3, decided with the user: a LATER DISPATCH voids a token, so a token from
/// an earlier session of a re-dispatched task cannot close the new session.
#[tokio::test]
async fn a_later_dispatch_voids_the_exit_token() {
    let (caller, rows, clock) = caller_with_clock();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "done").await;

    // The task goes back to backlog and is dispatched again.
    clock.advance(chrono::Duration::seconds(5));
    caller
        .patch_task(
            task,
            bindings::TaskPatch {
                status: Some("backlog".into()),
                sub_status: Some("none".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let dispatch = match caller
        .request_dispatch(task, HOST.into(), "user".into(), "dispatch".into())
        .await
        .unwrap()
    {
        ReducerOutcome::Applied(ids) => crate::models::DispatchRequestId(ids[0]),
        other => panic!("{other:?}"),
    };
    caller
        .claim_dispatch_request(dispatch, HOST.into())
        .await
        .unwrap();
    clock.advance(chrono::Duration::seconds(5));
    assert!(caller
        .report_dispatch_provisioned(dispatch, HOST.into(), "/w".into(), "task-9".into(), true)
        .await
        .unwrap()
        .won());
    assert_eq!(
        rows.dispatch_request(dispatch).unwrap().state,
        DispatchRequestState::Dispatched
    );

    assert_eq!(
        refusal(exit(&caller, task, token, "done", "").await),
        "invalid exit token"
    );
}

/// W1: a wrap_up rebase still running holds the task's one open slot, so the
/// close is refused until it ends.
#[tokio::test]
async fn an_open_session_request_refuses_the_close() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "done").await;
    applied_id(wrap_up(&caller, task, "rebase").await);

    let why = refusal(exit(&caller, task, token, "done", "").await);

    assert!(why.contains("open session request"), "{why}");
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Running);
}

/// The close rolls up the epic status in the same transaction.
#[tokio::test]
async fn an_exit_session_recalculates_the_epic() {
    let (caller, rows) = caller();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    let task = dispatched_task_in(
        &caller,
        bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            ..blank_task()
        },
    )
    .await;
    assert_ne!(
        rows.epic(epic).unwrap().status,
        crate::models::TaskStatus::Done,
        "a running subtask keeps the epic open"
    );
    let token = token_for(&caller, task, "done").await;

    applied_id(exit(&caller, task, token, "done", "").await);

    assert_eq!(
        rows.epic(epic).unwrap().status,
        crate::models::TaskStatus::Done,
        "its only subtask is done"
    );
}

/// W6: the module owns the epic chain. A close that had a wrap_up behind it
/// requests the epic's next backlog subtask as an `epic_chain` dispatch request
/// naming the same host, passing over a phoenix subtask, and records it on the
/// exit_session row for the tool's response.
#[tokio::test]
async fn an_exit_session_chains_the_next_subtask_passing_over_a_phoenix_one() {
    let (caller, rows) = caller();
    let epic = caller
        .create_epic(bindings::Epic {
            auto_dispatch: true,
            ..blank_epic()
        })
        .await
        .unwrap();
    let task = dispatched_task_in(
        &caller,
        bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            ..blank_task()
        },
    )
    .await;
    let phoenix = caller
        .create_task(bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            phoenix: true,
            sort_order: Some(1),
            ..blank_task()
        })
        .await
        .unwrap();
    let next = caller
        .create_task(bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            sort_order: Some(2),
            ..blank_task()
        })
        .await
        .unwrap();
    let token = token_for(&caller, task, "done").await;

    let close = applied_id(exit(&caller, task, token, "done", "").await);

    assert_eq!(
        rows.session_request(close).unwrap().chained_task,
        Some(next),
        "the exit_session row names the subtask the chain requested"
    );
    assert_eq!(rows.task(phoenix).unwrap().status, TaskStatus::Backlog);
    let claimed = rows.task(next).unwrap();
    assert_eq!(
        claimed.status,
        TaskStatus::Running,
        "the request is the claim"
    );
    let chained: Vec<_> = rows
        .dispatch_requests()
        .into_iter()
        .filter(|r| r.task_id == next)
        .collect();
    assert_eq!(chained.len(), 1);
    assert_eq!(chained[0].origin, DispatchOrigin::EpicChain);
    assert_eq!(chained[0].host, HOST);
    assert_eq!(chained[0].state, DispatchRequestState::Requested);
}

/// An epic that does not auto-dispatch, or one with nothing left in backlog,
/// stops the chain; the close stands.
#[tokio::test]
async fn the_chain_stops_without_auto_dispatch_or_a_backlog_subtask() {
    let (caller, rows) = caller();
    let epic = caller.create_epic(blank_epic()).await.unwrap();
    let task = dispatched_task_in(
        &caller,
        bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            ..blank_task()
        },
    )
    .await;
    let waiting = caller
        .create_task(bindings::Task {
            epic_id: epic.0,
            owner: String::new(),
            ..blank_task()
        })
        .await
        .unwrap();
    let token = token_for(&caller, task, "done").await;

    let close = applied_id(exit(&caller, task, token, "done", "").await);

    assert!(rows.session_request(close).unwrap().chained_task.is_none());
    assert_eq!(rows.task(waiting).unwrap().status, TaskStatus::Backlog);
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Done);
}

// -- SessionRequestTimesOut (the scheduler's stand-in) --------------------------------

/// The backstop fails an open row "timed out" and leaves the task as it was;
/// a late report on it is then refused. A finished row is left alone.
#[tokio::test]
async fn the_backstop_fails_an_open_row_and_leaves_the_task() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let before = rows.task(task).unwrap();
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    caller.claim_session_request(id, HOST.into()).await.unwrap();

    assert!(caller.expire_session_request(id).won());

    let row = rows.session_request(id).unwrap();
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("timed out"));
    assert_eq!(rows.task(task).unwrap(), before);
    assert!(!caller
        .report_session_request_done(id, HOST.into())
        .await
        .unwrap()
        .won());
    assert!(
        !caller.expire_session_request(id).won(),
        "a finished row is not failed again"
    );
}

// -- HostRestartFailsInterruptedSessionRequests ---------------------------------------

/// A starting board fails its OWN host's running rows "host restarted", and
/// leaves requested rows and other hosts' rows alone.
#[tokio::test]
async fn a_restart_fails_only_this_hosts_running_rows() {
    let (caller, rows) = caller();
    let running_task = dispatched_task(&caller).await;
    let running = applied_id(wrap_up(&caller, running_task, "rebase").await);
    caller
        .claim_session_request(running, HOST.into())
        .await
        .unwrap();
    let waiting_task = dispatched_task(&caller).await;
    let waiting = applied_id(wrap_up(&caller, waiting_task, "rebase").await);

    assert!(caller
        .fail_interrupted_session_requests(HOST.into())
        .await
        .unwrap()
        .won());

    let row = rows.session_request(running).unwrap();
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("host restarted"));
    assert_eq!(
        rows.session_request(waiting).unwrap().state,
        SessionRequestState::Requested
    );
    assert!(caller
        .fail_interrupted_session_requests(OTHER_HOST.into())
        .await
        .unwrap()
        .won());
    assert_eq!(
        rows.session_request(waiting).unwrap().state,
        SessionRequestState::Requested
    );
}

// -- DeletingATaskFailsItsOpenSessionRequest -------------------------------------------

/// Every task delete fails the task's open row "task deleted", through the
/// same hook the dispatch rows use.
#[tokio::test]
async fn deleting_a_task_fails_its_open_session_request() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);
    let finished_task = dispatched_task(&caller).await;
    let finished = token_for(&caller, finished_task, "done").await;

    // A task is deleted only from Done (`tasks.allium: DeleteTask`).
    for done in [task, finished_task] {
        caller
            .patch_task(
                done,
                bindings::TaskPatch {
                    status: Some("done".into()),
                    sub_status: Some("none".into()),
                    ..blank_task_patch()
                },
            )
            .await
            .unwrap();
    }
    assert!(caller.delete_task(task).await.unwrap().won());
    assert!(caller.delete_task(finished_task).await.unwrap().won());

    let row = rows.session_request(id).unwrap();
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("task deleted"));
    assert_eq!(
        rows.session_request(finished).unwrap().state,
        SessionRequestState::Done,
        "a finished row is history and stays"
    );
}

// -- D11 ---------------------------------------------------------------------------------

/// A request names a host its caller owns, and records the caller; the claim,
/// both reports, the exit_session request and the sweep are refused once the
/// host belongs to somebody else.
#[tokio::test]
async fn a_teammate_may_not_act_for_my_hosts_session_requests() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let token = token_for(&caller, task, "done").await;
    let running_task = dispatched_task(&caller).await;
    let running = applied_id(wrap_up(&caller, running_task, "rebase").await);
    caller
        .claim_session_request(running, HOST.into())
        .await
        .unwrap();
    let waiting_task = dispatched_task(&caller).await;
    let waiting = applied_id(wrap_up(&caller, waiting_task, "rebase").await);
    let caller = caller.with_registered_host(HOST, "beef");

    assert!(!wrap_up(&caller, task, "done").await.won());
    assert!(!exit(&caller, task, token, "done", "").await.won());
    assert!(!caller
        .claim_session_request(waiting, HOST.into())
        .await
        .unwrap()
        .won());
    assert!(!caller
        .report_session_request_done(running, HOST.into())
        .await
        .unwrap()
        .won());
    assert!(!caller
        .report_session_request_failed(running, HOST.into(), "x".into(), false)
        .await
        .unwrap()
        .won());
    assert!(!caller
        .fail_interrupted_session_requests(HOST.into())
        .await
        .unwrap()
        .won());
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Running);
    assert_eq!(
        rows.session_request(running).unwrap().state,
        SessionRequestState::Running
    );
}

// -- The rows reach the board --------------------------------------------------------------

/// A write wakes the board's row pump like any other.
#[tokio::test]
async fn session_rows_are_pushed_into_shared_rows_for_the_board() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let mut changed = rows.changed();
    changed.borrow_and_update();

    applied_id(wrap_up(&caller, task, "rebase").await);

    assert!(changed.has_changed().unwrap());
    assert_eq!(rows.session_requests().len(), 1);
}

/// An injected transport fault on a session reducer behaves as on M4's: the
/// call is never sent, or applied with its reply lost.
#[tokio::test]
async fn an_injected_fault_on_a_session_reducer_is_honoured() {
    let (caller, rows) = caller();
    let task = dispatched_task(&caller).await;
    let id = applied_id(wrap_up(&caller, task, "rebase").await);

    caller.fail_next("claim_session_request", InjectedFault::Unsent);
    assert!(caller.claim_session_request(id, HOST.into()).await.is_err());
    assert_eq!(
        rows.session_request(id).unwrap().state,
        SessionRequestState::Requested
    );

    caller.fail_next("claim_session_request", InjectedFault::ReplyLost);
    assert!(caller.claim_session_request(id, HOST.into()).await.is_err());
    assert_eq!(
        rows.session_request(id).unwrap().state,
        SessionRequestState::Running,
        "applied, but the reply never came back"
    );
}
