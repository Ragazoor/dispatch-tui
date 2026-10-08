use super::*;
use crate::models::test_tmux_window;

#[tokio::test]
async fn create_and_get() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "A description",
            ..CreateTaskRequest::fixture("My Task", "/repo/path")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().expect("task should exist");
    assert_eq!(task.id, id);
    assert_eq!(task.title, "My Task");
    assert_eq!(task.description, "A description");
    assert_eq!(task.repo_path, "/repo/path");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn list_all() {
    let db = in_memory_db().await;
    db.create_task(CreateTaskRequest {
        description: "desc",
        ..CreateTaskRequest::fixture("Task A", "/a")
    })
    .await
    .unwrap();
    db.create_task(CreateTaskRequest {
        description: "desc",
        ..CreateTaskRequest::fixture("Task B", "/b")
    })
    .await
    .unwrap();
    db.create_task(CreateTaskRequest {
        description: "desc",
        ..CreateTaskRequest::fixture("Task C", "/c")
    })
    .await
    .unwrap();
    let tasks = db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].title, "Task A");
    assert_eq!(tasks[1].title, "Task B");
    assert_eq!(tasks[2].title, "Task C");
}

#[tokio::test]
async fn get_nonexistent() {
    let db = in_memory_db().await;
    let result = db.get_task(TaskId(9999)).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn create_task_with_plan() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            plan: Some("docs/plan.md"),
            ..CreateTaskRequest::fixture("Planned Task", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.plan_path.as_deref(), Some("docs/plan.md"));
}

#[tokio::test]
async fn create_task_without_plan() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("Simple Task", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.plan_path.is_none());
}

#[tokio::test]
async fn find_task_by_plan_returns_match() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            plan: Some("/plans/my-plan.md"),
            ..CreateTaskRequest::fixture("Planned", "/repo")
        })
        .await
        .unwrap();

    let found = db.find_task_by_plan("/plans/my-plan.md").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, id);
}

#[tokio::test]
async fn find_task_by_plan_returns_none_when_no_match() {
    let db = in_memory_db().await;
    db.create_task(CreateTaskRequest {
        description: "desc",
        plan: Some("/plans/other.md"),
        ..CreateTaskRequest::fixture("Other", "/repo")
    })
    .await
    .unwrap();

    let found = db.find_task_by_plan("/plans/nonexistent.md").await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn find_task_by_plan_ignores_tasks_without_plan() {
    let db = in_memory_db().await;
    db.create_task(CreateTaskRequest {
        description: "desc",
        ..CreateTaskRequest::fixture("No Plan", "/repo")
    })
    .await
    .unwrap();

    let found = db.find_task_by_plan("/plans/any.md").await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn create_task_returning_returns_full_task() {
    let db = in_memory_db().await;
    let task = create_task_returning(&db, "Title", "Desc", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    assert_eq!(task.title, "Title");
    assert_eq!(task.description, "Desc");
    assert_eq!(task.repo_path, "/repo");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
    assert!(task.plan_path.is_none());
}

#[tokio::test]
async fn create_task_returning_with_plan() {
    let db = in_memory_db().await;
    let task = create_task_returning(&db, "T", "D", "/r", Some("plan.md"), TaskStatus::Backlog)
        .await
        .unwrap();
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn respawn_phoenix_successor_creates_task_with_labels_in_one_insert() {
    let db = in_memory_db().await;
    let predecessor = db
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Done,
            phoenix: true,
            ..CreateTaskRequest::fixture("Weekly audit", "/repo")
        })
        .await
        .unwrap();

    let labels = vec!["scala-common".to_string(), "security".to_string()];
    let successor_id = db
        .respawn_phoenix_successor(
            predecessor,
            CreateTaskRequest {
                description: "d",
                phoenix: true,
                ..CreateTaskRequest::fixture("Weekly audit", "/repo")
            },
            &labels,
        )
        .await
        .unwrap();

    let successor = db.get_task(successor_id).await.unwrap().unwrap();
    assert_eq!(
        successor.labels, labels,
        "labels land as part of the single insert, not a follow-up patch"
    );

    let predecessor_task = db.get_task(predecessor).await.unwrap().unwrap();
    assert!(
        !predecessor_task.phoenix,
        "the flag clears in the same transaction as the successor's creation"
    );
}

#[tokio::test]
async fn respawn_phoenix_successor_rolls_back_if_predecessor_is_gone() {
    let db = in_memory_db().await;
    // Far from any id sqlite would ever assign to the successor's own insert,
    // so the UPDATE below cannot accidentally hit the just-inserted successor
    // and must genuinely match zero rows.
    let predecessor = TaskId(999_999);

    let result = db
        .respawn_phoenix_successor(
            predecessor,
            CreateTaskRequest {
                description: "d",
                phoenix: true,
                ..CreateTaskRequest::fixture("Weekly audit", "/repo")
            },
            &[],
        )
        .await;
    assert!(
        result.is_err(),
        "a predecessor that vanished mid-flight must fail the whole call"
    );

    let all = db.list_all().await.unwrap();
    assert!(
        all.is_empty(),
        "no orphaned successor left behind when the transaction rolls back"
    );
}

#[tokio::test]
async fn patch_task_applies_all_fields() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    let patch = TaskPatch::new()
        .status(TaskStatus::Running)
        .plan_path(Some("plan.md"))
        .title("new title");
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.title, "new title");
    assert_eq!(task.description, "desc"); // unchanged
}

