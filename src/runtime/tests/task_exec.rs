/// Base-branch history tests, plus the broader grab-bag of task exec/dispatch/cleanup
/// tests that accumulated under this banner over time — the name reflects the
/// original section, not its full current scope.
use super::*;
use crate::models::test_tmux_window;

/// Serve the dispatch requests an arm wrote, the way this host's reactor would
/// once the row pump delivers them (`store-split.allium: HostClaimsDispatch`),
/// and wait for each to land. Since store-split M4 no launch arm provisions
/// inline.
async fn serve(rt: &TuiRuntime) {
    for handle in rt.serve_dispatch_requests().await {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn exec_save_base_branch_records_and_updates_app_state() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_save_base_branch(&mut app, "/repo".into(), "develop".into())
        .await;
    assert_eq!(
        app.base_branches_for("/repo"),
        &["develop".to_string()],
        "app.board.repo_base_branches should reflect the newly recorded branch"
    );
    let all = rt.database.list_all_base_branches().await;
    assert!(all.contains(&("/repo".to_string(), "develop".to_string())));
}

#[tokio::test]
async fn exec_save_base_branch_upsert_keeps_most_recent_first() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_save_base_branch(&mut app, "/repo".into(), "main".into())
        .await;
    rt.exec_save_base_branch(&mut app, "/repo".into(), "develop".into())
        .await;
    assert_eq!(
        app.base_branches_for("/repo"),
        &["develop".to_string(), "main".to_string()],
        "most-recently-used branch should be first"
    );
}

#[tokio::test]
async fn finish_task_creation_emits_save_repo_path_and_save_base_branch() {
    let (_rt, mut app) = test_runtime().await;

    // Drive the whole manual task-creation flow through the public Message
    // API (App fields are `pub(in crate::tui)` and unreachable from here).
    app.update(Message::Input(
        crate::tui::messages::InputMessage::StartNewTask,
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitTitle("T".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitTag(None),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitDescription("D".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/tmp".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch("develop".to_string()),
    ));
    // Wrap-up is the form's last step; submitting it commits the creation
    // (tasks.allium: CreateTask).
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitWrapUpMode(None),
    ));

    assert!(
        cmds.iter().any(
            |c| matches!(c, Command::Settings(SettingsCommand::SaveRepoPath(p)) if p == "/tmp")
        ),
        "expected a SaveRepoPath(\"/tmp\") command, got: {cmds:?}"
    );
    assert!(
        cmds.iter().any(
            |c| matches!(c, Command::Settings(SettingsCommand::SaveBaseBranch(repo, branch)) if repo == "/tmp" && branch == "develop")
        ),
        "expected a SaveBaseBranch(\"/tmp\", \"develop\") command, got: {cmds:?}"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_does_not_record_base_branch_history() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-quick-task")).unwrap();

    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::dispatch()
        .detecting_default_branch("main")
        .shared_runner();
    let rt = make_runtime(db.clone(), tx, mock).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Quick task".into(),
            description: String::new(),
            repo_path: repo.to_string(),
            ..Default::default()
        },
        None,
    )
    .await;

    // Repo path IS recorded (existing RecordRepoPath behavior)...
    assert!(app.repo_paths().contains(&repo.to_string()));
    // ...but base branch history is deliberately NOT recorded for quick
    // dispatch — see dispatch.allium: RecordBaseBranch's "recording scope
    // (deliberately narrow)" guidance. Only the manual new-task form records.
    assert!(
        app.base_branches_for(repo).is_empty(),
        "quick dispatch must not record base branch history"
    );
    assert!(rt.database.list_all_base_branches().await.is_empty());

    // Serve the request and drain the Dispatched message so the sender isn't
    // left dangling.
    serve(&rt).await;
    let _ = tokio::time::timeout(TEST_TIMEOUT, rx.recv()).await;
}

#[tokio::test]
async fn exec_refresh_from_db_syncs_external_changes() {
    let (rt, mut app) = test_runtime().await;
    // Insert directly into DB, bypassing app
    rt.db_write()
        .create_task(CreateTaskRequest {
            description: "Added via CLI",
            ..CreateTaskRequest::fixture("External", "/repo")
        })
        .await
        .unwrap();
    assert!(app.tasks().is_empty());
    rt.exec_refresh_from_db(&mut app).await;
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "External");
}

#[tokio::test]
async fn exec_refresh_from_db_returns_commands_from_refresh() {
    let (rt, mut app) = test_runtime().await;
    // Insert a task directly into DB as Running
    rt.db_write()
        .create_task(CreateTaskRequest {
            description: "Desc",
            status: models::TaskStatus::Running,
            ..CreateTaskRequest::fixture("Test", "/repo")
        })
        .await
        .unwrap();
    // Load it into app
    let cmds = rt.exec_refresh_from_db(&mut app).await;
    assert!(cmds.is_empty()); // First load — no transition

    let task = rt.database.list_all().await[0].clone();
    rt.db_write()
        .patch_task(
            task.id,
            &store::TaskPatch::new().status(models::TaskStatus::Review),
        )
        .await
        .unwrap();

    app.set_notifications_enabled(true);
    let cmds = rt.exec_refresh_from_db(&mut app).await;
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
    )));
}

