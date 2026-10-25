//! `schedules.allium` through the real reducers: what needs a DUE task. The
//! store arms `next_due` from its own clock and a client cannot plant one, so
//! these move `TestClock` past it instead of waiting.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// Every minute: due once the clock moves a minute on.
const EVERY_MINUTE: &str = "* * * * *";

fn scheduled_task(store: &Store, phoenix: bool) -> i64 {
    store.register_host_for(HOST_A, "alice");
    store.create(
        "alice",
        Task {
            schedule: EVERY_MINUTE.into(),
            phoenix,
            ..user_task("t", "alice")
        },
    )
}

fn request(store: &Store, task_id: i64, origin: &str) -> Result<(), String> {
    store.call("alice", |ctx| {
        request_dispatch(ctx, task_id, HOST_A.into(), origin.into(), String::new())
    })
}

/// The newest request row for `task_id`.
fn request_for(store: &Store, task_id: i64) -> DispatchRequest {
    store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == task_id)
        .max_by_key(|r| r.id)
        .expect("a request for the task")
}

fn claim(store: &Store, req: i64) {
    store
        .call("alice", |ctx| {
            claim_dispatch_request(ctx, req, HOST_A.into())
        })
        .unwrap();
}

fn fail(store: &Store, req: i64) {
    store
        .call("alice", |ctx| {
            report_dispatch_failed(ctx, req, HOST_A.into(), "repo missing".into())
        })
        .unwrap();
}

#[test]
fn a_schedule_origin_request_waits_for_the_due_time() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    assert!(!store.task(task).next_due.is_empty(), "create arms it");

    let why = request(&store, task, "schedule").unwrap_err();
    assert_eq!(why, format!("task {task} is not due"));

    store.advance_secs(61);
    request(&store, task, "schedule").unwrap();
    assert_eq!(request_for(&store, task).origin, "schedule");
    assert_eq!(store.task(task).status, "running");
}

#[test]
fn a_dispatched_report_consumes_the_due_time() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    store.advance_secs(61);
    request(&store, task, "schedule").unwrap();
    let req = request_for(&store, task).id;
    claim(&store, req);
    store
        .call("alice", |ctx| {
            report_dispatch_provisioned(ctx, req, HOST_A.into(), "/wt".into(), "w".into(), false)
        })
        .unwrap();

    let after = store.task(task);
    assert_eq!(after.next_due, "");
    assert_eq!(after.schedule, EVERY_MINUTE, "kept as a record");
}

#[test]
fn a_failed_scheduled_one_off_returns_to_backlog_marked_and_disarmed() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    store.advance_secs(61);
    request(&store, task, "schedule").unwrap();
    let req = request_for(&store, task).id;
    claim(&store, req);
    fail(&store, req);

    let after = store.task(task);
    assert_eq!(after.status, "backlog");
    assert_eq!(after.schedule_error, "repo missing");
    assert_eq!(after.next_due, "", "a one-off is not retried");
}

#[test]
fn a_failed_scheduled_phoenix_is_marked_and_rearmed_ahead_of_now() {
    let store = Store::new();
    let task = scheduled_task(&store, true);
    store.advance_secs(61);
    let due_before = store.task(task).next_due;
    request(&store, task, "schedule").unwrap();
    let req = request_for(&store, task).id;
    claim(&store, req);
    fail(&store, req);

    let after = store.task(task);
    assert_eq!(after.status, "backlog");
    assert_eq!(after.schedule_error, "repo missing");
    assert!(
        after.next_due > due_before,
        "re-armed from now: {} after {}",
        after.next_due,
        due_before
    );
    // ... and is not due again at once.
    let why = request(&store, task, "schedule").unwrap_err();
    assert_eq!(why, format!("task {task} is not due"));
}

#[test]
fn a_failed_launch_of_another_origin_leaves_the_schedule_alone() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    let due_before = store.task(task).next_due;
    request(&store, task, "user").unwrap();
    let req = request_for(&store, task).id;
    claim(&store, req);
    fail(&store, req);

    let after = store.task(task);
    assert_eq!(after.status, "backlog");
    assert_eq!(after.schedule_error, "");
    assert_eq!(after.next_due, due_before);
}

