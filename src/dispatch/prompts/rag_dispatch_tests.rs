use std::sync::Arc;

use crate::embeddings::{serialize_embedding, EmbeddingService};
use crate::models::{LearningKind, LearningScope};
use crate::store::{
    CreateLearningRow, CreateTaskRequest, LearningRetrievalStore, LearningStore, Store, TaskCrud,
    TaskRead,
};

use super::{build_and_record_injections, list_learnings_for_dispatch_rag, DISPATCH_INJECTION_CAP};

// The test EmbeddingService returns vec![0.1f32; 384]. Use the same dimensionality
// for stored embeddings so cosine similarity is computed correctly.
fn fake_emb_bytes() -> Vec<u8> {
    serialize_embedding(&vec![0.1f32; 384])
}

async fn seed_db() -> Arc<Store> {
    Arc::new(Store::open_in_memory().unwrap())
}

async fn make_task(db: &Arc<Store>) -> crate::models::Task {
    let id = db
        .create_task(CreateTaskRequest {
            description: "test description",
            ..CreateTaskRequest::fixture("test task", "/repo/test")
        })
        .await
        .unwrap();
    db.get_task(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn dispatch_injection_includes_procedural_learnings_without_prioritizing_them() {
    let db = seed_db().await;
    let task = make_task(&db).await;
    let emb = fake_emb_bytes();

    let proc_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Procedural,
            summary: "always run clippy",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();

    for i in 0..2 {
        db.create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: &format!("convention {i}"),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();
    }

    let emb_svc = EmbeddingService::new_test();
    // threshold=0.0 so all candidates pass the cosine filter
    let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

    assert!(!results.is_empty(), "should return at least one learning");
    // Procedural learnings are still included — just not artificially first.
    let ids: Vec<_> = results.iter().map(|l| l.id).collect();
    assert!(
        ids.contains(&proc_id),
        "procedural learning must be in results"
    );
}

#[tokio::test]
async fn dispatch_injection_excludes_task_scoped_learnings() {
    let db = seed_db().await;
    let task = make_task(&db).await;
    let emb = fake_emb_bytes();

    // Task-scoped learning — should be excluded by list_all_approved_non_task_learnings
    db.create_learning(CreateLearningRow {
        kind: LearningKind::Convention,
        summary: "task-scoped learning",
        detail: None,
        scope: LearningScope::Task,
        scope_ref: Some(&task.id.0.to_string()),
        tags: &[],
        source_task_id: Some(task.id),
        embedding: Some(&emb),
    })
    .await
    .unwrap();

    let emb_svc = EmbeddingService::new_test();
    let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

    assert!(
        results.iter().all(|l| l.scope != LearningScope::Task),
        "task-scoped learnings must not appear in dispatch injection"
    );
}

#[tokio::test]
async fn dispatch_injection_respects_cap_of_5() {
    let db = seed_db().await;
    let task = make_task(&db).await;
    let emb = fake_emb_bytes();

    // Seed 8 approved non-task learnings with embeddings
    for i in 0..8 {
        db.create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: &format!("convention {i}"),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();
    }

    let emb_svc = EmbeddingService::new_test();
    let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

    assert_eq!(
        results.len(),
        DISPATCH_INJECTION_CAP,
        "should return at most DISPATCH_INJECTION_CAP ({DISPATCH_INJECTION_CAP}) learnings"
    );
}

#[tokio::test]
async fn dispatch_injection_excludes_learnings_without_embeddings() {
    let db = seed_db().await;
    let task = make_task(&db).await;
    let emb = fake_emb_bytes();

    // One learning with embedding, one without
    let with_emb_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "has embedding",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();

    let no_emb_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
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

    let emb_svc = EmbeddingService::new_test();
    let results = list_learnings_for_dispatch_rag(&*db, &task, &emb_svc, 0.0).await;

    assert!(
        results.iter().any(|l| l.id == with_emb_id),
        "learning with embedding should be included"
    );
    assert!(
        results.iter().all(|l| l.id != no_emb_id),
        "learning without embedding should be excluded"
    );
}

#[tokio::test]
async fn build_and_record_injections_records_all_as_prompt_injection() {
    let db = seed_db().await;
    let task = make_task(&db).await;
    let emb = fake_emb_bytes();

    let proc_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Procedural,
            summary: "always run tests",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();

    let conv_id = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "use Arc for shared state",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo/test"),
            tags: &[],
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();

    let emb_svc = EmbeddingService::new_test();
    let injected = build_and_record_injections(&*db, &task, &emb_svc).await;

    assert_eq!(injected.len(), 2);
    let ids: Vec<_> = injected.iter().map(|l| l.id).collect();
    assert!(ids.contains(&proc_id));
    assert!(ids.contains(&conv_id));

    // All retrievals recorded as PromptInjection regardless of kind.
    let rows = db.list_retrievals_for_task(task.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| matches!(r.source, crate::models::RetrievalSource::PromptInjection)));
}
