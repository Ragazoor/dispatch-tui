#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn landscape_kind_can_be_recorded() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Landscape,
            summary: "Service X owns auth, service Y owns billing",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/home/user/repo"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let l = db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(l.kind, LearningKind::Landscape);
}

#[tokio::test]
async fn episodic_kind_does_not_parse() {
    use crate::models::LearningKind;
    assert!(LearningKind::parse("episodic").is_none());
}

#[tokio::test]
async fn test_learning_status_no_proposed() {
    // Proposed must no longer be a valid parse target
    assert!(crate::models::LearningStatus::parse("proposed").is_err());
    // Approved parses correctly
    assert_eq!(
        crate::models::LearningStatus::parse("approved").unwrap(),
        crate::models::LearningStatus::Approved,
    );
}

#[tokio::test]
async fn create_and_get_learning() {
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Always use Arc for shared DB state",
            detail: Some("Avoids locking issues in async contexts"),
            scope: LearningScope::Repo,
            scope_ref: Some("/home/user/repo"),
            tags: &["rust".to_string(), "async".to_string()],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let learning = db
        .get_learning(id)
        .await
        .unwrap()
        .expect("learning should exist");
    assert_eq!(learning.id, id);
    assert_eq!(learning.kind, LearningKind::Convention);
    assert_eq!(learning.summary, "Always use Arc for shared DB state");
    assert_eq!(
        learning.detail.as_deref(),
        Some("Avoids locking issues in async contexts")
    );
    assert_eq!(learning.scope, LearningScope::Repo);
    assert_eq!(learning.scope_ref.as_deref(), Some("/home/user/repo"));
    assert_eq!(learning.tags, vec!["rust", "async"]);
    assert_eq!(learning.status, LearningStatus::Approved);
    assert_eq!(learning.upvote_count, 0);
    assert!(learning.last_upvoted_at.is_none());
    assert!(learning.source_task_id.is_none());
}

#[tokio::test]
async fn create_learning_user_scope_has_null_scope_ref() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Preference,
            summary: "Prefer short commits",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let learning = db.get_learning(id).await.unwrap().unwrap();
    assert!(learning.scope_ref.is_none());
}

#[tokio::test]
async fn scope_ref_consistency_constraint_is_enforced() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    // user scope with a non-null scope_ref should violate the CHECK constraint
    let result = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Preference,
            summary: "Should fail",
            detail: None,
            scope: LearningScope::User,
            scope_ref: Some("should-not-be-here"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await;
    assert!(
        result.is_err(),
        "user scope with scope_ref must be rejected"
    );
}

#[tokio::test]
async fn list_learnings_filter_by_status() {
    use crate::db::LearningFilter;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let id1 = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Pitfall,
            summary: "A pitfall",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let id2 = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "A convention",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    // Both learnings default to Approved. Archive id2 so we can filter by status.
    db.patch_learning(
        id2,
        &crate::db::LearningPatch::new().status(LearningStatus::Archived),
    )
    .await
    .unwrap();

    let archived = db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Archived),
            ..Default::default()
        })
        .await
        .unwrap();
    let approved = db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Approved),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, id2);
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].id, id1);
}

#[tokio::test]
async fn list_learnings_approved_filter_excludes_rejected_and_archived() {
    use crate::db::{LearningFilter, LearningPatch};
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let approved_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Pitfall,
            summary: "approved one",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let rejected_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "rejected one",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let archived_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Preference,
            summary: "archived one",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    // Transition rejected and archived
    db.patch_learning(
        rejected_id,
        &LearningPatch::new().status(LearningStatus::Rejected),
    )
    .await
    .unwrap();
    db.patch_learning(
        archived_id,
        &LearningPatch::new().status(LearningStatus::Archived),
    )
    .await
    .unwrap();

    let results = db
        .list_learnings(LearningFilter {
            status: Some(LearningStatus::Approved),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, approved_id);
}

#[tokio::test]
async fn patch_learning_updates_summary_and_updated_at() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Original",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let before = db.get_learning(id).await.unwrap().unwrap();
    db.patch_learning(id, &LearningPatch::new().summary("Updated"))
        .await
        .unwrap();
    let after = db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(after.summary, "Updated");
    assert!(after.updated_at >= before.updated_at);
}

#[tokio::test]
async fn upvote_learning_increments_count_and_timestamps() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "A convention",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    // must be approved first
    db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
        .await
        .unwrap();
    let before = db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(before.upvote_count, 0);
    assert!(before.last_upvoted_at.is_none());

    db.upvote_learning(id).await.unwrap();
    let after = db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(after.upvote_count, 1);
    assert!(after.last_upvoted_at.is_some());
    assert!(after.updated_at >= before.updated_at);

    db.upvote_learning(id).await.unwrap();
    let after2 = db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(after2.upvote_count, 2);
}

