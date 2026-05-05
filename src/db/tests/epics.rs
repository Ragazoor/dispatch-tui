use super::*;

// --- Epic CRUD ---

#[test]
fn create_and_get_epic() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Auth Rewrite", "Rewrite auth", "/repo", None, ProjectId(1))
        .unwrap();
    assert_eq!(epic.title, "Auth Rewrite");
    assert_eq!(epic.description, "Rewrite auth");
    assert_eq!(epic.repo_path, "/repo");
    assert_eq!(epic.status, TaskStatus::Backlog);

    let fetched = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(fetched.id, epic.id);
    assert_eq!(fetched.title, "Auth Rewrite");
}

#[test]
fn list_epics() {
    let db = in_memory_db();
    db.create_epic("Epic A", "desc", "/a", None, ProjectId(1))
        .unwrap();
    db.create_epic("Epic B", "desc", "/b", None, ProjectId(1))
        .unwrap();
    let epics = db.list_epics().unwrap();
    assert_eq!(epics.len(), 2);
}

#[test]
fn get_epic_nonexistent() {
    let db = in_memory_db();
    assert!(db.get_epic(EpicId(999)).unwrap().is_none());
}

#[test]
fn delete_epic_cascades_subtasks() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
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
    })
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
        })
        .unwrap();

    // Link sub 2 to epic
    db.set_task_epic_id(sub_id, Some(epic.id)).unwrap();

    db.delete_epic(epic.id).unwrap();

    // Epic should be gone
    assert!(db.get_epic(epic.id).unwrap().is_none());
    // Sub 2 (linked to epic) should be deleted
    assert!(db.get_task(sub_id).unwrap().is_none());
    // Sub 1 (not linked) should still exist
    assert_eq!(db.list_all().unwrap().len(), 1);
}

#[test]
fn delete_epic_with_sub_epics_succeeds() {
    let db = in_memory_db();
    let parent = db
        .create_epic("Parent", "", "/repo", None, ProjectId(1))
        .unwrap();
    let child = db
        .create_epic("Child", "", "/repo", Some(parent.id), ProjectId(1))
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
        })
        .unwrap();
    db.set_task_epic_id(task_id, Some(child.id)).unwrap();

    db.delete_epic(parent.id)
        .expect("delete_epic with sub-epics should succeed");

    assert!(db.get_epic(parent.id).unwrap().is_none());
    assert!(db.get_epic(child.id).unwrap().is_none());
    assert!(db.get_task(task_id).unwrap().is_none());
}

#[test]
fn delete_epic_multi_level_sub_epics() {
    let db = in_memory_db();
    let root = db
        .create_epic("Root", "", "/repo", None, ProjectId(1))
        .unwrap();
    let child = db
        .create_epic("Child", "", "/repo", Some(root.id), ProjectId(1))
        .unwrap();
    let grandchild = db
        .create_epic("Grandchild", "", "/repo", Some(child.id), ProjectId(1))
        .unwrap();

    db.delete_epic(root.id).expect("deep delete should succeed");

    assert!(db.get_epic(root.id).unwrap().is_none());
    assert!(db.get_epic(child.id).unwrap().is_none());
    assert!(db.get_epic(grandchild.id).unwrap().is_none());
    assert_eq!(db.list_epics().unwrap().len(), 0);
}

#[test]
fn epic_has_status_field() {
    let db = Database::open_in_memory().unwrap();
    let epic = db
        .create_epic("Test", "Desc", "/repo", None, ProjectId(1))
        .unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[test]
fn patch_epic_status() {
    let db = Database::open_in_memory().unwrap();
    let epic = db
        .create_epic("Test", "Desc", "/repo", None, ProjectId(1))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn patch_epic_title() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Old Title", "desc", "/repo", None, ProjectId(1))
        .unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().title("New Title"))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.title, "New Title");
    assert_eq!(updated.description, "desc"); // unchanged
}

#[test]
fn task_epic_id_roundtrip() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
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
        })
        .unwrap();

    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));

    db.set_task_epic_id(task_id, None).unwrap();
    let task = db.get_task(task_id).unwrap().unwrap();
    assert!(task.epic_id.is_none());
}

#[test]
fn list_tasks_for_epic() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
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
        })
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
        })
        .unwrap();

    db.set_task_epic_id(id1, Some(epic.id)).unwrap();

    let subtasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].title, "Sub A");
}

#[test]
fn patch_epic_plan() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    assert!(epic.plan_path.is_none());

    db.patch_epic(epic.id, &EpicPatch::new().plan_path(Some("docs/plan.md")))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.plan_path.as_deref(), Some("docs/plan.md"));
}

#[test]
fn patch_epic_clear_plan() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().plan_path(Some("docs/plan.md")))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().plan_path(None))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert!(updated.plan_path.is_none());
}

