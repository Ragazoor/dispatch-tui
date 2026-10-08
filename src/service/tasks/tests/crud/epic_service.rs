use super::*;

// -- move_task_to_epic ----------------------------------------------------

#[tokio::test]
async fn move_task_to_epic_links_standalone_task() {
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = make_epic(&epic_svc, "E").await;
    let id = make_task(&task_svc, None).await;

    task_svc.move_task_to_epic(id, Some(epic.id)).await.unwrap();

    assert_eq!(task_svc.get_task(id).await.unwrap().epic_id, Some(epic.id));
}

#[tokio::test]
async fn move_task_to_epic_detaches_and_recalculates_old_epic() {
    // Epic A holds a Done task plus a Backlog task → A stays Backlog (not all
    // active children done). Detaching the Backlog task leaves only the Done
    // task, so A recalculates to Done.
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic_a = make_epic(&epic_svc, "A").await;
    let done_task = make_task(&task_svc, Some(epic_a.id)).await;
    let backlog_task = make_task(&task_svc, Some(epic_a.id)).await;

    task_svc
        .update_task(UpdateTaskParams::for_task(done_task).status(TaskStatus::Done))
        .await
        .unwrap();
    assert_eq!(
        epic_svc.get_epic(epic_a.id).await.unwrap().status,
        TaskStatus::Backlog,
        "epic with a non-done active child stays Backlog"
    );

    // Detach the Backlog task → A's only active child is now Done → A is Done.
    task_svc
        .move_task_to_epic(backlog_task, None)
        .await
        .unwrap();

    assert_eq!(task_svc.get_task(backlog_task).await.unwrap().epic_id, None);
    assert_eq!(
        epic_svc.get_epic(epic_a.id).await.unwrap().status,
        TaskStatus::Done,
        "old epic recalculates to Done after the non-done child leaves"
    );
}

#[tokio::test]
async fn move_task_to_epic_between_epics_recalculates_new_epic() {
    // Epic B holds a single Done task → B is Done. Moving a Backlog task into B
    // regresses B back to Backlog (it now has a non-done active child).
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic_a = make_epic(&epic_svc, "A").await;
    let epic_b = make_epic(&epic_svc, "B").await;

    let b_task = make_task(&task_svc, Some(epic_b.id)).await;
    task_svc
        .update_task(UpdateTaskParams::for_task(b_task).status(TaskStatus::Done))
        .await
        .unwrap();
    assert_eq!(
        epic_svc.get_epic(epic_b.id).await.unwrap().status,
        TaskStatus::Done,
        "epic with all active children done is Done"
    );

    let a_task = make_task(&task_svc, Some(epic_a.id)).await;
    task_svc
        .move_task_to_epic(a_task, Some(epic_b.id))
        .await
        .unwrap();

    assert_eq!(
        task_svc.get_task(a_task).await.unwrap().epic_id,
        Some(epic_b.id)
    );
    assert_eq!(
        epic_svc.get_epic(epic_b.id).await.unwrap().status,
        TaskStatus::Backlog,
        "new epic regresses to Backlog after a non-done task joins"
    );
}

#[tokio::test]
async fn move_task_to_epic_unknown_epic_errors() {
    let db = test_db().await;
    let task_svc = task_svc(&db);

    let id = make_task(&task_svc, None).await;

    let result = task_svc.move_task_to_epic(id, Some(EpicId(9999))).await;

    assert!(
        matches!(result, Err(ServiceError::NotFound(_))),
        "moving to a non-existent epic should be NotFound, got: {result:?}"
    );
    // The task is left untouched.
    assert_eq!(task_svc.get_task(id).await.unwrap().epic_id, None);
}

#[tokio::test]
async fn move_task_to_epic_unknown_task_errors() {
    let db = test_db().await;
    let task_svc = task_svc(&db);

    let result = task_svc.move_task_to_epic(TaskId(9999), None).await;

    assert!(
        result.is_err(),
        "moving a non-existent task should error, got: {result:?}"
    );
}

// -- EpicService ----------------------------------------------------------

#[tokio::test]
async fn create_and_get_epic() {
    let db = test_db().await;
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams {
            description: "desc".into(),
            ..CreateEpicParams::fixture("Epic 1")
        })
        .await
        .unwrap();

    let fetched = svc.get_epic(epic.id).await.unwrap();
    assert_eq!(fetched.title, "Epic 1");
}

