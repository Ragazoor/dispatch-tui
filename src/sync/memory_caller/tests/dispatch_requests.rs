//! The in-memory twin of the M4 dispatch-request reducers.
//!
//! Spec: `docs/specs/store-split.allium` — `RequestDispatch`,
//! `HostClaimsDispatch`, `HostReportsDispatched`, `HostReportsDispatchFailed`,
//! `DispatchRequestTimesOut`, `HostRestartFailsInterrupted`,
//! `DeletingATaskFailsItsOpenRequest`, `ReleaseClaimOfFailedRequest` and
//! `OneOpenDispatchRequestPerTask`. The board's tests drive dispatch through
//! this double, so it must refuse and release exactly where the module does;
//! `tests/memory_caller_conformance.rs` diffs the two.
//!
//! `expire_dispatch_request` is the double's stand-in for the module's
//! scheduler (`expire_dispatch_request` is a scheduled reducer no client may
//! call): an inherent method, not on [`ReducerCaller`], so production code
//! cannot reach it.

use super::*;
use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequestId, DispatchRequestState, TaskStatus,
};

const HOST: &str = "host-a";
const OTHER_HOST: &str = "host-b";
/// The identity this double's calls are sent as, and the owner of both hosts
/// above, so its calls may act for either (`store-split.allium`, D11).
const ME: &str = "c0ffee";

/// The shared fixture, with both hosts registered to [`ME`] and the calls
/// sent as [`ME`].
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
        .with_registered_host(HOST, ME);
    (caller, rows, clock)
}

/// The request id an applied `request_dispatch` reads back.
fn request_id(outcome: ReducerOutcome) -> DispatchRequestId {
    match outcome {
        ReducerOutcome::Applied(ids) => {
            assert_eq!(ids.len(), 1, "request_dispatch reads back one id: {ids:?}");
            DispatchRequestId(ids[0])
        }
        ReducerOutcome::Refused(why) => panic!("expected the request to apply, refused: {why}"),
    }
}

async fn request(
    caller: &MemoryReducerCaller,
    task: TaskId,
    host: &str,
    origin: &str,
    mode: &str,
) -> ReducerOutcome {
    caller
        .request_dispatch(task, host.into(), origin.into(), mode.into())
        .await
        .unwrap()
}

/// A backlog task, requested by [`HOST`] from the TUI with an explicit
/// `dispatch` mode, and claimed by its host.
async fn provisioning_request(caller: &MemoryReducerCaller) -> (TaskId, DispatchRequestId) {
    let task = caller.create_task(blank_task()).await.unwrap();
    let id = request_id(request(caller, task, HOST, "user", "dispatch").await);
    assert!(caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap()
        .won());
    (task, id)
}

// -- RequestDispatch -----------------------------------------------------------

/// The claim and the row are one transaction: the task goes running with the
/// seeded stamp, and a `requested` row names the requester, the origin, the
/// mode and a deadline 60s after `requested_at`.
#[tokio::test]
async fn request_dispatch_claims_the_task_and_writes_a_requested_row() {
    let (caller, rows) = caller();
    let task = caller.create_task(blank_task()).await.unwrap();

    let id = request_id(request(&caller, task, HOST, "user", "dispatch").await);

    let claimed = rows.task(task).unwrap();
    assert_eq!(claimed.status, TaskStatus::Running);
    assert!(
        claimed.last_pre_tool_use_at.is_some(),
        "the claim seeds the stamp"
    );
    assert!(claimed.worktree.is_none(), "the request provisions nothing");

    let row = rows
        .dispatch_request(id)
        .expect("the row reaches SharedRows");
    assert_eq!(row.task_id, task);
    assert_eq!(row.host, HOST);
    assert_eq!(row.origin, DispatchOrigin::User);
    assert_eq!(row.mode, DispatchMode::Dispatch);
    assert_eq!(row.state, DispatchRequestState::Requested);
    assert!(row.is_open());
    assert_eq!(
        row.deadline - row.requested_at,
        chrono::Duration::seconds(60),
        "config.dispatch_request_timeout, counted from requested_at"
    );
    assert!(row.claimed_at.is_none());
    assert!(row.finished_at.is_none());
    assert!(row.reason.is_none());
}