#[tokio::test]
async fn exec_delete_task_nonexistent_shows_error() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![]))).await;
    rt.exec_delete_task(TaskId(999)).await;
    assert!(matches!(
        rx.recv().await.unwrap(),
        Message::Task(crate::tui::messages::TaskMessage::DeleteRefused { .. })
    ));
}

#[tokio::test]
async fn exec_jump_to_tmux_calls_select_window() {
    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(
        MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // for select-window
        ])
        .with_windows(&["my-window"]),
    );
    let rt = make_runtime(db.clone(), tx, mock.clone()).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_jump_to_tmux(&mut app, test_tmux_window("my-window"));

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.contains(&"select-window".to_string()));
    // Targeted by resolved pane ID, not by name — see `tmux::window_target`.
    assert!(calls[0].1.contains(&mock.pane_id_of("my-window")));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_dispatch_sends_dispatched_message() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    // Create .worktrees/ and fake worktree directory so file writes succeed
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-test-task")).unwrap();

    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::dispatch().shared_runner();
    let rt = make_runtime(db.clone(), tx, mock).await;

    let task = create_task_returning(
        &*db,
        "Test Task",
        "desc",
        repo,
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let id = task.id;
    rt.exec_dispatch_agent(task.id, Some(models::DispatchMode::Dispatch))
        .await;

    // The request is the claim, and it owns the Running write
    // (`DispatchClaimExclusive` in docs/specs/dispatch.allium).
    let claimed = db.get_task(id).await.unwrap();
    assert_eq!(claimed.status, models::TaskStatus::Running);
    assert!(
        claimed.last_pre_tool_use_at.is_some(),
        "the claim seeds the activity stamp"
    );

    serve(&rt).await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
        ),
        "Expected Dispatched, got: {msg:?}"
    );
}

/// Mode routing at the board's entry point. The `DispatchMode` match lives once
/// (`dispatch::run_agent_for_mode`) and the service seam takes it too, so this
/// is the assertion that the board reaches the *same* match: `Research` must
/// launch the research agent. Every launcher shares one permission mode
/// (`EveryTaskAgentLaunchesInAutoMode`), so the prompt identifies the agent.
/// Its twin at the seam is
/// `service::tasks::tests::dispatch_requests::serve_launches_the_agent_the_row_names`.
#[tokio::test]
async fn exec_dispatch_agent_routes_research_mode_to_the_research_agent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-test-task")).unwrap();

    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::dispatch().shared_runner();
    let rt = make_runtime(db.clone(), tx, mock.clone()).await;

    let task = create_task_returning(
        &*db,
        "Test Task",
        "desc",
        repo,
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    rt.exec_dispatch_agent(task.id, Some(models::DispatchMode::Research))
        .await;
    serve(&rt).await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
        ),
        "Expected Dispatched, got: {msg:?}"
    );

    let prompt = std::fs::read_to_string(format!("{repo}/.worktrees/1-test-task/.claude-prompt"))
        .expect("dispatch should write the prompt file");
    assert!(
        prompt.contains(crate::dispatch::RESEARCH_AGENT_INTRO),
        "research mode must reach build_research_prompt: {prompt}"
    );
}

/// A refused request must stop the dispatch dead, before any provisioning
/// command runs, and report the failure so the spinner drains
/// (`LostClaimReported` in docs/specs/dispatch.allium).
#[tokio::test]
async fn exec_dispatch_agent_lost_claim_provisions_nothing() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    // An empty script is itself the assertion: any provisioning command would
    // panic the mock rather than pass quietly.
    let mock = Arc::new(MockProcessRunner::new(vec![]));
    let rt = make_runtime(db.clone(), tx, mock.clone()).await;

    // Backlog in the caller's snapshot, already Running in the DB — exactly the
    // race the claim exists to catch.
    let task = create_task_returning(
        &*db,
        "Contended Task",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    assert!(matches!(
        rt.task_svc
            .request_dispatch(task.id, models::DispatchOrigin::Mcp, None)
            .await
            .unwrap(),
        crate::store::RequestDispatchAnswer::Requested(_)
    ));

    rt.exec_dispatch_agent(task.id, Some(models::DispatchMode::Dispatch))
        .await;

    let msg1 = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::Task(crate::tui::messages::TaskMessage::DispatchAbandoned(id)) if id == task.id),
        "a refused request must report DispatchAbandoned and release nothing — the \
         claim belongs to the winner. Got: {msg1:?}"
    );
    let msg2 = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg2,
            Message::System(crate::tui::messages::SystemMessage::Error(_))
        ),
        "Expected Error, got: {msg2:?}"
    );
    assert!(
        mock.recorded_calls().is_empty(),
        "a lost claim must run no provisioning commands, got: {:?}",
        mock.recorded_calls()
    );
    // The winner's claim is untouched: still Running, still unprovisioned,
    // still theirs to finish.
    let after = db.get_task(task.id).await.unwrap();
    assert_eq!(after.status, models::TaskStatus::Running);
    assert!(after.worktree.is_none());
}

