//! `work-packages.allium`'s lifecycle rules, through the service over the
//! in-memory store.

use super::*;
use crate::models::{EpicId, TaskId, WorkPackageItem as Item};
use crate::store::{CreateTaskRequest, EpicCrud, Store, TaskCrud, TaskRead, WorkPackageRead};

struct Fixture {
    db: Arc<Store>,
    svc: WorkPackageService,
}

fn fixture() -> Fixture {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let svc = WorkPackageService::new(db.clone());
    Fixture { db, svc }
}

async fn task(db: &Store, title: &str) -> TaskId {
    db.create_task(CreateTaskRequest::fixture(title, "/repo"))
        .await
        .unwrap()
}

async fn epic(db: &Store, title: &str) -> EpicId {
    db.create_epic(title, "", None).await.unwrap().id
}

async fn links_of(db: &Store, id: WorkPackageId) -> Vec<Item> {
    db.list_work_package_links()
        .await
        .into_iter()
        .filter(|l| l.work_package_id == id)
        .map(|l| l.item)
        .collect()
}

fn create(title: &str, link: Vec<Item>) -> CreateWorkPackageParams {
    CreateWorkPackageParams {
        title: title.into(),
        description: String::new(),
        link,
    }
}

// --- CreateWorkPackage ---------------------------------------------------

#[tokio::test]
async fn create_makes_the_package_and_links_the_named_items() {
    let f = fixture();
    let t = task(&f.db, "migrate db-a").await;
    let e = epic(&f.db, "db-b").await;

    let id = f
        .svc
        .create_work_package(CreateWorkPackageParams {
            title: " Window 1 ".into(),
            description: "first maintenance window".into(),
            link: vec![Item::Task(t), Item::Epic(e)],
        })
        .await
        .unwrap();

    let views = f.svc.list_work_packages(Some(id)).await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].package.title, "Window 1");
    assert_eq!(views[0].package.description, "first maintenance window");
    let items: Vec<Item> = views[0].items.iter().map(|i| i.item).collect();
    assert!(items.contains(&Item::Task(t)) && items.contains(&Item::Epic(e)));
}

#[tokio::test]
async fn create_refuses_a_blank_title() {
    let f = fixture();
    let err = f
        .svc
        .create_work_package(create("   ", vec![]))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)), "{err}");
    assert!(f.db.list_work_packages().await.is_empty());
}

#[tokio::test]
async fn create_with_an_unknown_item_writes_nothing() {
    let f = fixture();
    let t = task(&f.db, "real").await;
    let err = f
        .svc
        .create_work_package(create("W", vec![Item::Task(t), Item::Epic(EpicId(404))]))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "{err}");
    assert!(err.to_string().contains("Epic 404"), "{err}");
    assert!(f.db.list_work_packages().await.is_empty());
    assert!(f.db.list_work_package_links().await.is_empty());
}

// --- list ----------------------------------------------------------------

#[tokio::test]
async fn list_resolves_each_item_to_its_title_and_status() {
    let f = fixture();
    let t = task(&f.db, "the task").await;
    let id = f
        .svc
        .create_work_package(create("W", vec![Item::Task(t)]))
        .await
        .unwrap();

    let views = f.svc.list_work_packages(None).await.unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].package.id, id);
    assert_eq!(
        views[0].items,
        vec![LinkedItem {
            item: Item::Task(t),
            title: "the task".into(),
            status: TaskStatus::Backlog,
        }]
    );
}

#[tokio::test]
async fn list_of_an_unknown_package_is_not_found() {
    let f = fixture();
    let err = f
        .svc
        .list_work_packages(Some(WorkPackageId(9)))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "{err}");
}

// --- UpdateWorkPackage / LinkItem / UnlinkItem ---------------------------

