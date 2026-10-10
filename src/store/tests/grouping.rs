//! Repo grouping as the store decides it (`epics.allium`: `RegroupEpic`,
//! `FlattenEpic`, and the routing note above them; store-split M2, task
//! #53255). These were `service::grouping`'s tests, when the service made the
//! decisions over several store calls.

use crate::models::{EpicId, LearningKind, LearningScope, LearningStatus};
use crate::store::{
    CreateLearningRow, EpicCrud, EpicRead, LearningFilter, LearningStore, Store, TaskCrud,
    TaskPatch,
};

async fn mk() -> Store {
    Store::open_in_memory().unwrap()
}

async fn add_task(db: &Store, epic: EpicId, repo: &str) -> crate::models::TaskId {
    db.create_task(crate::store::CreateTaskRequest {
        epic_id: Some(epic),
        ..crate::store::CreateTaskRequest::fixture("t", repo)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn regroup_migrates_all_direct_tasks() {
    let db = mk().await;
    let root = db.create_epic("root", "", None).await.unwrap();
    // Tasks that pre-date the flag sit directly on the root: only the flag flip
    // leaves them there, which is what regroup exists to repair.
    add_task(&db, root.id, "/x/alpha").await;
    add_task(&db, root.id, "/x/beta").await;
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    db.regroup_epic(root.id).await.unwrap();
    assert!(
        db.list_tasks_for_epic(root.id).await.is_empty(),
        "root has no direct tasks after regroup"
    );
    let subs = db.list_sub_epics(root.id).await;
    assert_eq!(subs.len(), 2);
}

#[tokio::test]
async fn flatten_rehomes_tasks_then_deletes_empty_repo_groups() {
    let db = mk().await;
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    add_task(&db, root.id, "/x/alpha").await;
    db.regroup_epic(root.id).await.unwrap();

    db.flatten_epic(root.id).await.unwrap();
    assert_eq!(
        db.list_tasks_for_epic(root.id).await.len(),
        1,
        "task re-homed, not deleted"
    );
    assert!(
        db.list_sub_epics(root.id).await.is_empty(),
        "emptied repo-group sub-epics deleted"
    );
}

#[tokio::test]
async fn flatten_preserves_manual_sub_epics() {
    let db = mk().await;
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    let manual = db.create_epic("notes", "", Some(root.id)).await.unwrap(); // origin=Manual
    db.flatten_epic(root.id).await.unwrap();
    assert!(
        db.get_epic(manual.id).await.is_some(),
        "Manual sub-epic must survive flatten"
    );
}

#[tokio::test]
async fn reroute_moves_task_to_correct_sub_epic_and_cleans_source() {
    let db = mk().await;
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    let t = add_task(&db, root.id, "/x/alpha").await;
    db.regroup_epic(root.id).await.unwrap();
    // task now in "alpha" sub-epic; change its repo and reroute.
    // The store re-routes inside the patch that changes the repo.
    db.patch_task(t, &TaskPatch::new().repo_path("/x/beta"))
        .await
        .unwrap();
    let subs = db.list_sub_epics(root.id).await;
    let titles: Vec<_> = subs.iter().map(|e| e.title.clone()).collect();
    assert!(titles.contains(&"beta".to_string()));
    assert!(
        !titles.contains(&"alpha".to_string()),
        "emptied source sub-epic cleaned up"
    );
}

#[tokio::test]
async fn rescope_learnings_when_repo_group_sub_epic_deleted_by_flatten() {
    let db = mk().await;
    // Create a group_by_repo root epic.
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    // Route a task into a RepoGroup sub-epic so the sub-epic exists.
    add_task(&db, root.id, "/x/alpha").await;
    db.regroup_epic(root.id).await.unwrap();

    // Capture the sub-epic's id before deletion.
    let subs = db.list_sub_epics(root.id).await;
    let sub = subs
        .iter()
        .find(|e| e.title == "alpha")
        .expect("alpha sub-epic must exist after regroup");
    let sub_id = sub.id;

    // Create an epic-scoped learning pointing at the sub-epic.
    let sub_id_str = sub_id.0.to_string();
    let lid = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "alpha repo convention",
            detail: None,
            scope: LearningScope::Epic,
            scope_ref: Some(&sub_id_str),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    // Re-home tasks to root and flatten (this deletes the empty sub-epic).
    db.flatten_epic(root.id).await.unwrap();

    // The sub-epic must be gone.
    assert!(
        db.get_epic(sub_id).await.is_none(),
        "sub-epic should be deleted after flatten"
    );

    // The learning must now point at the root epic (re-scoped), not the deleted sub.
    let learning = db.get_learning(lid).await.unwrap().unwrap();
    assert_eq!(
        learning.scope,
        LearningScope::Epic,
        "scope must remain 'epic'"
    );
    assert_eq!(
        learning.scope_ref,
        Some(root.id.0.to_string()),
        "scope_ref must be re-scoped to the root epic"
    );
    assert_eq!(
        learning.status,
        LearningStatus::Approved,
        "status must be unchanged"
    );
}

#[tokio::test]
async fn rescope_learnings_when_repo_group_sub_epic_deleted_by_reroute() {
    let db = mk().await;
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    let t = add_task(&db, root.id, "/x/alpha").await;
    db.regroup_epic(root.id).await.unwrap();

    let subs = db.list_sub_epics(root.id).await;
    let sub = subs
        .iter()
        .find(|e| e.title == "alpha")
        .expect("alpha sub-epic must exist after regroup");
    let sub_id = sub.id;
    let sub_id_str = sub_id.0.to_string();

    // Create an epic-scoped learning pointing at the alpha sub-epic.
    let lid = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "alpha-specific learning",
            detail: None,
            scope: LearningScope::Epic,
            scope_ref: Some(&sub_id_str),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    // Reroute the only task to beta — this should clean up the alpha sub-epic.
    db.patch_task(t, &TaskPatch::new().repo_path("/x/beta"))
        .await
        .unwrap();

    // Alpha sub-epic must be gone.
    assert!(
        db.get_epic(sub_id).await.is_none(),
        "alpha sub-epic should be deleted after reroute"
    );

    // Learning must be re-scoped to root.
    let learning = db.get_learning(lid).await.unwrap().unwrap();
    assert_eq!(
        learning.scope_ref,
        Some(root.id.0.to_string()),
        "scope_ref must be re-scoped to root after reroute cleanup"
    );
}

#[tokio::test]
async fn unscoped_and_other_learnings_untouched_during_rescope() {
    let db = mk().await;
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    add_task(&db, root.id, "/x/alpha").await;
    db.regroup_epic(root.id).await.unwrap();

    let subs = db.list_sub_epics(root.id).await;
    let sub_id = subs.iter().find(|e| e.title == "alpha").unwrap().id;
    let sub_id_str = sub_id.0.to_string();
    let root_id_str = root.id.0.to_string();

    // A user-scoped learning (no scope_ref) should be unaffected.
    let user_lid = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "user learning",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    // An epic-scoped learning pointing at the ROOT should also be unaffected.
    let root_lid = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "root learning",
            detail: None,
            scope: LearningScope::Epic,
            scope_ref: Some(&root_id_str),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    // The target learning on the sub-epic.
    let sub_lid = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "sub learning",
            detail: None,
            scope: LearningScope::Epic,
            scope_ref: Some(&sub_id_str),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    db.flatten_epic(root.id).await.unwrap();

    // User-scoped learning: unchanged.
    let ul = db.get_learning(user_lid).await.unwrap().unwrap();
    assert_eq!(ul.scope, LearningScope::User);
    assert_eq!(ul.scope_ref, None);

    // Root-epic-scoped learning: still points at root, not modified.
    let rl = db.get_learning(root_lid).await.unwrap().unwrap();
    assert_eq!(rl.scope_ref, Some(root_id_str));

    // Sub-epic learning: re-scoped to root.
    let sl = db.get_learning(sub_lid).await.unwrap().unwrap();
    assert_eq!(sl.scope_ref, Some(root.id.0.to_string()));

    // Verify no spurious learnings created via filter.
    let all = db.list_learnings(LearningFilter::default()).await.unwrap();
    assert_eq!(all.len(), 3, "no extra learnings created");
}
