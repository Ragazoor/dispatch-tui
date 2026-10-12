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
    let task = db.get_task(id).await.unwrap();
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
    let task = db.get_task(id).await.unwrap();
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
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();
    let (id, _) = running_task_with_window(&db, Some(epic.id)).await;

    svc.close_session(id, crate::service::CloseSessionOutcome::Done)
        .await
        .unwrap();

    let after = db.get_epic(epic.id).await.unwrap();
    assert_eq!(
        after.status,
        TaskStatus::Done,
        "an epic whose only subtask closed rolls up to Done"
    );
}

// -- The epic chain (store-split M5) ----------------------------------------------
//
// Since M5 the chain runs in the module, inside `request_exit_session`'s
// transaction (`store-split.allium: RequestExitSession`,
// `epics.allium: AutoDispatchNextSubtask`): it requests the epic's next backlog
// subtask through the same dispatch request every entry point writes
// (`RequestDispatch`) and records it on the exit_session row. The request is
// what makes the "at most one agent per closed session" guarantee hold under
// concurrent closes (docs/specs/epics.allium). Its candidate ordering is
// pinned in `src/store/tests/tasks_patch.rs`.

/// Create an auto-dispatching epic with `count` backlog subtasks, sort_order
/// 1..=count.
async fn epic_with_backlog_subtasks(
    db: &Arc<dyn store::TaskStore>,
    count: i64,
) -> (EpicId, Vec<TaskId>) {
    let epic_svc = epic_svc(db);
    let task_svc = task_svc(db);
    let epic = epic_svc
        .create_epic(CreateEpicParams::fixture("E"))
        .await
        .unwrap();
    db.patch_epic(epic.id, &crate::store::EpicPatch::new().auto_dispatch(true))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for i in 1..=count {
        let id = task_svc
            .create_task(CreateTaskParams {
                epic_id: Some(epic.id),
                sort_order: Some(i),
                ..CreateTaskParams::fixture(format!("Sub {i}"), "/repo".to_string())
            })
            .await
            .unwrap();
        ids.push(id);
    }
    (epic.id, ids)
}

fn asked_session(answer: crate::store::SessionRequestAnswer) -> crate::models::SessionRequestId {
    match answer {
        crate::store::SessionRequestAnswer::Requested(id) => id,
        crate::store::SessionRequestAnswer::Refused(why) => {
            panic!("expected the request to apply: {why}")
        }
    }
}

/// A running task of `epic_id` on this host, whose session is about to close.
async fn closing_task_in(db: &Arc<dyn store::TaskStore>, epic_id: EpicId) -> TaskId {
    let (closing, _) = running_task_with_window(db, Some(epic_id)).await;
    db.patch_task(
        closing,
        &crate::store::TaskPatch::new().host(Some(crate::store::MEMORY_HOST_ID)),
    )
    .await
    .unwrap();
    closing
}

/// Close `closing`'s session through the module — a done wrap-up, then
/// `request_exit_session` — and answer the subtask the chain requested in the
/// same transaction.
async fn close_session_of(db: &Arc<dyn store::TaskStore>, closing: TaskId) -> Option<TaskId> {
    let svc = task_svc(db);
    let token = asked_session(
        svc.request_wrap_up(closing, crate::models::WrapUpMode::Done)
            .await
            .unwrap(),
    );
    let close = asked_session(
        svc.request_exit_session(closing, token, crate::models::WrapUpMode::Done, None)
            .await
            .unwrap(),
    );
    db.session_request(close).await.unwrap().chained_task
}

#[tokio::test]
async fn the_chain_requests_its_first_subtask_and_marks_it_running() {
    let db = test_db().await;
    let (epic_id, ids) = epic_with_backlog_subtasks(&db, 2).await;
    let closing = closing_task_in(&db, epic_id).await;

    let chained = close_session_of(&db, closing).await;

    assert_eq!(
        chained,
        Some(ids[0]),
        "requests the first subtask by sort_order"
    );
    let claimed = db.get_task(ids[0]).await.unwrap();
    assert_eq!(claimed.status, TaskStatus::Running);
    assert_eq!(
        claimed.sub_status,
        SubStatus::default_for(TaskStatus::Running)
    );
    assert!(
        claimed.last_pre_tool_use_at.is_some(),
        "the claim seeds last_pre_tool_use_at so the tick classifier keeps the task Active"
    );
    let row = db.latest_dispatch_request_for_task(ids[0]).await.unwrap();
    assert_eq!(row.origin, crate::models::DispatchOrigin::EpicChain);
    assert!(
        db.get_task(ids[0]).await.unwrap().worktree.is_none(),
        "the request does not provision"
    );
    assert_eq!(
        db.get_task(ids[1]).await.unwrap().status,
        TaskStatus::Backlog,
        "only one subtask may be chained per closed session"
    );
}

