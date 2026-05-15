#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

// --- Epic CRUD ---

#[tokio::test]
async fn create_and_get_epic() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Auth Rewrite", "Rewrite auth", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert_eq!(epic.title, "Auth Rewrite");
    assert_eq!(epic.description, "Rewrite auth");
    assert_eq!(epic.repo_path, "/repo");
    assert_eq!(epic.status, TaskStatus::Backlog);

    let fetched = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, epic.id);
    assert_eq!(fetched.title, "Auth Rewrite");
}

#[tokio::test]
async fn list_epics() {
    let db = in_memory_db().await;
    db.create_epic("Epic A", "desc", "/a", None, ProjectId(1))
        .await
        .unwrap();
    db.create_epic("Epic B", "desc", "/b", None, ProjectId(1))
        .await
        .unwrap();
    let epics = db.list_epics().await.unwrap();
    assert_eq!(epics.len(), 2);
}

#[tokio::test]
async fn get_epic_nonexistent() {
    let db = in_memory_db().await;
    assert!(db.get_epic(EpicId(999)).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_epic_cascades_subtasks() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    db.create_task(CreateTaskRequest {
        title: "Sub 1",
        description: "desc",
        repo_path: "/repo",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        project_id: ProjectId(1),
        wrap_up_mode: None,
    })
    .await
    .unwrap();
    let sub_id = db
        .create_task(CreateTaskRequest {
            title: "Sub 2",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    // Link sub 2 to epic
    db.set_task_epic_id(sub_id, Some(epic.id)).await.unwrap();

    db.delete_epic(epic.id).await.unwrap();

    // Epic should be gone
    assert!(db.get_epic(epic.id).await.unwrap().is_none());
    // Sub 2 (linked to epic) should be deleted
    assert!(db.get_task(sub_id).await.unwrap().is_none());
    // Sub 1 (not linked) should still exist
    assert_eq!(db.list_all().await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_epic_with_sub_epics_succeeds() {
    let db = in_memory_db().await;
    let parent = db
        .create_epic("Parent", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let child = db
        .create_epic("Child", "", "/repo", Some(parent.id), ProjectId(1))
        .await
        .unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.set_task_epic_id(task_id, Some(child.id)).await.unwrap();

    db.delete_epic(parent.id)
        .await
        .expect("delete_epic with sub-epics should succeed");

    assert!(db.get_epic(parent.id).await.unwrap().is_none());
    assert!(db.get_epic(child.id).await.unwrap().is_none());
    assert!(db.get_task(task_id).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_epic_multi_level_sub_epics() {
    let db = in_memory_db().await;
    let root = db
        .create_epic("Root", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let child = db
        .create_epic("Child", "", "/repo", Some(root.id), ProjectId(1))
        .await
        .unwrap();
    let grandchild = db
        .create_epic("Grandchild", "", "/repo", Some(child.id), ProjectId(1))
        .await
        .unwrap();

    db.delete_epic(root.id)
        .await
        .expect("deep delete should succeed");

    assert!(db.get_epic(root.id).await.unwrap().is_none());
    assert!(db.get_epic(child.id).await.unwrap().is_none());
    assert!(db.get_epic(grandchild.id).await.unwrap().is_none());
    assert_eq!(db.list_epics().await.unwrap().len(), 0);
}

#[tokio::test]
async fn epic_has_status_field() {
    let db = Database::open_in_memory().await.unwrap();
    let epic = db
        .create_epic("Test", "Desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn patch_epic_status() {
    let db = Database::open_in_memory().await.unwrap();
    let epic = db
        .create_epic("Test", "Desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[tokio::test]
async fn patch_epic_title() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Old Title", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().title("New Title"))
        .await
        .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.title, "New Title");
    assert_eq!(updated.description, "desc"); // unchanged
}

#[tokio::test]
async fn task_epic_id_roundtrip() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    db.set_task_epic_id(task_id, Some(epic.id)).await.unwrap();
    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));

    db.set_task_epic_id(task_id, None).await.unwrap();
    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert!(task.epic_id.is_none());
}

#[tokio::test]
async fn list_tasks_for_epic() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let id1 = db
        .create_task(CreateTaskRequest {
            title: "Sub A",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    let _id2 = db
        .create_task(CreateTaskRequest {
            title: "Standalone",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    db.set_task_epic_id(id1, Some(epic.id)).await.unwrap();

    let subtasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].title, "Sub A");
}

#[tokio::test]
async fn patch_epic_plan() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert!(epic.plan_path.is_none());

    db.patch_epic(epic.id, &EpicPatch::new().plan_path(Some("docs/plan.md")))
        .await
        .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.plan_path.as_deref(), Some("docs/plan.md"));
}

#[tokio::test]
async fn patch_epic_clear_plan() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().plan_path(Some("docs/plan.md")))
        .await
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().plan_path(None))
        .await
        .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert!(updated.plan_path.is_none());
}

#[tokio::test]
async fn patch_epic_repo_path() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Epic", "desc", "/old", None, ProjectId(1))
        .await
        .unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().repo_path("/new"))
        .await
        .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.repo_path, "/new");
    assert_eq!(updated.title, "Epic"); // unchanged
}

