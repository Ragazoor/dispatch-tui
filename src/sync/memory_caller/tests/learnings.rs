use super::*;

// -- Learnings, retrievals and verdicts (task #5003) -----------------------

pub(super) fn blank_learning() -> bindings::Learning {
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
        workspace_id: 0,
        author: String::new(),
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

/// `Learning.ScopeRefConsistency` (`docs/specs/core.allium`),
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

/// `RecordLearningViaMcp`, mirroring the module's `create_learning`.
#[tokio::test]
async fn create_learning_refuses_a_blank_repo_ref_and_an_epic_ref_naming_no_epic() {
    let (caller, rows) = caller();
    for (scope, scope_ref) in [
        ("repo", ""),
        ("repo", "  "),
        ("epic", "9999"),
        ("epic", "abc"),
    ] {
        let outcome = caller
            .create_learning(bindings::Learning {
                scope: scope.into(),
                scope_ref: Some(scope_ref.into()),
                ..blank_learning()
            })
            .await;
        assert!(outcome.is_err(), "{scope} {scope_ref:?}");
    }
    assert!(rows.learning(LearningId(1)).is_none(), "nothing stored");
}

/// The startup repair pass changes nothing else: an embedding-only patch
/// leaves `updated_at`, a content patch moves it.
#[tokio::test]
async fn an_embedding_only_patch_leaves_updated_at() {
    let (caller, rows) = caller();
    let id = caller
        .create_learning(bindings::Learning {
            updated_at: "2020-01-01 00:00:00.000".into(),
            ..blank_learning()
        })
        .await
        .unwrap();
    let stamp = rows.learning(id).unwrap().updated_at;
    caller
        .patch_learning(
            id,
            bindings::LearningPatch {
                status: None,
                summary: None,
                embedding: Some(Some(vec![1, 2, 3])),
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.learning(id).unwrap().updated_at, stamp);
    caller
        .patch_learning(
            id,
            bindings::LearningPatch {
                status: None,
                summary: Some("revised".into()),
                embedding: None,
            },
        )
        .await
        .unwrap();
    assert_ne!(rows.learning(id).unwrap().updated_at, stamp);
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

const RATING_TASK: TaskId = TaskId(7);

/// Record that `RATING_TASK` was shown `learning`, the precondition of a verdict.
async fn retrieve(caller: &MemoryReducerCaller, learning: LearningId) {
    caller
        .record_learning_retrieval(RATING_TASK, learning, RetrievalSource::PromptInjection)
        .await
        .unwrap();
}

#[tokio::test]
async fn apply_learning_verdicts_applies_helped_and_wrong() {
    let (caller, rows) = caller();
    let helped = caller.create_learning(blank_learning()).await.unwrap();
    let wrong = caller.create_learning(blank_learning()).await.unwrap();
    retrieve(&caller, helped).await;
    retrieve(&caller, wrong).await;
    caller
        .apply_learning_verdicts(
            RATING_TASK,
            vec![
                (helped, LearningVerdict::Helped),
                (wrong, LearningVerdict::Wrong),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rows.learning(helped).unwrap().upvote_count, 1);
    assert_eq!(rows.learning(wrong).unwrap().upvote_count, -1);
}

/// One task rating a learning twice moves the score once: the second
/// verdict replaces the first, and an identical repeat changes nothing.
#[tokio::test]
async fn apply_learning_verdicts_counts_a_repeat_from_one_task_once() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    retrieve(&caller, id).await;
    for _ in 0..2 {
        caller
            .apply_learning_verdicts(RATING_TASK, vec![(id, LearningVerdict::Helped)])
            .await
            .unwrap();
    }
    assert_eq!(rows.learning(id).unwrap().upvote_count, 1);
}

/// A changed verdict from the same task moves the score by the difference
/// (helped then wrong is -2 from the helped state), so the latest wins.
#[tokio::test]
async fn apply_learning_verdicts_latest_verdict_from_a_task_wins() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    retrieve(&caller, id).await;
    caller
        .apply_learning_verdicts(RATING_TASK, vec![(id, LearningVerdict::Helped)])
        .await
        .unwrap();
    caller
        .apply_learning_verdicts(RATING_TASK, vec![(id, LearningVerdict::Wrong)])
        .await
        .unwrap();
    let stored = rows.learning(id).unwrap();
    assert_eq!(stored.upvote_count, -1);
    assert!(
        stored.last_upvoted_at.is_some(),
        "replacing a helped verdict does not roll last_upvoted_at back"
    );
}

/// Two tasks each get their own verdict on one learning.
#[tokio::test]
async fn apply_learning_verdicts_counts_each_task_once() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    for task in [TaskId(7), TaskId(8)] {
        caller
            .record_learning_retrieval(task, id, RetrievalSource::PromptInjection)
            .await
            .unwrap();
        caller
            .apply_learning_verdicts(task, vec![(id, LearningVerdict::Helped)])
            .await
            .unwrap();
    }
    assert_eq!(rows.learning(id).unwrap().upvote_count, 2);
}

/// Two entries naming the SAME learning in one batch apply in order against
/// each other: helped then wrong from one task ends at -1, not 0.
#[tokio::test]
async fn apply_learning_verdicts_duplicate_entries_in_one_batch_latest_wins() {
    let (caller, rows) = caller();
    let id = caller.create_learning(blank_learning()).await.unwrap();
    retrieve(&caller, id).await;
    caller
        .apply_learning_verdicts(
            RATING_TASK,
            vec![(id, LearningVerdict::Helped), (id, LearningVerdict::Wrong)],
        )
        .await
        .unwrap();
    assert_eq!(rows.learning(id).unwrap().upvote_count, -1);
}

#[tokio::test]
async fn apply_learning_verdicts_refuses_a_learning_never_retrieved_for_the_task() {
    let (caller, rows) = caller();
    let retrieved = caller.create_learning(blank_learning()).await.unwrap();
    let stranger = caller.create_learning(blank_learning()).await.unwrap();
    retrieve(&caller, retrieved).await;
    let outcome = caller
        .apply_learning_verdicts(
            RATING_TASK,
            vec![
                (retrieved, LearningVerdict::Helped),
                (stranger, LearningVerdict::Helped),
            ],
        )
        .await
        .unwrap();
    assert!(!outcome.won(), "the batch is refused");
    assert_eq!(
        rows.learning(retrieved).unwrap().upvote_count,
        0,
        "a refused batch changes nothing"
    );
}

#[tokio::test]
async fn apply_learning_verdicts_refuses_a_missing_learning_as_not_found() {
    let (caller, _rows) = caller();
    retrieve(&caller, LearningId(999)).await;
    let outcome = caller
        .apply_learning_verdicts(
            RATING_TASK,
            vec![(LearningId(999), LearningVerdict::Helped)],
        )
        .await
        .unwrap();
    assert!(
        matches!(&outcome, ReducerOutcome::Refused(why) if why.contains("learning 999 not found")),
        "{outcome:?}"
    );
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

/// `delete_task` archives the task and keeps its learning links
/// (`tasks.allium: ArchiveDeletedTask`): a learning it sourced still names
/// it, and its retrievals stay in the store but leave the board's rows, since
/// a board receives retrievals only for tasks it holds.
#[tokio::test]
async fn delete_task_keeps_source_task_id_and_hides_retrievals_from_the_board() {
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

    assert_eq!(
        rows.learning(sourced).unwrap().source_task_id,
        Some(task_id)
    );
    assert!(rows.learning(unrelated).is_some());
    assert!(rows.retrievals_for_task(task_id).is_empty());
    assert!(caller.read_archived_task(task_id).await.unwrap().is_some());
}
