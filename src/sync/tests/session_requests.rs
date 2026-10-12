//! Session request rows reach the board's reads (store-split M5).
//!
//! `docs/specs/sync.allium: SubscribeToThisHostsSessionRequests` delivers the
//! rows; these pin that a delivered row decodes into the domain value the
//! board reads, sentinels included (an absent optional column is `""` or `0`
//! in the module, `None` on the board), and that an undecodable one is dropped
//! rather than misread.

use chrono::{NaiveDateTime, Utc};

use crate::models::{
    SessionRequestId, SessionRequestKind, SessionRequestState, TaskId, WrapUpMode,
};
use crate::spacetime::bindings;
use crate::sync::SharedRows;

fn at(stamp: &str) -> chrono::DateTime<Utc> {
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.3f")
        .unwrap()
        .and_utc()
}

fn requested_row(id: i64) -> bindings::SessionRequest {
    bindings::SessionRequest {
        id,
        task_id: 7,
        kind: "wrap_up".into(),
        action: "rebase".into(),
        host: "host-a".into(),
        requested_by: "c0ffee".into(),
        state: "requested".into(),
        requested_at: "2026-10-12 12:00:00.000".into(),
        claimed_at: String::new(),
        finished_at: String::new(),
        reason: String::new(),
        pr_url: String::new(),
        wrap_up_request: 0,
        tmux_window: String::new(),
        chained_task: 0,
        deadline: "2026-10-12 12:02:00.000".into(),
    }
}

/// A requested wrap_up row: every sentinel column reads as absent, the
/// vocabulary columns as their enums, and the deadline is the stored one (W7:
/// the host reads the deadline off the row, never from a copy of the config).
#[test]
fn a_requested_wrap_up_row_decodes_with_its_sentinels_absent() {
    let rows = SharedRows::new();
    rows.upsert_session_request(&requested_row(3));

    let row = rows
        .session_request(SessionRequestId(3))
        .expect("the delivered row is readable");
    assert_eq!(row.id, SessionRequestId(3));
    assert_eq!(row.task_id, TaskId(7));
    assert_eq!(row.kind, SessionRequestKind::WrapUp);
    assert_eq!(row.action, WrapUpMode::Rebase);
    assert_eq!(row.host, "host-a");
    assert_eq!(
        row.requested_by, "c0ffee",
        "D11: who asked, as the module stamped it"
    );
    assert_eq!(row.state, SessionRequestState::Requested);
    assert_eq!(row.requested_at, at("2026-10-12 12:00:00.000"));
    assert_eq!(row.deadline, Some(at("2026-10-12 12:02:00.000")));
    assert!(row.claimed_at.is_none());
    assert!(row.finished_at.is_none());
    assert!(row.reason.is_none());
    assert!(row.pr_url.is_none());
    assert!(row.wrap_up_request.is_none());
    assert!(row.tmux_window.is_none());
    assert!(row.chained_task.is_none());
    assert!(row.is_open());
    assert!(row.has_host_work());
}

/// A wrap_up with action done or pr is written already done, with no
/// deadline and no host work (W2): the deadline reads as absent, not as an
/// undecodable empty timestamp.
#[test]
fn a_done_wrap_up_with_no_host_work_has_no_deadline() {
    let rows = SharedRows::new();
    rows.upsert_session_request(&bindings::SessionRequest {
        action: "pr".into(),
        state: "done".into(),
        finished_at: "2026-10-12 12:00:00.000".into(),
        deadline: String::new(),
        ..requested_row(4)
    });

    let row = rows.session_request(SessionRequestId(4)).unwrap();
    assert_eq!(row.action, WrapUpMode::Pr);
    assert_eq!(row.state, SessionRequestState::Done);
    assert!(row.deadline.is_none());
    assert!(
        !row.has_host_work(),
        "a pr wrap_up needs nothing from its host"
    );
    assert!(!row.is_open());
}

/// An exit_session row carries the token it consumed, the window to tear
/// down, the pr url the close recorded and the subtask the chain requested.
#[test]
fn an_exit_session_row_decodes_its_references() {
    let rows = SharedRows::new();
    rows.upsert_session_request(&bindings::SessionRequest {
        kind: "exit_session".into(),
        action: "pr".into(),
        pr_url: "https://github.com/o/r/pull/9".into(),
        wrap_up_request: 3,
        tmux_window: "task-7".into(),
        chained_task: 12,
        deadline: "2026-10-12 12:00:30.000".into(),
        ..requested_row(5)
    });

    let row = rows.session_request(SessionRequestId(5)).unwrap();
    assert_eq!(row.kind, SessionRequestKind::ExitSession);
    assert_eq!(row.pr_url.as_deref(), Some("https://github.com/o/r/pull/9"));
    assert_eq!(row.wrap_up_request, Some(SessionRequestId(3)));
    assert_eq!(row.tmux_window.as_ref().map(|w| w.as_str()), Some("task-7"));
    assert_eq!(row.chained_task, Some(TaskId(12)));
    assert!(row.has_host_work());
}

/// A failed row carries its reason.
#[test]
fn a_failed_row_decodes_its_reason() {
    let rows = SharedRows::new();
    rows.upsert_session_request(&bindings::SessionRequest {
        state: "failed".into(),
        claimed_at: "2026-10-12 12:00:01.000".into(),
        finished_at: "2026-10-12 12:02:00.000".into(),
        reason: "deadline passed".into(),
        ..requested_row(6)
    });

    let row = rows.session_request(SessionRequestId(6)).unwrap();
    assert_eq!(row.state, SessionRequestState::Failed);
    assert_eq!(row.reason.as_deref(), Some("deadline passed"));
    assert_eq!(row.claimed_at, Some(at("2026-10-12 12:00:01.000")));
    assert_eq!(row.finished_at, Some(at("2026-10-12 12:02:00.000")));
    assert_eq!(rows.session_requests().len(), 1);
}

/// A row with a state, kind or action this binary does not know is dropped,
/// not misread as open or finished.
#[test]
fn an_undecodable_row_is_dropped() {
    let rows = SharedRows::new();
    for (state, kind, action) in [
        ("teleporting", "wrap_up", "rebase"),
        ("requested", "teardown", "rebase"),
        ("requested", "wrap_up", "squash"),
    ] {
        rows.upsert_session_request(&bindings::SessionRequest {
            state: state.into(),
            kind: kind.into(),
            action: action.into(),
            ..requested_row(8)
        });
        assert!(
            rows.session_request(SessionRequestId(8)).is_none(),
            "{state}/{kind}/{action} must not be readable"
        );
    }
    assert!(rows.session_requests().is_empty());
}

/// A delete from the subscription removes the row, and the rows are listed
/// oldest first.
#[test]
fn a_removed_row_is_gone_and_the_rest_are_ordered_by_id() {
    let rows = SharedRows::new();
    rows.upsert_session_request(&requested_row(11));
    rows.upsert_session_request(&requested_row(9));
    rows.upsert_session_request(&requested_row(10));
    rows.remove_session_request(10);

    let ids: Vec<i64> = rows.session_requests().iter().map(|r| r.id.0).collect();
    assert_eq!(ids, vec![9, 11]);
    assert!(rows.session_request(SessionRequestId(10)).is_none());
}

/// Rows move the change signal a waiting `wrap_up` wakes on, like any other
/// subscribed row.
#[test]
fn a_session_row_moves_the_change_signal() {
    let rows = SharedRows::new();
    let mut changes = rows.changed();
    changes.borrow_and_update();
    rows.upsert_session_request(&requested_row(1));
    assert!(changes.has_changed().unwrap());
}
