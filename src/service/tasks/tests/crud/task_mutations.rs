use super::*;

// -- create_task_returning ---------------------------------------------------

#[tokio::test]
async fn create_task_returning_gives_full_task() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let task = svc
        .create_task_returning(CreateTaskParams {
            title: "Full task".into(),
            description: "desc".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: Some(TaskTag::Feature),
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    assert_eq!(task.title, "Full task");
    assert_eq!(task.description, "desc");
    assert_eq!(task.tag, Some(TaskTag::Feature));
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn create_task_with_auto_run_plan_true_persists() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let task = svc
        .create_task_returning(CreateTaskParams {
            title: "T".to_string(),
            description: "d".to_string(),
            repo_path: "/r".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: true,
            phoenix: false,
        })
        .await
        .unwrap();
    assert!(task.auto_run_plan);
}

#[tokio::test]
async fn create_task_returning_with_epic() {
    let db = test_db().await;
    let tsvc = task_svc(&db);
    let esvc = epic_svc(&db);

    let epic = esvc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    let task = tsvc
        .create_task_returning(CreateTaskParams {
            title: "Sub".into(),
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

    assert_eq!(task.epic_id, Some(epic.id));
}

#[tokio::test]
async fn create_task_returning_sets_all_optional_fields_atomically() {
    let db = test_db().await;
    let tsvc = task_svc(&db);
    let esvc = epic_svc(&db);

    let epic = esvc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    let task = tsvc
        .create_task_returning(CreateTaskParams {
            title: "Atomic".into(),
            description: "".into(),
            repo_path: "/repo".to_string(),
            plan_path: None,
            epic_id: Some(epic.id),
            sort_order: Some(3),
            tag: Some(TaskTag::Feature),
            base_branch: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    assert_eq!(task.epic_id, Some(epic.id));
    assert_eq!(task.sort_order, Some(3));
    assert_eq!(task.tag, Some(TaskTag::Feature));
}

// -- delete_task -------------------------------------------------------------

#[tokio::test]
async fn delete_task_removes_it() {
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
    // The store only deletes a finished task.
    db.patch_task(id, &crate::store::TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    svc.delete_task(id).await.unwrap();

    let err = svc.get_task(id).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn delete_task_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc.delete_task(TaskId(999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// -- update_task with worktree/tmux_window -----------------------------------

#[tokio::test]
async fn update_task_sets_worktree_and_tmux_window() {
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

    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
            .tmux_window(crate::service::TmuxWindowUpdate::Set(test_tmux_window(
                "task-1",
            ))),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/feat"));
    assert_eq!(
        task.tmux_window.as_ref().map(|w| w.as_str()),
        Some("task-1")
    );
}

#[tokio::test]
async fn update_task_clears_worktree() {
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

    // Set worktree
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Running)
            .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
            .tmux_window(crate::service::TmuxWindowUpdate::Set(test_tmux_window(
                "task-1",
            ))),
    )
    .await
    .unwrap();

    // Clear worktree via FieldUpdate::Clear
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Done)
            .worktree(FieldUpdate::Clear)
            .tmux_window(crate::service::TmuxWindowUpdate::Clear),
    )
    .await
    .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

// -- update_task allows done (the MCP-layer restriction is a handler concern) -----

#[tokio::test]
async fn update_task_allows_done_status() {
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

    svc.update_task(UpdateTaskParams::for_task(id).status(TaskStatus::Done))
        .await
        .unwrap();

    let task = svc.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Done);
}

// -- delete_epic -------------------------------------------------------------

#[tokio::test]
async fn delete_epic_removes_it() {
    let db = test_db().await;
    let svc = epic_svc(&db);

    let epic = svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    svc.delete_epic(epic.id).await.unwrap();

    let err = svc.get_epic(epic.id).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn delete_epic_not_found() {
    let db = test_db().await;
    let svc = epic_svc(&db);
    let err = svc.delete_epic(EpicId(999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// DeleteEpic's guard (epics.allium: `requires: epic.subtree_tasks.all(t =>
// t.status = done)`). It lives on the rule, not only on the TUI confirmation,
// because DeleteEpic has internal callers besides the TUI.

async fn epic_under(svc: &EpicService, title: &str, parent: Option<EpicId>) -> EpicId {
    svc.create_epic(CreateEpicParams {
        parent_epic_id: parent,
        ..CreateEpicParams::fixture(title)
    })
    .await
    .unwrap()
    .id
}

async fn task_in(svc: &TaskService, epic: EpicId, status: TaskStatus) -> TaskId {
    let id = svc
        .create_task(CreateTaskParams {
            epic_id: Some(epic),
            ..make_task_params_on_branch("/repo", "main")
        })
        .await
        .unwrap();
    svc.update_task(UpdateTaskParams::for_task(id).status(status))
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn delete_epic_refuses_while_a_nested_subtree_task_is_not_done() {
    let db = test_db().await;
    let epics = epic_svc(&db);
    let tasks = task_svc(&db);
    let root = epic_under(&epics, "Root", None).await;
    let child = epic_under(&epics, "Child", Some(root)).await;
    let done = task_in(&tasks, root, TaskStatus::Done).await;
    let open = task_in(&tasks, child, TaskStatus::Running).await;

    let err = epics.delete_epic(root).await.unwrap_err();
    assert!(
        matches!(err, ServiceError::Validation(_)),
        "an unfinished task anywhere in the subtree refuses the delete, got {err:?}"
    );
    assert!(epics.get_epic(root).await.is_ok(), "nothing is deleted");
    assert!(epics.get_epic(child).await.is_ok());
    assert!(tasks.get_task(done).await.is_ok());
    assert!(tasks.get_task(open).await.is_ok());
}

#[tokio::test]
async fn delete_epic_admits_a_subtree_whose_tasks_are_all_done() {
    let db = test_db().await;
    let epics = epic_svc(&db);
    let tasks = task_svc(&db);
    let root = epic_under(&epics, "Root", None).await;
    let child = epic_under(&epics, "Child", Some(root)).await;
    let a = task_in(&tasks, root, TaskStatus::Done).await;
    let b = task_in(&tasks, child, TaskStatus::Done).await;
    // An empty sub-epic does not block the parent either.
    let empty = epic_under(&epics, "Empty", Some(root)).await;

    epics.delete_epic(root).await.unwrap();

    for id in [root, child, empty] {
        assert!(matches!(
            epics.get_epic(id).await.unwrap_err(),
            ServiceError::NotFound(_)
        ));
    }
    for id in [a, b] {
        assert!(matches!(
            tasks.get_task(id).await.unwrap_err(),
            ServiceError::NotFound(_)
        ));
    }
}

/// NotifyWatchersOnDelete now fires on TaskRowRemoved, which an epic delete
/// produces for every task in its subtree. A finished target's leftover rows
/// and a deleted watcher's own rows must not be left dangling (task-watchers
/// .allium; the design doc's "pre-existing gap fixed in passing").
#[tokio::test]
async fn delete_epic_leaves_no_watch_rows_for_the_tasks_it_removes() {
    let db = test_db().await;
    let epics = epic_svc(&db);
    let tasks = task_svc(&db);
    let root = epic_under(&epics, "Root", None).await;
    let doomed_target = task_in(&tasks, root, TaskStatus::Done).await;
    let doomed_watcher = task_in(&tasks, root, TaskStatus::Done).await;
    let outside = tasks
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();
    let outside_target = tasks
        .create_task(make_task_params_on_branch("/repo", "main"))
        .await
        .unwrap();

    // A row left on a finished target (the feed-completed case that bypasses
    // NotifyWatchersOnFinish), and a row whose watcher is being deleted.
    db.create_task_watcher(outside, doomed_target)
        .await
        .unwrap();
    db.create_task_watcher(doomed_watcher, outside_target)
        .await
        .unwrap();

    epics.delete_epic(root).await.unwrap();

    assert!(db.list_watchers_of(doomed_target).await.unwrap().is_empty());
    assert!(
        db.list_watchers_of(outside_target)
            .await
            .unwrap()
            .is_empty(),
        "the deleted watcher's own subscription goes with it"
    );
}
#[tokio::test]
async fn list_tasks_filters_by_epic_id() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let esvc = epic_svc(&db);

    let epic = esvc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();

    let id1 = svc
        .create_task(CreateTaskParams {
            title: "In epic".into(),
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

    let _id2 = svc
        .create_task(CreateTaskParams {
            title: "No epic".into(),
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
            epic_id: Some(epic.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, id1);
}

#[tokio::test]
async fn list_tasks_filters_by_repo_paths() {
    let db = test_db().await;
    let svc = task_svc(&db);

    svc.create_task(CreateTaskParams {
        title: "Repo A".into(),
        description: "".into(),
        repo_path: "/repo/a".to_string(),
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

    svc.create_task(CreateTaskParams {
        title: "Repo B".into(),
        description: "".into(),
        repo_path: "/repo/b".to_string(),
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
            repo_paths: Some(vec!["/repo/a".to_string()]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Repo A");
}

#[tokio::test]
async fn list_tasks_excludes_caller_task() {
    let db = test_db().await;
    let svc = task_svc(&db);

    let id1 = svc
        .create_task(CreateTaskParams {
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

    svc.create_task(CreateTaskParams {
        title: "T2".into(),
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
            exclude_task_id: Some(id1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "T2");
}
