use dispatch_tui::db::{CreateTaskRequest, Database, EpicCrud, EpicPatch, LearningStore, TaskCrud, TaskPatch};
use dispatch_tui::models::*;

#[test]
fn full_epic_lifecycle() {
    let db = Database::open_in_memory().unwrap();

    // 1. Create an epic
    let epic = db
        .create_epic(
            "Auth Rewrite",
            "Rewrite auth system",
            "/repo",
            None,
            ProjectId(1),
        )
        .unwrap();

    // 2. Create subtasks linked to epic
    let sub1 = db
        .create_task(CreateTaskRequest {
            title: "Extract middleware",
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
    let sub2 = db
        .create_task(CreateTaskRequest {
            title: "Add JWT validation",
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
    db.set_task_epic_id(sub1, Some(epic.id)).unwrap();
    db.set_task_epic_id(sub2, Some(epic.id)).unwrap();

    // 3. Verify epic status is Backlog (new epics start as Backlog)
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);

    // 4. Move epic status to Running
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);

    // 5. Move all subtasks to Done, advance epic to Review
    db.patch_task(sub1, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();
    db.patch_task(sub2, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);

    // 6. Mark epic as done
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Done))
        .unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);

    // 7. Delete epic cascades
    db.delete_epic(epic.id).unwrap();
    assert!(db.get_epic(epic.id).unwrap().is_none());
    assert!(db.get_task(sub1).unwrap().is_none());
    assert!(db.get_task(sub2).unwrap().is_none());
}

/// Regression: archiving an epic must not violate FK constraints, even when
/// subtasks have rows in `task_usage` and `learnings.source_task_id`.
///
/// Soft-archive transitions epic + subtasks to status='archived' via
/// `patch_epic` / `patch_task` rather than `DELETE FROM tasks`, so the FK
/// columns are not exercised. Before the fix, `Command::DeleteEpic` ran
/// `DELETE FROM tasks WHERE epic_id = ?` which failed with FK violations.
#[test]
fn soft_archive_epic_does_not_violate_foreign_keys() {
    let db = Database::open_in_memory().unwrap();

    let epic = db
        .create_epic("Auth Rewrite", "desc", "/repo", None, ProjectId(1))
        .unwrap();

    let task_id = db
        .create_task(CreateTaskRequest {
            title: "Subtask",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap();
    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();

    // Insert a task_usage row — every dispatched task gets one.
    db.report_usage(
        task_id,
        &UsageReport {
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        },
    )
    .unwrap();

    // Insert a learning that references the task as its source.
    db.create_learning(
        LearningKind::Convention,
        "Test learning",
        None,
        LearningScope::Repo,
        Some("/repo"),
        &[],
        Some(task_id),
    )
    .unwrap();

    // Soft-archive code path: patch the task and the epic to status=Archived.
    // This is what the TUI's handle_archive_epic now produces (one PersistTask
    // per subtask + one PersistEpic for the epic).
    db.patch_task(task_id, &TaskPatch::new().status(TaskStatus::Archived))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Archived))
        .unwrap();

    // Both rows survive and are now archived; FK rows are untouched.
    let archived_task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(archived_task.status, TaskStatus::Archived);
    let archived_epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(archived_epic.status, TaskStatus::Archived);

    // Usage row still references the (still-existing) task.
    let usage_rows = db.get_all_usage().unwrap();
    assert!(usage_rows.iter().any(|u| u.task_id == task_id));
}