#[tokio::test]
async fn recalculate_epic_status_advances_to_running() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).await.unwrap();
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[tokio::test]
async fn recalculate_epic_status_moves_backward_from_review_to_running() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).await.unwrap();
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[tokio::test]
async fn recalculate_epic_status_moves_backward_from_review_to_backlog() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).await.unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn recalculate_epic_status_moves_backward_when_review_subtask_completes() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).await.unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).await.unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    // Manually set epic to Review (simulating a subtask that was in review and then moved to done)
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    // Should drop back to Running since no subtask is in review but one is running
    assert_eq!(epic.status, TaskStatus::Running);
}

#[tokio::test]
async fn recalculate_epic_status_all_done() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).await.unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).await.unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);
}

#[tokio::test]
async fn recalculate_epic_status_all_review_or_done() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).await.unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).await.unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[tokio::test]
async fn recalculate_epic_status_review_beats_running() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let t3 = create_task_returning(&db, "T3", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).await.unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).await.unwrap();
    db.set_task_epic_id(t3.id, Some(epic.id)).await.unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();
    db.patch_task(t3.id, &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[tokio::test]
async fn cli_update_conditional_sets_epic_to_review() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running)
        .await
        .unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).await.unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();

    // Simulate hook: dispatch update <id> review --only-if running
    let svc = TaskService::new(db.clone());
    let updated = svc
        .cli_update_task(task.id, TaskStatus::Review, Some(TaskStatus::Running), None)
        .await
        .unwrap();
    assert!(updated);

    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[tokio::test]
async fn cli_update_unconditional_sets_epic_to_running() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).await.unwrap();

    // Simulate: dispatch update <id> running (no --only-if)
    let svc = TaskService::new(db.clone());
    let updated = svc
        .cli_update_task(task.id, TaskStatus::Running, None, None)
        .await
        .unwrap();
    assert!(updated);

    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[tokio::test]
async fn cli_update_epic_drops_back_when_review_task_done() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running)
        .await
        .unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Review)
        .await
        .unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).await.unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).await.unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();
    assert_eq!(
        db.get_epic(epic.id).await.unwrap().unwrap().status,
        TaskStatus::Review
    );

    // t2 moves to done — epic should drop to Running (t1 still running)
    let svc = TaskService::new(db.clone());
    svc.cli_update_task(t2.id, TaskStatus::Done, Some(TaskStatus::Review), None)
        .await
        .unwrap();

    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[tokio::test]
async fn cli_update_with_substatus_keeps_running_and_recalculates_epic() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running)
        .await
        .unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).await.unwrap();
    db.recalculate_epic_status(epic.id).await.unwrap();

    // Hook sets needs_input while staying running:
    // dispatch update <id> running --only-if running --sub-status needs_input
    let svc = TaskService::new(db.clone());
    svc.cli_update_task(
        task.id,
        TaskStatus::Running,
        Some(TaskStatus::Running),
        Some(SubStatus::NeedsInput),
    )
    .await
    .unwrap();

    // Epic should still be Running
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);

    // Task sub_status should be NeedsInput
    let task = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
}