#[tokio::test]
async fn get_epic_not_found() {
    let db = test_db().await;
    let svc = epic_svc(&db);
    let err = svc.get_epic(EpicId(999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn update_epic_status() {
    let db = test_db().await;
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    svc.update_epic(UpdateEpicParams {
        status: Some(TaskStatus::Running),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    let updated = svc.get_epic(epic.id).await.unwrap();
    assert_eq!(updated.status, TaskStatus::Running);
}

#[tokio::test]
async fn update_epic_no_fields_returns_error() {
    let db = test_db().await;
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    let err = svc
        .update_epic(UpdateEpicParams {
            ..UpdateEpicParams::for_epic(epic.id)
        })
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)));
}

#[tokio::test]
async fn update_epic_auto_dispatch_persists() {
    let db = test_db().await;
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    // Default is false.
    assert!(!db.get_epic(epic.id).await.unwrap().unwrap().auto_dispatch);

    svc.update_epic(UpdateEpicParams {
        auto_dispatch: Some(true),
        ..UpdateEpicParams::for_epic(epic.id)
    })
    .await
    .unwrap();

    assert!(db.get_epic(epic.id).await.unwrap().unwrap().auto_dispatch);
}

#[tokio::test]
async fn list_epics_with_progress() {
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    task_svc
        .create_task(CreateTaskParams {
            epic_id: Some(epic.id),
            ..CreateTaskParams::fixture("Sub1", "/repo")
        })
        .await
        .unwrap();

    let list = epic_svc.list_epics_with_progress().await.unwrap();
    assert_eq!(list.len(), 1);
    let (_, done, total) = &list[0];
    assert_eq!(*done, 0);
    assert_eq!(*total, 1);
}

#[tokio::test]
async fn list_epics_with_progress_multiple_epics() {
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let e1 = epic_svc
        .create_epic(CreateEpicParams::fixture("E1"))
        .await
        .unwrap();
    let e2 = epic_svc
        .create_epic(CreateEpicParams::fixture("E2"))
        .await
        .unwrap();

    // 2 tasks in E1
    let t1 = task_svc
        .create_task(CreateTaskParams {
            epic_id: Some(e1.id),
            ..CreateTaskParams::fixture("T1", "/repo")
        })
        .await
        .unwrap();
    task_svc
        .create_task(CreateTaskParams {
            epic_id: Some(e1.id),
            ..CreateTaskParams::fixture("T2", "/repo")
        })
        .await
        .unwrap();
    // 1 task in E2
    task_svc
        .create_task(CreateTaskParams {
            epic_id: Some(e2.id),
            ..CreateTaskParams::fixture("T3", "/repo")
        })
        .await
        .unwrap();

    // Mark T1 as done
    task_svc
        .update_task(UpdateTaskParams::for_task(t1).status(TaskStatus::Done))
        .await
        .unwrap();

    let list = epic_svc.list_epics_with_progress().await.unwrap();
    assert_eq!(list.len(), 2);
    let e1_progress = list.iter().find(|(e, _, _)| e.id == e1.id).unwrap();
    assert_eq!(e1_progress.1, 1); // 1 done
    assert_eq!(e1_progress.2, 2); // 2 total
    let e2_progress = list.iter().find(|(e, _, _)| e.id == e2.id).unwrap();
    assert_eq!(e2_progress.1, 0);
    assert_eq!(e2_progress.2, 1);
}

#[tokio::test]
async fn update_task_status_recalculates_epic() {
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    let task_id = task_svc
        .create_task(CreateTaskParams {
            epic_id: Some(epic.id),
            ..CreateTaskParams::fixture("Sub", "/repo")
        })
        .await
        .unwrap();

    task_svc
        .update_task(UpdateTaskParams::for_task(task_id).status(TaskStatus::Done))
        .await
        .unwrap();

    let updated_epic = epic_svc.get_epic(epic.id).await.unwrap();
    assert_eq!(updated_epic.status, TaskStatus::Done);
}

#[tokio::test]
async fn get_epic_with_subtasks() {
    let db = test_db().await;
    let task_svc = task_svc(&db);
    let epic_svc = epic_svc(&db);

    let epic = epic_svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    task_svc
        .create_task(CreateTaskParams {
            epic_id: Some(epic.id),
            ..CreateTaskParams::fixture("Sub", "/repo")
        })
        .await
        .unwrap();

    let (e, subtasks) = epic_svc.get_epic_with_subtasks(epic.id).await.unwrap();
    assert_eq!(e.title, "E");
    assert_eq!(subtasks.len(), 1);
}
