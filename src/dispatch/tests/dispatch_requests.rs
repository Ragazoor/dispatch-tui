//! Which delivered dispatch request rows this host must serve (store-split M4).
//!
//! [`DispatchRequestObserver`](crate::dispatch::DispatchRequestObserver) is
//! the pure decision behind `store-split.allium: HostClaimsDispatch`'s
//! trigger, kept free of I/O so it can be pinned without a store: of the rows
//! a subscription delivers (a live insert, or the initial rows a starting
//! board receives — D5), the board claims and provisions the `requested` rows
//! naming THIS host, each handed out once however often the row pump
//! redelivers the whole set.
//!
//! What the board does with FAILED rows (the DispatchingFeedback popup and
//! `epics.allium: ReportChainedDispatchFailure`) is the App's, pinned in
//! `src/tui/tests/dispatch_requests.rs`.

use chrono::{TimeZone, Utc};

use crate::dispatch::{DispatchRequestObserver, DEADLINE_PASSED, DISPATCH_REQUEST_TIMEOUT};
use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchRequestState, TaskId,
};

const HOST: &str = "host-a";
const OTHER_HOST: &str = "host-b";

fn row(
    id: i64,
    task: i64,
    host: &str,
    origin: DispatchOrigin,
    state: DispatchRequestState,
) -> DispatchRequest {
    let requested_at = Utc.with_ymd_and_hms(2026, 10, 11, 12, 0, 0).unwrap();
    let finished = matches!(
        state,
        DispatchRequestState::Dispatched | DispatchRequestState::Failed
    );
    DispatchRequest {
        id: DispatchRequestId(id),
        task_id: TaskId(task),
        host: host.to_string(),
        requested_by: "owner".to_string(),
        mode: DispatchMode::Dispatch,
        origin,
        state,
        requested_at,
        claimed_at: (state != DispatchRequestState::Requested).then_some(requested_at),
        finished_at: finished.then_some(requested_at + chrono::Duration::seconds(5)),
        reason: (state == DispatchRequestState::Failed).then(|| "worktree add failed".into()),
        worktree: None,
        tmux_window: None,
        deadline: requested_at + chrono::Duration::seconds(60),
    }
}

/// `config.dispatch_request_timeout` (store-split.allium): the one deadline
/// the host stops at, which UnprovisionedIndicator's claim-stamp fallback
/// reads too.
#[test]
fn the_request_timeout_is_sixty_seconds() {
    assert_eq!(DISPATCH_REQUEST_TIMEOUT, std::time::Duration::from_secs(60));
}

/// `ProvisioningStopsAtTheDeadline`'s reason, as the row records it.
#[test]
fn the_deadline_stop_reports_deadline_passed() {
    assert_eq!(DEADLINE_PASSED, "deadline passed");
}

/// Only `requested` rows naming this host are this board's to provision.
#[test]
fn only_this_hosts_requested_rows_are_served() {
    let mut observer = DispatchRequestObserver::new(HOST);
    let rows = vec![
        row(
            1,
            10,
            HOST,
            DispatchOrigin::User,
            DispatchRequestState::Requested,
        ),
        row(
            2,
            11,
            OTHER_HOST,
            DispatchOrigin::User,
            DispatchRequestState::Requested,
        ),
        row(
            3,
            12,
            HOST,
            DispatchOrigin::Mcp,
            DispatchRequestState::Provisioning,
        ),
        row(
            4,
            13,
            HOST,
            DispatchOrigin::Quick,
            DispatchRequestState::Dispatched,
        ),
        row(
            5,
            14,
            HOST,
            DispatchOrigin::EpicChain,
            DispatchRequestState::Failed,
        ),
    ];

    let to_serve = observer.observe(&rows);

    let ids: Vec<_> = to_serve.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![DispatchRequestId(1)]);
}

/// A redelivered row (every row change re-reads the whole board) is handed
/// out once. The module refuses a second claim anyway; this keeps the board
/// from asking twice.
#[test]
fn a_requested_row_is_served_once() {
    let mut observer = DispatchRequestObserver::new(HOST);
    let rows = vec![row(
        1,
        10,
        HOST,
        DispatchOrigin::User,
        DispatchRequestState::Requested,
    )];

    assert_eq!(observer.observe(&rows).len(), 1);
    assert!(observer.observe(&rows).is_empty());
}

/// D5 catch-up: a row already `requested` in the initial rows is served on
/// the first observation, like a live insert — whatever origin wrote it.
#[test]
fn requested_rows_in_the_initial_rows_are_served() {
    let mut observer = DispatchRequestObserver::new(HOST);
    let rows = vec![
        row(
            5,
            10,
            HOST,
            DispatchOrigin::EpicChain,
            DispatchRequestState::Requested,
        ),
        row(
            6,
            11,
            HOST,
            DispatchOrigin::Quick,
            DispatchRequestState::Requested,
        ),
    ];

    assert_eq!(observer.observe(&rows).len(), 2);
}
