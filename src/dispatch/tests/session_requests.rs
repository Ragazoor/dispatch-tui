//! Which delivered session request rows this host must act on (store-split M5).
//!
//! [`SessionRequestObserver`](crate::dispatch::SessionRequestObserver) is the
//! pure decision behind `store-split.allium: HostClaimsSessionRequest`'s
//! trigger and `pr-workflow.allium: KillWindowOfFailedExitSession`, kept free
//! of I/O so it can be pinned without a store: of the rows a subscription
//! delivers, the board serves the `requested` rows naming THIS host that have
//! host work, each handed out once however often the pump redelivers the set,
//! and kills the window of an exit_session row it SAW fail.

use chrono::{TimeZone, Utc};

use crate::dispatch::{
    SessionRequestObserver, EXIT_SESSION_TEARDOWN_TIMEOUT, WRAP_UP_REBASE_TIMEOUT,
};
use crate::models::{
    SessionRequest, SessionRequestId, SessionRequestKind, SessionRequestState, TaskId, TmuxWindow,
    WrapUpMode,
};

const HOST: &str = "host-a";
const OTHER_HOST: &str = "host-b";

fn row(
    id: i64,
    host: &str,
    kind: SessionRequestKind,
    action: WrapUpMode,
    state: SessionRequestState,
) -> SessionRequest {
    let requested_at = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
    let has_host_work = kind == SessionRequestKind::ExitSession || action == WrapUpMode::Rebase;
    SessionRequest {
        id: SessionRequestId(id),
        task_id: TaskId(id * 10),
        kind,
        action,
        host: host.to_string(),
        requested_by: "owner".to_string(),
        state,
        requested_at,
        claimed_at: None,
        finished_at: None,
        reason: None,
        pr_url: None,
        wrap_up_request: None,
        tmux_window: (kind == SessionRequestKind::ExitSession)
            .then(|| TmuxWindow::parse(&format!("task-{}", id * 10)).unwrap()),
        chained_task: None,
        deadline: has_host_work.then(|| requested_at + chrono::Duration::seconds(30)),
    }
}

fn wrap_up(id: i64, host: &str, action: WrapUpMode, state: SessionRequestState) -> SessionRequest {
    row(id, host, SessionRequestKind::WrapUp, action, state)
}

fn exit(id: i64, host: &str, state: SessionRequestState) -> SessionRequest {
    row(
        id,
        host,
        SessionRequestKind::ExitSession,
        WrapUpMode::Done,
        state,
    )
}

fn ids(rows: &[SessionRequest]) -> Vec<i64> {
    rows.iter().map(|r| r.id.0).collect()
}

/// `config.wrap_up_rebase_timeout` and `config.exit_session_teardown_timeout`
/// (store-split.allium), which the host reads off the row and the tool's wait
/// cap is derived from.
#[test]
fn the_session_timeouts_are_one_hundred_twenty_and_thirty_seconds() {
    assert_eq!(WRAP_UP_REBASE_TIMEOUT, std::time::Duration::from_secs(120));
    assert_eq!(
        EXIT_SESSION_TEARDOWN_TIMEOUT,
        std::time::Duration::from_secs(30)
    );
}

/// Only `requested` rows naming this host with host work are this board's to
/// serve: not another host's, not running or finished ones, and not a wrap_up
/// written already done.
#[test]
fn only_this_hosts_requested_rows_with_host_work_are_served() {
    let mut observer = SessionRequestObserver::new(HOST);
    let rows = vec![
        wrap_up(1, HOST, WrapUpMode::Rebase, SessionRequestState::Requested),
        exit(2, HOST, SessionRequestState::Requested),
        wrap_up(
            3,
            OTHER_HOST,
            WrapUpMode::Rebase,
            SessionRequestState::Requested,
        ),
        wrap_up(4, HOST, WrapUpMode::Rebase, SessionRequestState::Running),
        wrap_up(5, HOST, WrapUpMode::Rebase, SessionRequestState::Done),
        wrap_up(6, HOST, WrapUpMode::Pr, SessionRequestState::Done),
    ];

    let work = observer.observe(&rows);

    assert_eq!(ids(&work.serve), vec![1, 2]);
    assert!(work.tear_down.is_empty());
}

/// Each row is handed out once, however often the set is redelivered; a row
/// the reactor released (its claim never reached the module) is handed out
/// again.
#[test]
fn a_row_is_handed_out_once_unless_released() {
    let mut observer = SessionRequestObserver::new(HOST);
    let rows = vec![wrap_up(
        1,
        HOST,
        WrapUpMode::Rebase,
        SessionRequestState::Requested,
    )];

    assert_eq!(observer.observe(&rows).serve.len(), 1);
    assert!(observer.observe(&rows).serve.is_empty());

    observer.release(SessionRequestId(1));
    assert_eq!(observer.observe(&rows).serve.len(), 1);
}

/// An exit_session row seen open and then failed owes its window a kill: the
/// close already cleared it from the task. Once.
#[test]
fn an_exit_session_row_seen_to_fail_is_torn_down_once() {
    let mut observer = SessionRequestObserver::new(HOST);
    observer.observe(&[exit(1, HOST, SessionRequestState::Running)]);

    let failed = [exit(1, HOST, SessionRequestState::Failed)];
    let work = observer.observe(&failed);
    assert_eq!(ids(&work.tear_down), vec![1]);

    assert!(observer.observe(&failed).tear_down.is_empty(), "only once");
}

/// A row first seen already failed is history — this board was not running
/// when it failed — and a row that is not an exit_session, has no window, or
/// names another host is never torn down.
#[test]
fn only_a_seen_open_exit_session_row_of_this_host_with_a_window_is_torn_down() {
    let mut observer = SessionRequestObserver::new(HOST);
    observer.observe(&[
        wrap_up(2, HOST, WrapUpMode::Rebase, SessionRequestState::Running),
        exit(3, OTHER_HOST, SessionRequestState::Running),
        SessionRequest {
            tmux_window: None,
            ..exit(4, HOST, SessionRequestState::Running)
        },
    ]);

    let work = observer.observe(&[
        exit(1, HOST, SessionRequestState::Failed),
        wrap_up(2, HOST, WrapUpMode::Rebase, SessionRequestState::Failed),
        exit(3, OTHER_HOST, SessionRequestState::Failed),
        SessionRequest {
            tmux_window: None,
            ..exit(4, HOST, SessionRequestState::Failed)
        },
    ]);

    assert!(work.tear_down.is_empty(), "{:?}", ids(&work.tear_down));
}

/// Startup: the rows a previous board left open are noted WITHOUT being
/// served, so the sweep that fails them is then seen as a failure and their
/// windows are killed — and the serving waits for the sweep
/// (`HostRestartFailsInterruptedSessionRequests` runs first).
#[test]
fn startup_notes_open_rows_without_serving_them() {
    let mut observer = SessionRequestObserver::new(HOST);
    let before_sweep = [
        exit(1, HOST, SessionRequestState::Running),
        wrap_up(2, HOST, WrapUpMode::Rebase, SessionRequestState::Requested),
    ];
    observer.note_open(&before_sweep);

    let after_sweep = [
        exit(1, HOST, SessionRequestState::Failed),
        wrap_up(2, HOST, WrapUpMode::Rebase, SessionRequestState::Requested),
    ];
    let work = observer.observe(&after_sweep);

    assert_eq!(ids(&work.tear_down), vec![1]);
    assert_eq!(
        ids(&work.serve),
        vec![2],
        "noting did not hand the requested row out"
    );
}
