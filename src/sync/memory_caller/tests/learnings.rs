use super::*;

// -- Learnings, retrievals and verdicts (task #5003) -----------------------

fn blank_learning() -> bindings::Learning {
    bindings::Learning {
        id: 0,
        kind: "pitfall".into(),
        summary: "summary".into(),
        detail: None,
        scope: "user".into(),
        scope_ref: None,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: TEST_STAMP.into(),
        updated_at: TEST_STAMP.into(),
        embedding: None,
    }
}

#[tokio::test]
async fn create_learning_assigns_an_id_and_pushes_the_row() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    let stored = rows.learning(id).unwrap();
    assert_eq!(stored.summary, "summary");
    assert_eq!(stored.upvote_count, 0);
}

/// `ApprovedLearningsHaveScopeRef` (`docs/specs/learnings.allium`),
/// enforced server-side via `validate_learning_scope` — mirrors the
/// module's own `create_learning`.
#[tokio::test]
async fn create_learning_refuses_a_user_scoped_learning_with_a_scope_ref() {
    let (caller, _rows) = caller();
    let outcome = caller
        .create_learning(bindings::Learning {
            scope: "user".into(),
            scope_ref: Some("1".into()),
            ..blank_learning()
        })
        .await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn create_learning_refuses_a_scoped_learning_with_no_scope_ref() {
    let (caller, _rows) = caller();
    let outcome = caller
        .create_learning(bindings::Learning {
            scope: "epic".into(),
            scope_ref: None,
            ..blank_learning()
        })
        .await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn patch_learning_updates_fields() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    caller
        .patch_learning(
            id,
            bindings::LearningPatch {
                status: Some("archived".into()),
                summary: Some("revised".into()),
                embedding: None,
            },
        )
        .await
        .unwrap();
    let stored = rows.learning(id).unwrap();
    assert_eq!(stored.status.as_str(), "archived");
    assert_eq!(stored.summary, "revised");
}

#[tokio::test]
async fn patch_learning_is_a_silent_no_op_for_a_missing_id() {
    let (caller, _rows) = caller();
    let outcome = caller
        .patch_learning(
            LearningId(999),
            bindings::LearningPatch {
                status: None,
                summary: None,
                embedding: None,
            },
        )
        .await
        .unwrap();
    assert!(outcome.won());
}

/// Unlike `patch_learning`/`delete_task`, a missing id is refused rather
/// than a silent no-op — mirrors the module's own `delete_learning`.
#[tokio::test]
async fn delete_learning_refuses_a_missing_id() {
    let (caller, _rows) = caller();
    let outcome = caller.delete_learning(LearningId(999)).await.unwrap();
    assert!(!outcome.won());
}

#[tokio::test]
async fn delete_learning_cascades_its_retrievals() {
    let (caller, rows) = caller();
    let task_id = caller.create_task(blank_task()).await.unwrap();
    let learning_id = caller.create_learning(blank_learning()).await.unwrap();
    caller
        .record_learning_retrieval(task_id, learning_id, RetrievalSource::QueryLearnings)
        .await
        .unwrap();
    assert_eq!(rows.retrievals_for_task(task_id).len(), 1);

    let outcome = caller.delete_learning(learning_id).await.unwrap();
    assert!(outcome.won());
    assert!(rows.learning(learning_id).is_none());
    assert!(rows.retrievals_for_task(task_id).is_empty());
}

#[tokio::test]
async fn rescope_epic_learnings_moves_matching_epic_scoped_rows() {
    let (caller, rows) = caller();
    let from = caller.create_epic(blank_epic()).await.unwrap();
    let to = caller.create_epic(blank_epic()).await.unwrap();
    let moved = caller
        .create_learning(bindings::Learning {
            scope: "epic".into(),
            scope_ref: Some(from.to_string()),
            ..blank_learning()
        })
        .await
        .unwrap();
    let untouched = caller
        .create_learning(bindings::Learning {
            scope: "epic".into(),
            scope_ref: Some(to.to_string()),
            ..blank_learning()
        })
        .await
        .unwrap();

    caller.rescope_epic_learnings(from, to).await.unwrap();

    assert_eq!(
        rows.learning(moved).unwrap().scope_ref,
        Some(to.to_string())
    );
    assert_eq!(
        rows.learning(untouched).unwrap().scope_ref,
        Some(to.to_string())
    );
}

#[tokio::test]
async fn record_learning_retrieval_inserts_a_row() {
    let (caller, rows) = caller();
    let task_id = caller.create_task(blank_task()).await.unwrap();
    let learning_id = caller.create_learning(blank_learning()).await.unwrap();
    caller
        .record_learning_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
        .await
        .unwrap();
    let retrievals = rows.retrievals_for_task(task_id);
    assert_eq!(retrievals.len(), 1);
    assert_eq!(retrievals[0].learning_id, learning_id);
}

#[tokio::test]
async fn apply_learning_verdicts_applies_helped_and_wrong() {
    let (caller, rows) = caller();
    let helped = caller.create_learning(blank_learning()).await.unwrap();
    let wrong = caller.create_learning(blank_learning()).await.unwrap();
    caller
        .apply_learning_verdicts(vec![
            (helped, LearningVerdict::Helped),
            (wrong, LearningVerdict::Wrong),
        ])
        .await
        .unwrap();
    assert_eq!(rows.learning(helped).unwrap().upvote_count, 1);
    assert_eq!(rows.learning(wrong).unwrap().upvote_count, -1);
}

/// Two entries naming the SAME learning apply against each other's
/// result, not both against the pre-batch row — mirrors the module
/// re-reading via `ctx.db.learnings().id().find()` on every loop
/// iteration. Net count is 0 (+1 then -1), and `last_upvoted_at` is
/// retained from the "helped" step: "wrong" only ever keeps whatever is
/// already there, never clears it.
#[tokio::test]
async fn apply_learning_verdicts_applies_duplicate_entries_for_the_same_id_in_order() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    caller
        .apply_learning_verdicts(vec![
            (id, LearningVerdict::Helped),
            (id, LearningVerdict::Wrong),
        ])
        .await
        .unwrap();
    let stored = rows.learning(id).unwrap();
    assert_eq!(stored.upvote_count, 0);
    assert!(stored.last_upvoted_at.is_some());
}