// ---------------------------------------------------------------------------
// Query coverage: patch_epic edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_epic_nonexistent_errors() {
    let db = in_memory_db().await;
    let result = db
        .patch_epic(EpicId(9999), &EpicPatch::new().title("x"))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn patch_epic_no_changes_is_noop() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("Title", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    // Empty patch — has_changes() is false, so this should succeed without touching DB
    db.patch_epic(epic.id, &EpicPatch::new()).await.unwrap();
    let fetched = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(fetched.title, "Title");
}

#[tokio::test]
async fn patch_epic_sort_order() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert!(epic.sort_order.is_none());

    db.patch_epic(epic.id, &EpicPatch::new().sort_order(Some(42)))
        .await
        .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.sort_order, Some(42));

    // Clear sort_order
    db.patch_epic(epic.id, &EpicPatch::new().sort_order(None))
        .await
        .unwrap();
    let cleared = db.get_epic(epic.id).await.unwrap().unwrap();
    assert!(cleared.sort_order.is_none());
}

#[tokio::test]
async fn delete_epic_nonexistent_errors() {
    let db = in_memory_db().await;
    let result = db.delete_epic(EpicId(9999)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn recalculate_epic_status_ignores_archived_subtasks() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).await.unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).await.unwrap();

    // t1 done, t2 archived — only non-archived counted, so all done → Done
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Archived))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);
}

#[tokio::test]
async fn recalculate_epic_status_no_subtasks_stays_backlog() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    db.recalculate_epic_status(epic.id).await.unwrap();
    let epic = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn recalculate_epic_status_nonexistent_is_noop() {
    let db = in_memory_db().await;
    // Should not error for nonexistent epic
    db.recalculate_epic_status(EpicId(9999)).await.unwrap();
}

#[tokio::test]
async fn patch_epic_auto_dispatch_persists() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert!(epic.auto_dispatch); // default true

    db.patch_epic(epic.id, &EpicPatch::new().auto_dispatch(false))
        .await
        .unwrap();
    let updated = db.get_epic(epic.id).await.unwrap().unwrap();
    assert!(!updated.auto_dispatch);

    db.patch_epic(epic.id, &EpicPatch::new().auto_dispatch(true))
        .await
        .unwrap();
    let re_enabled = db.get_epic(epic.id).await.unwrap().unwrap();
    assert!(re_enabled.auto_dispatch);
}

