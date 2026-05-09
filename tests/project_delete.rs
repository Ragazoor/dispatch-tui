#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: project delete cascades tasks and epics to Default,
//! and the Default project itself cannot be deleted.

use dispatch_tui::db::{CreateTaskRequest, Database, EpicCrud, ProjectCrud, TaskCrud};
use dispatch_tui::models::TaskStatus;

#[tokio::test]
async fn delete_project_moves_tasks_and_epics_to_default() {
    let db = Database::open_in_memory().unwrap();
    let default = db.get_default_project().await.unwrap();

    // Create a non-default project P with 3 tasks and 1 epic.
    let p = db.create_project("P", 100).await.unwrap();
    assert_ne!(p.id, default.id);
    assert!(!p.is_default);

    let task_ids: Vec<_> = (0..3)
        .map(|i| {
            db.create_task(CreateTaskRequest {
                title: &format!("T{i}"),
                description: "",
                repo_path: "/repo",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: None,
                sort_order: None,
                tag: None,
                project_id: p.id,
            })
            .unwrap()
        })
        .collect();
    let epic = db.create_epic("Epic", "", "/repo", None, p.id).unwrap();

    // Sanity: items belong to P.
    for id in &task_ids {
        assert_eq!(db.get_task(*id).unwrap().unwrap().project_id, p.id);
    }
    assert_eq!(db.get_epic(epic.id).unwrap().unwrap().project_id, p.id);

    // Delete P → all items move to Default in a single transaction.
    db.delete_project_and_move_items(p.id, default.id)
        .await
        .unwrap();

    for id in &task_ids {
        assert_eq!(
            db.get_task(*id).unwrap().unwrap().project_id,
            default.id,
            "task {id:?} should be reassigned to default"
        );
    }
    assert_eq!(
        db.get_epic(epic.id).unwrap().unwrap().project_id,
        default.id,
        "epic should be reassigned to default"
    );

    // Project row is gone.
    let projects = db.list_projects().await.unwrap();
    assert!(
        projects.iter().all(|proj| proj.id != p.id),
        "deleted project must not appear in list_projects"
    );
}

#[tokio::test]
async fn delete_default_project_is_rejected() {
    let db = Database::open_in_memory().unwrap();
    let default = db.get_default_project().await.unwrap();

    let result = db
        .delete_project_and_move_items(default.id, default.id)
        .await;
    assert!(result.is_err(), "deleting the default project must error");

    // Default still present.
    let projects = db.list_projects().await.unwrap();
    assert!(projects.iter().any(|p| p.id == default.id && p.is_default));
}