#[tokio::test]
async fn update_renames_and_rewrites_the_description() {
    let f = fixture();
    let id = f
        .svc
        .create_work_package(create("W", vec![]))
        .await
        .unwrap();

    f.svc
        .update_work_package(UpdateWorkPackageParams {
            title: Some(" Window 2 ".into()),
            description: Some("moved".into()),
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap();

    let p = &f.db.list_work_packages().await[0];
    assert_eq!(
        (p.title.as_str(), p.description.as_str()),
        ("Window 2", "moved")
    );
}

#[tokio::test]
async fn update_sets_sort_order_alone() {
    let f = fixture();
    let id = f
        .svc
        .create_work_package(create("W", vec![]))
        .await
        .unwrap();

    f.svc
        .update_work_package(UpdateWorkPackageParams {
            sort_order: Some(7),
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap();

    let p = &f.db.list_work_packages().await[0];
    assert_eq!((p.title.as_str(), p.sort_order), ("W", Some(7)));
}

#[tokio::test]
async fn update_unlinks_then_links() {
    let f = fixture();
    let a = task(&f.db, "a").await;
    let b = task(&f.db, "b").await;
    let id = f
        .svc
        .create_work_package(create("W", vec![Item::Task(a)]))
        .await
        .unwrap();

    f.svc
        .update_work_package(UpdateWorkPackageParams {
            link: vec![Item::Task(b)],
            unlink: vec![Item::Task(a)],
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap();

    assert_eq!(links_of(&f.db, id).await, vec![Item::Task(b)]);
}

#[tokio::test]
async fn linking_twice_and_unlinking_an_absent_item_are_silent() {
    let f = fixture();
    let a = task(&f.db, "a").await;
    let b = task(&f.db, "b").await;
    let id = f
        .svc
        .create_work_package(create("W", vec![Item::Task(a)]))
        .await
        .unwrap();

    f.svc
        .update_work_package(UpdateWorkPackageParams {
            link: vec![Item::Task(a), Item::Task(a)],
            unlink: vec![Item::Task(b)],
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap();

    assert_eq!(links_of(&f.db, id).await, vec![Item::Task(a)]);
}

#[tokio::test]
async fn an_item_in_both_lists_is_refused_before_any_write() {
    let f = fixture();
    let a = task(&f.db, "a").await;
    let id = f
        .svc
        .create_work_package(create("W", vec![]))
        .await
        .unwrap();

    let err = f
        .svc
        .update_work_package(UpdateWorkPackageParams {
            title: Some("renamed".into()),
            link: vec![Item::Task(a)],
            unlink: vec![Item::Task(a)],
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap_err();

    assert!(matches!(err, ServiceError::Validation(_)), "{err}");
    assert_eq!(f.db.list_work_packages().await[0].title, "W");
    assert!(links_of(&f.db, id).await.is_empty());
}

#[tokio::test]
async fn an_unknown_item_to_link_is_refused_before_any_write() {
    let f = fixture();
    let a = task(&f.db, "a").await;
    let id = f
        .svc
        .create_work_package(create("W", vec![Item::Task(a)]))
        .await
        .unwrap();

    let err = f
        .svc
        .update_work_package(UpdateWorkPackageParams {
            title: Some("renamed".into()),
            unlink: vec![Item::Task(a)],
            link: vec![Item::Task(TaskId(404))],
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap_err();

    assert!(matches!(err, ServiceError::NotFound(_)), "{err}");
    assert_eq!(f.db.list_work_packages().await[0].title, "W");
    assert_eq!(links_of(&f.db, id).await, vec![Item::Task(a)]);
}

#[tokio::test]
async fn update_of_an_unknown_package_is_not_found() {
    let f = fixture();
    let err = f
        .svc
        .update_work_package(UpdateWorkPackageParams {
            title: Some("x".into()),
            ..UpdateWorkPackageParams::for_package(WorkPackageId(9))
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "{err}");
}

#[tokio::test]
async fn update_refuses_a_blank_title_and_an_empty_call() {
    let f = fixture();
    let id = f
        .svc
        .create_work_package(create("W", vec![]))
        .await
        .unwrap();

    let blank = f
        .svc
        .update_work_package(UpdateWorkPackageParams {
            title: Some(" ".into()),
            ..UpdateWorkPackageParams::for_package(id)
        })
        .await
        .unwrap_err();
    assert!(matches!(blank, ServiceError::Validation(_)), "{blank}");

    let empty = f
        .svc
        .update_work_package(UpdateWorkPackageParams::for_package(id))
        .await
        .unwrap_err();
    assert!(matches!(empty, ServiceError::Validation(_)), "{empty}");
}

// --- DeleteWorkPackage ---------------------------------------------------

#[tokio::test]
async fn delete_removes_the_package_and_its_links_but_keeps_the_items() {
    let f = fixture();
    let t = task(&f.db, "kept").await;
    let e = epic(&f.db, "kept too").await;
    let id = f
        .svc
        .create_work_package(create("W", vec![Item::Task(t), Item::Epic(e)]))
        .await
        .unwrap();

    f.svc.delete_work_package(id).await.unwrap();

    assert!(f.db.list_work_packages().await.is_empty());
    assert!(f.db.list_work_package_links().await.is_empty());
    assert!(f.db.get_task(t).await.is_some());
    assert!(crate::store::EpicRead::get_epic(&*f.db, e).await.is_some());
}

#[tokio::test]
async fn delete_of_an_unknown_package_is_not_found() {
    let f = fixture();
    let err = f
        .svc
        .delete_work_package(WorkPackageId(9))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "{err}");
}