// `exec_release_claim_returns_the_task_to_backlog` pinned
// `TaskCommand::ReleaseClaim`, retired by store-split M4: a failed request
// releases the claim in the module (`ReleaseClaimOfFailedRequest`), which
// `exec_dispatch_failure_fails_the_request_and_releases_the_claim` below pins
// from the board's side.

/// A provisioning failure fails the request with its reason, and the module
/// releases the claim. The board learns of it from the row
/// (`FailedRequestReported`), not from a message of the arm's own.
#[tokio::test]
async fn exec_dispatch_failure_fails_the_request_and_releases_the_claim() {
    use crate::store::DispatchRequestRead;

    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(),                                // git worktree prune
        MockProcessRunner::fail("fatal: not a git repository"), // git worktree add fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    let task = create_task_returning(
        &*db,
        "Fail Task",
        "desc",
        "/nonexistent",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    rt.exec_dispatch_agent(task.id, Some(models::DispatchMode::Dispatch))
        .await;
    serve(&rt).await;

    let row = db
        .latest_dispatch_request_for_task(task.id)
        .await
        .expect("the arm wrote a request");
    assert_eq!(row.state, models::DispatchRequestState::Failed);
    assert!(row.reason.is_some(), "the failure carries its reason");
    assert_eq!(
        db.get_task(task.id).await.unwrap().status,
        models::TaskStatus::Backlog,
        "the module released the claim"
    );
}

#[tokio::test]
async fn exec_check_window_sends_window_gone_when_absent() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows returns other window names (not our window)
        MockProcessRunner::ok_with_stdout(b"other-window\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    rt.exec_check_window(TaskId(1), test_tmux_window("gone-window"));

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::WindowGone(TaskId(1)))
        ),
        "Expected WindowGone, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_check_window_sends_nothing_when_present() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows returns our window
        MockProcessRunner::ok_with_stdout(b"task-1\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    rt.exec_check_window(TaskId(1), test_tmux_window("task-1"))
        .await
        .unwrap();
    assert!(
        rx.try_recv().is_err(),
        "Expected no message but received one"
    );
}

#[tokio::test]
async fn exec_check_window_sends_nothing_when_query_fails() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    // The runner itself errors (e.g. tmux binary missing) — a transient
    // failure must not be mistaken for the window (and therefore the agent)
    // being gone.
    let mock = Arc::new(MockProcessRunner::new(vec![Err(anyhow::anyhow!(
        "failed to run tmux"
    ))]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    rt.exec_check_window(TaskId(1), test_tmux_window("task-1"))
        .await
        .unwrap();

    assert!(
        rx.try_recv().is_err(),
        "a tmux query failure must not send WindowGone"
    );
}

#[tokio::test]
async fn exec_batch_check_windows_sends_window_gone_only_for_absent() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    // Single `tmux list-windows -a` reports task-1 present, task-2 gone (died mid-run).
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-1\nother-window\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone()).await;

    rt.exec_batch_check_windows(vec![
        (TaskId(1), TmuxWindow::for_task(TaskId(1))),
        (TaskId(2), TmuxWindow::for_task(TaskId(2))),
    ])
    .await
    .unwrap();

    // Exactly one WindowGone, for the absent window (task-2).
    let mut gone = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let Message::Task(crate::tui::messages::TaskMessage::WindowGone(id)) = msg {
            gone.push(id);
        } else {
            panic!("unexpected message: {msg:?}");
        }
    }
    assert_eq!(gone, vec![TaskId(2)], "only the absent window should crash");

    // A single batched tmux call, not one per window. (The exact argv of
    // list-windows is owned by tmux.rs's own unit tests — assert only the
    // batching guarantee here.)
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "batch check should issue one tmux call");
    assert_eq!(calls[0].1[0], "list-windows");
}

#[tokio::test]
async fn exec_batch_check_windows_sends_nothing_when_all_present() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-1\ntask-2\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    rt.exec_batch_check_windows(vec![
        (TaskId(1), TmuxWindow::for_task(TaskId(1))),
        (TaskId(2), TmuxWindow::for_task(TaskId(2))),
    ])
    .await
    .unwrap();

    assert!(
        rx.try_recv().is_err(),
        "no WindowGone expected when all windows are present"
    );
}

#[tokio::test]
async fn exec_batch_check_windows_stays_silent_when_tmux_cannot_be_spawned() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    // The runner itself errors (e.g. tmux binary missing) — `list_all_window_names`
    // propagates the Err, and the batch check bails without marking any window
    // gone, so a transient tmux failure can't crash every running task at once.
    let mock = Arc::new(MockProcessRunner::new(vec![Err(anyhow::anyhow!(
        "failed to run tmux"
    ))]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    rt.exec_batch_check_windows(vec![(TaskId(1), TmuxWindow::for_task(TaskId(1)))])
        .await
        .unwrap();

    assert!(
        rx.try_recv().is_err(),
        "a tmux spawn error must not be treated as every window being gone"
    );
}

#[tokio::test]
async fn exec_jump_to_tmux_failure_shows_error() {
    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no such window"), // simulate tmux failure
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone()).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_jump_to_tmux(&mut app, test_tmux_window("nonexistent-window"));

    assert!(app.error_popup().is_some());
}