/// D3: no mode means the module derives it — a plan-less research-tagged task
/// gets the research agent; `quick` is only ever named, never derived.
#[tokio::test]
async fn request_dispatch_derives_the_mode_only_when_none_is_named() {
    let (caller, rows) = caller();
    let research = caller
        .create_task(bindings::Task {
            tag: "research".into(),
            ..blank_task()
        })
        .await
        .unwrap();
    let plain = caller.create_task(blank_task()).await.unwrap();
    let quick = caller.create_task(blank_task()).await.unwrap();

    let r = request_id(request(&caller, research, HOST, "mcp", "").await);
    let p = request_id(request(&caller, plain, HOST, "epic_chain", "").await);
    let q = request_id(request(&caller, quick, HOST, "quick", "quick").await);

    assert_eq!(
        rows.dispatch_request(r).unwrap().mode,
        DispatchMode::Research
    );
    assert_eq!(
        rows.dispatch_request(p).unwrap().mode,
        DispatchMode::Dispatch
    );
    assert_eq!(rows.dispatch_request(q).unwrap().mode, DispatchMode::Quick);
    assert_eq!(
        rows.dispatch_request(p).unwrap().origin,
        DispatchOrigin::EpicChain
    );
    assert_eq!(
        rows.dispatch_request(q).unwrap().origin,
        DispatchOrigin::Quick
    );
}

