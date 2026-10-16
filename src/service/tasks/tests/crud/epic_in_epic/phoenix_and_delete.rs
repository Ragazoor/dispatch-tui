use super::*;

// -- PhoenixRespawn --------------------------------------------------------
//
// A phoenix task recreates itself on completion, and the flag MOVES to the
// copy. See the `== Phoenix ==` section and `PhoenixRespawn` in
// docs/specs/tasks.allium — each test below names the clause it pins.

/// A backlog phoenix task with every inheritable field set to something
/// distinguishable, so `WhatTheSuccessorInherits` can be asserted field by
/// field rather than on a couple of representatives.
async fn phoenix_task(db: &Arc<dyn store::TaskStore>, epic_id: Option<EpicId>) -> TaskId {
    let svc = task_svc(db);
    let id = svc
        .create_task(CreateTaskParams {
            description: "check every direct dependency".into(),
            plan_path: Some("/repo/docs/plans/audit.md".to_string()),
            epic_id,
            tag: Some(TaskTag::Chore),
            base_branch: Some("develop".to_string()),
            wrap_up_mode: Some(crate::models::WrapUpMode::Done),
            auto_run_plan: true,
            phoenix: true,
            ..CreateTaskParams::fixture("Weekly dep audit", "/repo")
        })
        .await
        .unwrap();
    id
}

/// The successor of `predecessor`, or `None` when nothing was respawned.
async fn successor_of(
    db: &Arc<dyn store::TaskStore>,
    predecessor: TaskId,
) -> Option<crate::models::Task> {
    task_svc(db)
        .list_tasks(ListTasksFilter::default())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id != predecessor && t.status == TaskStatus::Backlog)
}

#[tokio::test]
async fn phoenix_task_entering_done_spawns_a_backlog_successor() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    let successor = successor_of(&db, id)
        .await
        .expect("entering Done must create a fresh backlog copy");
    assert_eq!(successor.status, TaskStatus::Backlog);
    assert!(successor.phoenix, "the flag moves to the successor");
    assert_eq!(
        successor.sub_status,
        SubStatus::default_for(TaskStatus::Backlog)
    );
}

/// `TheFlagIsTheReceipt`: the predecessor's flag is cleared exactly when the
/// successor lands, so the flag's own absence records that it already fired.
#[tokio::test]
async fn a_successful_respawn_clears_the_predecessors_flag() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    let predecessor = svc.get_task(id).await.unwrap();
    assert_eq!(predecessor.status, TaskStatus::Done);
    assert!(
        !predecessor.phoenix,
        "a task that respawned is no longer the live phoenix"
    );
    assert!(
        !predecessor.respawn_failed(),
        "a cleared flag is what makes respawn_failed false in Done"
    );
}

/// `WhatTheSuccessorInherits`: everything the operator configured, and nothing
/// the finished run produced.
#[tokio::test]
async fn the_successor_inherits_the_operators_settings_and_none_of_the_run() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let epic = epic_svc(&db)
        .create_epic(CreateEpicParams::fixture("recurring work"))
        .await
        .unwrap();
    let id = phoenix_task(&db, Some(epic.id)).await;
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/wt".to_string()))
            .tmux_window(crate::service::TmuxWindowUpdate::Set(test_tmux_window(
                "task-x",
            ))),
    )
    .await
    .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    let s = successor_of(&db, id).await.unwrap();
    assert_eq!(s.title, "Weekly dep audit", "title is carried verbatim");
    assert_eq!(s.description, "check every direct dependency");
    assert_eq!(s.repo_path, "/repo");
    assert_eq!(s.tag, Some(TaskTag::Chore));
    assert_eq!(s.base_branch, "develop");
    assert_eq!(s.wrap_up_mode, Some(crate::models::WrapUpMode::Done));
    assert_eq!(s.plan_path.as_deref(), Some("/repo/docs/plans/audit.md"));
    assert!(s.auto_run_plan);
    assert_eq!(s.epic_id, Some(epic.id), "EpicMembershipIsInherited");

    assert!(
        s.worktree.is_none(),
        "the worktree belongs to the finished run"
    );
    assert!(s.tmux_window.is_none());
    assert!(s.url.is_none());
    assert!(s.external_id.is_none());
    assert!(
        s.sort_order.is_none(),
        "the copy sorts by its own id, at the bottom of backlog"
    );
}

/// Labels land as part of the successor's own creation, not a follow-up patch
/// that could fail independently and silently drop them.
#[tokio::test]
async fn the_successor_inherits_labels() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;
    let labels = vec!["scala-common".to_string(), "security".to_string()];
    db.patch_task(id, &store::TaskPatch::new().labels(&labels))
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    let s = successor_of(&db, id).await.unwrap();
    assert_eq!(s.labels, labels);
}

#[tokio::test]
async fn an_ordinary_task_entering_done_spawns_nothing() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    assert!(successor_of(&db, id).await.is_none());
}

