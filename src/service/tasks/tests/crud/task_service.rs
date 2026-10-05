use super::*;

// -- TaskService ----------------------------------------------------------

#[tokio::test]
async fn create_and_get_task() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "Test".into(),
            description: "desc".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.title, "Test");
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn create_task_with_tag() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "Bug fix".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: Some(5),
            tag: Some(TaskTag::Bug),
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.tag, Some(TaskTag::Bug));
    assert_eq!(task.sort_order, Some(5));
}

#[tokio::test]
async fn create_task_with_sort_order() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "Sorted".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: Some(42),
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.sort_order, Some(42));
}

#[tokio::test]
async fn update_task_status() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

// Note: the Done restriction (close-only via MarkTaskDoneViaMcp) lives at the
// MCP handler layer. The service itself allows any status transition (the
// TUI needs it).

#[tokio::test]
async fn update_task_no_fields_returns_error() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let err = svc
        .update_task(UpdateTaskParams::for_task(id))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn update_task_params_builder_compiles() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

#[tokio::test]
async fn update_task_invalid_substatus_for_status() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id = svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    // active is not valid for backlog
    let err = svc
        .update_task(UpdateTaskParams::for_task(id).sub_status(SubStatus::Active))
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn update_task_entering_done_stamps_completed_at() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Review))
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(
        task.completed_at.is_some(),
        "expected a completion time on entering Done"
    );
    assert_eq!(
        task.sort_order, None,
        "and the status transition must not touch sort_order"
    );
}

/// Leaving Done writes nothing. `completed_at` records the last completion,
/// not the current status (tasks.allium, ConfirmDone), so it survives the move
/// back out and the next entry overwrites it.
#[tokio::test]
async fn update_task_leaving_done_keeps_completed_at() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    let finished = svc.get_task(id).await.unwrap().completed_at;
    assert!(finished.is_some());

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Review))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.completed_at, finished);
}

/// A task's manual ordering survives a round trip through Done. It used to be
/// cleared on the way out, because `sort_order` carried the completion rank as
/// well; it carries only manual and feed ordering now.
#[tokio::test]
async fn a_round_trip_through_done_preserves_sort_order() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).sort_order(7))
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    assert_eq!(svc.get_task(id).await.unwrap().sort_order, Some(7));

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Backlog))
        .await
        .unwrap();
    assert_eq!(svc.get_task(id).await.unwrap().sort_order, Some(7));
}

/// Reproduces the `exec_persist_task` shape: a caller sends both a status
/// change into Done AND a stale `completed_at` from the in-memory snapshot.
/// The service-derived stamp must win over the caller's value.
#[tokio::test]
async fn update_task_entering_done_overrides_a_stale_caller_completed_at() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let stale = chrono::DateTime::from_timestamp(1_600_000_000, 0).unwrap();
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Done)
            .completed_at(Some(stale)),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_ne!(
        task.completed_at,
        Some(stale),
        "the Done-transition stamp must win over a caller-supplied stale value"
    );
    assert!(task.completed_at.is_some());
}

#[tokio::test]
async fn update_task_edit_while_done_leaves_completed_at_untouched() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    let after_entry = svc.get_task(id).await.unwrap().completed_at;

    // An unrelated field edit while already Done (no status change at all).
    svc.update_task(UpdateTaskParams::for_task(id).title("Renamed".to_string()))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.completed_at, after_entry);
}

#[tokio::test]
async fn update_task_non_done_status_change_preserves_sort_order() {
    // The complement of the two leaving-Done tests above: when neither the
    // prior nor the new status is Done, completed_at_for_status_transition
    // returns None and an explicitly-set sort_order must survive the status
    // change untouched. Distinct from
    // update_task_edit_while_done_leaves_completed_at_untouched,
    // which covers a no-status-change edit on an already-Done task.
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).sort_order(7))
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.sort_order, Some(7));
}