/// The three `requires`: a task that is not backlog, a task whose worktree is
/// on another machine, and a task that still holds an open request. Each is an
/// ordinary refusal that writes nothing.
#[tokio::test]
async fn request_dispatch_refuses_and_writes_nothing() {
    let (caller, rows) = caller();

    // Not backlog: already claimed by an earlier request.
    let claimed = caller.create_task(blank_task()).await.unwrap();
    request_id(request(&caller, claimed, HOST, "user", "dispatch").await);
    let before = rows.dispatch_requests().len();
    match request(&caller, claimed, HOST, "mcp", "").await {
        ReducerOutcome::Refused(why) => assert!(why.contains("backlog"), "{why}"),
        other => panic!("a claimed task must be refused, got {other:?}"),
    }
    assert_eq!(
        rows.dispatch_requests().len(),
        before,
        "a refusal writes no row"
    );

    // Owned elsewhere: backlog, but its worktree is on another machine.
    let foreign = caller
        .create_task(bindings::Task {
            worktree: "/elsewhere/.worktrees/9-x".into(),
            host: OTHER_HOST.into(),
            ..blank_task()
        })
        .await
        .unwrap();
    assert!(
        matches!(
            request(&caller, foreign, HOST, "user", "dispatch").await,
            ReducerOutcome::Refused(_)
        ),
        "a task owned by another machine is refused"
    );
    assert_eq!(rows.task(foreign).unwrap().status, TaskStatus::Backlog);

    // An open request already exists: the task was moved back to backlog by
    // hand while its request is still open.
    let moved_back = caller.create_task(blank_task()).await.unwrap();
    request_id(request(&caller, moved_back, HOST, "user", "dispatch").await);
    caller
        .patch_task(
            moved_back,
            bindings::TaskPatch {
                status: Some("backlog".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    match request(&caller, moved_back, HOST, "user", "dispatch").await {
        ReducerOutcome::Refused(why) => assert!(why.contains("open"), "{why}"),
        other => panic!("OneOpenDispatchRequestPerTask must refuse, got {other:?}"),
    }
}

/// Origins and modes are closed vocabularies (D3, D9).
#[tokio::test]
async fn request_dispatch_refuses_an_unknown_origin_or_mode() {
    let (caller, rows) = caller();
    let task = caller.create_task(blank_task()).await.unwrap();

    assert!(matches!(
        request(&caller, task, HOST, "cron", "").await,
        ReducerOutcome::Refused(_)
    ));
    assert!(matches!(
        request(&caller, task, HOST, "user", "turbo").await,
        ReducerOutcome::Refused(_)
    ));
    assert!(matches!(
        request(&caller, task, "", "user", "").await,
        ReducerOutcome::Refused(_)
    ));
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Backlog);
    assert!(rows.dispatch_requests().is_empty());
}

// -- HostClaimsDispatch --------------------------------------------------------

/// Only the row's own host claims it, and only once.
#[tokio::test]
async fn claim_dispatch_request_is_the_hosts_and_happens_once() {
    let (caller, rows) = caller();
    let task = caller.create_task(blank_task()).await.unwrap();
    let id = request_id(request(&caller, task, HOST, "user", "dispatch").await);

    let foreign = caller
        .claim_dispatch_request(id, OTHER_HOST.into())
        .await
        .unwrap();
    assert!(
        !foreign.won(),
        "another host must not claim this host's row"
    );

    assert!(caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap()
        .won());
    let row = rows.dispatch_request(id).unwrap();
    assert_eq!(row.state, DispatchRequestState::Provisioning);
    assert!(row.claimed_at.is_some());

    let twice = caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap();
    assert!(!twice.won(), "a redelivered row is claimed exactly once");

    let missing = caller
        .claim_dispatch_request(DispatchRequestId(9999), HOST.into())
        .await
        .unwrap();
    assert!(!missing.won());
}

// -- HostReportsDispatched ----------------------------------------------------

/// The module, not the host, writes the task's worktree, window and host.
#[tokio::test]
async fn report_dispatch_provisioned_records_the_result_on_the_row_and_the_task() {
    let (caller, rows) = caller();
    let (task, id) = provisioning_request(&caller).await;

    let outcome = caller
        .report_dispatch_provisioned(
            id,
            HOST.into(),
            "/repo/.worktrees/1-t".into(),
            "task-1".into(),
            false,
        )
        .await
        .unwrap();
    assert!(outcome.won(), "{outcome:?}");

    let row = rows.dispatch_request(id).unwrap();
    assert_eq!(row.state, DispatchRequestState::Dispatched);
    assert_eq!(row.worktree.as_deref(), Some("/repo/.worktrees/1-t"));
    assert_eq!(row.tmux_window.as_ref().map(|w| w.as_str()), Some("task-1"));
    assert!(row.finished_at.is_some());
    assert!(!row.is_open());

    let t = rows.task(task).unwrap();
    assert_eq!(t.status, TaskStatus::Running);
    assert_eq!(t.worktree.as_deref(), Some("/repo/.worktrees/1-t"));
    assert_eq!(t.tmux_window.as_ref().map(|w| w.as_str()), Some("task-1"));
    assert_eq!(t.host.as_deref(), Some(HOST), "HostTracksWorktree");
}

/// The host's reused-worktree answer is kept on the row
/// (`DispatchRequest.reused_worktree`), for `dispatch_task`'s success text.
#[tokio::test]
async fn report_dispatch_provisioned_records_whether_the_worktree_was_reused() {
    let (caller, rows) = caller();
    let (_task, id) = provisioning_request(&caller).await;
    assert!(
        !rows.dispatch_request(id).unwrap().reused_worktree,
        "false until reported"
    );

    caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), true)
        .await
        .unwrap();

    assert!(rows.dispatch_request(id).unwrap().reused_worktree);
}

/// `HostRepeatsDispatchedReport`: a host whose report got no answer sends it
/// again; the repeat is accepted and changes nothing, even past the deadline.
/// A report naming another worktree is still refused.
#[tokio::test]
async fn a_repeated_dispatched_report_is_accepted_and_changes_nothing() {
    let (caller, rows, clock) = caller_with_clock();
    let (task, id) = provisioning_request(&caller).await;
    assert!(caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), false)
        .await
        .unwrap()
        .won());
    let row = rows.dispatch_request(id).unwrap();
    let stored = rows.task(task).unwrap();

    clock.advance(chrono::Duration::seconds(120));
    let again = caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), false)
        .await
        .unwrap();
    assert!(again.won(), "{again:?}");
    assert_eq!(rows.dispatch_request(id).unwrap(), row);
    assert_eq!(rows.task(task).unwrap().updated_at, stored.updated_at);

    let other = caller
        .report_dispatch_provisioned(id, HOST.into(), "/other".into(), "task-1".into(), false)
        .await
        .unwrap();
    assert!(!other.won(), "a different worktree is not a repeat");
}