/// `TheFlagIsTheReceipt`, second property: a Done -> Review -> Done round-trip
/// cannot duplicate, because the first pass cleared the flag.
#[tokio::test]
async fn re_entering_done_does_not_spawn_a_second_successor() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;

    for status in [TaskStatus::Done, TaskStatus::Review, TaskStatus::Done] {
        svc.update_task(UpdateTaskParams::for_task(id).status(status))
            .await
            .unwrap();
    }

    let backlog = svc
        .list_tasks(ListTasksFilter {
            statuses: Some(vec![TaskStatus::Backlog]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(backlog.len(), 1, "exactly one successor, got {backlog:?}");
}

/// `transitions_to` fires on an actual change of value only — the same
/// property `DetectTaskDone` relies on.
#[tokio::test]
async fn rewriting_done_over_done_does_not_respawn() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;
    // Land in Done without the flag ever being set, so the respawn this test
    // rules out could only come from the no-op rewrite below.
    svc.update_task(UpdateTaskParams::for_task(id).phoenix(false))
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).phoenix(true))
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    assert!(
        successor_of(&db, id).await.is_none(),
        "Done -> Done is not a transition into Done"
    );
}

/// `FeedTasksAreExempt`: a feed epic recreates and reconciles its own rows, so
/// a phoenix copy would either duplicate one or be deleted as stale.
#[tokio::test]
async fn a_feed_owned_task_does_not_respawn() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;
    db.patch_task(id, &store::TaskPatch::new().external_id(Some("pr-42")))
        .await
        .unwrap();

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    assert!(successor_of(&db, id).await.is_none());
    assert!(
        svc.get_task(id).await.unwrap().phoenix,
        "the flag is ignored, not consumed — nothing respawned to consume it"
    );
}

#[tokio::test]
async fn close_task_respawns_a_phoenix_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;
    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Running))
        .await
        .unwrap();

    svc.close_task(id).await.unwrap();

    let successor = successor_of(&db, id)
        .await
        .expect("the done close lands in Done and respawns, in the module");
    assert!(successor.phoenix);
    assert!(!svc.get_task(id).await.unwrap().phoenix);
}

/// `DoneOutranksTheRespawn`: a close whose respawn is skipped still succeeds
/// and still moves the task, so the completion never waits on the recurrence.
/// Feed ownership is the skip used here because it is the one the spec makes
/// reachable without a fault-injecting store.
#[tokio::test]
async fn a_skipped_respawn_does_not_contaminate_the_close_result() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = phoenix_task(&db, None).await;
    db.patch_task(id, &store::TaskPatch::new().external_id(Some("pr-42")))
        .await
        .unwrap();
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .tmux_window(crate::service::TmuxWindowUpdate::Set(test_tmux_window(
                "task-x",
            ))),
    )
    .await
    .unwrap();

    svc.close_task(id)
        .await
        .expect("the close succeeded; the respawn is a follow-on, not part of it");

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done, "the completion sticks");
    assert!(
        task.respawn_failed(),
        "a phoenix flag surviving in Done is the visible 'did not respawn' state"
    );
}

// -- ensure_deletable: the delete pre-check ------------------------------------

#[tokio::test]
async fn ensure_deletable_passes_a_done_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();
    db.patch_task(id, &store::TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    svc.ensure_deletable(id).await.unwrap();
}

#[tokio::test]
async fn ensure_deletable_refuses_a_task_that_is_not_done_in_the_store() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let err = svc.ensure_deletable(id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn ensure_deletable_reports_a_missing_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc.ensure_deletable(TaskId(999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn epic_ensure_deletable_refuses_when_any_subtree_task_is_not_done() {
    let db = test_db().await;
    let tasks = task_svc(&db);
    let epics = epic_svc(&db);
    let epic = epics
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();
    let t = tasks.create_task(make_task_params("/repo")).await.unwrap();
    db.set_task_epic_id(t, Some(epic.id)).await.unwrap();

    let err = epics.ensure_deletable(epic.id).await.unwrap_err();
    assert!(matches!(err, ServiceError::Validation(_)), "got {err:?}");

    db.patch_task(t, &store::TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    epics.ensure_deletable(epic.id).await.unwrap();
}

#[tokio::test]
async fn epic_ensure_deletable_names_the_lowest_id_blocking_task_across_sub_epics() {
    let db = test_db().await;
    let tasks = task_svc(&db);
    let epics = epic_svc(&db);
    let mk = |parent| CreateEpicParams {
        parent_epic_id: parent,
        ..CreateEpicParams::fixture("E")
    };
    let root = epics.create_epic(mk(None)).await.unwrap();
    let sub = epics.create_epic(mk(Some(root.id))).await.unwrap();
    // The lower id lives in the sub-epic, the higher one directly in the root.
    let deep = tasks
        .create_task(CreateTaskParams::fixture("Deep blocker", "/repo"))
        .await
        .unwrap();
    db.set_task_epic_id(deep, Some(sub.id)).await.unwrap();
    let shallow = tasks
        .create_task(CreateTaskParams::fixture("Shallow blocker", "/repo"))
        .await
        .unwrap();
    db.set_task_epic_id(shallow, Some(root.id)).await.unwrap();

    let err = epics.ensure_deletable(root.id).await.unwrap_err();

    let ServiceError::Validation(msg) = err else {
        panic!("expected Validation, got {err:?}");
    };
    assert!(msg.contains(&format!("#{}", deep.0)), "{msg}");
    assert!(msg.contains("Deep blocker"), "{msg}");
    assert!(!msg.contains("Shallow blocker"), "{msg}");
}