#[tokio::test]
async fn the_chain_finds_nothing_once_no_backlog_remains() {
    let db = test_db().await;
    let (epic_id, _) = epic_with_backlog_subtasks(&db, 1).await;
    let first = closing_task_in(&db, epic_id).await;
    let second = closing_task_in(&db, epic_id).await;

    assert!(close_session_of(&db, first).await.is_some());
    assert!(
        close_session_of(&db, second).await.is_none(),
        "a requested task is out of contention"
    );
}

#[tokio::test]
async fn the_chain_is_exclusive_under_concurrency() {
    let db = test_db().await;
    let (epic_id, _) = epic_with_backlog_subtasks(&db, 2).await;
    let first_closer = closing_task_in(&db, epic_id).await;
    let second_closer = closing_task_in(&db, epic_id).await;
    let third_closer = closing_task_in(&db, epic_id).await;

    let a = {
        let db = db.clone();
        tokio::spawn(async move { close_session_of(&db, first_closer).await })
    };
    let b = {
        let db = db.clone();
        tokio::spawn(async move { close_session_of(&db, second_closer).await })
    };
    let first = a.await.unwrap().expect("first request");
    let second = b.await.unwrap().expect("second request");

    assert_ne!(
        first, second,
        "two concurrent closes must never request the same subtask"
    );
    assert!(
        close_session_of(&db, third_closer).await.is_none(),
        "both subtasks are requested, so a third close chains nothing"
    );
}

#[tokio::test]
async fn the_chain_passes_over_a_phoenix_subtask() {
    let db = test_db().await;
    let (epic_id, ids) = epic_with_backlog_subtasks(&db, 2).await;
    db.patch_task(ids[0], &crate::store::TaskPatch::new().phoenix(true))
        .await
        .unwrap();
    let closing = closing_task_in(&db, epic_id).await;

    let claimed = close_session_of(&db, closing).await;

    assert_eq!(claimed, Some(ids[1]), "epics.allium: PhoenixIsNeverChained");
}

/// A store that is DOWN fails the close rather than reporting a close with no
/// chain: an outage must not look like a finished epic, and nothing was
/// applied, so the token is still valid for a retry.
#[tokio::test]
async fn a_close_with_the_store_down_fails_loudly() {
    let db = test_db().await;
    let (epic_id, _) = epic_with_backlog_subtasks(&db, 1).await;
    let closing = closing_task_in(&db, epic_id).await;
    // The same rows, written through a transport that was never connected:
    // every reducer call is refused as an outage.
    let rows = std::sync::Arc::new(crate::sync::SharedRows::new());
    for epic in db.list_epics().await {
        let mut row = crate::sync::encode::create_epic_row(
            &epic.title,
            "",
            None,
            "u",
            "2026-10-11 12:00:00.000",
        );
        row.id = epic.id.0;
        rows.upsert_epic(&row);
    }
    for task in db.list_all().await {
        let mut row = crate::sync::encode::create_task_row(
            &crate::store::CreateTaskRequest {
                epic_id: task.epic_id,
                ..crate::store::CreateTaskRequest::fixture(&task.title, &task.repo_path)
            },
            "",
            "u",
            "2026-10-11 12:00:00.000",
        );
        row.id = task.id.0;
        rows.upsert_task(&row);
    }
    let offline = crate::sync::SdkReducerCaller::new(
        std::sync::Arc::new(crate::sync::SpacetimeSdkConnector::new(
            "offline",
            rows.clone(),
        )),
        std::sync::Arc::new(crate::sync::SettledIdentity::default()),
    );
    let host_dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn store::TaskStore> = Arc::new(Store::new(
        rows,
        std::sync::Arc::new(offline),
        std::sync::Arc::new(crate::sync::SettledIdentity::default()),
        std::sync::Arc::new(crate::clock::SystemClock),
        "test-host".to_string(),
        host_dir.path(),
    ));
    let svc = task_svc(&store);

    assert!(svc
        .request_exit_session(
            closing,
            crate::models::SessionRequestId(1),
            crate::models::WrapUpMode::Done,
            None
        )
        .await
        .is_err());
}