#[tokio::test]
async fn delete_learning_removes_row() {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Pitfall,
            summary: "To be deleted",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    assert!(db.get_learning(id).await.unwrap().is_some());
    db.delete_learning(id).await.unwrap();
    assert!(db.get_learning(id).await.unwrap().is_none());
}

#[tokio::test]
async fn list_learnings_for_dispatch_unions_scopes() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;

    // user-scoped: should appear
    let u = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "User learning",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    // repo-scoped matching: should appear
    let r = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Repo learning",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo/a"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    // repo-scoped not matching: should NOT appear
    let _r2 = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Other repo",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo/b"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    // task-scoped: should NOT appear (task scope excluded from auto-dispatch)
    let _t = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Task outcome",
            detail: None,
            scope: LearningScope::Task,
            scope_ref: Some("42"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    // approve all
    for id in [u, r, _r2, _t] {
        db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
            .await
            .unwrap();
    }

    let results = db
        .list_learnings_for_dispatch("/repo/a", None)
        .await
        .unwrap();
    let ids: Vec<_> = results.iter().map(|l| l.id).collect();
    assert!(ids.contains(&u), "user-scoped learning should appear");
    assert!(ids.contains(&r), "matching repo learning should appear");
    assert!(
        !ids.contains(&_r2),
        "non-matching repo learning should not appear"
    );
    assert!(!ids.contains(&_t), "task-scoped learning should not appear");
}

#[tokio::test]
async fn list_learnings_for_dispatch_procedural_first() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;

    let convention = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "A convention",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let procedural = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Procedural,
            summary: "A procedure",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    for id in [convention, procedural] {
        db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
            .await
            .unwrap();
    }

    let results = db.list_learnings_for_dispatch("/any", None).await.unwrap();
    assert_eq!(
        results[0].id, procedural,
        "procedural learning must be first"
    );
}

#[tokio::test]
async fn list_learnings_for_dispatch_excludes_non_approved() {
    use crate::db::LearningPatch;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;

    // Create a learning and reject it — rejected learnings should be excluded from dispatch.
    let rejected = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Rejected",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    db.patch_learning(
        rejected,
        &LearningPatch::new().status(LearningStatus::Rejected),
    )
    .await
    .unwrap();

    // Create an approved learning (default).
    let approved = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Approved",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    let results = db.list_learnings_for_dispatch("/any", None).await.unwrap();
    let ids: Vec<_> = results.iter().map(|l| l.id).collect();
    assert!(!ids.contains(&rejected));
    assert!(ids.contains(&approved));
}

async fn make_db_with_task_and_learning() -> (Database, crate::models::TaskId, LearningId) {
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    let task = create_task_returning(
        &db,
        "t",
        "d",
        "/repo/a",
        None,
        crate::models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let learning = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Some convention",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo/a"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    (db, task.id, learning)
}

#[tokio::test]
async fn record_and_list_retrievals() {
    use crate::models::RetrievalSource;
    let (db, task_id, learning_id) = make_db_with_task_and_learning().await;
    db.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.record_retrieval(task_id, learning_id, RetrievalSource::QueryLearnings)
        .await
        .unwrap();
    let rows = db.list_retrievals_for_task(task_id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].source, RetrievalSource::PromptInjection);
    assert_eq!(rows[0].task_id, task_id);
    assert_eq!(rows[0].learning_id, learning_id);
    assert_eq!(rows[1].source, RetrievalSource::QueryLearnings);
}

#[tokio::test]
async fn apply_verdicts_helped_increments_count() {
    use crate::models::{LearningStatus, LearningVerdict, RetrievalSource};
    let (db, task_id, learning_id) = make_db_with_task_and_learning().await;
    db.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.apply_verdicts_tx(task_id, &[(learning_id, LearningVerdict::Helped)])
        .await
        .unwrap();
    let l = db.get_learning(learning_id).await.unwrap().unwrap();
    assert_eq!(l.upvote_count, 1);
    assert!(l.last_upvoted_at.is_some());
    assert_eq!(l.status, LearningStatus::Approved);
}

