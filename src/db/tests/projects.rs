#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn create_and_list_projects() {
    let db = in_memory_db();
    // Migration v39 seeds one "Default" project; we add two more.
    let p1 = db.create_project("Alpha", 10).await.unwrap();
    let p2 = db.create_project("Beta", 11).await.unwrap();
    let projects = db.list_projects().await.unwrap();
    // 1 seeded + 2 new = 3
    assert_eq!(projects.len(), 3);
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Alpha"));
    assert!(names.contains(&"Beta"));
    assert_eq!(
        projects.iter().find(|p| p.id == p1.id).unwrap().name,
        "Alpha"
    );
    assert_eq!(
        projects.iter().find(|p| p.id == p2.id).unwrap().name,
        "Beta"
    );
}

#[tokio::test]
async fn get_default_project_returns_is_default_row() {
    let db = in_memory_db();
    // Migration v39 seeds one "Default" project with is_default=1 and id=1.
    let seeded = db.get_default_project().await.unwrap();
    assert_eq!(seeded.name, "Default");
    assert!(seeded.is_default);

    // Switch the default to a new project.
    let p2 = db.create_project("My Project", 10).await.unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE projects SET is_default = CASE WHEN id = ?1 THEN 1 ELSE 0 END",
            rusqlite::params![p2.id.0],
        )
        .unwrap();
    let default = db.get_default_project().await.unwrap();
    assert_eq!(default.id, p2.id);
    assert!(default.is_default);
}

#[tokio::test]
async fn rename_project_changes_name() {
    let db = in_memory_db();
    let p = db.create_project("Old Name", 0).await.unwrap();
    db.rename_project(p.id, "New Name").await.unwrap();
    let projects = db.list_projects().await.unwrap();
    assert_eq!(
        projects.iter().find(|proj| proj.id == p.id).unwrap().name,
        "New Name"
    );
}

#[tokio::test]
async fn delete_project_and_move_items_removes_row_and_reassigns() {
    let db = in_memory_db();
    let default = db.get_default_project().await.unwrap();
    let before = db.list_projects().await.unwrap().len();

    let src = db.create_project("Temporary", 99).await.unwrap();
    let task = create_task_returning(&db, "T", "", "/r", None, TaskStatus::Backlog).unwrap();
    db.patch_task(task.id, &TaskPatch::new().project_id(src.id))
        .unwrap();
    let epic = db.create_epic("E", "", "/r", None, src.id).unwrap();

    db.delete_project_and_move_items(src.id, default.id)
        .await
        .unwrap();

    // Project row is gone
    assert_eq!(db.list_projects().await.unwrap().len(), before);
    // Items moved to default
    assert_eq!(
        db.get_task(task.id).unwrap().unwrap().project_id,
        default.id
    );
    assert_eq!(
        db.get_epic(epic.id).unwrap().unwrap().project_id,
        default.id
    );
}

#[tokio::test]
async fn reorder_project_updates_sort_order() {
    let db = in_memory_db();
    let p = db.create_project("P", 5).await.unwrap();
    db.reorder_project(p.id, 10).await.unwrap();
    let projects = db.list_projects().await.unwrap();
    assert_eq!(
        projects
            .iter()
            .find(|proj| proj.id == p.id)
            .unwrap()
            .sort_order,
        10
    );
}

#[tokio::test]
async fn delete_default_project_returns_error() {
    let db = in_memory_db();
    let default = db.get_default_project().await.unwrap();
    let result = db
        .delete_project_and_move_items(default.id, default.id)
        .await;
    assert!(
        result.is_err(),
        "Expected error when deleting default project"
    );
}
