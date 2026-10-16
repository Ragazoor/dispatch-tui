use super::*;

// -------------------------------------------------------------------------
// Epic-in-epic service tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn create_sub_epic_links_parent() {
    let db = test_db().await;
    let svc = epic_svc(&db);

    let parent = svc
        .create_epic(CreateEpicParams::fixture("Parent"))
        .await
        .unwrap();

    let child = svc
        .create_epic(CreateEpicParams {
            parent_epic_id: Some(parent.id),
            ..CreateEpicParams::fixture("Child")
        })
        .await
        .unwrap();

    assert_eq!(child.parent_epic_id, Some(parent.id));

    let fetched = svc.get_epic(child.id).await.unwrap();
    assert_eq!(fetched.parent_epic_id, Some(parent.id));
}

// -- TOCTOU regression -----------------------------------------------------
//
// `validate_sub_status` in `crud.rs` reads the current task status before
// writing the patch. A second writer can land between the read and the
// write. Per the docs/conventions.md "Sub-status validation TOCTOU" note,
// this is accepted: simultaneous status changes from two agents on the
// same task are user error, and the result is last-write-wins. These
// tests pin that behaviour so the policy can't drift silently.

#[tokio::test]
async fn update_task_toctou_last_write_wins() {
    let db = test_db().await;
    let svc_a = task_svc(&db);
    let svc_b = task_svc(&db);

    let id = svc_a
        .create_task(CreateTaskParams::fixture("T", "/repo"))
        .await
        .unwrap();

    // svc_a moves the task to Running/Active.
    svc_a
        .update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Running)
                .sub_status(SubStatus::Active),
        )
        .await
        .unwrap();

    // svc_b moves it on to Review/AwaitingReview. The sub_status is valid
    // for the requested status, so validation passes despite the write
    // landing on top of svc_a's state. Last write wins.
    svc_b
        .update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Review)
                .sub_status(SubStatus::AwaitingReview),
        )
        .await
        .unwrap();

    let task = svc_a.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
}

#[tokio::test]
async fn update_task_sub_status_validated_against_persisted_status() {
    // A sub-status update without a status change is validated against the
    // currently-persisted status. If a previous writer changed status, the
    // later sub_status-only update sees the new status — this is the
    // TOCTOU-accepting behaviour: validation uses *current* state, not the
    // state the caller may have observed earlier.
    let db = test_db().await;
    let svc_a = task_svc(&db);
    let svc_b = task_svc(&db);

    let id = svc_a
        .create_task(CreateTaskParams::fixture("T", "/repo"))
        .await
        .unwrap();

    svc_a
        .update_task(
            UpdateTaskParams::for_task(id)
                .status(TaskStatus::Running)
                .sub_status(SubStatus::Active),
        )
        .await
        .unwrap();

    // svc_b sees Running (sub_status Stale is valid for Running).
    svc_b
        .update_task(UpdateTaskParams::for_task(id).sub_status(SubStatus::Stale))
        .await
        .unwrap();
    assert_eq!(
        svc_a.get_task(id).await.unwrap().sub_status,
        SubStatus::Stale
    );

    // Now svc_a moves status to Review without specifying sub_status.
    svc_a
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Review))
        .await
        .unwrap();

    // svc_b attempts a sub_status-only update with `Active`, which is
    // valid for Running but NOT for Review. Validation reads the current
    // status (Review) and rejects the update — no panic, just a
    // Validation error.
    let err = svc_b
        .update_task(UpdateTaskParams::for_task(id).sub_status(SubStatus::Active))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn update_repo_path_reroutes_within_grouped_epic() {
    use crate::store::EpicCrud;
    let db = std::sync::Arc::new(crate::store::Store::open_in_memory().unwrap());
    let svc =
        crate::service::TaskService::new(db.clone(), crate::process::MockProcessRunner::unused());
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    let task = svc
        .create_task_returning(crate::service::CreateTaskParams {
            epic_id: Some(root.id),
            ..crate::service::CreateTaskParams::fixture("t", "/x/alpha")
        })
        .await
        .unwrap();

    svc.update_task(
        crate::service::UpdateTaskParams::for_task(task.id).repo_path("/x/beta".into()),
    )
    .await
    .unwrap();

    let reloaded = db.get_task(task.id).await.unwrap();
    let placed = db.get_epic(reloaded.epic_id.unwrap()).await.unwrap();
    assert_eq!(placed.title, "beta");
}