/// Seed one Done task that owns `worktree`, and a runtime whose runner
/// answers `script`. Returns the runtime, the task id, the message receiver and
/// the concrete runner, so a caller that needs to assert on the issued commands
/// can reach `flattened_calls()`.
async fn cleanup_fixture(
    script: Vec<anyhow::Result<std::process::Output>>,
    worktree: &str,
) -> (
    TuiRuntime,
    models::TaskId,
    mpsc::UnboundedReceiver<Message>,
    Arc<MockProcessRunner>,
) {
    cleanup_fixture_owning(script, Some(worktree), None).await
}

/// `cleanup_fixture` with both resources spelled out — for the window-only row
/// shape (`TeardownIsOwedWheneverThereIsSomethingToRelease`), which owns a tmux
/// window and no worktree.
async fn cleanup_fixture_owning(
    script: Vec<anyhow::Result<std::process::Output>>,
    worktree: Option<&str>,
    window: Option<&crate::models::TmuxWindow>,
) -> (
    TuiRuntime,
    models::TaskId,
    mpsc::UnboundedReceiver<Message>,
    Arc<MockProcessRunner>,
) {
    let db = test_db().await;
    let (tx, rx) = mpsc::unbounded_channel();
    let runner = Arc::new(MockProcessRunner::new(script));
    let rt = make_runtime(db.clone(), tx, runner.clone()).await;

    let task = create_task_returning(
        &*db,
        "Doomed",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Done,
    )
    .await
    .unwrap();
    db.patch_task(
        task.id,
        &store::TaskPatch::new()
            .worktree(worktree)
            .tmux_window(window),
    )
    .await
    .unwrap();

    (rt, task.id, rx, runner)
}

/// A failed `git worktree remove` must not let the operation forget the path.
/// The row keeps its pointer so the leftover directory stays reachable from the
/// board, and the failure is reported. `WorktreeReleaseIsGated` in
/// docs/specs/tasks.allium; the silent-orphan mechanism from
/// docs/plans/archive/2026-08-11-3897-worktree-cleanup-investigation.md §3.
#[tokio::test]
async fn exec_cleanup_failure_keeps_the_worktree_pointer() {
    let worktree = "/repo/.worktrees/1-doomed";
    let (rt, id, mut rx, _runner) = cleanup_fixture(
        // A LOCKED worktree is the failure that still leaves the directory on
        // disk without needing one: `GitsExitCodeDoesNotDecideStepTwo` in
        // docs/specs/tasks.allium means any other git failure over an absent
        // path now RELEASES the worktree, which is not the case this gate is
        // about.
        vec![MockProcessRunner::fail(
            "fatal: cannot remove a locked working tree",
        )],
        worktree,
    )
    .await;

    let handle = rt.exec_cleanup(
        id,
        "/repo".into(),
        Some(worktree.into()),
        None,
        crate::tui::commands::CleanupFollowUp::ClearPointer,
    );
    handle.await.unwrap();

    let row = rt.database.get_task(id).await.unwrap();
    assert_eq!(
        row.worktree.as_deref(),
        Some(worktree),
        "a failed removal must leave the pointer in place"
    );

    let msg = rx.recv().await.unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupFailed { worktree: ref w, .. })
                if w == worktree
        ),
        "the failure must reach the app, got: {msg:?}"
    );
}

/// The success half: only a removal that actually happened earns the follow-up.
#[tokio::test]
async fn exec_cleanup_success_reports_its_follow_up() {
    let worktree = "/repo/.worktrees/1-doomed";
    let (rt, id, mut rx, _runner) = cleanup_fixture(
        vec![
            MockProcessRunner::ok(), // git worktree remove
            MockProcessRunner::ok(), // git branch -D
        ],
        worktree,
    )
    .await;

    let handle = rt.exec_cleanup(
        id,
        "/repo".into(),
        Some(worktree.into()),
        None,
        crate::tui::commands::CleanupFollowUp::DeleteRow,
    );
    handle.await.unwrap();

    let msg = rx.recv().await.unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded {
                follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
                ..
            })
        ),
        "a successful removal must carry its follow-up back, got: {msg:?}"
    );
}

