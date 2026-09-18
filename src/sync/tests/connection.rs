//! The connection lifecycle: every edge in `sync.allium`'s transition graph,
//! and every edge that is not in it.

use crate::sync::{
    backoff, BoardConnection, ConnectionEvent, ConnectionStatus, RECONNECT_BACKOFF_BASE,
    RECONNECT_BACKOFF_MAX,
};
use std::time::{Duration, Instant};

fn failed(reason: &str) -> ConnectionEvent {
    ConnectionEvent::AttemptFailed {
        reason: reason.into(),
    }
}

fn dropped(reason: &str) -> ConnectionEvent {
    ConnectionEvent::Dropped {
        reason: reason.into(),
    }
}

/// `OpenBoardConnection`: the board draws first and connects after, so the
/// state a board is drawn in is `Connecting` and it is a healthy one.
#[test]
fn a_new_connection_starts_connecting_with_nothing_wrong() {
    let connection = BoardConnection::opening("store.example");

    assert_eq!(connection.status(), ConnectionStatus::Connecting);
    assert_eq!(connection.attempts(), 0);
    assert_eq!(connection.last_error(), None);
    assert!(connection.status().is_healthy());
    assert_eq!(connection.next_attempt_at(), None);
}

#[test]
fn acceptance_connects_and_clears_the_outage() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");
    connection.apply(failed("refused"), now);
    connection.apply(ConnectionEvent::RetryDue, now);

    assert!(connection.apply(ConnectionEvent::Accepted, now));

    assert_eq!(connection.status(), ConnectionStatus::Connected);
    assert_eq!(
        connection.attempts(),
        0,
        "an outage that ends is over, not partly over"
    );
    assert_eq!(connection.last_error(), None);
}

/// Test 5 of the phase plan. A board whose store was never reachable is UP,
/// disconnected, and says why. It is not an abort: the board's own data is
/// local, and a store that is down does not make it unreadable.
#[test]
fn a_first_attempt_that_fails_leaves_the_board_up_and_saying_why() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");

    assert!(connection.apply(failed("connection refused"), now));

    assert_eq!(connection.status(), ConnectionStatus::Disconnected);
    assert_eq!(connection.attempts(), 1);
    assert_eq!(connection.last_error(), Some("connection refused"));
    assert!(
        !connection.status().is_healthy(),
        "the operator must be told"
    );
}

#[test]
fn a_drop_from_connected_lands_in_the_same_place_as_a_failed_attempt() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");
    connection.apply(ConnectionEvent::Accepted, now);

    assert!(connection.apply(dropped("socket closed"), now));

    assert_eq!(connection.status(), ConnectionStatus::Disconnected);
    assert_eq!(
        connection.attempts(),
        1,
        "an outage beginning from health backs off like any other"
    );
    assert_eq!(connection.last_error(), Some("socket closed"));
}

#[test]
fn retrying_clears_the_reason_and_goes_back_to_connecting() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");
    connection.apply(failed("refused"), now);

    assert!(connection.apply(ConnectionEvent::RetryDue, now));

    assert_eq!(connection.status(), ConnectionStatus::Connecting);
    assert_eq!(connection.last_error(), None);
    assert_eq!(
        connection.next_attempt_at(),
        None,
        "nothing is pending while an attempt is in flight"
    );
    assert_eq!(
        connection.attempts(),
        1,
        "the attempt counter belongs to the outage, not to the state"
    );
}

/// `StopOnAUserIdentityConflict`. Terminal, and the only fatal thing here.
#[test]
fn an_identity_conflict_is_terminal_and_names_both_identities() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");
    connection.apply(ConnectionEvent::Accepted, now);

    assert!(connection.apply(
        ConnectionEvent::IdentityConflict {
            stored: "user-a".into(),
            offered: "user-b".into(),
        },
        now,
    ));

    assert_eq!(connection.status(), ConnectionStatus::Failed);
    let message = connection.last_error().unwrap();
    assert!(message.contains("user-a"), "{message}");
    assert!(message.contains("user-b"), "{message}");
}

/// Retrying an identity conflict retries it forever: nothing about a retry
/// changes which person the store believes this is.
#[test]
fn a_failed_connection_never_becomes_due_for_retry() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");
    connection.apply(ConnectionEvent::Accepted, now);
    connection.apply(
        ConnectionEvent::IdentityConflict {
            stored: "user-a".into(),
            offered: "user-b".into(),
        },
        now,
    );

    assert_eq!(connection.next_attempt_at(), None);
    assert!(!connection.is_retry_due(now + Duration::from_secs(86_400)));
    assert!(
        !connection.apply(ConnectionEvent::RetryDue, now),
        "`failed` is terminal in the graph"
    );
}