#[tokio::test]
async fn move_task_to_grouped_epic_routes_into_sub_epic() {
    // TDD — blocker fix: move_task_to_epic must route through route_target.
    // Create a group_by_repo non-feed root, a standalone task, call
    // move_task_to_epic(task, Some(root)) and assert the task lands in a
    // per-repo RepoGroup sub-epic, NOT directly on the root.
    use crate::store::EpicCrud;
    let db = std::sync::Arc::new(crate::store::Store::open_in_memory().unwrap());
    let svc =
        crate::service::TaskService::new(db.clone(), crate::process::MockProcessRunner::unused());

    // Create a grouped (non-feed) root.
    let root = db.create_epic("root", "", None).await.unwrap();
    db.patch_epic(root.id, &crate::store::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();

    // Create a standalone task with a known repo path.
    let task_id = svc
        .create_task(CreateTaskParams::fixture("T", "/x/dispatch"))
        .await
        .unwrap();

    // Move the task onto the grouped root.
    svc.move_task_to_epic(task_id, Some(root.id)).await.unwrap();

    // The task must land in a sub-epic, not directly on the root.
    let task = svc.get_task(task_id).await.unwrap();
    assert_ne!(
        task.epic_id,
        Some(root.id),
        "task must NOT be placed directly on the grouped root"
    );
    let placed_id = task.epic_id.expect("task must have an epic");
    let placed = db.get_epic(placed_id).await.unwrap();
    assert_eq!(
        placed.origin,
        crate::models::EpicOrigin::RepoGroup,
        "placed epic must be a RepoGroup sub-epic"
    );
    assert_eq!(placed.title, "dispatch", "sub-epic title = repo basename");
}

#[tokio::test]
async fn move_task_to_non_grouped_epic_lands_directly() {
    // Regression guard: moving to a plain (non-grouped) epic must NOT route.
    use crate::store::EpicCrud;
    let db = std::sync::Arc::new(crate::store::Store::open_in_memory().unwrap());
    let svc =
        crate::service::TaskService::new(db.clone(), crate::process::MockProcessRunner::unused());

    let plain = db.create_epic("plain", "", None).await.unwrap();

    let task_id = svc
        .create_task(CreateTaskParams::fixture("T", "/x/dispatch"))
        .await
        .unwrap();

    svc.move_task_to_epic(task_id, Some(plain.id))
        .await
        .unwrap();

    let task = svc.get_task(task_id).await.unwrap();
    assert_eq!(
        task.epic_id,
        Some(plain.id),
        "plain epic: task must land directly on it"
    );
}

// -- DefaultBaseBranchIsDetectedNotAssumed --------------------------------
//
// docs/specs/dispatch.allium. An omitted base_branch is resolved from the
// repository, never taken as the literal config.default_branch. Assuming
// "main" produced tasks in "master" repos that could not be dispatched at
// all: `git worktree add` had no such ref to cut a worktree from.

#[tokio::test]
async fn create_task_without_base_branch_detects_the_repo_default() {
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![
        crate::process::MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/master\n"),
    ]));
    let svc = task_svc_with_runner(&db, runner);

    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    assert_eq!(
        svc.get_task(id).await.unwrap().base_branch,
        "master",
        "an omitted base_branch must come from the repo, not from the literal \"main\""
    );
}

#[tokio::test]
async fn create_task_with_an_explicit_base_branch_does_not_probe_the_repo() {
    // The invariant governs only the ABSENT case. A named branch is honoured
    // verbatim, including one the repo does not have yet — and asking git
    // about it would be both wasted and misleading.
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![]));
    let svc = task_svc_with_runner(&db, runner.clone());

    let id = svc
        .create_task(make_task_params_on_branch("/repo", "develop"))
        .await
        .unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().base_branch, "develop");
    assert!(
        runner.recorded_calls().is_empty(),
        "a named base branch settles the question; nothing should be run: {:?}",
        runner.recorded_calls()
    );
}

#[tokio::test]
async fn create_task_falls_back_to_main_when_the_repo_names_no_default() {
    // config.default_branch is the detection helper's own last resort, not a
    // first answer. create_task stays total: it never refuses over a branch
    // the caller did not ask about.
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![
        crate::process::MockProcessRunner::fail(
            "fatal: ref refs/remotes/origin/HEAD is not a symbolic ref",
        ),
    ]));
    let svc = task_svc_with_runner(&db, runner);

    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().base_branch, "main");
}

#[tokio::test]
async fn create_task_with_no_repo_path_does_not_probe_and_defaults_to_main() {
    // There is no repository to ask. An empty repo_path is a task the user has
    // not pointed anywhere yet, not a reason to shell out.
    let db = test_db().await;
    let runner = Arc::new(crate::process::MockProcessRunner::new(vec![]));
    let svc = task_svc_with_runner(&db, runner.clone());

    let id = svc.create_task(make_task_params("")).await.unwrap();

    assert_eq!(svc.get_task(id).await.unwrap().base_branch, "main");
    assert!(
        runner.recorded_calls().is_empty(),
        "nothing to ask when there is no repo: {:?}",
        runner.recorded_calls()
    );
}
