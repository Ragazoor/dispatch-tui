//! The in-memory twin of `schedules.allium`'s due path, on a clock the test
//! moves: what `tests/memory_caller_conformance/schedules.rs` cannot reach
//! without waiting a real minute. The module's side of the same is
//! `spacetime/module-tests/tests/schedules.rs`.

use super::dispatch_requests::{caller_with_clock, request, request_id, HOST};
use super::*;
use crate::models::DispatchRequestId;

async fn scheduled(caller: &MemoryReducerCaller, phoenix: bool) -> TaskId {
    caller
        .create_task(bindings::Task {
            schedule: "* * * * *".into(),
            phoenix,
            ..blank_task()
        })
        .await
        .unwrap()
}

async fn claim_and_fail(caller: &MemoryReducerCaller, id: DispatchRequestId) {
    assert!(caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap()
        .won());
    assert!(caller
        .report_dispatch_failed(id, HOST.into(), "repo missing".into())
        .await
        .unwrap()
        .won());
}

#[tokio::test]
async fn a_schedule_origin_request_waits_for_the_due_time() {
    let (caller, _rows, clock) = caller_with_clock();
    let task = scheduled(&caller, false).await;
    match request(&caller, task, HOST, "schedule", "").await {
        ReducerOutcome::Refused(why) => assert_eq!(why, format!("task {} is not due", task.0)),
        other => panic!("expected a refusal, got {other:?}"),
    }
    clock.advance(chrono::Duration::seconds(61));
    request_id(request(&caller, task, HOST, "schedule", "").await);
}

#[tokio::test]
async fn a_dispatched_report_consumes_the_due_time() {
    let (caller, rows, clock) = caller_with_clock();
    let task = scheduled(&caller, false).await;
    clock.advance(chrono::Duration::seconds(61));
    let id = request_id(request(&caller, task, HOST, "schedule", "").await);
    assert!(caller
        .claim_dispatch_request(id, HOST.into())
        .await
        .unwrap()
        .won());
    assert!(caller
        .report_dispatch_provisioned(id, HOST.into(), "/wt".into(), "w".into(), false)
        .await
        .unwrap()
        .won());
    let after = rows.task(task).unwrap();
    assert_eq!(after.next_due, None);
    assert_eq!(after.schedule.as_deref(), Some("* * * * *"));
}

#[tokio::test]
async fn a_failed_scheduled_one_off_is_marked_and_disarmed() {
    let (caller, rows, clock) = caller_with_clock();
    let task = scheduled(&caller, false).await;
    clock.advance(chrono::Duration::seconds(61));
    let id = request_id(request(&caller, task, HOST, "schedule", "").await);
    claim_and_fail(&caller, id).await;
    let after = rows.task(task).unwrap();
    assert_eq!(after.status, crate::models::TaskStatus::Backlog);
    assert_eq!(after.schedule_error.as_deref(), Some("repo missing"));
    assert_eq!(after.next_due, None);
}

#[tokio::test]
async fn a_failed_scheduled_phoenix_is_rearmed_ahead_of_now() {
    let (caller, rows, clock) = caller_with_clock();
    let task = scheduled(&caller, true).await;
    clock.advance(chrono::Duration::seconds(61));
    let id = request_id(request(&caller, task, HOST, "schedule", "").await);
    claim_and_fail(&caller, id).await;
    let after = rows.task(task).unwrap();
    assert_eq!(after.schedule_error.as_deref(), Some("repo missing"));
    assert!(after.next_due.is_some_and(|due| due > clock.now()));
}

#[tokio::test]
async fn a_failed_launch_of_another_origin_leaves_the_schedule_alone() {
    let (caller, rows, _clock) = caller_with_clock();
    let task = scheduled(&caller, false).await;
    let due_before = rows.task(task).unwrap().next_due;
    let id = request_id(request(&caller, task, HOST, "user", "").await);
    claim_and_fail(&caller, id).await;
    let after = rows.task(task).unwrap();
    assert_eq!(after.schedule_error, None);
    assert_eq!(after.next_due, due_before);
}

#[tokio::test]
async fn a_hand_move_out_of_backlog_cancels_the_pending_run() {
    let (caller, rows, _clock) = caller_with_clock();
    let task = scheduled(&caller, false).await;
    let outcome = caller
        .patch_task(
            task,
            bindings::TaskPatch {
                status: Some("review".into()),
                sub_status: Some("awaiting_review".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    assert!(outcome.won(), "{outcome:?}");
    assert_eq!(rows.task(task).unwrap().next_due, None);
}
