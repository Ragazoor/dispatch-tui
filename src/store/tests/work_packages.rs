//! `work-packages.allium`, through the store over the in-memory reducer
//! caller: what a write leaves in the rows a board reads.

use super::*;
use crate::models::{WorkPackageId, WorkPackageItem, WorkPackageLink};

async fn task(db: &Store, title: &str) -> TaskId {
    db.create_task(CreateTaskRequest::fixture(title, "/repo"))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_created_package_is_listed_with_its_trimmed_title_and_creator() {
    let db = in_memory_db().await;

    let id = db
        .create_work_package(" Window 1 ", "the plan", &[])
        .await
        .unwrap();

    let packages = db.list_work_packages().await;
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].id, id);
    assert_eq!(packages[0].title, "Window 1");
    assert_eq!(packages[0].description, "the plan");
    assert!(packages[0].created_by.is_some());
}

#[tokio::test]
async fn a_blank_title_is_refused_on_create_and_on_patch() {
    let db = in_memory_db().await;
    assert!(db.create_work_package("  ", "", &[]).await.is_err());
    assert!(db.list_work_packages().await.is_empty());

    let id = db.create_work_package("W", "", &[]).await.unwrap();
    assert!(db
        .patch_work_package(id, Some(" "), None, None)
        .await
        .is_err());
    assert_eq!(db.list_work_packages().await[0].title, "W");
}

#[tokio::test]
async fn patching_a_missing_package_is_refused() {
    let db = in_memory_db().await;
    assert!(db
        .patch_work_package(WorkPackageId(99), Some("x"), None, None)
        .await
        .is_err());
}

#[tokio::test]
async fn a_description_patch_leaves_the_title_alone() {
    let db = in_memory_db().await;
    let id = db.create_work_package("W", "", &[]).await.unwrap();

    db.patch_work_package(id, None, Some("new plan"), None)
        .await
        .unwrap();

    let package = &db.list_work_packages().await[0];
    assert_eq!(package.title, "W");
    assert_eq!(package.description, "new plan");
}

#[tokio::test]
async fn linking_is_idempotent_and_unlinking_an_absent_link_is_not_an_error() {
    let db = in_memory_db().await;
    let id = db.create_work_package("W", "", &[]).await.unwrap();
    let t = task(&db, "t").await;
    let e = db.create_epic("E", "", None).await.unwrap().id;

    for _ in 0..2 {
        db.link_work_package_item(id, WorkPackageItem::Task(t))
            .await
            .unwrap();
    }
    db.link_work_package_item(id, WorkPackageItem::Epic(e))
        .await
        .unwrap();
    assert_eq!(
        db.list_work_package_links().await,
        vec![
            WorkPackageLink {
                work_package_id: id,
                item: WorkPackageItem::Task(t),
            },
            WorkPackageLink {
                work_package_id: id,
                item: WorkPackageItem::Epic(e),
            },
        ]
    );

    db.unlink_work_package_item(id, WorkPackageItem::Task(t))
        .await
        .unwrap();
    db.unlink_work_package_item(id, WorkPackageItem::Task(t))
        .await
        .unwrap();
    assert_eq!(db.list_work_package_links().await.len(), 1);
}

#[tokio::test]
async fn a_link_to_an_unknown_item_or_package_is_refused() {
    let db = in_memory_db().await;
    let id = db.create_work_package("W", "", &[]).await.unwrap();
    let t = task(&db, "t").await;

    assert!(db
        .link_work_package_item(id, WorkPackageItem::Task(TaskId(999)))
        .await
        .is_err());
    assert!(db
        .link_work_package_item(id, WorkPackageItem::Epic(crate::models::EpicId(999)))
        .await
        .is_err());
    assert!(db
        .link_work_package_item(WorkPackageId(999), WorkPackageItem::Task(t))
        .await
        .is_err());
    assert!(db.list_work_package_links().await.is_empty());
}

#[tokio::test]
async fn deleting_a_package_removes_its_links_and_keeps_the_items() {
    let db = in_memory_db().await;
    let id = db.create_work_package("W", "", &[]).await.unwrap();
    let t = task(&db, "t").await;
    db.link_work_package_item(id, WorkPackageItem::Task(t))
        .await
        .unwrap();

    db.delete_work_package(id).await.unwrap();

    assert!(db.list_work_packages().await.is_empty());
    assert!(db.list_work_package_links().await.is_empty());
    assert!(db.get_task(t).await.is_some());
}

#[tokio::test]
async fn deleting_an_item_removes_its_links_in_every_package() {
    let db = in_memory_db().await;
    let a = db.create_work_package("A", "", &[]).await.unwrap();
    let b = db.create_work_package("B", "", &[]).await.unwrap();
    let doomed = task(&db, "doomed").await;
    let kept = task(&db, "kept").await;
    for package in [a, b] {
        db.link_work_package_item(package, WorkPackageItem::Task(doomed))
            .await
            .unwrap();
    }
    db.link_work_package_item(a, WorkPackageItem::Task(kept))
        .await
        .unwrap();

    db.patch_task(doomed, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.delete_task(doomed).await.unwrap();

    assert_eq!(
        db.list_work_package_links().await,
        vec![WorkPackageLink {
            work_package_id: a,
            item: WorkPackageItem::Task(kept),
        }],
        "the packages stay, possibly empty; only the deleted item's links go"
    );
    assert_eq!(db.list_work_packages().await.len(), 2);
}

#[tokio::test]
async fn deleting_an_epic_removes_the_links_of_everything_in_its_subtree() {
    let db = in_memory_db().await;
    let id = db.create_work_package("W", "", &[]).await.unwrap();
    let epic = db.create_epic("E", "", None).await.unwrap().id;
    let sub = db.create_epic("S", "", Some(epic)).await.unwrap().id;
    let inner = task(&db, "inner").await;
    db.set_task_epic_id(inner, Some(sub)).await.unwrap();
    db.patch_task(inner, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    for item in [
        WorkPackageItem::Epic(epic),
        WorkPackageItem::Epic(sub),
        WorkPackageItem::Task(inner),
    ] {
        db.link_work_package_item(id, item).await.unwrap();
    }

    db.delete_epic(epic).await.unwrap();

    assert!(db.list_work_package_links().await.is_empty());
}
