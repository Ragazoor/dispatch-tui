#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: end-to-end hook-event flow through `TaskService`.
//!
//! Exercises `record_hook_event` for all `HookEventKind` variants on a Running
//! task and asserts that sub_status, status, and timestamp fields update as
//! specified in `docs/specs/tasks.allium`.

use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use dispatch_tui::db::{self, Database};
use dispatch_tui::models::{HookEventKind, ProjectId, SubStatus, TaskStatus};
use dispatch_tui::service::{ClaimTaskParams, CreateTaskParams, TaskService};

#[test]
fn hook_event_flow_drives_sub_status_and_lifecycle() {
    let db: Arc<dyn db::TaskAndEpicStore> = Arc::new(Database::open_in_memory().unwrap());
    let svc = TaskService::new(db);

    // 1. Create a Backlog task and claim it into Running with a worktree +
    //    tmux_window. claim_task derives the repo from the worktree path by
    //    stripping `/.worktrees/<anything>` so the worktree must live under
    //    `<repo_path>/.worktrees/...`.
    let id = svc
        .create_task(CreateTaskParams {
            title: "active health".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: ProjectId(1),
        })
        .unwrap();

    svc.claim_task(ClaimTaskParams {
        task_id: id,
        worktree: "/repo/.worktrees/active-health".into(),
        tmux_window: "task-active".into(),
    })
    .unwrap();

    let t = svc.get_task(id).unwrap();
    assert_eq!(t.status, TaskStatus::Running);

    // 2. PreToolUse → Active + last_pre_tool_use_at set.
    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .unwrap();
    let t = svc.get_task(id).unwrap();
    assert_eq!(t.sub_status, SubStatus::Active);
    assert!(t.last_pre_tool_use_at.is_some());

    // Sleep ≥ 1s so subsequent timestamps are strictly ordered. Timestamps
    // are persisted at second resolution (%Y-%m-%d %H:%M:%S), so finer sleeps
    // round to the same value.
    sleep(Duration::from_millis(1100));

    // 3. Notification → NeedsInput + last_notification_at set.
    svc.record_hook_event(id, HookEventKind::Notification)
        .unwrap();
    let t = svc.get_task(id).unwrap();
    assert_eq!(t.sub_status, SubStatus::NeedsInput);
    assert!(t.last_notification_at.is_some());

    sleep(Duration::from_millis(1100));

    // 4. PreToolUse again → newer event wins, classifier returns Active.
    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .unwrap();
    let t = svc.get_task(id).unwrap();
    assert_eq!(t.sub_status, SubStatus::Active);
    let pre = t.last_pre_tool_use_at.unwrap();
    let notif = t.last_notification_at.unwrap();
    assert!(
        pre > notif,
        "PreToolUse timestamp ({pre}) should be newer than Notification timestamp ({notif})"
    );

    // 5. Stop → Review with default Review sub_status, both timestamps cleared.
    svc.record_hook_event(id, HookEventKind::Stop).unwrap();
    let t = svc.get_task(id).unwrap();
    assert_eq!(t.status, TaskStatus::Review);
    assert_eq!(t.sub_status, SubStatus::default_for(TaskStatus::Review));
    assert!(t.last_pre_tool_use_at.is_none());
    assert!(t.last_notification_at.is_none());
}
