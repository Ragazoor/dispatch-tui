use super::*;

// -- close_session ---------------------------------------------------------
//
// The one purpose-built terminal write. Callers gate the tmux teardown and the
// epic chain on its Result, so `Err` must mean "the write did not land" and
// nothing else — see ExitSession in docs/specs/pr-workflow.allium.

#[tokio::test]
async fn close_session_done_moves_task_to_done_and_clears_the_window() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let (id, window) = running_task_with_window(&db, None).await;

    let closed = svc
        .close_session(id, crate::service::CloseSessionOutcome::Done)
        .await
        .unwrap();

    assert_eq!(
        closed.window,
        Some(window),
        "the caller tears down the window this close cleared"
    );
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Done));
    assert!(task.tmux_window.is_none());
    assert!(
        task.worktree.is_some(),
        "the worktree survives the close; it is removed on delete"
    );
    assert!(
        task.completed_at.is_some(),
        "the Done transition stamps the completion time"
    );
    assert!(task.url.is_none());
}

#[tokio::test]
async fn close_session_pr_moves_task_to_review_and_records_the_url() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let (id, window) = running_task_with_window(&db, None).await;

    let closed = svc
        .close_session(
            id,
            crate::service::CloseSessionOutcome::Review {
                pr_url: crate::models::TaskUrl::new(
                    "https://github.com/acme/repo/pull/7".to_string(),
                    crate::models::UrlType::Pr,
                ),
            },
        )
        .await
        .unwrap();

    assert_eq!(closed.window, Some(window));
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::default_for(TaskStatus::Review));
    assert!(task.tmux_window.is_none());
    let url = task.url.expect("pr url recorded");
    assert_eq!(url.url, "https://github.com/acme/repo/pull/7");
    assert!(url.is_pr());
}

#[tokio::test]
async fn close_session_reports_a_missing_window_as_none() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let closed = svc
        .close_session(id, crate::service::CloseSessionOutcome::Done)
        .await
        .unwrap();

    assert!(closed.window.is_none(), "nothing to tear down");
}

#[tokio::test]
async fn close_session_missing_task_is_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc
        .close_session(TaskId(999_999), crate::service::CloseSessionOutcome::Done)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

#[tokio::test]
async fn close_session_recalculates_the_parent_epic() {
    let db = test_db().await;
    let svc = task_svc(&db);
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
    let (id, _) = running_task_with_window(&db, Some(epic.id)).await;

    svc.close_session(id, crate::service::CloseSessionOutcome::Done)
        .await
        .unwrap();

    let after = db.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(
        after.status,
        TaskStatus::Done,
        "an epic whose only subtask closed rolls up to Done"
    );
}

// -- claim_next_backlog_task -----------------------------------------------
//
// The atomic claim is what makes AutoDispatchNextSubtask's "at most one agent
// per closed session" guarantee hold under concurrent closes
// (docs/specs/epics.allium).

/// Create an epic with `count` backlog subtasks, sort_order 1..=count.
async fn epic_with_backlog_subtasks(
    db: &Arc<dyn db::TaskStore>,
    count: i64,
) -> (EpicId, Vec<TaskId>) {
    let epic_svc = epic_svc(db);
    let task_svc = task_svc(db);
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
    let mut ids = Vec::new();
    for i in 1..=count {
        let id = task_svc
            .create_task(CreateTaskParams {
                title: format!("Sub {i}"),
                description: "".into(),
                repo_path: "/repo".to_string(),
                plan_path: None,
                epic_id: Some(epic.id),
                sort_order: Some(i),
                tag: None,
                base_branch: None,
                wrap_up_mode: None,
                auto_run_plan: false,
                phoenix: false,
            })
            .await
            .unwrap();
        ids.push(id);
    }
    (epic.id, ids)
}

#[tokio::test]
async fn claim_next_backlog_task_marks_the_claimed_task_running() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let (epic_id, ids) = epic_with_backlog_subtasks(&db, 2).await;

    let claimed = svc.claim_next_backlog_task(epic_id).await.unwrap().unwrap();
    assert_eq!(claimed.id, ids[0], "claims the first subtask by sort_order");
    assert_eq!(claimed.status, TaskStatus::Running);
    assert_eq!(
        claimed.sub_status,
        SubStatus::default_for(TaskStatus::Running)
    );
    assert!(
        claimed.last_pre_tool_use_at.is_some(),
        "the claim seeds last_pre_tool_use_at so the tick classifier keeps the task Active"
    );

    let persisted = db.get_task(ids[0]).await.unwrap().unwrap();
    assert_eq!(persisted.status, TaskStatus::Running);
    assert!(persisted.worktree.is_none(), "the claim does not provision");
}

#[tokio::test]
async fn claim_next_backlog_task_returns_none_when_no_backlog_remains() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let (epic_id, _) = epic_with_backlog_subtasks(&db, 1).await;

    assert!(svc
        .claim_next_backlog_task(epic_id)
        .await
        .unwrap()
        .is_some());
    assert!(
        svc.claim_next_backlog_task(epic_id)
            .await
            .unwrap()
            .is_none(),
        "a claimed task is out of contention"
    );
}

