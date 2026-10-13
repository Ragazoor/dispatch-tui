//! The in-memory twin of the M6 PR reducers.
//!
//! Spec: `docs/specs/store-split.allium` — `ReportPrStatus`,
//! `ReportPrUnreadable`. The board's tests drive PR state through this double,
//! so it must write the task exactly where the module does;
//! `tests/memory_caller_conformance.rs` diffs the two.

use super::*;
use crate::models::{SubStatus, TaskStatus};

const HOST: &str = "host-a";
const ME: &str = "c0ffee";

fn caller() -> (MemoryReducerCaller, Arc<SharedRows>) {
    let (caller, rows) = super::caller();
    (caller.with_sender(ME).with_registered_host(HOST, ME), rows)
}

async fn review_task(caller: &MemoryReducerCaller, sub_status: &str) -> TaskId {
    caller
        .create_task(bindings::Task {
            status: "review".into(),
            sub_status: sub_status.into(),
            url: "https://github.com/acme/repo/pull/7".into(),
            url_type: "pr".into(),
            tmux_window: "task-1".into(),
            ..blank_task()
        })
        .await
        .unwrap()
}

async fn report(
    caller: &MemoryReducerCaller,
    task: TaskId,
    state: &str,
    decision: &str,
) -> ReducerOutcome {
    caller
        .report_pr_status(task, HOST.into(), state.into(), decision.into())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_merged_report_takes_the_task_to_done_and_clears_its_window() {
    let (caller, rows) = caller();
    let task = review_task(&caller, "approved").await;
    assert!(report(&caller, task, "merged", "APPROVED").await.won());
    let row = rows.task(task).unwrap();
    assert_eq!(row.status, TaskStatus::Done);
    assert_eq!(row.sub_status, SubStatus::None);
    assert!(row.tmux_window.is_none());
    assert!(row.completed_at.is_some());
}

#[tokio::test]
async fn a_closed_report_marks_pr_closed_and_an_open_one_follows_the_decision() {
    let (caller, rows) = caller();
    let task = review_task(&caller, "awaiting_review").await;
    assert!(report(&caller, task, "OPEN", "CHANGES_REQUESTED")
        .await
        .won());
    assert_eq!(
        rows.task(task).unwrap().sub_status,
        SubStatus::ChangesRequested
    );
    assert!(report(&caller, task, "CLOSED", "").await.won());
    let row = rows.task(task).unwrap();
    assert_eq!(row.status, TaskStatus::Review);
    assert_eq!(row.sub_status, SubStatus::PrClosed);
}

#[tokio::test]
async fn an_open_report_leaves_a_conflict_alone() {
    let (caller, rows) = caller();
    let task = review_task(&caller, "conflict").await;
    assert!(report(&caller, task, "OPEN", "APPROVED").await.won());
    assert_eq!(rows.task(task).unwrap().sub_status, SubStatus::Conflict);
}

#[tokio::test]
async fn an_unknown_string_is_refused_and_writes_nothing() {
    let (caller, rows) = caller();
    let task = review_task(&caller, "awaiting_review").await;
    assert!(!report(&caller, task, "DRAFT", "").await.won());
    assert!(!report(&caller, task, "OPEN", "MAYBE").await.won());
    assert_eq!(
        rows.task(task).unwrap().sub_status,
        SubStatus::AwaitingReview
    );
}

#[tokio::test]
async fn a_report_for_a_missing_task_is_accepted_and_changes_nothing() {
    let (caller, _rows) = caller();
    assert!(report(&caller, TaskId(999), "MERGED", "").await.won());
}

#[tokio::test]
async fn a_host_the_caller_does_not_own_is_refused() {
    let (caller, rows) = caller();
    let task = review_task(&caller, "awaiting_review").await;
    let outcome = caller
        .report_pr_status(task, "host-elsewhere".into(), "MERGED".into(), "".into())
        .await
        .unwrap();
    assert!(!outcome.won());
    assert_eq!(rows.task(task).unwrap().status, TaskStatus::Review);
}

#[tokio::test]
async fn an_unreadable_report_marks_pr_unreachable() {
    let (caller, rows) = caller();
    let task = review_task(&caller, "awaiting_review").await;
    assert!(caller
        .report_pr_unreadable(task, HOST.into(), "no access".into())
        .await
        .unwrap()
        .won());
    assert_eq!(
        rows.task(task).unwrap().sub_status,
        SubStatus::PrUnreachable
    );
    // A later successful read clears it.
    assert!(report(&caller, task, "OPEN", "APPROVED").await.won());
    assert_eq!(rows.task(task).unwrap().sub_status, SubStatus::Approved);
}