#[test]
fn patch_epic_repo_path() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Epic", "desc", "/old", None, ProjectId(1))
        .unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().repo_path("/new"))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.repo_path, "/new");
    assert_eq!(updated.title, "Epic"); // unchanged
}

#[test]
fn recalculate_epic_status_advances_to_running() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_moves_backward_from_review_to_running() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_moves_backward_from_review_to_backlog() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[test]
fn recalculate_epic_status_moves_backward_when_review_subtask_completes() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();

    // Manually set epic to Review (simulating a subtask that was in review and then moved to done)
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    // Should drop back to Running since no subtask is in review but one is running
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_all_done() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);
}

#[test]
fn recalculate_epic_status_all_review_or_done() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Review))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[test]
fn recalculate_epic_status_review_beats_running() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t3 = create_task_returning(&db, "T3", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t3.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Review))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Review))
        .unwrap();
    db.patch_task(t3.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[test]
fn cli_update_conditional_sets_epic_to_review() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.recalculate_epic_status(epic.id).unwrap();

    // Simulate hook: dispatch update <id> review --only-if running
    let svc = TaskService::new(db.clone());
    let updated = svc
        .cli_update_task(task.id, TaskStatus::Review, Some(TaskStatus::Running), None)
        .unwrap();
    assert!(updated);

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[test]
fn cli_update_unconditional_sets_epic_to_running() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();

    // Simulate: dispatch update <id> running (no --only-if)
    let svc = TaskService::new(db.clone());
    let updated = svc
        .cli_update_task(task.id, TaskStatus::Running, None, None)
        .unwrap();
    assert!(updated);

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn cli_update_epic_drops_back_when_review_task_done() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Review).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.recalculate_epic_status(epic.id).unwrap();
    assert_eq!(
        db.get_epic(epic.id).unwrap().unwrap().status,
        TaskStatus::Review
    );

    // t2 moves to done — epic should drop to Running (t1 still running)
    let svc = TaskService::new(db.clone());
    svc.cli_update_task(t2.id, TaskStatus::Done, Some(TaskStatus::Review), None)
        .unwrap();

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn cli_update_with_substatus_keeps_running_and_recalculates_epic() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.recalculate_epic_status(epic.id).unwrap();

    // Hook sets needs_input while staying running:
    // dispatch update <id> running --only-if running --sub-status needs_input
    let svc = TaskService::new(db.clone());
    svc.cli_update_task(
        task.id,
        TaskStatus::Running,
        Some(TaskStatus::Running),
        Some(SubStatus::NeedsInput),
    )
    .unwrap();

    // Epic should still be Running
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);

    // Task sub_status should be NeedsInput
    let task = db.get_task(task.id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
}

// ---------------------------------------------------------------------------
// Query coverage: patch_epic edge cases
// ---------------------------------------------------------------------------

#[test]
fn patch_epic_nonexistent_errors() {
    let db = in_memory_db();
    let result = db.patch_epic(EpicId(9999), &EpicPatch::new().title("x"));
    assert!(result.is_err());
}

#[test]
fn patch_epic_no_changes_is_noop() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Title", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    // Empty patch — has_changes() is false, so this should succeed without touching DB
    db.patch_epic(epic.id, &EpicPatch::new()).unwrap();
    let fetched = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(fetched.title, "Title");
}

#[test]
fn patch_epic_sort_order() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    assert!(epic.sort_order.is_none());

    db.patch_epic(epic.id, &EpicPatch::new().sort_order(Some(42)))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.sort_order, Some(42));

    // Clear sort_order
    db.patch_epic(epic.id, &EpicPatch::new().sort_order(None))
        .unwrap();
    let cleared = db.get_epic(epic.id).unwrap().unwrap();
    assert!(cleared.sort_order.is_none());
}

#[test]
fn delete_epic_nonexistent_errors() {
    let db = in_memory_db();
    let result = db.delete_epic(EpicId(9999));
    assert!(result.is_err());
}

#[test]
fn recalculate_epic_status_ignores_archived_subtasks() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();

    // t1 done, t2 archived — only non-archived counted, so all done → Done
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Archived))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);
}

#[test]
fn recalculate_epic_status_no_subtasks_stays_backlog() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[test]
fn recalculate_epic_status_nonexistent_is_noop() {
    let db = in_memory_db();
    // Should not error for nonexistent epic
    db.recalculate_epic_status(EpicId(9999)).unwrap();
}

#[test]
fn patch_epic_auto_dispatch_persists() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    assert!(epic.auto_dispatch); // default true

    db.patch_epic(epic.id, &EpicPatch::new().auto_dispatch(false))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert!(!updated.auto_dispatch);

    db.patch_epic(epic.id, &EpicPatch::new().auto_dispatch(true))
        .unwrap();
    let re_enabled = db.get_epic(epic.id).unwrap().unwrap();
    assert!(re_enabled.auto_dispatch);
}

