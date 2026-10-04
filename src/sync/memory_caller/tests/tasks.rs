use super::*;

// -- Tasks ----------------------------------------------------------------

#[tokio::test]
async fn create_task_generates_sequential_ids_starting_at_one() {
    let (caller, rows) = caller();
    let id1 = caller.create_task(blank_task()).await.unwrap();
    let id2 = caller.create_task(blank_task()).await.unwrap();
    assert_eq!(id1, TaskId(1));
    assert_eq!(id2, TaskId(2));
    assert_eq!(rows.tasks().len(), 2);
}

#[tokio::test]
async fn create_task_pushes_the_row_into_shared_rows() {
    let (caller, rows) = caller();
    let id = caller
        .create_task(bindings::Task {
            title: "hello".into(),
            ..blank_task()
        })
        .await
        .unwrap();
    let task = rows.task(id).expect("task landed in SharedRows");
    assert_eq!(task.title, "hello");
}

#[tokio::test]
async fn create_task_refuses_an_epicless_task_with_no_owner() {
    let (caller, _rows) = caller();
    let result = caller
        .create_task(bindings::Task {
            owner: String::new(),
            epic_id: 0,
            ..blank_task()
        })
        .await;
    assert!(result.is_err(), "OwnerTracksUserBoardTask must refuse this");
}

#[tokio::test]
async fn patch_task_on_a_missing_id_is_a_silent_no_op() {
    let (caller, _rows) = caller();
    let outcome = caller
        .patch_task(TaskId(999), blank_task_patch())
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Applied(_)));
}

#[tokio::test]
async fn patch_task_stamps_completed_at_on_transition_into_done() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let task = rows.task(id).unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Done);
    assert!(task.completed_at.is_some());
}

#[tokio::test]
async fn patch_task_regressing_out_of_done_keeps_the_old_completed_at() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let completed_at = rows.task(id).unwrap().completed_at;
    assert!(completed_at.is_some());

    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                status: Some("backlog".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let task = rows.task(id).unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
    assert_eq!(task.completed_at, completed_at);
}

#[tokio::test]
async fn delete_task_removes_it_from_shared_rows() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    // `delete_task` refuses anything but a `done` task (task #4971).
    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                status: Some("done".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    caller.delete_task(id).await.unwrap();
    assert!(rows.task(id).is_none());
}

#[tokio::test]
async fn delete_task_refuses_a_task_that_is_not_done() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    let outcome = caller.delete_task(id).await.unwrap();
    assert!(!outcome.won());
    assert!(rows.task(id).is_some());
}

#[tokio::test]
async fn claim_backlog_task_then_release_round_trips() {
    let (caller, rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();

    let claimed = caller
        .claim_backlog_task(id, "host-a".into())
        .await
        .unwrap();
    assert!(claimed.won());
    assert_eq!(
        rows.task(id).unwrap().status,
        crate::models::TaskStatus::Running
    );

    // A second claim by another host loses — the task is no longer
    // backlog.
    let raced = caller
        .claim_backlog_task(id, "host-b".into())
        .await
        .unwrap();
    assert!(!raced.won());

    let released = caller.release_backlog_claim(id).await.unwrap();
    assert!(released.won());
    assert_eq!(
        rows.task(id).unwrap().status,
        crate::models::TaskStatus::Backlog
    );
}

#[tokio::test]
async fn release_backlog_claim_refuses_when_a_worktree_is_attached() {
    let (caller, _rows) = caller();
    let id = caller.create_task(blank_task()).await.unwrap();
    caller
        .claim_backlog_task(id, "host-a".into())
        .await
        .unwrap();
    caller
        .patch_task(
            id,
            bindings::TaskPatch {
                worktree: Some("/tmp/some-worktree".into()),
                ..blank_task_patch()
            },
        )
        .await
        .unwrap();
    let released = caller.release_backlog_claim(id).await.unwrap();
    assert!(!released.won());
}