/// The delete path's half of the gate: a failed removal means the row survives,
/// still Done and still pointing at what is on disk, so deleting again
/// retries the removal.
#[tokio::test]
async fn exec_cleanup_failure_does_not_delete_the_row() {
    let worktree = "/repo/.worktrees/1-doomed";
    let (rt, id, mut rx, _runner) = cleanup_fixture(
        // A LOCKED worktree is the failure that still leaves the directory on
        // disk without needing one: `GitsExitCodeDoesNotDecideStepTwo` in
        // docs/specs/tasks.allium means any other git failure over an absent
        // path now RELEASES the worktree, which is not the case this gate is
        // about.
        vec![MockProcessRunner::fail(
            "fatal: cannot remove a locked working tree",
        )],
        worktree,
    )
    .await;

    let handle = rt.exec_cleanup(
        id,
        "/repo".into(),
        Some(worktree.into()),
        None,
        crate::tui::commands::CleanupFollowUp::DeleteRow,
    );
    handle.await.unwrap();

    let row = rt
        .database
        .get_task(id)
        .await
        .expect("the row must survive a failed removal");
    assert_eq!(row.status, models::TaskStatus::Done);
    assert_eq!(row.worktree.as_deref(), Some(worktree));

    let msg = rx.recv().await.unwrap();
    assert!(
        !matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded { .. })
        ),
        "a failed removal must never report success, got: {msg:?}"
    );
}

/// `TeardownIsOwedWheneverThereIsSomethingToRelease` in docs/specs/tasks.allium:
/// a task with a window and no worktree still owes step 1. Before #4096
/// `take_cleanup` dropped the whole command for this shape, so nothing ever ran.
#[tokio::test]
async fn exec_cleanup_kills_the_window_of_a_task_with_no_worktree() {
    let (rt, id, mut rx, runner) = cleanup_fixture_owning(
        vec![
            MockProcessRunner::ok_with_stdout(b"task-1\n"), // has_window
            MockProcessRunner::ok(),                        // tmux kill-window
        ],
        None,
        Some(&test_tmux_window("task-1")),
    )
    .await;

    rt.exec_cleanup(
        id,
        "/repo".into(),
        None,
        Some(test_tmux_window("task-1")),
        crate::tui::commands::CleanupFollowUp::DeleteRow,
    )
    .await
    .unwrap();

    let calls = runner.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("kill-window")),
        "the window must be reclaimed even with no worktree, got: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("worktree remove")),
        "there is no worktree to remove, got: {calls:?}"
    );

    let msg = rx.recv().await.unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded {
                follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
                ..
            })
        ),
        "the follow-up must come back so the row is deleted, got: {msg:?}"
    );
}

/// The gate is keyed on step 2 and only on step 2 (`WorktreeReleaseIsGated`).
/// With no worktree there is nothing to release and nothing to retry, so a failed
/// window kill is warn-logged and the follow-up still applies — withholding it
/// would strand the row instead of the resource.
#[tokio::test]
async fn exec_cleanup_window_only_kill_failure_still_applies_the_follow_up() {
    let (rt, id, mut rx, _runner) = cleanup_fixture_owning(
        vec![
            MockProcessRunner::ok_with_stdout(b"task-1\n"), // has_window
            MockProcessRunner::fail("can't find window"),   // kill-window fails
        ],
        None,
        Some(&test_tmux_window("task-1")),
    )
    .await;

    rt.exec_cleanup(
        id,
        "/repo".into(),
        None,
        Some(test_tmux_window("task-1")),
        crate::tui::commands::CleanupFollowUp::DeleteRow,
    )
    .await
    .unwrap();

    let msg = rx.recv().await.unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded {
                follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
                ..
            })
        ),
        "a window-only teardown must not withhold its follow-up, got: {msg:?}"
    );
}

/// `exec_cleanup` tears the worktree down unconditionally — deliberately.
///
/// Hand-builds the state the removed sharing exception described — a second live
/// row naming the very same worktree, which the dispatch flow cannot produce —
/// and pins that the full teardown runs anyway, follow-up and all. A reinstated
/// guard fails here rather than passing silently. `WorktreeIsNeverShared` in
/// docs/specs/tasks.allium is the argument; this is only its tripwire.
#[tokio::test]
async fn exec_cleanup_tears_down_even_if_another_row_names_the_worktree() {
    let worktree = "/repo/.worktrees/1-doomed";
    let (rt, id, mut rx, runner) = cleanup_fixture(
        vec![
            MockProcessRunner::ok_with_stdout(b"task-1\n"), // has_window
            MockProcessRunner::ok(),                        // tmux kill-window
            MockProcessRunner::ok(),                        // git worktree remove
            MockProcessRunner::ok(),                        // git branch -D
        ],
        worktree,
    )
    .await;

    // The impossible second holder of the same path.
    let sharer = create_task_returning(
        &**rt.db_write(),
        "Impossible sharer",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Running,
    )
    .await
    .unwrap();
    rt.db_write()
        .patch_task(sharer.id, &store::TaskPatch::new().worktree(Some(worktree)))
        .await
        .unwrap();

    rt.exec_cleanup(
        id,
        "/repo".into(),
        Some(worktree.into()),
        Some(test_tmux_window("task-1")),
        crate::tui::commands::CleanupFollowUp::DeleteRow,
    )
    .await
    .unwrap();

    let calls = runner.flattened_calls();
    let removed_at = calls
        .iter()
        .position(|c| c.contains("worktree remove") && c.contains(worktree))
        .unwrap_or_else(|| {
            panic!("the worktree goes regardless of what other rows name, got: {calls:?}")
        });
    let killed_at = calls
        .iter()
        .position(|c| c.contains("kill-window"))
        .unwrap_or_else(|| panic!("the window is reclaimed too, got: {calls:?}"));
    // TaskTeardown's clause order: the window goes before the worktree.
    assert!(
        killed_at < removed_at,
        "the window must be killed before the worktree is removed, got: {calls:?}"
    );

    let msg = rx.recv().await.unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded {
                follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
                ..
            })
        ),
        "a real removal must earn its follow-up, got: {msg:?}"
    );
}

