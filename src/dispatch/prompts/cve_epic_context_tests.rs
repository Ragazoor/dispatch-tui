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
    db.get_task(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_task_on_the_cve_root_is_under_the_cve_feed() {
    let db = Store::open_in_memory().unwrap();
    let cve = db
        .create_managed_role_epic("CVE", None, FeedRole::Cve, Some("./fetch-cve.sh"), None)
        .await
        .unwrap();
    let task = task_in_epic(&db, cve).await;
    let ctx = EpicContext::from_db(&task, &db).await.unwrap();
    assert!(ctx.under_cve_feed, "the CVE root itself must answer true");
}

#[tokio::test]
async fn a_task_on_a_repo_group_sub_epic_of_the_cve_root_is_under_the_cve_feed() {
    let db = Store::open_in_memory().unwrap();
    let cve = db
        .create_managed_role_epic("CVE", None, FeedRole::Cve, Some("./fetch-cve.sh"), None)
        .await
        .unwrap();
    // What `group_by_repo` produces: feed_role stays `none` on the child.
    let group = db
        .create_repo_group_sub_epic(cve, "dispatch")
        .await
        .unwrap();
    assert_eq!(
        db.get_epic(group).await.unwrap().unwrap().feed_role,
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
    let parent = db
        .create_managed_role_epic(
            "PR Reviews",
            None,
            FeedRole::ReviewsParent,
            Some("./fetch-reviews.sh"),
            None,
        )
        .await
        .unwrap();
    let bots = db
        .create_managed_role_epic("Bots", Some(parent), FeedRole::Bots, None, None)
        .await
        .unwrap();
    let task = task_in_epic(&db, bots).await;
    let ctx = EpicContext::from_db(&task, &db).await.unwrap();
    assert!(!ctx.under_cve_feed, "a reviews sub-epic is not CVE work");
}