// -- request_dispatch (by id) -------------------------------------------------
//
// The request every dispatch entry point writes. The claim is in the same
// module transaction, which is what makes DispatchClaimExclusive
// (docs/specs/dispatch.allium) hold across entry points.

fn requested(answer: crate::store::RequestDispatchAnswer) -> crate::models::DispatchRequestId {
    match answer {
        crate::store::RequestDispatchAnswer::Requested(id) => id,
        crate::store::RequestDispatchAnswer::Refused(why) => {
            panic!("expected the request to apply: {why}")
        }
    }
}

async fn request(svc: &TaskService, id: TaskId) -> crate::store::RequestDispatchAnswer {
    svc.request_dispatch(
        id,
        crate::models::DispatchOrigin::User,
        Some(crate::models::DispatchMode::Dispatch),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_request_moves_the_task_running_without_provisioning() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    requested(request(&svc, id).await);

    let claimed = db.get_task(id).await.unwrap();
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
        "the request runs ahead of provisioning"
    );
}

#[tokio::test]
async fn a_request_clears_leftover_subagents_without_flipping() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    // Leftovers from a previous run of this same task.
    db.subagent_start(id, "stale", "old-session", chrono::Utc::now())
        .await
        .unwrap();
    db.patch_task(id, &crate::store::TaskPatch::new().stop_pending(true))
        .await
        .unwrap();

    requested(request(&svc, id).await);

    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.live_subagents, 0, "a fresh dispatch starts from zero");
    assert!(!task.stop_pending);
    assert_eq!(
        task.status,
        TaskStatus::Running,
        "dispatch owns the status; the drain path must not flip it to Review"
    );
}

#[tokio::test]
async fn a_refused_request_writes_nothing() {
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

    assert!(matches!(
        request(&svc, id).await,
        crate::store::RequestDispatchAnswer::Refused(_)
    ));

    // A refused request must not stamp last_pre_tool_use_at on someone
    // else's task, nor leave a row behind.
    let after = db.get_task(id).await.unwrap();
    assert_eq!(after.status, TaskStatus::Review);
    assert_eq!(after.sub_status, SubStatus::default_for(TaskStatus::Review));
    assert!(
        after.last_pre_tool_use_at.is_none(),
        "a refused request must not seed the activity stamp"
    );
    assert!(db.latest_dispatch_request_for_task(id).await.is_none());
}

#[tokio::test]
async fn a_request_for_a_missing_task_is_refused() {
    let db = test_db().await;
    let svc = task_svc(&db);
    assert!(matches!(
        request(&svc, TaskId(999_999)).await,
        crate::store::RequestDispatchAnswer::Refused(_)
    ));
}

#[tokio::test]
async fn a_request_is_exclusive_under_concurrency() {
    let db = test_db().await;
    let svc = Arc::new(task_svc(&db));
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();

    let a = {
        let svc = svc.clone();
        tokio::spawn(async move { request(&svc, id).await })
    };
    let b = {
        let svc = svc.clone();
        tokio::spawn(async move { request(&svc, id).await })
    };
    let won = |a: &crate::store::RequestDispatchAnswer| {
        matches!(a, crate::store::RequestDispatchAnswer::Requested(_))
    };
    let (first, second) = (a.await.unwrap(), b.await.unwrap());

    assert!(
        won(&first) ^ won(&second),
        "exactly one of two concurrent requests for the same task may apply"
    );
}

/// The release a failed request makes is the module's
/// (`store-split.allium: ReleaseClaimOfFailedRequest`), not the caller's: the
/// task returns to backlog with the stamp cleared and is dispatchable again.
#[tokio::test]
async fn a_failed_request_returns_the_task_to_backlog() {
    let db = test_db().await;
    let svc = task_svc(&db);
    let id = svc.create_task(make_task_params("/repo")).await.unwrap();
    let request_id = requested(request(&svc, id).await);
    assert!(db.claim_dispatch_request(request_id).await.unwrap());

    assert!(db
        .report_dispatch_failed(request_id, "worktree add failed")
        .await
        .unwrap());

    let released = db.get_task(id).await.unwrap();
    assert_eq!(released.status, TaskStatus::Backlog);
    assert_eq!(
        released.sub_status,
        SubStatus::default_for(TaskStatus::Backlog)
    );
    assert!(
        released.last_pre_tool_use_at.is_none(),
        "the release clears the stamp the claim seeded"
    );
    requested(request(&svc, id).await);
}