#[tokio::test]
async fn send_system_error_sends_error_message() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let rt = make_runtime(db, tx, runner).await;

    rt.send_system_error("something went wrong");

    let msg = rx.recv().await.unwrap();
    assert!(
        matches!(msg, Message::System(crate::tui::messages::SystemMessage::Error(ref e)) if e == "something went wrong"),
        "Expected SystemMessage::Error, got: {msg:?}"
    );
}

// `TaskCommand::Finish`/`exec_finish` and `TaskCommand::CloseSession`/
// `exec_close_session` no longer exist — the TUI wrap-up entry point (`W`)
// that used to dispatch them is gone. Wrap-up rebase/merge and session close
// are now exclusively the MCP `wrap_up`/`exit_session` tools' job (see
// src/mcp/handlers/tasks/wrap_up.rs), which write request rows
// (`TaskService::request_wrap_up`, `request_exit_session`) that this host
// serves, rather than going through a runtime command. The ExitSession
// invariant — a close is whole or absent, and the teardown is the host's, from
// the row the same transaction wrote — is covered at that layer by
// `a_refused_exit_session_changes_nothing_kills_nothing_and_does_not_chain` in
// src/mcp/handlers/tests/tasks/dispatch.rs.

#[tokio::test]
async fn exec_send_notification_calls_notify_send() {
    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // notify-send call
    ]));
    let rt = make_runtime(db, tx, mock.clone()).await;
    rt.exec_send_notification("Task #1: Fix bug", "Ready for review", false)
        .await
        .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "notify-send");
    assert!(calls[0].1.contains(&"Task #1: Fix bug".to_string()));
    assert!(calls[0].1.contains(&"Ready for review".to_string()));
}

#[tokio::test]
async fn exec_send_notification_urgent_uses_critical() {
    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let rt = make_runtime(db, tx, mock.clone()).await;
    rt.exec_send_notification("Task #1: Fix bug", "Agent needs your input", true)
        .await
        .unwrap();
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"critical".to_string()));
}

#[tokio::test]
async fn exec_send_notification_failure_does_not_panic() {
    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "command not found",
    )]));
    let rt = make_runtime(db, tx, mock.clone()).await;
    // Should not panic — just logs a warning
    rt.exec_send_notification("Task #1: Fix bug", "Ready for review", false)
        .await
        .unwrap();
}

#[tokio::test]
async fn exec_persist_setting_writes_to_db() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_persist_setting(&mut app, "notifications_enabled", true)
        .await;
    assert_eq!(
        rt.database
            .get_setting_bool("notifications_enabled")
            .await
            .unwrap(),
        Some(true)
    );
}

/// A review task with a pr-typed url, a window and the given sub-status, in the
/// store the runtime reports to.
async fn review_task_with_pr(rt: &TuiRuntime, sub_status: models::SubStatus) -> TaskId {
    let mut app = App::new(vec![]);
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "PR Task".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;
    let url = models::TaskUrl::new("https://github.com/org/repo/pull/42", models::UrlType::Pr);
    rt.db_write()
        .patch_task(
            id,
            &store::TaskPatch::new()
                .status(models::TaskStatus::Review)
                .sub_status(sub_status)
                .tmux_window(Some(&models::TmuxWindow::for_task(id)))
                .url(Some(&url)),
        )
        .await
        .unwrap();
    id
}

/// `ReportPrStatus`: the runtime posts what `gh` printed and the MODULE writes
/// the task; the message only picks the host's own effects. The window is read
/// before the post, because a merge clears it in the store.
#[tokio::test]
async fn exec_check_pr_status_posts_merged_and_the_module_moves_the_task_to_done() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"MERGED\n"), // gh pr view (no review decision line)
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;
    let id = review_task_with_pr(&rt, models::SubStatus::AwaitingReview).await;

    rt.exec_check_pr_status(id, "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    match msg {
        Message::Pr(crate::tui::messages::PrMessage::Merged { id: got, window }) => {
            assert_eq!(got, id);
            assert_eq!(
                window.as_ref().map(|w| w.as_str()),
                Some(format!("task-{}", id.0).as_str())
            );
        }
        other => panic!("expected Merged, got {other:?}"),
    }
    let stored = db.get_task(id).await.unwrap();
    assert_eq!(stored.status, models::TaskStatus::Done);
    assert!(stored.tmux_window.is_none(), "the module clears the window");
    assert!(stored.completed_at.is_some());
}

