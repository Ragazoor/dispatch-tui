//! Dispatch request rows reach the board's reads (store-split M4).
//!
//! `docs/specs/sync.allium: SubscribeToThisHostsDispatchRequests` delivers the
//! rows; these pin that a delivered row decodes into the domain value the
//! board reads, sentinels included (an absent optional column is `""` in the
//! module, `None` on the board), and that an undecodable one is dropped rather
//! than misread.

use chrono::{NaiveDateTime, Utc};

use crate::models::{
    DispatchMode, DispatchOrigin, DispatchRequestId, DispatchRequestState, TaskId,
};
use crate::spacetime::bindings;
use crate::sync::SharedRows;

fn at(stamp: &str) -> chrono::DateTime<Utc> {
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.3f")
        .unwrap()
        .and_utc()
}

fn requested_row(id: i64) -> bindings::DispatchRequest {
    bindings::DispatchRequest {
        id,
        task_id: 7,
        kind: "dispatch".into(),
        host: "host-a".into(),
        mode: "research".into(),
        origin: "epic_chain".into(),
        state: "requested".into(),
        requested_at: "2026-10-11 12:00:00.000".into(),
        claimed_at: String::new(),
        finished_at: String::new(),
        reason: String::new(),
        worktree: String::new(),
        tmux_window: String::new(),
        deadline: "2026-10-11 12:01:00.000".into(),
        requested_by: "c0ffee".into(),
        reused_worktree: false,
    }
}

/// A requested row: every sentinel column reads as absent, the vocabulary
/// columns as their enums, and the deadline is the stored one (D4: the host
/// reads the deadline off the row, never from a copy of the config).
#[test]
fn a_requested_row_decodes_with_its_sentinels_absent() {
    let rows = SharedRows::new();
    rows.upsert_dispatch_request(&requested_row(3));

    let row = rows
        .dispatch_request(DispatchRequestId(3))
        .expect("the delivered row is readable");
    assert_eq!(row.id, DispatchRequestId(3));
    assert_eq!(row.task_id, TaskId(7));
    assert_eq!(row.host, "host-a");
    assert_eq!(
        row.requested_by, "c0ffee",
        "D11: who asked, as the module stamped it"
    );
    assert_eq!(row.mode, DispatchMode::Research);
    assert_eq!(row.origin, DispatchOrigin::EpicChain);
    assert_eq!(row.state, DispatchRequestState::Requested);
    assert_eq!(row.requested_at, at("2026-10-11 12:00:00.000"));
    assert_eq!(row.deadline, at("2026-10-11 12:01:00.000"));
    assert!(row.claimed_at.is_none());
    assert!(row.finished_at.is_none());
    assert!(row.reason.is_none());
    assert!(row.worktree.is_none());
    assert!(row.tmux_window.is_none());
    assert!(row.is_open());
}

/// A finished row carries its outcome: a dispatched one its worktree and
/// window, a failed one its reason.
#[test]
fn finished_rows_decode_their_outcome() {
    let rows = SharedRows::new();
    rows.upsert_dispatch_request(&bindings::DispatchRequest {
        state: "dispatched".into(),
        claimed_at: "2026-10-11 12:00:01.000".into(),
        finished_at: "2026-10-11 12:00:09.000".into(),
        worktree: "/repo/.worktrees/7-t".into(),
        tmux_window: "task-7".into(),
        ..requested_row(1)
    });
    rows.upsert_dispatch_request(&bindings::DispatchRequest {
        state: "failed".into(),
        finished_at: "2026-10-11 12:01:00.000".into(),
        reason: "deadline passed".into(),
        ..requested_row(2)
    });

    let dispatched = rows.dispatch_request(DispatchRequestId(1)).unwrap();
    assert_eq!(dispatched.state, DispatchRequestState::Dispatched);
    assert_eq!(dispatched.worktree.as_deref(), Some("/repo/.worktrees/7-t"));
    assert_eq!(
        dispatched.tmux_window.as_ref().map(|w| w.as_str()),
        Some("task-7")
    );
    assert_eq!(dispatched.finished_at, Some(at("2026-10-11 12:00:09.000")));
    assert!(!dispatched.is_open());

    let failed = rows.dispatch_request(DispatchRequestId(2)).unwrap();
    assert_eq!(failed.state, DispatchRequestState::Failed);
    assert_eq!(failed.reason.as_deref(), Some("deadline passed"));
    assert!(failed.worktree.is_none());

    assert_eq!(rows.dispatch_requests().len(), 2);
}

/// A row with a state this binary does not know is dropped, not misread as
/// open or finished.
#[test]
fn an_undecodable_row_is_dropped() {
    let rows = SharedRows::new();
    rows.upsert_dispatch_request(&bindings::DispatchRequest {
        state: "teleporting".into(),
        ..requested_row(4)
    });
    assert!(rows.dispatch_request(DispatchRequestId(4)).is_none());
    assert!(rows.dispatch_requests().is_empty());
}

/// A delete from the subscription removes the row.
#[test]
fn a_removed_row_is_gone() {
    let rows = SharedRows::new();
    rows.upsert_dispatch_request(&requested_row(5));
    rows.remove_dispatch_request(5);
    assert!(rows.dispatch_request(DispatchRequestId(5)).is_none());
}