/// The backstop fails a schedule-origin request too, through the same path.
#[test]
fn the_backstop_marks_a_scheduled_launch_that_never_reported() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    store.advance_secs(61);
    request(&store, task, "schedule").unwrap();
    let req = request_for(&store, task).id;
    store.advance_secs(DISPATCH_REQUEST_TIMEOUT_SECS + DISPATCH_REQUEST_BACKSTOP_GRACE_SECS);
    let timer = store
        .db()
        .dispatch_request_timeouts()
        .iter()
        .find(|t| t.request_id == req)
        .expect("the request's timer");
    store
        .call_as_scheduler(|ctx| expire_dispatch_request(ctx, timer))
        .unwrap();

    let after = store.task(task);
    assert_eq!(after.status, "backlog");
    assert_eq!(after.schedule_error, DISPATCH_REASON_TIMED_OUT);
    assert_eq!(after.next_due, "");
}

#[test]
fn a_hand_move_out_of_backlog_cancels_the_run_and_a_move_back_does_not_rearm() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("review")))
        .unwrap();
    assert_eq!(store.task(task).next_due, "");
    store
        .call("alice", |ctx| {
            patch_task(ctx, task, status_patch("backlog"))
        })
        .unwrap();
    assert_eq!(store.task(task).next_due, "", "a spent one-off stays spent");
    assert_eq!(store.task(task).schedule, EVERY_MINUTE);
}

/// Review 2026-10-23 C4 (task #98505): a phoenix run that fired and was
/// abandoned is re-armed by a hand move back to backlog.
#[test]
fn a_hand_move_back_to_backlog_rearms_a_fired_phoenix() {
    let store = Store::new();
    let task = scheduled_task(&store, true);
    store.advance_secs(61);
    request(&store, task, "schedule").unwrap();
    let req = request_for(&store, task).id;
    claim(&store, req);
    store
        .call("alice", |ctx| {
            report_dispatch_provisioned(ctx, req, HOST_A.into(), "/wt".into(), "w".into(), false)
        })
        .unwrap();
    assert_eq!(store.task(task).next_due, "", "the launch consumed it");

    store
        .call("alice", |ctx| {
            patch_task(ctx, task, status_patch("backlog"))
        })
        .unwrap();
    let after = store.task(task);
    assert!(!after.next_due.is_empty(), "re-armed");
    let why = request(&store, task, "schedule").unwrap_err();
    assert_eq!(why, format!("task {task} is not due"), "armed ahead of now");
}

/// Review 2026-10-23 C5 (task #98505): a close fails the open schedule-origin
/// request after the Done write, and the Done row keeps no schedule state.
#[test]
fn a_close_with_an_open_scheduled_launch_leaves_no_schedule_state_on_done() {
    for phoenix in [false, true] {
        let store = Store::new();
        let task = scheduled_task(&store, phoenix);
        store.advance_secs(61);
        request(&store, task, "schedule").unwrap();
        store
            .call("alice", |ctx| close_task(ctx, task, HOST_A.into()))
            .unwrap();

        let after = store.task(task);
        assert_eq!(after.status, "done");
        assert_eq!(after.next_due, "", "phoenix {phoenix}");
        assert_eq!(after.schedule_error, "", "phoenix {phoenix}");
        assert_eq!(request_for(&store, task).state, DISPATCH_FAILED);
    }
}

/// Review 2026-10-23 C5: a close while a user-origin request holds the claim
/// spends a one-off, so a later move back to backlog does not fire it.
#[test]
fn a_closed_one_off_moved_back_to_backlog_stays_spent() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    request(&store, task, "user").unwrap();
    store
        .call("alice", |ctx| close_task(ctx, task, HOST_A.into()))
        .unwrap();
    assert_eq!(store.task(task).next_due, "");
    store
        .call("alice", |ctx| {
            patch_task(ctx, task, status_patch("backlog"))
        })
        .unwrap();
    assert_eq!(store.task(task).next_due, "");
}

/// Review 2026-10-23 C5: a hand move into done is a close too, so a claimed
/// one-off moved there and back to backlog does not fire at once.
#[test]
fn a_hand_move_of_a_claimed_one_off_into_done_spends_it() {
    let store = Store::new();
    let task = scheduled_task(&store, false);
    request(&store, task, "user").unwrap();
    assert!(!store.task(task).next_due.is_empty(), "the claim keeps it");
    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("done")))
        .unwrap();
    assert_eq!(store.task(task).next_due, "");
    store
        .call("alice", |ctx| {
            patch_task(ctx, task, status_patch("backlog"))
        })
        .unwrap();
    assert_eq!(store.task(task).next_due, "");
}