#[tokio::test]
async fn update_task_done_to_backlog_is_unaffected_by_done_rule() {
    // With `archived` gone, Done is the last status in the enum and the only
    // finished one, so it is where MoveTaskBackward (prev_status) and the task
    // editor's freeform STATUS field both start a move back out. That write
    // routes through this same update_task, and the entering-done stamp
    // (tasks.allium: ConfirmDone, EditTask) must leave it alone: leaving done
    // clears nothing, so completed_at is KEPT and sort_order survives, and the
    // move must not error.
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).sort_order(4))
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    let finished = svc.get_task(id).await.unwrap().completed_at;
    assert!(finished.is_some(), "entering done stamps completed_at");

    // prev_status(done) is review: the one-step backward move.
    assert_eq!(TaskStatus::Done.prev(), TaskStatus::Review);
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done.prev()))
        .await
        .unwrap();
    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.completed_at, finished);

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    let refinished = svc.get_task(id).await.unwrap().completed_at;

    // The editor's freeform jump from done straight to backlog.
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Backlog))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Backlog));
    assert_eq!(
        task.completed_at, refinished,
        "leaving done keeps the last completion time"
    );
    assert_eq!(task.sort_order, Some(4), "and never touches sort_order");
}

#[tokio::test]
async fn list_tasks_with_filter() {
    let db = test_db().await;
    let svc = task_svc(&db);

    svc.create_task(CreateTaskParams {
        title: "T1".into(),
        description: "".into(),
        repo_path: "/repo".to_string(),
        plan_path: None,
        epic_id: None,
        sort_order: None,
        tag: None,
        base_branch: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    })
    .await
    .unwrap();

    let tasks = svc
        .list_tasks(ListTasksFilter {
            statuses: Some(vec![TaskStatus::Backlog]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let tasks = svc
        .list_tasks(ListTasksFilter {
            statuses: Some(vec![TaskStatus::Running]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn get_task_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc.get_task(TaskId(999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn update_task_with_epic_linkage() {
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "Epic".into(),
            description: "".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
        })
        .await
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    task_svc
        .update_task(UpdateTaskParams::for_task(id).epic_id(epic.id))
        .await
        .unwrap();

    let task = task_svc.get_task(id).await.unwrap();
    assert_eq!(task.epic_id, Some(epic.id));
}

#[tokio::test]
async fn update_task_status_recalculates_parent_epic() {
    // recalculate_epic_for_task: epic with running task stays in backlog
    // (running and review tasks do not auto-advance epic status)
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams {
            title: "E".into(),
            description: "".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
        })
        .await
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    task_svc
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    let refreshed = epic_svc.get_epic(epic.id).await.unwrap();
    assert_eq!(refreshed.status, TaskStatus::Backlog); // running task → epic stays backlog
}

#[tokio::test]
async fn update_task_relink_recalculates_old_and_new_epic() {
    // Linkage-change branch of recalculate_epic_for_task: moving a Running
    // task between two epics. Both epics stay in Backlog because running
    // tasks do not auto-advance epic status.
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic_a = epic_svc
        .create_epic(CreateEpicParams {
            title: "A".into(),
            description: "".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
        })
        .await
        .unwrap();
    let epic_b = epic_svc
        .create_epic(CreateEpicParams {
            title: "B".into(),
            description: "".into(),
            sort_order: None,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
        })
        .await
        .unwrap();

    let id = task_svc
        .create_task(CreateTaskParams {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: Some(epic_a.id),
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    task_svc
        .update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    // Sanity: epic A stays in Backlog (running task doesn't auto-advance).
    assert_eq!(
        epic_svc.get_epic(epic_a.id).await.unwrap().status,
        TaskStatus::Backlog
    );

    task_svc
        .update_task(UpdateTaskParams::for_task(id).epic_id(epic_b.id))
        .await
        .unwrap();

    // After relinking, both epics stay in Backlog (running task doesn't auto-advance)
    assert_eq!(
        epic_svc.get_epic(epic_a.id).await.unwrap().status,
        TaskStatus::Backlog
    );
    assert_eq!(
        epic_svc.get_epic(epic_b.id).await.unwrap().status,
        TaskStatus::Backlog
    );
}