#[test]
fn list_all_tasks_with_epic_id_returns_only_tasks_with_epic() {
    let db = in_memory_db();
    let epic1_id = db
        .create_epic("E1", "", "/repo", None, ProjectId(1))
        .unwrap()
        .id;
    let epic2_id = db
        .create_epic("E2", "", "/repo", None, ProjectId(1))
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
        })
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
        })
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
        })
        .unwrap();

    db.set_task_epic_id(t1, Some(epic1_id)).unwrap();
    db.set_task_epic_id(t2, Some(epic2_id)).unwrap();

    let tasks = db.list_all_tasks_with_epic_id().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t.id == t1));
    assert!(tasks.iter().any(|t| t.id == t2));
}

// ---------------------------------------------------------------------------
// Epic-in-epic (nested epics)
// ---------------------------------------------------------------------------

#[test]
fn sub_epic_has_parent_id() {
    let db = in_memory_db();
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    let child = db
        .create_epic("Child", "desc", "/repo", Some(parent.id), ProjectId(1))
        .unwrap();
    assert_eq!(child.parent_epic_id, Some(parent.id));

    let fetched = db.get_epic(child.id).unwrap().unwrap();
    assert_eq!(fetched.parent_epic_id, Some(parent.id));
}

#[test]
fn root_epic_has_no_parent() {
    let db = in_memory_db();
    let root = db
        .create_epic("Root", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    assert_eq!(root.parent_epic_id, None);
}

#[test]
fn list_root_epics_excludes_sub_epics() {
    let db = in_memory_db();
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    db.create_epic("Child", "desc", "/repo", Some(parent.id), ProjectId(1))
        .unwrap();

    let roots = db.list_root_epics().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, parent.id);
}

#[test]
fn list_sub_epics_returns_children() {
    let db = in_memory_db();
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    let child1 = db
        .create_epic("Child 1", "desc", "/repo", Some(parent.id), ProjectId(1))
        .unwrap();
    let child2 = db
        .create_epic("Child 2", "desc", "/repo", Some(parent.id), ProjectId(1))
        .unwrap();
    // unrelated root epic — must not appear
    db.create_epic("Other", "desc", "/repo", None, ProjectId(1))
        .unwrap();

    let children = db.list_sub_epics(parent.id).unwrap();
    assert_eq!(children.len(), 2);
    let ids: Vec<_> = children.iter().map(|e| e.id).collect();
    assert!(ids.contains(&child1.id));
    assert!(ids.contains(&child2.id));
}

#[test]
fn recalculate_parent_status_from_sub_epic() {
    let db = in_memory_db();
    let parent = db
        .create_epic("Parent", "desc", "/repo", None, ProjectId(1))
        .unwrap();
    let child = db
        .create_epic("Child", "desc", "/repo", Some(parent.id), ProjectId(1))
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
        })
        .unwrap();
    db.set_task_epic_id(task_id, Some(child.id)).unwrap();
    db.patch_task(
        task_id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(crate::models::SubStatus::Active),
    )
    .unwrap();

    // Recalculating the sub-epic should also propagate up to the parent
    db.recalculate_epic_status(child.id).unwrap();

    let updated_child = db.get_epic(child.id).unwrap().unwrap();
    assert_eq!(updated_child.status, TaskStatus::Running);

    let updated_parent = db.get_epic(parent.id).unwrap().unwrap();
    assert_eq!(updated_parent.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_terminates_on_cycle() {
    // Manually create a cycle at the DB level by bypassing FK checks,
    // then verify recalculate_epic_status returns Ok(()) rather than hanging.
    let db = in_memory_db();
    {
        let conn = db.conn().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    }
    let a = db
        .create_epic("A", "", "/repo", None, ProjectId(1))
        .unwrap();
    let b = db
        .create_epic("B", "", "/repo", Some(a.id), ProjectId(1))
        .unwrap();
    // Point a's parent back to b → a→b→a cycle
    {
        let conn = db.conn().unwrap();
        conn.execute(
            "UPDATE epics SET parent_epic_id = ?1 WHERE id = ?2",
            rusqlite::params![b.id.0, a.id.0],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    }
    // Must return without stack overflow
    let result = db.recalculate_epic_status(a.id);
    assert!(result.is_ok());
}

#[test]
fn self_referential_epic_is_rejected() {
    let db = in_memory_db();
    let epic = db
        .create_epic("E", "", "/repo", None, ProjectId(1))
        .unwrap();
    let conn = db.conn().unwrap();
    let result = conn.execute(
        "UPDATE epics SET parent_epic_id = id WHERE id = ?1",
        rusqlite::params![epic.id.0],
    );
    assert!(
        result.is_err(),
        "self-link should be rejected by CHECK constraint"
    );
}

#[test]
fn epic_patch_default_has_no_changes() {
    assert!(!EpicPatch::default().has_changes());
}

#[test]
fn epic_patch_each_setter_marks_has_changes() {
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