#[tokio::test]
async fn patch_task_none_fields_unchanged() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            plan: Some("plan.md"),
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    let patch = TaskPatch::new();
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.title, "title");
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn create_task_defaults_labels_to_empty() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest::fixture("t", "/r"))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.labels, Vec::<String>::new());
}

#[tokio::test]
async fn patch_task_sets_labels() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest::fixture("t", "/r"))
        .await
        .unwrap();
    let labels = vec!["scala-common".to_string(), "security".to_string()];
    db.patch_task(id, &TaskPatch::new().labels(&labels))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.labels, labels);
}

#[tokio::test]
async fn patch_task_round_trips_hook_event_timestamps() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest::fixture("t", "/r"))
        .await
        .unwrap();

    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.last_pre_tool_use_at.is_none());
    assert!(task.last_notification_at.is_none());

    let pre_tool = chrono::Utc::now();
    let notification = pre_tool - chrono::Duration::seconds(30);
    db.patch_task(
        id,
        &TaskPatch::new()
            .last_pre_tool_use_at(Some(pre_tool))
            .last_notification_at(Some(notification)),
    )
    .await
    .unwrap();

    let task = db.get_task(id).await.unwrap().unwrap();
    let stored_pre = task.last_pre_tool_use_at.expect("pre_tool_use written");
    let stored_notif = task.last_notification_at.expect("notification written");
    assert!(
        (stored_pre - pre_tool).num_seconds().abs() <= 1,
        "stored pre_tool_use {stored_pre} too far from {pre_tool}"
    );
    assert!(
        (stored_notif - notification).num_seconds().abs() <= 1,
        "stored notification {stored_notif} too far from {notification}"
    );
}

#[tokio::test]
async fn patch_task_round_trips_peer_message_timestamps() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest::fixture("t", "/r"))
        .await
        .unwrap();

    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.last_peer_message_sent_at.is_none());
    assert!(task.last_peer_message_received_at.is_none());

    let sent = chrono::Utc::now();
    let received = sent - chrono::Duration::seconds(5);
    db.patch_task(
        id,
        &TaskPatch::new()
            .last_peer_message_sent_at(Some(sent))
            .last_peer_message_received_at(Some(received)),
    )
    .await
    .unwrap();

    let task = db.get_task(id).await.unwrap().unwrap();
    let stored_sent = task
        .last_peer_message_sent_at
        .expect("peer message sent timestamp written");
    let stored_received = task
        .last_peer_message_received_at
        .expect("peer message received timestamp written");
    assert!(
        (stored_sent - sent).num_seconds().abs() <= 1,
        "stored sent {stored_sent} too far from {sent}"
    );
    assert!(
        (stored_received - received).num_seconds().abs() <= 1,
        "stored received {stored_received} too far from {received}"
    );
}

