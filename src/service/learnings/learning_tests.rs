
use std::sync::Arc;

use super::{CreateLearningParams, LearningService, QueryLearningsParams};
use crate::embeddings::EmbeddingService;
use crate::models::{
    LearningId, LearningKind, LearningScope, LearningStatus, LearningVerdict, RetrievalSource,
    TaskId,
};
use crate::service::ServiceError;
use crate::store::{CreateTaskRequest, Store, TaskStore};

async fn service() -> LearningService {
    let db = Arc::new(Store::open_in_memory().unwrap());
    LearningService::new(db, EmbeddingService::new_test())
}

async fn service_with_db() -> (LearningService, Arc<dyn TaskStore>) {
    let db: Arc<dyn TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    (
        LearningService::new(db.clone(), EmbeddingService::new_test()),
        db,
    )
}

async fn seed_task(db: &Arc<dyn TaskStore>) -> TaskId {
    db.create_task(CreateTaskRequest::fixture("test task", "/repo"))
        .await
        .unwrap()
}

async fn seed_approved_learning(svc: &LearningService) -> LearningId {
    svc.create_learning(CreateLearningParams {
        kind: LearningKind::Convention,
        summary: "A convention".to_string(),
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: vec![],
        source_task_id: None,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn create_learning_rejects_empty_summary() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn create_learning_rejects_user_scope_with_scope_ref() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Preference,
            summary: "Some preference".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: Some("should-be-null".to_string()),
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn create_learning_rejects_non_user_scope_without_scope_ref() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "A convention".to_string(),
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn create_learning_succeeds_with_valid_params() {
    let svc = service().await;
    let id = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "Use Arc for shared state".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap();
    let learning = svc.get_learning(id).await.unwrap();
    assert_eq!(learning.status, LearningStatus::Approved);
}

#[tokio::test]
async fn get_learning_not_found_returns_error() {
    let svc = service().await;
    let err = svc.get_learning(LearningId(99999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn apply_verdicts_validation_rejects_unknown_retrieval() {
    let (svc, db) = service_with_db().await;
    let task_id = seed_task(&db).await;
    let learning_id = seed_approved_learning(&svc).await;
    let err = svc
        .apply_verdicts(task_id, vec![(learning_id, LearningVerdict::Helped)])
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn apply_verdicts_succeeds_when_retrieval_exists() {
    let (svc, db) = service_with_db().await;
    let task_id = seed_task(&db).await;
    let learning_id = seed_approved_learning(&svc).await;
    svc.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    svc.apply_verdicts(task_id, vec![(learning_id, LearningVerdict::Helped)])
        .await
        .unwrap();
    let l = svc.get_learning(learning_id).await.unwrap();
    assert_eq!(l.upvote_count, 1);
}

#[tokio::test]
async fn apply_verdicts_empty_is_ok() {
    let (svc, db) = service_with_db().await;
    let task_id = seed_task(&db).await;
    svc.apply_verdicts(task_id, vec![]).await.unwrap();
}

#[tokio::test]
async fn delete_learning_removes_entry() {
    let svc = service().await;
    let id = seed_approved_learning(&svc).await;
    svc.delete_learning(id).await.unwrap();
    let err = svc.get_learning(id).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn delete_learning_not_found_returns_error() {
    let svc = service().await;
    let err = svc.delete_learning(LearningId(99999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn query_learnings_ranks_and_records_retrieval() {
    let (svc, db) = service_with_db().await;
    let task_id = seed_task(&db).await;
    let learning_id = seed_approved_learning(&svc).await;

    let ranked = svc
        .query_learnings(QueryLearningsParams {
            task_id,
            query: Some("A convention".to_string()),
            tag_filter: vec![],
            limit: 10,
        })
        .await
        .unwrap();

    assert!(
        ranked.iter().any(|l| l.id == learning_id),
        "expected the seeded learning among ranked results"
    );

    let retrievals = db.list_retrievals_for_task(task_id).await.unwrap();
    assert!(
        retrievals.iter().any(|r| r.learning_id == learning_id),
        "query_learnings must record a retrieval for each ranked learning"
    );
}

#[tokio::test]
async fn query_learnings_unknown_task_returns_error() {
    let svc = service().await;
    let err = svc
        .query_learnings(QueryLearningsParams {
            task_id: TaskId(99999),
            query: Some("q".to_string()),
            tag_filter: vec![],
            limit: 10,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn create_learning_embeds_on_write() {
    let db: Arc<dyn TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let emb_svc = EmbeddingService::new_test();
    let svc = LearningService::new(db.clone(), emb_svc);
    let id = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "test summary".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap();
    // Retrieve the raw row and verify embedding bytes are stored.
    let learnings_with_emb = db.list_all_approved_non_task_learnings().await.unwrap();
    let emb_entry = learnings_with_emb
        .iter()
        .find(|(l, _)| l.id == id)
        .expect("newly created learning should appear in approved non-task learnings");
    // EmbeddingService::new_test() returns vec![0.1f32; 384], which is 384 * 4 = 1536 bytes.
    assert_eq!(
        emb_entry.1.len(),
        384 * 4,
        "embedding should be 1536 bytes for 384 f32 values"
    );
}

#[test]
fn find_code_citation_rejects_path_rs_symbol() {
    let hit = super::find_code_citation(
        "A step that must behave identically on both feed paths goes in \
             src/feed/cycle.rs::run_feed_cycle.",
    );
    assert_eq!(hit, Some("src/feed/cycle.rs::run_feed_cycle"));
}

#[test]
fn find_code_citation_rejects_type_method() {
    let hit = super::find_code_citation("The FeedCycle::run entry point drives both paths.");
    assert_eq!(hit, Some("FeedCycle::run"));
}

#[test]
fn find_code_citation_rejects_long_bare_snake_case() {
    let hit = super::find_code_citation(
        "Pinned by exec_trigger_epic_feed_quiet_command_reports_no_stderr today.",
    );
    assert_eq!(
        hit,
        Some("exec_trigger_epic_feed_quiet_command_reports_no_stderr")
    );
}

#[test]
fn find_code_citation_rejects_long_bare_snake_case_even_backticked() {
    let hit = super::find_code_citation(
        "See `exec_trigger_epic_feed_quiet_command_reports_no_stderr` for the case.",
    );
    assert!(hit.is_some());
}

#[test]
fn find_code_citation_allows_short_backticked_tool_names() {
    assert_eq!(
        super::find_code_citation("Call `query_learnings` before guessing."),
        None
    );
    assert_eq!(
        super::find_code_citation("Rate it with `rate_learning`, then `wrap_up`."),
        None
    );
}

#[test]
fn find_code_citation_allows_plain_prose() {
    assert_eq!(
        super::find_code_citation(
            "TaskPatch double-Option means Some(None) clears a field, None leaves it unchanged."
        ),
        None
    );
    assert_eq!(
        super::find_code_citation("Feed-cycle logic must live in one shared place."),
        None
    );
}

#[tokio::test]
async fn create_learning_rejects_summary_with_code_citation() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "A step goes in src/feed/cycle.rs::run_feed_cycle.".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn create_learning_rejects_detail_with_code_citation() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "Feed-cycle logic must live in one shared place.".to_string(),
            detail: Some("See FeedCycle::run for the exact entry point.".to_string()),
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn create_learning_allows_tool_name_reference() {
    let svc = service().await;
    svc.create_learning(CreateLearningParams {
        kind: LearningKind::Convention,
        summary: "Call `query_learnings` before guessing, not after.".to_string(),
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: vec![],
        source_task_id: None,
    })
    .await
    .unwrap();
}

// -----------------------------------------------------------------
// Empty-parens calls, macro invocations and source-file references
// -----------------------------------------------------------------

#[test]
fn find_code_citation_rejects_empty_parens_call() {
    assert_eq!(
        super::find_code_citation("Migration tests must build the store with in_memory_db()."),
        Some("in_memory_db()")
    );
    assert_eq!(
        super::find_code_citation("Assert on `recorded_calls()` rather than the queue."),
        Some("recorded_calls()")
    );
}

#[test]
fn find_code_citation_rejects_macro_invocation() {
    assert_eq!(
        super::find_code_citation("The service_api_delegate! macro generates the impl."),
        Some("service_api_delegate!")
    );
    // No underscore, but the bracket makes it unambiguous.
    assert_eq!(
        super::find_code_citation("Reach for vec![] over an explicit push loop."),
        Some("vec!")
    );
}

#[test]
fn find_code_citation_rejects_a_path_into_the_tree() {
    assert_eq!(
        super::find_code_citation("The wire format is parsed in src/feed/ingest.rs."),
        Some("src/feed/ingest.rs")
    );
    // A doc path rots the same way a source path does.
    assert_eq!(
        super::find_code_citation("Read docs/testing.md before adding a test target."),
        Some("docs/testing.md")
    );
}

#[test]
fn find_code_citation_rejects_a_bare_source_filename() {
    assert_eq!(
        super::find_code_citation("The poll loop lives in cycle.rs."),
        Some("cycle.rs")
    );
}

/// The prose the new shapes must NOT swallow. Every line here is something
/// an agent has a real reason to write, so a regression in any of them
/// costs an entry that should have been recorded — the failure mode the
/// filter is deliberately kept narrow to avoid.
#[test]
fn find_code_citation_allows_prose_and_stable_names() {
    for text in [
        // `()` not preceded by an identifier is a unit type, not a call.
        "A delete returning Result<()> cannot say \"not found\".",
        // An exclamation mark in prose is not a macro.
        "Never push to main! Open a PR instead.",
        // No directory separator, and the extension names a document or a
        // manifest rather than a language — these do not rot.
        "A fresh worktree needs npm ci because package-lock.json is not shared.",
        "Version pins belong in Cargo.toml, not in a build script.",
        "That convention is already in CLAUDE.md.",
        "Shared feed-cycle behaviour is specified in feeds.allium.",
        // A git ref has no extension; a URL is exempt.
        "Rebase onto origin/main before wrapping up.",
        "The upstream issue is at https://github.com/foo/bar.",
    ] {
        assert_eq!(super::find_code_citation(text), None, "text: {text}");
    }
}

// -----------------------------------------------------------------
// Procedural entries must state their boundary
// -----------------------------------------------------------------

#[tokio::test]
async fn create_learning_rejects_procedural_without_detail() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Procedural,
            summary: "Always sync the repo before starting work.".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn create_learning_allows_procedural_with_detail() {
    let svc = service().await;
    svc.create_learning(CreateLearningParams {
        kind: LearningKind::Procedural,
        summary: "Always sync the repo before starting work.".to_string(),
        detail: Some(
            "Stop and ask the user when the sync reports a conflict you did not cause.".to_string(),
        ),
        scope: LearningScope::User,
        scope_ref: None,
        tags: vec![],
        source_task_id: None,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn create_learning_rejects_procedural_with_a_blank_detail() {
    let svc = service().await;
    let err = svc
        .create_learning(CreateLearningParams {
            kind: LearningKind::Procedural,
            summary: "Always sync the repo before starting work.".to_string(),
            detail: Some("   \n  ".to_string()),
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

/// Driven off `LearningKind::ALL` rather than a hand-written list so a
/// seventh kind cannot quietly opt out of the assertion that only
/// `procedural` needs a detail.
#[tokio::test]
async fn create_learning_allows_other_kinds_without_detail() {
    let svc = service().await;
    for kind in LearningKind::ALL
        .iter()
        .copied()
        .filter(|k| *k != LearningKind::Procedural)
    {
        svc.create_learning(CreateLearningParams {
            kind,
            summary: "A summary that stands on its own without a detail.".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .await
        .unwrap();
    }
}