#[tokio::test]
async fn exec_check_pr_status_open_posts_the_decision_and_the_module_sets_the_sub_status() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n"), // gh pr view
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;
    let id = review_task_with_pr(&rt, models::SubStatus::AwaitingReview).await;

    rt.exec_check_pr_status(id, "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        msg,
        Message::Pr(crate::tui::messages::PrMessage::ReviewState(got)) if got == id
    ));
    assert_eq!(
        db.get_task(id).await.unwrap().sub_status,
        models::SubStatus::Approved
    );
}

#[tokio::test]
async fn exec_check_pr_status_closed_posts_and_the_module_marks_pr_closed() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"CLOSED\n"), // gh pr view (no review decision line)
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;
    let id = review_task_with_pr(&rt, models::SubStatus::Conflict).await;

    rt.exec_check_pr_status(id, "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        msg,
        Message::Pr(crate::tui::messages::PrMessage::Closed(got)) if got == id
    ));
    assert_eq!(
        db.get_task(id).await.unwrap().sub_status,
        models::SubStatus::PrClosed
    );
}

/// A report that changes nothing (the task already shows `pr_closed`) sends no
/// `Closed`, so a polled-every-tick closed PR notifies once.
#[tokio::test]
async fn exec_check_pr_status_a_repeat_closed_report_sends_no_second_closed() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"CLOSED\n"),
    ]));
    let rt = make_runtime(db, tx, mock).await;
    let id = review_task_with_pr(&rt, models::SubStatus::PrClosed).await;

    rt.exec_check_pr_status(id, "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        msg,
        Message::Pr(crate::tui::messages::PrMessage::ReviewState(got)) if got == id
    ));
}

/// A state the module does not know is refused, which stays true however often
/// it is retried: a permanent failure, and the task is left alone.
#[tokio::test]
async fn exec_check_pr_status_a_state_the_module_refuses_is_a_permanent_failure() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"FUNKY_NEW_STATE\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;
    let id = review_task_with_pr(&rt, models::SubStatus::AwaitingReview).await;

    rt.exec_check_pr_status(id, "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    match msg {
        Message::Pr(crate::tui::messages::PrMessage::CheckFailed {
            permanent, error, ..
        }) => {
            assert!(permanent);
            assert!(error.contains("FUNKY_NEW_STATE"), "{error}");
        }
        other => panic!("expected CheckFailed, got {other:?}"),
    }
    assert_eq!(
        db.get_task(id).await.unwrap().sub_status,
        models::SubStatus::AwaitingReview
    );
}

/// `ReportPrUnreadable`: giving up is reported and the module marks the task.
#[tokio::test]
async fn exec_report_pr_unreadable_marks_the_task_pr_unreachable() {
    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db.clone(), tx, Arc::new(MockProcessRunner::new(vec![]))).await;
    let id = review_task_with_pr(&rt, models::SubStatus::AwaitingReview).await;

    rt.exec_report_pr_unreadable(id, "no access".into())
        .await
        .unwrap();

    assert_eq!(
        db.get_task(id).await.unwrap().sub_status,
        models::SubStatus::PrUnreachable
    );
}

#[tokio::test]
async fn exec_persist_string_setting_writes_to_db() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_persist_string_setting(&mut app, "repo_filter", "/repo1\n/repo2")
        .await;
    assert_eq!(
        rt.database.get_setting_string("repo_filter").await.unwrap(),
        Some("/repo1\n/repo2".to_string())
    );
}

#[tokio::test]
async fn exec_quick_dispatch_creates_task_and_dispatches() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    // Pre-create worktree directory so provision_worktree skips git worktree add
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-my-task")).unwrap();

    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::dispatch()
        .detecting_default_branch("main")
        .shared_runner();
    let rt = make_runtime(db.clone(), tx, mock).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "My Task".into(),
            description: "Do stuff".into(),
            repo_path: repo.to_string(),
            ..Default::default()
        },
        None,
    )
    .await;

    // Task was created in app and DB synchronously
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "My Task");
    assert_eq!(db.list_all().await.len(), 1);

    // Repo path was saved
    assert!(app.repo_paths().contains(&repo.to_string()));

    // The request is served by this host's reactor, and the Dispatched message
    // arrives asynchronously.
    serve(&rt).await;
    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::Dispatched {
                switch_focus: true,
                ..
            })
        ),
        "Expected Dispatched, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_sets_base_branch_to_repo_default() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-quick-task")).unwrap();

    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::dispatch()
        .detecting_default_branch("master")
        .shared_runner();
    let rt = make_runtime(db.clone(), tx, mock).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Quick task".into(),
            description: String::new(),
            repo_path: repo.to_string(),
            // The draft default doesn't matter — quick-dispatch resolves
            // base_branch from the repo's `origin/HEAD`.
            ..Default::default()
        },
        None,
    )
    .await;

    let stored = db.list_all().await;
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].base_branch, "master",
        "quick-dispatch should resolve and persist the repo's default branch"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_with_epic_dispatches_successfully() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-epic-task")).unwrap();

    let db = test_db().await;
    let epic = db.create_epic("My Epic", "epic desc", None).await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::dispatch()
        .detecting_default_branch("main")
        .shared_runner();
    let rt = make_runtime(db.clone(), tx, mock).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Epic Task".into(),
            description: "do stuff".into(),
            repo_path: repo.to_string(),
            ..Default::default()
        },
        Some(epic.id),
    )
    .await;

    // Task was created with epic linkage
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].epic_id, Some(epic.id));
    serve(&rt).await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
        ),
        "Expected Dispatched, got: {msg:?}"
    );
}

