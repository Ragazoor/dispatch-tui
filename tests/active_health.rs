#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: end-to-end hook-event flow through `TaskService`.

use std::sync::Arc;
use std::time::Duration;

use dispatch_tui::db::{self, Database};
use dispatch_tui::models::{HookEventKind, ProjectId, SubStatus, TaskStatus};
use dispatch_tui::service::{ClaimTaskParams, CreateTaskParams, TaskService};

#[tokio::test]
async fn hook_event_flow_drives_sub_status_and_lifecycle() {
    let db: Arc<dyn db::TaskAndEpicStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let svc = TaskService::new(db);

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
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    svc.claim_task(ClaimTaskParams {
        task_id: id,
        worktree: "/repo/.worktrees/active-health".into(),
        tmux_window: "task-active".into(),
    })
    .await
    .unwrap();

    let t = svc.get_task(id).await.unwrap();
    assert_eq!(t.status, TaskStatus::Running);

    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .await
        .unwrap();
    let t = svc.get_task(id).await.unwrap();
    assert_eq!(t.sub_status, SubStatus::Active);
    assert!(t.last_pre_tool_use_at.is_some());

    // Timestamps are persisted at second resolution; sleep ≥1s to maintain order.
    tokio::time::sleep(Duration::from_millis(1100)).await;

    svc.record_hook_event(id, HookEventKind::Notification)
        .await
        .unwrap();
    let t = svc.get_task(id).await.unwrap();
    assert_eq!(t.sub_status, SubStatus::NeedsInput);
    assert!(t.last_notification_at.is_some());

    tokio::time::sleep(Duration::from_millis(1100)).await;

    svc.record_hook_event(id, HookEventKind::PreToolUse)
        .await
        .unwrap();
    let t = svc.get_task(id).await.unwrap();
    assert_eq!(t.sub_status, SubStatus::Active);
    let pre = t.last_pre_tool_use_at.unwrap();
    let notif = t.last_notification_at.unwrap();
    assert!(
        pre > notif,
        "PreToolUse {pre} should be newer than Notification {notif}"
    );

    svc.record_hook_event(id, HookEventKind::Stop)
        .await
        .unwrap();
    let t = svc.get_task(id).await.unwrap();
    assert_eq!(t.status, TaskStatus::Review);
    assert_eq!(t.sub_status, SubStatus::default_for(TaskStatus::Review));
    assert!(t.last_pre_tool_use_at.is_none());
    assert!(t.last_notification_at.is_none());
}
