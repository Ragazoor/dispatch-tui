//! `record_notification` / `record_pre_tool_use` — the two Claude Code hook
//! writes that carry their own guard.
//!
//! Both apply as a single conditional UPDATE rather than a snapshot read
//! followed by a patch. Every hook is its own OS process, so a value read
//! beforehand can already be stale by the time the write lands; these tests
//! exercise exactly that ordering. The sibling hook writes with the same shape
//! are covered in [`super::subagents`] (`try_record_stop`) — they live there
//! because their fixture is a live subagent.
//!
//! See `HookNotification` and `HookPreToolUse` in
//! `docs/specs/agent-health.allium`.
use super::*;
use crate::models::NotificationWrite;
use chrono::Utc;

/// A plain Running task with no activity stamps.
///
/// Deliberately not `subagents::set_running` (allow-phantom-symbol: removed helper), which also stamps
/// `last_pre_tool_use_at` and `last_notification_at` — every assertion below
/// turns on one of those still being null when the write under test runs.
async fn running_task(db: &Store) -> Task {
    let task = make_task(db, "t").await;
    db.patch_task(
        task.id,
        &crate::store::TaskPatch::new().status(TaskStatus::Running),
    )
    .await
    .unwrap();
    task
}

#[tokio::test]
async fn record_notification_is_a_no_op_on_a_task_that_left_running() {
    let db = in_memory_db().await;
    let task = make_task(&db, "t").await;
    // Never running: the status predicate must reject the write silently
    // rather than error, because the hook observed a state that has moved on.
    let write = NotificationWrite::from_kind(None);

    db.record_notification(task.id, write, Utc::now())
        .await
        .unwrap();

    let reread = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(reread.status, TaskStatus::Backlog);
    assert_eq!(reread.sub_status, SubStatus::None);
    assert!(reread.last_notification_at.is_none());
}

#[tokio::test]
async fn record_pre_tool_use_is_a_no_op_on_a_task_that_left_running() {
    let db = in_memory_db().await;
    let task = make_task(&db, "t").await;
    // The status guard has to ride in the write for the same reason
    // record_notification's does: a concurrent Stop can flip the row to review
    // between the service's read and this write, and an unconditional patch
    // would then write (review, active) and trip the tasks CHECK constraint.
    db.record_pre_tool_use(task.id, SubStatus::Active, Utc::now())
        .await
        .unwrap();

    let reread = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(reread.status, TaskStatus::Backlog);
    assert_eq!(reread.sub_status, SubStatus::None);
    assert!(reread.last_pre_tool_use_at.is_none());
}

#[tokio::test]
async fn record_pre_tool_use_stamps_and_sets_sub_status_on_a_running_task() {
    let db = in_memory_db().await;
    let task = running_task(&db).await;

    db.record_pre_tool_use(task.id, SubStatus::Stale, Utc::now())
        .await
        .unwrap();

    let reread = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(reread.sub_status, SubStatus::Stale);
    assert!(reread.last_pre_tool_use_at.is_some());
}