/// D4: a dispatched report that lands at or past the row's deadline, by the
/// store's clock, is refused and records nothing; the row stays provisioning
/// for the host's failure report.
#[tokio::test]
async fn a_dispatched_report_past_the_deadline_is_refused() {
    let (caller, rows, clock) = caller_with_clock();
    let (task, id) = provisioning_request(&caller).await;

    clock.advance(chrono::Duration::seconds(60));
    let late = caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), false)
        .await
        .unwrap();

    match late {
        ReducerOutcome::Refused(why) => assert!(why.contains("deadline"), "{why}"),
        other => panic!("a report past the deadline must be refused, got {other:?}"),
    }
    assert_eq!(
        rows.dispatch_request(id).unwrap().state,
        DispatchRequestState::Provisioning
    );
    assert!(rows.task(task).unwrap().worktree.is_none());
}

/// `PhoenixIsNeverChained`, enforced by the request: the epic chain's request
/// for a phoenix task is refused and writes nothing; the TUI's is accepted.
#[tokio::test]
async fn the_epic_chain_may_not_request_a_phoenix_task() {
    let (caller, rows) = caller();
    let task = caller
        .create_task(bindings::Task {
            phoenix: true,
            ..blank_task()
        })
        .await
        .unwrap();

    match request(&caller, task, HOST, "epic_chain", "").await {
        ReducerOutcome::Refused(why) => assert!(why.contains("phoenix"), "{why}"),
        other => panic!("the chain must not take a phoenix task, got {other:?}"),
    }
    assert!(rows.dispatch_requests().is_empty());
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Backlog);

    request_id(request(&caller, task, HOST, "user", "dispatch").await);
}

/// Only the row's host reports, and only while it is provisioning.
#[tokio::test]
async fn reports_are_refused_from_another_host_or_off_a_requested_row() {
    let (caller, _rows) = caller();
    let task = caller.create_task(blank_task()).await.unwrap();
    let id = request_id(request(&caller, task, HOST, "user", "dispatch").await);

    let unclaimed = caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), false)
        .await
        .unwrap();
    assert!(!unclaimed.won(), "a requested row has not been claimed yet");

    caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap();
    let foreign = caller
        .report_dispatch_failed(id, OTHER_HOST.into(), "nope".into())
        .await
        .unwrap();
    assert!(!foreign.won(), "another host cannot report on this row");
}

// -- HostReportsDispatchFailed + ReleaseClaimOfFailedRequest -------------------

/// A reported failure finishes the row with its reason and releases the claim:
/// backlog, default sub-status, the seeded stamp cleared.
#[tokio::test]
async fn report_dispatch_failed_fails_the_row_and_releases_the_claim() {
    let (caller, rows) = caller();
    let (task, id) = provisioning_request(&caller).await;

    assert!(caller
        .report_dispatch_failed(id, HOST.into(), "git worktree add failed".into())
        .await
        .unwrap()
        .won());

    let row = rows.dispatch_request(id).unwrap();
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("git worktree add failed"));
    assert!(row.finished_at.is_some());

    let t = rows.task(task).unwrap();
    assert_eq!(t.status, TaskStatus::Backlog);
    assert_eq!(
        t.sub_status,
        crate::models::SubStatus::default_for(TaskStatus::Backlog)
    );
    assert!(t.last_pre_tool_use_at.is_none());

    // Released, so the task can be requested again at once.
    request_id(request(&caller, task, HOST, "user", "dispatch").await);
}