#[tokio::test]
async fn apply_verdicts_wrong_sets_needs_review() {
    use crate::models::{LearningStatus, LearningVerdict, RetrievalSource};
    let (db, task_id, learning_id) = make_db_with_task_and_learning().await;
    db.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.apply_verdicts_tx(task_id, &[(learning_id, LearningVerdict::Wrong)])
        .await
        .unwrap();
    let l = db.get_learning(learning_id).await.unwrap().unwrap();
    assert_eq!(l.status, LearningStatus::NeedsReview);
    assert_eq!(l.upvote_count, 0);
}

#[tokio::test]
async fn apply_verdicts_unused_records_only() {
    use crate::models::{LearningStatus, LearningVerdict, RetrievalSource};
    let (db, task_id, learning_id) = make_db_with_task_and_learning().await;
    db.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.apply_verdicts_tx(task_id, &[(learning_id, LearningVerdict::Unused)])
        .await
        .unwrap();
    let l = db.get_learning(learning_id).await.unwrap().unwrap();
    assert_eq!(l.status, LearningStatus::Approved);
    assert_eq!(l.upvote_count, 0);
    let lid = learning_id.0;
    let n: i64 = db
        .db_call(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM learning_verdicts WHERE learning_id = ?1",
                rusqlite::params![lid],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn count_needs_review() {
    use crate::models::{LearningVerdict, RetrievalSource};
    let (db, task_id, learning_id) = make_db_with_task_and_learning().await;
    db.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.apply_verdicts_tx(task_id, &[(learning_id, LearningVerdict::Wrong)])
        .await
        .unwrap();
    assert_eq!(db.count_learnings_needs_review().await.unwrap(), 1);
}

#[tokio::test]
async fn list_learnings_for_dispatch_excludes_needs_review() {
    use crate::models::{LearningKind, LearningScope, LearningVerdict, RetrievalSource};
    let (db, task_id, flagged) = make_db_with_task_and_learning().await;
    let healthy = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Healthy convention",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo/a"),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    db.record_retrieval(task_id, flagged, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    db.apply_verdicts_tx(task_id, &[(flagged, LearningVerdict::Wrong)])
        .await
        .unwrap();

    let results = db
        .list_learnings_for_dispatch("/repo/a", None)
        .await
        .unwrap();
    let ids: Vec<_> = results.iter().map(|l| l.id).collect();
    assert!(
        !ids.contains(&flagged),
        "needs_review learning must be excluded from dispatch"
    );
    assert!(
        ids.contains(&healthy),
        "approved learning should still surface"
    );
}

#[tokio::test]
async fn create_learning_with_embedding_stores_it() {
    use crate::db::{LearningPatch, LearningStore};
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let fake_emb = vec![0u8; 1536]; // 384 f32s * 4 bytes
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "test learning",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&fake_emb),
        })
        .await
        .unwrap();
    // Approve it (already approved by default, but patch status to confirm)
    db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
        .await
        .unwrap();
    let results = db.list_all_approved_non_task_learnings().await.unwrap();
    let found = results.iter().find(|(l, _)| l.id == id).unwrap();
    assert_eq!(found.1.len(), 1536);
}

#[tokio::test]
async fn list_learnings_missing_embedding_finds_null_rows() {
    use crate::db::{LearningPatch, LearningStore};
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Pitfall,
            summary: "no embedding",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
        .await
        .unwrap();
    let missing = db.list_learnings_missing_embedding().await.unwrap();
    assert!(missing.iter().any(|l| l.id == id));
}

#[tokio::test]
async fn patch_learning_can_set_embedding() {
    use crate::db::{LearningPatch, LearningStore};
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "update test",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    db.patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
        .await
        .unwrap();

    // Initially missing
    let missing = db.list_learnings_missing_embedding().await.unwrap();
    assert!(missing.iter().any(|l| l.id == id));

    // Patch with embedding
    let fake_emb = vec![1u8; 1536];
    db.patch_learning(id, &LearningPatch::new().embedding(&fake_emb))
        .await
        .unwrap();

    // No longer missing
    let missing_after = db.list_learnings_missing_embedding().await.unwrap();
    assert!(!missing_after.iter().any(|l| l.id == id));
}

#[tokio::test]
async fn get_learning_errors_on_unknown_kind() {
    use crate::db::CreateLearningRow;
    use crate::models::{LearningKind, LearningScope};
    let db = in_memory_db().await;
    let id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "test",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let learning_id = id.0;
    db.db_call(move |conn| {
        conn.execute(
            "UPDATE learnings SET kind = 'xyzzy_unknown' WHERE id = ?1",
            rusqlite::params![learning_id],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let result = db.get_learning(id).await;
    assert!(
        result.is_err(),
        "expected Err on unknown kind, got {:?}",
        result
    );
}