#[tokio::test]
async fn claim_next_backlog_task_is_exclusive_under_concurrency() {
    let db = test_db().await;
    let svc = Arc::new(task_svc(&db));
    let (epic_id, _) = epic_with_backlog_subtasks(&db, 2).await;

    let a = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.claim_next_backlog_task(epic_id).await })
    };
    let b = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.claim_next_backlog_task(epic_id).await })
    };
    let first = a.await.unwrap().unwrap().expect("first claim");
    let second = b.await.unwrap().unwrap().expect("second claim");

    assert_ne!(
        first.id, second.id,
        "two concurrent claims must never win the same subtask"
    );
    assert!(
        svc.claim_next_backlog_task(epic_id)
            .await
            .unwrap()
            .is_none(),
        "both subtasks are claimed, so a third caller gets None"
    );
}

#[tokio::test]
async fn claim_next_backlog_task_clears_leftover_subagents_without_flipping() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let (epic_id, ids) = epic_with_backlog_subtasks(&db, 1).await;

    // Leftovers from a previous run of this same task.
    db.subagent_start(ids[0], "stale", "old-session", chrono::Utc::now())
        .await
        .unwrap();
    db.patch_task(ids[0], &crate::db::TaskPatch::new().stop_pending(true))
        .await
        .unwrap();

    let claimed = svc.claim_next_backlog_task(epic_id).await.unwrap().unwrap();
    assert_eq!(claimed.id, ids[0]);

    let task = db.get_task(ids[0]).await.unwrap().unwrap();
    assert_eq!(task.live_subagents, 0, "a fresh dispatch starts from zero");
    assert!(!task.stop_pending);
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "dispatch owns the status; the drain path must not flip it to Review"
    );
}

#[tokio::test]
async fn claim_next_backlog_task_epic_not_found() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let err = svc.claim_next_backlog_task(EpicId(999)).await.unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)));
}

// -- claim_backlog_task (by id) ---------------------------------------------
//
// The by-id twin of the claim above. Every dispatch entry point takes this
// before it provisions, which is what makes DispatchClaimExclusive
// (docs/specs/dispatch.allium) hold across entry points and not merely
// between chains.

#[tokio::test]
async fn claim_backlog_task_moves_the_task_running_without_provisioning() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    assert!(svc.claim_backlog_task(id).await.unwrap());

    let claimed = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(claimed.status, TaskStatus::Running);
    assert_eq!(
        claimed.sub_status,
        SubStatus::default_for(TaskStatus::Running)
    );
    assert!(
        claimed.last_pre_tool_use_at.is_some(),
        "the claim seeds last_pre_tool_use_at so the tick classifier keeps the task Active"
    );
    assert!(
        claimed.worktree.is_none(),
        "the claim runs ahead of provisioning"
    );
}

#[tokio::test]
async fn dispatch_claim_clears_leftover_subagents_without_flipping() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    // Leftovers from a previous run of this same task.
    db.subagent_start(id, "stale", "old-session", chrono::Utc::now())
        .await
        .unwrap();
    db.patch_task(id, &crate::db::TaskPatch::new().stop_pending(true))
        .await
        .unwrap();

    assert!(svc.claim_backlog_task(id).await.unwrap());

    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.live_subagents, 0, "a fresh dispatch starts from zero");
    assert!(!task.stop_pending);
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "dispatch owns the status; the drain path must not flip it to Review"
    );
}

#[tokio::test]
async fn claim_backlog_task_lost_claim_writes_nothing() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();
    svc.update_task(
        UpdateTaskParams::for_task(id)
            .status(TaskStatus::Review)
            .sub_status(SubStatus::default_for(TaskStatus::Review)),
    )
    .await
    .unwrap();

    assert!(!svc.claim_backlog_task(id).await.unwrap());

    // The extras patch must be gated on the transition winning, or a lost
    // claim would stamp last_pre_tool_use_at on someone else's task.
    let after = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(after.status, TaskStatus::Review);
    assert_eq!(after.sub_status, SubStatus::default_for(TaskStatus::Review));
    assert!(
        after.last_pre_tool_use_at.is_none(),
        "a lost claim must not seed the activity stamp"
    );
}

#[tokio::test]
async fn claim_backlog_task_is_false_for_a_missing_task() {
    let db = test_db().await;
    let svc = task_svc(&db);
    assert!(!svc.claim_backlog_task(TaskId(999_999)).await.unwrap());
}

#[tokio::test]
async fn claim_backlog_task_is_exclusive_under_concurrency() {
    let db = test_db().await;
    let svc = Arc::new(task_svc(&db));
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let a = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.claim_backlog_task(id).await })
    };
    let b = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.claim_backlog_task(id).await })
    };
    let first = a.await.unwrap().unwrap();
    let second = b.await.unwrap().unwrap();

    assert!(
        first ^ second,
        "exactly one of two concurrent claims on the same task may win"
    );
}

#[tokio::test]
async fn release_claim_undoes_a_by_id_claim() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();
    assert!(svc.claim_backlog_task(id).await.unwrap());

    assert!(svc.release_claim(id).await.unwrap());

    let released = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(released.status, TaskStatus::Backlog);
    assert_eq!(
        released.sub_status,
        SubStatus::default_for(TaskStatus::Backlog)
    );
    assert!(
        released.last_pre_tool_use_at.is_none(),
        "the release clears the stamp the claim seeded"
    );
    assert!(
        svc.claim_backlog_task(id).await.unwrap(),
        "a released task is dispatchable again"
    );
}