#[tokio::test]
async fn list_all_tasks_with_epic_id_returns_only_tasks_with_epic() {
    let db = in_memory_db().await;
    let epic1_id = db
        .create_epic("E1", "", "/repo", None, ProjectId(1))
        .await
        .unwrap()
        .id;
    let epic2_id = db
        .create_epic("E2", "", "/repo", None, ProjectId(1))
        .await
        .unwrap()
        .id;

    let t1 = db
        .create_task(CreateTaskRequest {
            title: "Task1",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    let t2 = db
        .create_task(CreateTaskRequest {
            title: "Task2",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    let _t3 = db
        .create_task(CreateTaskRequest {
            title: "Orphan",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    db.set_task_epic_id(t1, Some(epic1_id)).await.unwrap();
    db.set_task_epic_id(t2, Some(epic2_id)).await.unwrap();

    let tasks = db.list_all_tasks_with_epic_id().await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t.id == t1));
    assert!(tasks.iter().any(|t| t.id == t2));
}

// ---------------------------------------------------------------------------
// Epic-in-epic (nested epics)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sub_epic_has_parent_id() {
    let db = in_memory_db().await;
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let child = db
        .create_epic("Child", "desc", "/repo", Some(parent.id), ProjectId(1))
        .await
        .unwrap();
    assert_eq!(child.parent_epic_id, Some(parent.id));

    let fetched = db.get_epic(child.id).await.unwrap().unwrap();
    assert_eq!(fetched.parent_epic_id, Some(parent.id));
}

#[tokio::test]
async fn root_epic_has_no_parent() {
    let db = in_memory_db().await;
    let root = db
        .create_epic("Root", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert_eq!(root.parent_epic_id, None);
}

#[tokio::test]
async fn list_root_epics_excludes_sub_epics() {
    let db = in_memory_db().await;
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    db.create_epic("Child", "desc", "/repo", Some(parent.id), ProjectId(1))
        .await
        .unwrap();

    let roots = db.list_root_epics().await.unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, parent.id);
}

#[tokio::test]
async fn list_sub_epics_returns_children() {
    let db = in_memory_db().await;
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let child1 = db
        .create_epic("Child 1", "desc", "/repo", Some(parent.id), ProjectId(1))
        .await
        .unwrap();
    let child2 = db
        .create_epic("Child 2", "desc", "/repo", Some(parent.id), ProjectId(1))
        .await
        .unwrap();
    // unrelated root epic — must not appear
    db.create_epic("Other", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let children = db.list_sub_epics(parent.id).await.unwrap();
    assert_eq!(children.len(), 2);
    let ids: Vec<_> = children.iter().map(|e| e.id).collect();
    assert!(ids.contains(&child1.id));
    assert!(ids.contains(&child2.id));
}

#[tokio::test]
async fn recalculate_parent_status_from_sub_epic() {
    let db = in_memory_db().await;
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let child = db
        .create_epic("Child", "desc", "/repo", Some(parent.id), ProjectId(1))
        .await
        .unwrap();

    // Add a task to the sub-epic and move it to running
    let task_id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    db.set_task_epic_id(task_id, Some(child.id)).await.unwrap();
    db.patch_task(
        task_id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(crate::models::SubStatus::Active),
    )
    .await
    .unwrap();

    // Recalculating the sub-epic should also propagate up to the parent
    db.recalculate_epic_status(child.id).await.unwrap();

    let updated_child = db.get_epic(child.id).await.unwrap().unwrap();
    assert_eq!(updated_child.status, TaskStatus::Running);

    let updated_parent = db.get_epic(parent.id).await.unwrap().unwrap();
    assert_eq!(updated_parent.status, TaskStatus::Running);
}

#[tokio::test]
async fn recalculate_epic_status_terminates_on_cycle() {
    // Manually create a cycle at the DB level by bypassing FK checks,
    // then verify recalculate_epic_status returns Ok(()) rather than hanging.
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();
    let a = db
        .create_epic("A", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let b = db
        .create_epic("B", "", "/repo", Some(a.id), ProjectId(1))
        .await
        .unwrap();
    // Point a's parent back to b → a→b→a cycle
    let (a_id, b_id) = (a.id.0, b.id.0);
    db.db_call(move |conn| {
        conn.execute(
            "UPDATE epics SET parent_epic_id = ?1 WHERE id = ?2",
            rusqlite::params![b_id, a_id],
        )?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    })
    .await
    .unwrap();
    // Must return without stack overflow
    let result = db.recalculate_epic_status(a.id).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn self_referential_epic_is_rejected() {
    let db = in_memory_db().await;
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    let eid = epic.id.0;
    let rejected = db
        .db_call(move |conn| {
            let result = conn.execute(
                "UPDATE epics SET parent_epic_id = id WHERE id = ?1",
                rusqlite::params![eid],
            );
            Ok(result.is_err())
        })
        .await
        .unwrap();
    assert!(rejected, "self-link should be rejected by CHECK constraint");
}

#[tokio::test]
async fn epic_patch_default_has_no_changes() {
    assert!(!EpicPatch::default().has_changes());
}

#[tokio::test]
async fn epic_patch_each_setter_marks_has_changes() {
    assert!(EpicPatch::new().title("t").has_changes());
    assert!(EpicPatch::new().description("d").has_changes());
    assert!(EpicPatch::new().status(TaskStatus::Running).has_changes());
    assert!(EpicPatch::new().plan_path(Some("p")).has_changes());
    assert!(EpicPatch::new().plan_path(None).has_changes());
    assert!(EpicPatch::new().sort_order(Some(1)).has_changes());
    assert!(EpicPatch::new().sort_order(None).has_changes());
    assert!(EpicPatch::new().repo_path("/r").has_changes());
    assert!(EpicPatch::new().auto_dispatch(true).has_changes());
    assert!(EpicPatch::new().feed_command(Some("cmd")).has_changes());
    assert!(EpicPatch::new().feed_command(None).has_changes());
    assert!(EpicPatch::new().feed_interval_secs(Some(60)).has_changes());
    assert!(EpicPatch::new().feed_interval_secs(None).has_changes());
    assert!(EpicPatch::new().project_id(ProjectId(1)).has_changes());
}
