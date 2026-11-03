use super::*;

// -- attach_plan ----------------------------------------------------------

#[tokio::test]
async fn attach_plan_sets_plan_path() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();
    assert_eq!(svc.get_task(id).await.unwrap().plan_path, None);

    svc.attach_plan(id, "/abs/plan.md").await.unwrap();

    assert_eq!(
        svc.get_task(id).await.unwrap().plan_path.as_deref(),
        Some("/abs/plan.md")
    );
}

#[tokio::test]
async fn attach_plan_on_missing_task_is_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc
        .attach_plan(TaskId(9999), "/abs/plan.md")
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "got {err:?}");
}

// -- mark_pr_learnings_gate_shown -----------------------------------------

#[tokio::test]
async fn mark_pr_learnings_gate_shown_first_then_idempotent() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    assert!(svc.mark_pr_learnings_gate_shown(id).await.unwrap());
    assert!(!svc.mark_pr_learnings_gate_shown(id).await.unwrap());
}

// -- validate_wrap_up ------------------------------------------------------

#[tokio::test]
async fn validate_wrap_up_running_with_worktree_succeeds() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into())),
    )
    .await
    .unwrap();

    let task = svc.validate_wrap_up(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn validate_wrap_up_review_with_worktree_succeeds() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Review)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into())),
    )
    .await
    .unwrap();

    let task = svc.validate_wrap_up(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
}

#[tokio::test]
async fn validate_wrap_up_backlog_task_fails() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let err = svc.validate_wrap_up(id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

// A review task ends at its PR, not at wrap-up — ReviewTasksAreNotWrappedUp
// in docs/specs/mcp-task-tools.allium. Both review tags are covered because
// the guard is on TaskTag::is_review, not on either literal.
#[tokio::test]
async fn validate_wrap_up_refuses_a_review_tagged_task() {
    for tag in [
        TaskTag::PrReview,
        TaskTag::Dependabot,
        TaskTag::ReleasePlease,
    ] {
        let db = test_db().await;
        let svc = task_svc(&db);
        let id = svc.create_task(make_task_params("/repo")).await.unwrap();

        svc.update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
                .tag(Some(Some(tag))),
        )
        .await
        .unwrap();

        let err = svc.validate_wrap_up(id).await.unwrap_err();
        let ServiceError::Validation(msg) = err else {
            panic!("{tag:?} should fail validation, got a different error");
        };
        assert!(
            msg.contains(tag.as_str()),
            "the refusal must name the tag that caused it, got: {msg}"
        );
        assert!(
            msg.contains("update_task"),
            "the refusal must name the retag escape hatch, got: {msg}"
        );
    }
}

// The guard is on the tag alone, so a review task that never ran is refused
// for the tag rather than for its status — the tag is the actionable half.
#[tokio::test]
async fn validate_wrap_up_refuses_a_review_tag_before_it_checks_status() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).tag(Some(Some(TaskTag::PrReview))))
        .await
        .unwrap();

    let ServiceError::Validation(msg) = svc.validate_wrap_up(id).await.unwrap_err() else {
        panic!("expected a validation error");
    };
    assert!(msg.contains("pr-review"), "got: {msg}");
}

// Retagging is the way out, and it has to actually work — otherwise the
// escape hatch the refusal names is a dead end.
#[tokio::test]
async fn validate_wrap_up_succeeds_once_a_review_task_is_retagged() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
            .tag(Some(Some(TaskTag::PrReview))),
    )
    .await
    .unwrap();
    svc.validate_wrap_up(id).await.unwrap_err();

    svc.update_task(UpdateTaskParams::for_task(id).tag(Some(Some(TaskTag::Chore))))
        .await
        .unwrap();
    svc.validate_wrap_up(id).await.unwrap();
}

// A non-review tag is untouched by the guard.
#[tokio::test]
async fn validate_wrap_up_allows_every_non_review_tag() {
    for tag in [
        TaskTag::Bug,
        TaskTag::Feature,
        TaskTag::Chore,
        TaskTag::Research,
        TaskTag::Fix,
    ] {
        let db = test_db().await;
        let svc = task_svc(&db);
        let id = svc.create_task(make_task_params("/repo")).await.unwrap();

        svc.update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
                .tag(Some(Some(tag))),
        )
        .await
        .unwrap();

        svc.validate_wrap_up(id)
            .await
            .unwrap_or_else(|e| panic!("{tag:?} should wrap up, got {e:?}"));
    }
}

#[tokio::test]
async fn validate_wrap_up_running_without_worktree_fails() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    let err = svc.validate_wrap_up(id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

// -- was_pr_finalisation ---------------------------------------------------

#[tokio::test]
async fn update_task_pr_finalisation_true_when_first_pr_and_review_status() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let result = svc
        .update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Review)
                .url(crate::service::UrlUpdate::Set(crate::models::TaskUrl::new(
                    "https://github.com/org/repo/pull/1",
                    crate::models::UrlType::Pr,
                ))),
        )
        .await
        .unwrap();

    assert!(result.was_pr_finalisation);
}

#[tokio::test]
async fn update_task_pr_finalisation_false_when_pr_already_existed() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(
        UpdateTaskParams::for_task(id).url(crate::service::UrlUpdate::Set(
            crate::models::TaskUrl::new(
                "https://github.com/org/repo/pull/1",
                crate::models::UrlType::Pr,
            ),
        )),
    )
    .await
    .unwrap();

    let result = svc
        .update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Review)
                .url(crate::service::UrlUpdate::Set(crate::models::TaskUrl::new(
                    "https://github.com/org/repo/pull/1",
                    crate::models::UrlType::Pr,
                ))),
        )
        .await
        .unwrap();

    assert!(!result.was_pr_finalisation);
}

#[tokio::test]
async fn update_task_pr_finalisation_false_when_not_moving_to_review() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let result = svc
        .update_task(
            UpdateTaskParams::for_task(id).url(crate::service::UrlUpdate::Set(
                crate::models::TaskUrl::new(
                    "https://github.com/org/repo/pull/1",
                    crate::models::UrlType::Pr,
                ),
            )),
        )
        .await
        .unwrap();

    assert!(!result.was_pr_finalisation);
}

#[tokio::test]
async fn update_task_pr_finalisation_false_with_non_pr_url() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    // A non-PR-typed url moving to Review is not a PR finalisation.
    let result = svc
        .update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Review)
                .url(crate::service::UrlUpdate::Set(crate::models::TaskUrl::new(
                    "https://github.com/org/repo/issues/1",
                    crate::models::UrlType::Issue,
                ))),
        )
        .await
        .unwrap();

    assert!(!result.was_pr_finalisation);
}
