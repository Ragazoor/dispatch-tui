use super::*;
use crate::models::FeedRole;
use crate::store::{CreateTaskRequest, EpicCrud, EpicRead, Store, TaskCrud, TaskRead};

async fn task_in_epic(db: &Store, epic_id: EpicId) -> crate::models::Task {
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            epic_id: Some(epic_id),
            tag: Some(TaskTag::Fix),
            ..CreateTaskRequest::fixture("[HIGH] repo: CVE-1", "/repo/test")
        })
        .await
        .unwrap();
    db.get_task(id).await.unwrap()
}

/// The managed epic carrying `role`, provisioned the way the board does it.
async fn managed_epic(db: &Store, role: FeedRole) -> EpicId {
    db.ensure_managed_epics(
        Some("./fetch-reviews.sh"),
        None,
        Some("./fetch-cve.sh"),
        None,
    )
    .await
    .unwrap();
    db.list_epics()
        .await
        .into_iter()
        .find(|e| e.feed_role == role)
        .unwrap()
        .id
}

#[tokio::test]
async fn a_task_on_the_cve_root_is_under_the_cve_feed() {
    let db = Store::open_in_memory().unwrap();
    let cve = managed_epic(&db, FeedRole::Cve).await;
    let task = task_in_epic(&db, cve).await;
    let ctx = EpicContext::from_db(&task, &db).await.unwrap();
    assert!(ctx.under_cve_feed, "the CVE root itself must answer true");
}

#[tokio::test]
async fn a_task_on_a_repo_group_sub_epic_of_the_cve_root_is_under_the_cve_feed() {
    let db = Store::open_in_memory().unwrap();
    let cve = managed_epic(&db, FeedRole::Cve).await;
    // What `group_by_repo` produces: feed_role stays `none` on the child.
    db.patch_epic(cve, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    // A feed epic is grouped by ingestion, not by `regroup_epic`.
    let item = crate::models::FeedItem {
        external_id: "CVE-1".to_string(),
        title: "CVE-1".to_string(),
        description: String::new(),
        url: "https://github.com/kognic/dispatch/issues/1".to_string(),
        url_type: None,
        status: crate::models::TaskStatus::Backlog,
        tag: TaskTag::Fix,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    };
    db.ingest_feed(
        cve,
        &[item],
        &["/x/dispatch".to_string()],
        &["main".to_string()],
        false,
    )
    .await
    .unwrap();
    let group = db
        .list_sub_epics(cve)
        .await
        .into_iter()
        .find(|e| e.title == "dispatch")
        .unwrap()
        .id;
    assert_eq!(
        db.get_epic(group).await.unwrap().feed_role,
        FeedRole::None,
        "the grouping sub-epic carries no role of its own — that is the \
whole reason the answer is an ancestry walk"
    );

    let task = task_in_epic(&db, group).await;
    let ctx = EpicContext::from_db(&task, &db).await.unwrap();
    assert!(
        ctx.under_cve_feed,
        "a grouped CVE board must still answer true"
    );
}

#[tokio::test]
async fn an_ordinary_epic_tree_is_not_under_the_cve_feed() {
    let db = Store::open_in_memory().unwrap();
    let root = db.create_epic("Dispatch", "", None).await.unwrap().id;
    let child = db.create_epic("Sub", "", Some(root)).await.unwrap().id;

    for epic_id in [root, child] {
        let task = task_in_epic(&db, epic_id).await;
        let ctx = EpicContext::from_db(&task, &db).await.unwrap();
        assert!(
            !ctx.under_cve_feed,
            "epic #{} is not a CVE feed epic",
            epic_id.0
        );
    }
}

/// The reviews tree is a managed feed too, and a `bots` sub-epic of it is
/// exactly the shape the walk must not confuse for CVE work.
#[tokio::test]
async fn the_reviews_tree_is_not_under_the_cve_feed() {
    let db = Store::open_in_memory().unwrap();
    let bots = managed_epic(&db, FeedRole::Bots).await;
    let task = task_in_epic(&db, bots).await;
    let ctx = EpicContext::from_db(&task, &db).await.unwrap();
    assert!(!ctx.under_cve_feed, "a reviews sub-epic is not CVE work");
}