/// The release is conditional: a task that got a worktree meanwhile is left
/// alone.
#[tokio::test]
async fn a_failure_does_not_release_a_task_that_moved_on() {
    let (caller, rows) = caller();
    let (task, id) = provisioning_request(&caller).await;
    caller
        .patch_task(
            task,
            bindings::TaskPatch {
                worktree: Some("/manual".into()),
                host: Some(HOST.into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();

    caller
        .report_dispatch_failed(id, HOST.into(), "late".into())
        .await
        .unwrap();

    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Running);
    assert_eq!(
        rows.dispatch_request(id).unwrap().state,
        DispatchRequestState::Failed
    );
}

// -- DispatchRequestTimesOut (the scheduler's stand-in) -------------------------

/// The backstop fails an open row "timed out" and releases its claim; a late
/// dispatched report on it is then refused and writes nothing to the task.
#[tokio::test]
async fn the_backstop_fails_an_open_row_and_a_late_report_is_refused() {
    let (caller, rows) = caller();
    let (task, id) = provisioning_request(&caller).await;

    assert!(caller.expire_dispatch_request(id).won());

    let row = rows.dispatch_request(id).unwrap();
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("timed out"));
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Backlog);

    let late = caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), false)
        .await
        .unwrap();
    assert!(
        !late.won(),
        "OnlyTheClaimedRequestProvisions: a finished row takes no report"
    );
    assert!(
        rows.task(task).unwrap().worktree.is_none(),
        "the late worktree is never recorded on the task"
    );
}

/// A row its host already finished is left alone by the backstop.
#[tokio::test]
async fn the_backstop_leaves_a_finished_row_alone() {
    let (caller, rows) = caller();
    let (_task, id) = provisioning_request(&caller).await;
    caller
        .report_dispatch_provisioned(id, HOST.into(), "/w".into(), "task-1".into(), false)
        .await
        .unwrap();

    caller.expire_dispatch_request(id);

    assert_eq!(
        rows.dispatch_request(id).unwrap().state,
        DispatchRequestState::Dispatched
    );
}

// -- HostRestartFailsInterrupted ----------------------------------------------

/// Only THIS host's PROVISIONING rows fail ("host restarted"); a requested row
/// is left for catch-up, and another host's rows are untouched.
#[tokio::test]
async fn fail_interrupted_fails_only_this_hosts_provisioning_rows() {
    let (caller, rows) = caller();
    let (interrupted_task, interrupted) = provisioning_request(&caller).await;
    let waiting_task = caller.create_task(blank_task()).await.unwrap();
    let waiting = request_id(request(&caller, waiting_task, HOST, "user", "dispatch").await);
    let other_task = caller.create_task(blank_task()).await.unwrap();
    let other = request_id(request(&caller, other_task, OTHER_HOST, "user", "dispatch").await);
    caller
        .claim_dispatch_request(other, OTHER_HOST.into())
        .await
        .unwrap();

    assert!(caller
        .fail_interrupted_dispatch_requests(HOST.into())
        .await
        .unwrap()
        .won());

    let row = rows.dispatch_request(interrupted).unwrap();
    assert_eq!(row.state, DispatchRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("host restarted"));
    assert_eq!(
        rows.task(interrupted_task).unwrap().status,
        TaskStatus::Backlog
    );

    assert_eq!(
        rows.dispatch_request(waiting).unwrap().state,
        DispatchRequestState::Requested,
        "D5: a requested row is picked up, not failed"
    );
    assert_eq!(
        rows.dispatch_request(other).unwrap().state,
        DispatchRequestState::Provisioning,
        "another host's row is not this restart's to fail"
    );
}

// -- DeletingATaskFailsItsOpenRequest -----------------------------------------