#[tokio::test]
async fn patch_task_none_preserves_labels() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest::fixture("t", "/r"))
        .await
        .unwrap();
    let labels = vec!["keep-me".to_string()];
    db.patch_task(id, &TaskPatch::new().labels(&labels))
        .await
        .unwrap();
    // Patching unrelated field must not touch labels.
    db.patch_task(id, &TaskPatch::new().title("new"))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.labels, labels);
}

#[tokio::test]
async fn patch_task_sets_tag() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(Some(TaskTag::Bug)))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.tag, Some(TaskTag::Bug));
}

#[tokio::test]
async fn patch_task_clears_tag() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(Some(TaskTag::Feature)))
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(None))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.tag.is_none());
}

#[tokio::test]
async fn patch_task_clears_plan() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            plan: Some("plan.md"),
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    let patch = TaskPatch::new().plan_path(None);
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.plan_path.is_none());
}

#[tokio::test]
async fn patch_task_sets_dispatch_fields() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    let window = test_tmux_window("session:1-my-task");
    let patch = TaskPatch::new()
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some(&window));
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-my-task"));
    assert_eq!(
        task.tmux_window.as_ref().map(|w| w.as_str()),
        Some("session:1-my-task")
    );
}

#[tokio::test]
async fn patch_task_clears_dispatch_fields() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    // Set dispatch fields first
    let window = test_tmux_window("session:1-my-task");
    let patch = TaskPatch::new()
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some(&window));
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.worktree.is_some());
    assert!(task.tmux_window.is_some());

    // Clear them
    let patch = TaskPatch::new().worktree(None).tmux_window(None);
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[tokio::test]
async fn patch_task_status_and_dispatch_together() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("title", "/repo")
        })
        .await
        .unwrap();
    let window = test_tmux_window("session:1-my-task");
    let patch = TaskPatch::new()
        .status(TaskStatus::Running)
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some(&window));
    db.patch_task(id, &patch).await.unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-my-task"));
    assert_eq!(
        task.tmux_window.as_ref().map(|w| w.as_str()),
        Some("session:1-my-task")
    );
}

#[tokio::test]
async fn task_patch_status_does_not_set_sub_status() {
    // status() no longer auto-sets sub_status; patch_task handles the default
    let patch = TaskPatch::new().status(TaskStatus::Review);
    assert_eq!(patch.status, Some(TaskStatus::Review));
    assert_eq!(patch.sub_status, None);
}

#[tokio::test]
async fn task_patch_status_and_sub_status_independent() {
    // Order of builder calls doesn't matter — both fields are set independently
    let patch_a = TaskPatch::new()
        .status(TaskStatus::Running)
        .sub_status(SubStatus::NeedsInput);
    let patch_b = TaskPatch::new()
        .sub_status(SubStatus::NeedsInput)
        .status(TaskStatus::Running);
    assert_eq!(patch_a.status, Some(TaskStatus::Running));
    assert_eq!(patch_a.sub_status, Some(SubStatus::NeedsInput));
    assert_eq!(patch_b.status, Some(TaskStatus::Running));
    assert_eq!(patch_b.sub_status, Some(SubStatus::NeedsInput));
}

#[tokio::test]
async fn task_roundtrip_with_pr_fields() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("PR task", "/repo")
        })
        .await
        .unwrap();

    let url = crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    );
    db.patch_task(id, &TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();

    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.url, Some(url));
}

#[tokio::test]
async fn task_pr_fields_default_to_none() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("No PR", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert!(task.url.is_none());
}

#[tokio::test]
async fn patch_sets_and_clears_typed_url_together() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("t", "/r")
        })
        .await
        .unwrap();

    // Set
    let url = TaskUrl::new("https://github.com/o/r/pull/9", UrlType::Pr);
    db.patch_task(id, &TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();
    let t = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(
        t.url,
        Some(TaskUrl::new("https://github.com/o/r/pull/9", UrlType::Pr))
    );

    // Clear (both columns null)
    db.patch_task(id, &TaskPatch::new().url(None))
        .await
        .unwrap();
    let t = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(t.url, None);
}