/// Every edge the transition graph does NOT have.
///
/// Asserted as a table rather than one test per pair, because the property
/// under test is the closed set: an implementation that accepted one extra edge
/// would pass a test suite that only checked the edges it knows about.
#[test]
fn events_that_do_not_match_the_current_state_do_not_fire() {
    let now = Instant::now();

    /// (name, the state to start from, the event that must not fire).
    type Case = (&'static str, fn() -> BoardConnection, ConnectionEvent);

    let cases: Vec<Case> = vec![
        // A retry while an attempt is already in flight.
        ("connecting/retry", connecting, ConnectionEvent::RetryDue),
        // A drop reported against a connection that was never up.
        ("connecting/drop", connecting, dropped("x")),
        // Acceptance of a connection nobody attempted.
        (
            "disconnected/accept",
            disconnected,
            ConnectionEvent::Accepted,
        ),
        // A second drop for one outage; the counter must not double-count.
        ("disconnected/drop", disconnected, dropped("x")),
        ("disconnected/fail", disconnected, failed("x")),
        // A conflict cannot be discovered on a connection that is not up.
        (
            "connecting/conflict",
            connecting,
            ConnectionEvent::IdentityConflict {
                stored: "a".into(),
                offered: "b".into(),
            },
        ),
        ("connected/accept", connected, ConnectionEvent::Accepted),
        ("connected/fail", connected, failed("x")),
    ];

    for (name, build, event) in cases {
        let mut connection = build();
        let before = connection.status();
        let attempts_before = connection.attempts();

        assert!(
            !connection.apply(event, now),
            "{name}: the event should not have fired"
        );
        assert_eq!(connection.status(), before, "{name}: status changed anyway");
        assert_eq!(
            connection.attempts(),
            attempts_before,
            "{name}: the attempt counter moved anyway"
        );
    }
}

fn connecting() -> BoardConnection {
    BoardConnection::opening("store.example")
}

fn connected() -> BoardConnection {
    let mut connection = connecting();
    connection.apply(ConnectionEvent::Accepted, Instant::now());
    connection
}

fn disconnected() -> BoardConnection {
    let mut connection = connecting();
    connection.apply(failed("refused"), Instant::now());
    connection
}

// -- backoff ---------------------------------------------------------------

#[test]
fn the_first_retry_waits_the_base_and_each_one_after_doubles() {
    assert_eq!(backoff(1), RECONNECT_BACKOFF_BASE);
    assert_eq!(backoff(2), RECONNECT_BACKOFF_BASE * 2);
    assert_eq!(backoff(3), RECONNECT_BACKOFF_BASE * 4);
    assert_eq!(backoff(4), RECONNECT_BACKOFF_BASE * 8);
}

/// The ceiling is for the hour-long outage. Unbounded doubling would have the
/// board checking once a day by the time somebody fixed it.
#[test]
fn the_backoff_never_exceeds_the_ceiling() {
    for attempts in 1..=64 {
        assert!(
            backoff(attempts) <= RECONNECT_BACKOFF_MAX,
            "attempt {attempts} waited {:?}",
            backoff(attempts)
        );
    }
    assert_eq!(backoff(64), RECONNECT_BACKOFF_MAX);
}

/// The specific trap: an exponent large enough to overflow must saturate at the
/// ceiling, never wrap to a short wait. Wrapping would turn the longest outage
/// into the busiest retry loop.
#[test]
fn an_overflowing_exponent_saturates_rather_than_wrapping() {
    assert_eq!(backoff(u32::MAX), RECONNECT_BACKOFF_MAX);
}

#[test]
fn the_retry_is_due_exactly_at_the_backoff_and_not_before() {
    let now = Instant::now();
    let mut connection = BoardConnection::opening("store.example");
    connection.apply(failed("refused"), now);

    let due = connection.next_attempt_at().unwrap();
    assert_eq!(due, now + RECONNECT_BACKOFF_BASE);
    assert!(!connection.is_retry_due(now));
    assert!(!connection.is_retry_due(due - Duration::from_millis(1)));
    assert!(connection.is_retry_due(due));
}

/// The schedule is measured from the MOST RECENT failure, not the first, so a
/// long outage does not fire a burst of retries the moment it is noticed.
#[test]
fn each_failure_restarts_the_clock_at_a_longer_interval() {
    let start = Instant::now();
    let mut connection = BoardConnection::opening("store.example");

    connection.apply(failed("one"), start);
    assert_eq!(
        connection.next_attempt_at(),
        Some(start + RECONNECT_BACKOFF_BASE)
    );

    let second = start + Duration::from_secs(5);
    connection.apply(ConnectionEvent::RetryDue, second);
    connection.apply(failed("two"), second);
    assert_eq!(
        connection.next_attempt_at(),
        Some(second + RECONNECT_BACKOFF_BASE * 2)
    );
}