/// Every task delete fails the task's open request "task deleted", in the same
/// call: `delete_task` and `batch_delete` alike.
#[tokio::test]
async fn deleting_a_task_fails_its_open_request() {
    let (caller, rows) = caller();
    let (single, single_req) = provisioning_request(&caller).await;
    let batched = caller.create_task(blank_task()).await.unwrap();
    let batched_req = request_id(request(&caller, batched, HOST, "user", "dispatch").await);
    // Only a done task may be deleted (`tasks.allium: DeleteTask`): both are
    // moved to done by hand while their requests are still open.
    for task in [single, batched] {
        caller
            .patch_task(
                task,
                bindings::TaskPatch {
                    status: Some("done".into()),
                    sub_status: Some("none".into()),
                    ..blank_task_patch()
                },
            )
            .await
            .unwrap()
            .applied()
            .unwrap();
    }

    caller.delete_task(single).await.unwrap().applied().unwrap();
    caller
        .batch_delete(vec![batched], vec![])
        .await
        .unwrap()
        .applied()
        .unwrap();

    for id in [single_req, batched_req] {
        let row = rows
            .dispatch_request(id)
            .expect("finished rows stay as history");
        assert_eq!(row.state, DispatchRequestState::Failed);
        assert_eq!(row.reason.as_deref(), Some("task deleted"));
    }
}

/// The double's rows carry the same vocabulary the module stores, so the board
/// decodes them the same way whichever store wrote them.
#[tokio::test]
async fn request_rows_are_pushed_into_shared_rows_for_the_board() {
    let (caller, rows) = caller();
    let mut changed = rows.changed();
    let task = caller.create_task(blank_task()).await.unwrap();
    changed.borrow_and_update();

    request_id(request(&caller, task, HOST, "user", "research").await);

    assert!(
        changed.has_changed().unwrap(),
        "a request must wake the board's row pump like any other write"
    );
    let all = rows.dispatch_requests();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].mode, DispatchMode::Research);
}

// -- D11: only a host's owner may act for it ------------------------------------

/// A request records who made it.
#[tokio::test]
async fn a_request_records_its_caller_as_requested_by() {
    let (caller, rows) = caller();
    let task = caller.create_task(blank_task()).await.unwrap();

    let id = request_id(request(&caller, task, HOST, "user", "dispatch").await);

    assert_eq!(rows.dispatch_request(id).unwrap().requested_by, ME);
}

/// Naming a host somebody else owns, an ownerless host or an unregistered one
/// is refused, and writes nothing.
#[tokio::test]
async fn a_request_for_a_host_the_caller_does_not_own_is_refused() {
    let (caller, rows) = caller();
    let caller = caller
        .with_registered_host("their-host", "beef")
        .with_registered_host("fresh-host", "");
    let task = caller.create_task(blank_task()).await.unwrap();

    for host in ["their-host", "fresh-host", "nowhere"] {
        match request(&caller, task, host, "user", "dispatch").await {
            ReducerOutcome::Refused(why) => assert!(why.contains(host), "{why}"),
            other => panic!("{host} must be refused, got {other:?}"),
        }
    }
    assert!(rows.dispatch_requests().is_empty());
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Backlog);
}

/// The claim, both reports and the sweep are refused once the host belongs to
/// somebody else.
#[tokio::test]
async fn claims_reports_and_the_sweep_are_refused_for_a_host_the_caller_does_not_own() {
    let (caller, rows) = caller();
    let (_task, provisioning) = provisioning_request(&caller).await;
    let waiting_task = caller.create_task(blank_task()).await.unwrap();
    let waiting = request_id(request(&caller, waiting_task, HOST, "user", "dispatch").await);
    let caller = caller.with_registered_host(HOST, "beef");

    assert!(!caller
        .claim_dispatch_request(waiting, HOST.into())
        .await
        .unwrap()
        .won());
    assert!(!caller
        .report_dispatch_provisioned(
            provisioning,
            HOST.into(),
            "/w".into(),
            "task-1".into(),
            false
        )
        .await
        .unwrap()
        .won());
    assert!(!caller
        .report_dispatch_failed(provisioning, HOST.into(), "x".into())
        .await
        .unwrap()
        .won());
    assert!(!caller
        .fail_interrupted_dispatch_requests(HOST.into())
        .await
        .unwrap()
        .won());
    assert_eq!(
        rows.dispatch_request(waiting).unwrap().state,
        DispatchRequestState::Requested
    );
    assert_eq!(
        rows.dispatch_request(provisioning).unwrap().state,
        DispatchRequestState::Provisioning
    );
}