#[tokio::test]
async fn apply_learning_verdicts_skips_a_missing_learning() {
    let (caller, _rows) = caller();
    let outcome = caller
        .apply_learning_verdicts(vec![(LearningId(999), LearningVerdict::Helped)])
        .await
        .unwrap();
    assert!(outcome.won());
}

#[tokio::test]
async fn archive_stale_learnings_archives_eligible_rows_only() {
    let (caller, rows) = caller();
    let stale = caller
        .create_learning(bindings::Learning {
            upvote_count: 0,
            updated_at: "2025-01-01 00:00:00.000".into(),
            ..blank_learning()
        })
        .await
        .unwrap();
    let upvoted = caller
        .create_learning(bindings::Learning {
            upvote_count: 1,
            updated_at: "2025-01-01 00:00:00.000".into(),
            ..blank_learning()
        })
        .await
        .unwrap();
    let recent = caller
        .create_learning(bindings::Learning {
            upvote_count: 0,
            updated_at: TEST_STAMP.into(),
            ..blank_learning()
        })
        .await
        .unwrap();

    caller
        .archive_stale_learnings(at("2025-06-01 00:00:00.000"))
        .await
        .unwrap();

    assert_eq!(rows.learning(stale).unwrap().status.as_str(), "archived");
    assert_eq!(rows.learning(upvoted).unwrap().status.as_str(), "approved");
    assert_eq!(rows.learning(recent).unwrap().status.as_str(), "approved");
}

/// `delete_task`'s cascade: a learning it sourced loses that link rather
/// than being deleted (`SET NULL`), and every retrieval recorded against
/// it is dropped — mirrors the module's `detach_learnings_from_task`.
#[tokio::test]
async fn delete_task_detaches_source_task_id_and_cascades_retrievals() {
    let (caller, rows) = caller();
    let task_id = caller
        .create_task(bindings::Task {
            status: DONE.into(),
            ..blank_task()
        })
        .await
        .unwrap();
    let sourced = caller
        .create_learning(bindings::Learning {
            source_task_id: Some(task_id.0),
            ..blank_learning()
        })
        .await
        .unwrap();
    let unrelated = caller.create_learning(blank_learning()).await.unwrap();
    caller
        .record_learning_retrieval(task_id, sourced, RetrievalSource::QueryLearnings)
        .await
        .unwrap();

    caller.delete_task(task_id).await.unwrap();

    assert_eq!(rows.learning(sourced).unwrap().source_task_id, None);
    assert!(rows.learning(unrelated).is_some());
    assert!(rows.retrievals_for_task(task_id).is_empty());
}
