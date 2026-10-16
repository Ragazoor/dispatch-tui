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

// -- ClosePrOnDone (docs/specs/pr-workflow.allium) -------------------------
//
// Fires on any transition of task.status into Done while the task carries a
// pr-typed url: `gh pr close <url>` runs best-effort, on the host. `update_task`
// does it here; a module-side close (`close_task`, `request_exit_session`)
// leaves a row whose host does it (see `session_requests.rs`).

const PR_URL: &str = "https://github.com/acme/repo/pull/42";

fn pr_task_url() -> crate::models::TaskUrl {
    crate::models::TaskUrl::new(PR_URL, crate::models::UrlType::Pr)
}

#[tokio::test]
async fn update_task_closes_the_pr_on_entering_done() {
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![
        crate::process::MockProcessRunner::ok(),
    ]));
    let svc = task_svc_with_runner(&db, runner.clone());
    let id = svc
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();

    // Attach the PR and move to Review first — not itself a Done transition,
    // so this must not shell out.
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Review)
            .url(crate::service::UrlUpdate::Set(pr_task_url())),
    )
    .await
    .unwrap();
    assert!(
        runner.recorded_calls().is_empty(),
        "moving to Review must not close anything: {:?}",
        runner.recorded_calls()
    );

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    // The close is spawned detached (never awaited by update_task itself —
    // see `TaskService::spawn_close_attached_pr`), so wait for it to actually
    // run before asserting on the mock, deterministically rather than
    // sleeping.
    svc.wait_for_background_pr_closes().await;

    assert_eq!(
        runner.recorded_calls(),
        vec![(
            "gh".to_string(),
            vec!["pr".to_string(), "close".to_string(), PR_URL.to_string()]
        )]
    );
    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Done);
}

#[tokio::test]
async fn no_pr_close_when_task_has_no_url() {
    let db = test_db().await;
    // `unused()` panics on any shell-out, so an accidental close call fails
    // this test loudly rather than passing by accident.
    let svc = task_svc(&db);
    let id = svc
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Review))
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Done);
}

#[tokio::test]
async fn no_pr_close_when_url_is_not_pr_typed() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Review)
            .url(crate::service::UrlUpdate::Set(crate::models::TaskUrl::new(
                "https://github.com/acme/repo/issues/9",
                crate::models::UrlType::Issue,
            ))),
    )
    .await
    .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Done);
}

#[tokio::test]
async fn no_pr_close_on_a_done_to_done_resave() {
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![
        crate::process::MockProcessRunner::ok(),
    ]));
    let svc = task_svc_with_runner(&db, runner.clone());
    let id = svc
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();

    // The genuine transition into Done: consumes the single queued response.
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Done)
            .url(crate::service::UrlUpdate::Set(pr_task_url())),
    )
    .await
    .unwrap();
    svc.wait_for_background_pr_closes().await;
    assert_eq!(runner.recorded_calls().len(), 1);

    // A re-save that sets status to Done again is not a transition
    // (transitions_to semantics) — this must make no further call. If it
    // did, the mock's response queue is empty and it panics.
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    svc.wait_for_background_pr_closes().await;

    assert_eq!(
        runner.recorded_calls().len(),
        1,
        "a done -> done resave must not close the PR again"
    );
}

#[tokio::test]
async fn update_task_persists_done_even_when_the_pr_close_fails() {
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![
        crate::process::MockProcessRunner::fail(
            "GraphQL: Pull request Update failed: Pull request is already merged",
        ),
    ]));
    let svc = task_svc_with_runner(&db, runner.clone());
    let id = svc
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Review)
            .url(crate::service::UrlUpdate::Set(pr_task_url())),
    )
    .await
    .unwrap();

    let result = svc
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await;
    svc.wait_for_background_pr_closes().await;

    assert!(
        result.is_ok(),
        "a failed gh pr close must not fail the Done transition: {result:?}"
    );
    assert_eq!(
        runner.recorded_calls().len(),
        1,
        "the close must actually have been attempted, not skipped"
    );
    assert_eq!(svc.get_task(id).await.unwrap().status, TaskStatus::Done);
}