#[tokio::test]
async fn patch_task_sets_sort_order() {
    let db = Store::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(Some(500)))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sort_order, Some(500));
}

#[tokio::test]
async fn patch_task_clears_sort_order() {
    let db = Store::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(Some(100)))
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(None))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sort_order, None);
}

#[tokio::test]
async fn task_sub_status_persists() {
    let db = Store::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("Test", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Stale);
}

#[tokio::test]
async fn task_sub_status_pr_closed_persists_for_review() {
    let db = Store::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Review,
            ..CreateTaskRequest::fixture("Test", "/repo")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::PrClosed))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::PrClosed);
}

#[tokio::test]
async fn task_sub_status_defaults_to_none() {
    let db = Store::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            ..CreateTaskRequest::fixture("Test", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[tokio::test]
async fn create_task_sets_default_sub_status_for_running() {
    // create_task with status=Running must produce sub_status=active, not 'none'
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
}

#[tokio::test]
async fn create_task_sets_default_sub_status_for_backlog() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[tokio::test]
async fn create_task_with_epic_sort_tag_single_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            epic_id: Some(epic.id),
            sort_order: Some(7),
            tag: Some(TaskTag::Bug),
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));
    assert_eq!(task.sort_order, Some(7));
    assert_eq!(task.tag, Some(TaskTag::Bug));
}

// ---------------------------------------------------------------------------
// Query coverage: delete_task
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Query coverage: batch_delete
// ---------------------------------------------------------------------------

/// tasks.allium: `BatchDelete`'s atomic counterpart to `delete_task`/
/// `delete_epic` looped per item, covering both domains in one call.
#[tokio::test]
async fn batch_delete_removes_a_task_and_an_epic_subtree_together() {
    let db = in_memory_db().await;
    let plain = create_task_returning(&db, "plain", "desc", "/repo", None, TaskStatus::Done)
        .await
        .unwrap();
    let epic = db.create_epic("E", "", None).await.unwrap();
    let in_epic_id = db
        .create_task(CreateTaskRequest {
            description: "desc",
            status: TaskStatus::Done,
            epic_id: Some(epic.id),
            ..CreateTaskRequest::fixture("in epic", "/repo")
        })
        .await
        .unwrap();

    db.batch_delete(&[plain.id], &[epic.id]).await.unwrap();

    assert!(db.get_task(plain.id).await.unwrap().is_none());
    assert!(db.get_task(in_epic_id).await.unwrap().is_none());
    assert!(db.get_epic(epic.id).await.unwrap().is_none());
}

/// A selection holding an epic AND one of its own sub-epics is a legal batch:
/// the client (`handle_batch_delete`) passes every selected epic id, and the
/// selection order is a set's, so either may come first. Whichever order, the
/// nested epic is gone by the time its own turn comes, and that is "already
/// deleted by this batch", not a missing id — the reducer (`batch_delete` in
/// spacetime/module/src/lib.rs) skips it the same way.
#[tokio::test]
async fn batch_delete_accepts_an_epic_and_its_own_sub_epic_in_either_order() {
    for parent_first in [true, false] {
        let db = in_memory_db().await;
        let parent = db.create_epic("P", "", None).await.unwrap();
        let child = db.create_epic("C", "", Some(parent.id)).await.unwrap();
        let ids = if parent_first {
            [parent.id, child.id]
        } else {
            [child.id, parent.id]
        };

        db.batch_delete(&[], &ids).await.unwrap_or_else(|e| {
            panic!("parent_first={parent_first}: nested selection must delete, got {e}")
        });

        assert!(db.get_epic(parent.id).await.unwrap().is_none());
        assert!(db.get_epic(child.id).await.unwrap().is_none());
    }
}

// ---------------------------------------------------------------------------
// Query coverage: task_exists
// ---------------------------------------------------------------------------