/// `tasks.allium: CreateQuickTask` — "A request that fails leaves the created
/// task in backlog, like any failed dispatch". Here provisioning cannot even
/// start (no repository, and nothing scripted past the default-branch probe).
#[tokio::test]
async fn exec_quick_dispatch_failure_leaves_the_created_task_in_backlog() {
    use crate::store::DispatchRequestRead;

    let db = test_db().await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("not a git repo"), // detect_default_branch
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;
    let tasks = db.list_all().await;
    let mut app = App::new(tasks);

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Fail Task".into(),
            description: "desc".into(),
            repo_path: "/nonexistent".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    // The task was created synchronously, and requested as a quick row.
    let created_id = app.tasks()[0].id;
    serve(&rt).await;

    let row = db
        .latest_dispatch_request_for_task(created_id)
        .await
        .expect("quick dispatch requests its task");
    assert_eq!(row.mode, models::DispatchMode::Quick);
    assert_eq!(row.state, models::DispatchRequestState::Failed);
    assert_eq!(
        db.get_task(created_id).await.unwrap().status,
        models::TaskStatus::Backlog
    );
}

#[tokio::test]
async fn exec_resume_sends_resumed_message() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = DispatchScript::resume().shared_runner();
    let rt = make_runtime(db.clone(), tx, mock).await;

    let mut task = create_task_returning(
        &*db,
        "Resume Me",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Running,
    )
    .await
    .unwrap();
    task.worktree = Some("/repo/.worktrees/1-resume-me".into());
    let id = task.id;

    rt.exec_resume(task.id, task.worktree.clone());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::Task(crate::tui::messages::TaskMessage::Resumed {
        id: tid,
        tmux_window,
    }) = msg
    else {
        panic!("Expected Resumed, got: {msg:?}");
    };
    assert_eq!(tid, id);
    assert_eq!(tmux_window, test_tmux_window(&format!("task-{id}")));
}

#[tokio::test]
async fn exec_resume_sends_error_on_failure() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window: not alive)
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::fail("no tmux session"), // tmux new-window fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    let task = create_task_returning(
        &*db,
        "Fail Resume",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Running,
    )
    .await
    .unwrap();
    rt.exec_resume(task.id, task.worktree.clone());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::System(crate::tui::messages::SystemMessage::Error(_))
        ),
        "Expected Error, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_kill_tmux_window_failure_does_not_send_error() {
    let db = test_db().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no such window"), // tmux kill-window fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock).await;

    rt.exec_kill_tmux_window(test_tmux_window("task-99"))
        .await
        .unwrap();

    // Channel should be empty — no error message sent
    assert!(rx.try_recv().is_err(), "Expected no message, but got one");
}

#[tokio::test]
async fn exec_patch_sub_status_updates_db() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Test".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;

    // Move task to Running first
    rt.db_write()
        .patch_task(
            id,
            &store::TaskPatch::new().status(models::TaskStatus::Running),
        )
        .await
        .unwrap();

    rt.exec_patch_sub_status(&mut app, id, models::SubStatus::NeedsInput)
        .await;

    let db_task = rt.database.get_task(id).await.unwrap();
    assert_eq!(db_task.sub_status, models::SubStatus::NeedsInput);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_patch_sub_status_shows_error_for_missing_task() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_patch_sub_status(&mut app, TaskId(999), models::SubStatus::Active)
        .await;
    assert!(app.error_popup().is_some());
}

#[tokio::test]
async fn exec_move_task_to_epic_links_and_refreshes() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .db_write()
        .create_epic("Epic", "desc", None)
        .await
        .unwrap();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;

    rt.exec_move_task_to_epic(&mut app, id, Some(epic.id)).await;

    assert_eq!(
        rt.database.get_task(id).await.unwrap().epic_id,
        Some(epic.id)
    );
    // Board reflects the new membership after refresh.
    assert_eq!(
        app.tasks().iter().find(|t| t.id == id).unwrap().epic_id,
        Some(epic.id)
    );
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_move_task_to_epic_detaches_to_none() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .db_write()
        .create_epic("Epic", "desc", None)
        .await
        .unwrap();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        Some(epic.id),
    )
    .await;
    let id = app.tasks()[0].id;

    rt.exec_move_task_to_epic(&mut app, id, None).await;

    assert_eq!(rt.database.get_task(id).await.unwrap().epic_id, None);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_move_task_to_epic_shows_error_for_missing_epic() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "T".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;

    rt.exec_move_task_to_epic(&mut app, id, Some(models::EpicId(9999)))
        .await;

    assert!(app.error_popup().is_some());
    assert_eq!(rt.database.get_task(id).await.unwrap().epic_id, None);
}
