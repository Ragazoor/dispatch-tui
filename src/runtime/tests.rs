#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

use crate::db::{CreateLearningRow, CreateTaskRequest, Database};
use crate::models::ProjectId;
use crate::process::MockProcessRunner;

/// Timeout for async receive assertions in tests.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn db_error_formats_consistently() {
    assert_eq!(
        TuiRuntime::db_error("creating task", "disk full"),
        "DB error creating task: disk full"
    );
}

#[tokio::test]
async fn setup_tmux_for_tui_renames_window_and_binds_key() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // current_pane_id (display-message)
        MockProcessRunner::ok(), // rename_window
        MockProcessRunner::ok(), // bind_key
    ]);
    setup_tmux_for_tui(&mock);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].1, vec!["display-message", "-p", "#{pane_id}"]);
    assert_eq!(calls[1].1, vec!["rename-window", "-t", "", TUI_WINDOW_NAME]);
    assert_eq!(
        calls[2].1,
        vec![
            "bind-key",
            "g",
            &format!("select-window -t {TUI_WINDOW_NAME}")
        ]
    );
}

#[tokio::test]
async fn teardown_tmux_for_tui_unbinds_and_restores_name() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // unbind_key
        MockProcessRunner::ok(), // rename_window
    ]);
    teardown_tmux_for_tui(Some("my-shell"), &mock);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, vec!["unbind-key", "g"]);
    assert_eq!(
        calls[1].1,
        vec!["rename-window", "-t", TUI_WINDOW_NAME, "my-shell"]
    );
}

#[tokio::test]
async fn teardown_tmux_for_tui_skips_rename_when_no_original_name() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // unbind_key
    ]);
    teardown_tmux_for_tui(None, &mock);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, vec!["unbind-key", "g"]);
}

fn make_runtime(
    db: Arc<dyn db::TaskStore>,
    tx: mpsc::UnboundedSender<Message>,
    runner: Arc<dyn ProcessRunner>,
) -> TuiRuntime {
    let (feed_tx, _) = mpsc::unbounded_channel();
    TuiRuntime {
        task_svc: crate::service::TaskService::new(db.clone()),
        epic_svc: crate::service::EpicService::new(db.clone()),
        feed_runner: Some(crate::feed::FeedRunner::new(
            db.clone(),
            feed_tx,
            runner.clone(),
        )),
        database: db,
        msg_tx: tx,
        runner,
        editor_session: Arc::new(std::sync::Mutex::new(None)),
        emb_svc: crate::service::embeddings::EmbeddingService::new_noop(),
    }
}

async fn test_runtime() -> (TuiRuntime, App) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let rt = make_runtime(db.clone(), tx, runner);
    let tasks = db.list_all().await.unwrap();
    let app = App::new(tasks, ProjectId(1));
    (rt, app)
}

/// Helper: create_task + get_task in one step (replaces removed trait method).
async fn create_task_returning(
    db: &dyn db::TaskStore,
    title: &str,
    description: &str,
    repo_path: &str,
    plan: Option<&str>,
    status: models::TaskStatus,
) -> anyhow::Result<models::Task> {
    let id = db
        .create_task(CreateTaskRequest {
            title,
            description,
            repo_path,
            plan,
            status,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await?;
    db.get_task(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Task {id} vanished after insert"))
}

#[tokio::test]
async fn exec_insert_task_adds_to_db_and_app() {
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
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "Test");
    assert_eq!(rt.database.list_all().await.unwrap().len(), 1);
}

#[tokio::test]
async fn exec_delete_task_removes_from_db() {
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
    rt.exec_delete_task(&mut app, id).await;
    assert!(rt.database.list_all().await.unwrap().is_empty());
}

#[tokio::test]
async fn exec_persist_task_saves_status_to_db() {
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
    let mut task = app.tasks()[0].clone();
    task.status = models::TaskStatus::Running;
    task.sub_status = models::SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-test".into());
    rt.exec_persist_task(&mut app, task).await;
    let db_task = rt
        .database
        .get_task(app.tasks()[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(db_task.status, models::TaskStatus::Running);
    assert_eq!(db_task.worktree.as_deref(), Some("/repo/.worktrees/1-test"));
}

#[tokio::test]
async fn exec_persist_task_preserves_sub_status() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "PR Task".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;
    // Put task in Review+Approved state in DB, then sync to app
    rt.database
        .patch_task(
            id,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Review)
                .sub_status(models::SubStatus::Approved)
                .pr_url(Some("https://github.com/org/repo/pull/42")),
        )
        .await
        .unwrap();
    rt.exec_refresh_from_db(&mut app).await;
    assert_eq!(app.tasks()[0].sub_status, models::SubStatus::Approved);

    // Persist the in-memory task (simulates handle_pr_review_state saving after PR approval)
    let task = app.tasks()[0].clone();
    rt.exec_persist_task(&mut app, task).await;

    // sub_status must survive the round-trip to DB
    let db_task = rt.database.get_task(id).await.unwrap().unwrap();
    assert_eq!(db_task.sub_status, models::SubStatus::Approved);
}

/// Persist must not clobber `last_pre_tool_use_at`. Hooks own that column —
/// if the TUI's in-memory snapshot races against a fresh hook write and wins,
/// the task flickers Active → Stale on the next tick.
#[tokio::test]
async fn exec_persist_task_does_not_overwrite_last_pre_tool_use_at() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Hook race".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;

    // Simulate the hook CLI writing a fresh PreToolUse timestamp directly.
    let hook_ts = chrono::Utc::now();
    rt.database
        .patch_task(
            id,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Running)
                .sub_status(models::SubStatus::Active)
                .last_pre_tool_use_at(Some(hook_ts)),
        )
        .await
        .unwrap();

    // In-memory still holds the pre-hook (NULL) snapshot. Persist it.
    let mut stale = app.tasks()[0].clone();
    stale.status = models::TaskStatus::Running;
    stale.sub_status = models::SubStatus::Active;
    stale.last_pre_tool_use_at = None;
    rt.exec_persist_task(&mut app, stale).await;

    // The hook's timestamp must survive — Persist owns status/sub_status,
    // not the activity stamp.
    let db_task = rt.database.get_task(id).await.unwrap().unwrap();
    assert_eq!(
        db_task.last_pre_tool_use_at.map(|t| t.timestamp()),
        Some(hook_ts.timestamp()),
        "Persist clobbered hook-written last_pre_tool_use_at"
    );
}

/// SeedActivity writes only `last_pre_tool_use_at`, leaving every other
/// column untouched.
#[tokio::test]
async fn exec_seed_activity_writes_only_timestamp() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Seed".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;
    rt.database
        .patch_task(
            id,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Running)
                .sub_status(models::SubStatus::NeedsInput),
        )
        .await
        .unwrap();

    let seed_at = chrono::Utc::now();
    rt.exec_seed_activity(&mut app, id, seed_at).await;

    let db_task = rt.database.get_task(id).await.unwrap().unwrap();
    assert_eq!(
        db_task.last_pre_tool_use_at.map(|t| t.timestamp()),
        Some(seed_at.timestamp())
    );
    // SeedActivity must not touch status/sub_status — those are owned by
    // the dispatch lifecycle, not the activity stamp.
    assert_eq!(db_task.status, models::TaskStatus::Running);
    assert_eq!(db_task.sub_status, models::SubStatus::NeedsInput);
}

#[tokio::test]
async fn exec_save_repo_path_updates_app_state() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_save_repo_path(&mut app, "/repo".into()).await;
    assert!(app.repo_paths().contains(&"/repo".to_string()));
}

#[tokio::test]
async fn exec_save_repo_path_expands_tilde() {
    let (rt, mut app) = test_runtime().await;
    let home = std::env::var("HOME").unwrap();
    rt.exec_save_repo_path(&mut app, "~/myrepo".into()).await;
    let expected = format!("{home}/myrepo");
    assert!(
        app.repo_paths().contains(&expected),
        "Expected repo_paths to contain '{expected}', got: {:?}",
        app.repo_paths()
    );
    // Verify the DB also has the expanded path, not the tilde version
    let db_paths = rt.database.list_repo_paths().await.unwrap();
    assert!(db_paths.contains(&expected));
    assert!(!db_paths.iter().any(|p| p.starts_with("~/")));
}

#[tokio::test]
async fn exec_refresh_from_db_syncs_external_changes() {
    let (rt, mut app) = test_runtime().await;
    // Insert directly into DB, bypassing app
    rt.database
        .create_task(CreateTaskRequest {
            title: "External",
            description: "Added via CLI",
            repo_path: "/repo",
            plan: None,
            status: models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
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
    rt.database
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "Desc",
            repo_path: "/repo",
            plan: None,
            status: models::TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();
    // Load it into app
    let cmds = rt.exec_refresh_from_db(&mut app).await;
    assert!(cmds.is_empty()); // First load — no transition

    let task = rt.database.list_all().await.unwrap()[0].clone();
    rt.database
        .patch_task(
            task.id,
            &db::TaskPatch::new().status(models::TaskStatus::Review),
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
    let (rt, mut app) = test_runtime().await;
    rt.exec_delete_task(&mut app, TaskId(999)).await;
    assert!(app.error_popup().is_some());
}

#[tokio::test]
async fn exec_jump_to_tmux_calls_select_window() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // for select-window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_jump_to_tmux(&mut app, "my-window".to_string());

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.contains(&"select-window".to_string()));
    assert!(calls[0].1.contains(&"my-window".to_string()));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_dispatch_sends_dispatched_message() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    // Create .worktrees/ and fake worktree directory so file writes succeed
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-test-task")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        // git worktree add is skipped (dir pre-created above)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

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
    rt.exec_dispatch_agent(task, models::DispatchMode::Dispatch)
        .await;

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

#[tokio::test]
async fn exec_dispatch_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: not a git repository"), // git worktree add fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

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
    rt.exec_dispatch_agent(task.clone(), models::DispatchMode::Dispatch)
        .await;

    let msg1 = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::Task(crate::tui::messages::TaskMessage::DispatchFailed(id)) if id == task.id),
        "Expected DispatchFailed, got: {msg1:?}"
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
}

#[tokio::test]
async fn exec_check_window_sends_window_gone_when_absent() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows returns other window names (not our window)
        MockProcessRunner::ok_with_stdout(b"other-window\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    rt.exec_check_window(TaskId(1), "gone-window".to_string());

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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows returns our window
        MockProcessRunner::ok_with_stdout(b"task-1\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    rt.exec_check_window(TaskId(1), "task-1".to_string());

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rx.try_recv().is_err(), "Expected no message but received one");
}

#[tokio::test]
async fn exec_jump_to_tmux_failure_shows_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no such window"), // simulate tmux failure
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_jump_to_tmux(&mut app, "nonexistent-window".to_string());

    assert!(app.error_popup().is_some());
}

#[tokio::test]
async fn exec_cleanup_detaches_when_shared() {
    let (rt, mut app) = test_runtime().await;

    // Create two tasks sharing the same worktree
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Task A".into(),
            description: "desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Task B".into(),
            description: "desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;

    let id_a = app.tasks()[0].id;
    let id_b = app.tasks()[1].id;

    let worktree = "/repo/.worktrees/1-task-a";
    rt.database
        .patch_task(
            id_a,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Running)
                .worktree(Some(worktree))
                .tmux_window(Some("task-1")),
        )
        .await
        .unwrap();
    rt.database
        .patch_task(
            id_b,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Running)
                .worktree(Some(worktree))
                .tmux_window(Some("task-1")),
        )
        .await
        .unwrap();

    // Cleanup task A — should detach only (worktree is shared)
    rt.exec_cleanup(id_a, "/repo".into(), worktree.into(), Some("task-1".into()))
        .await;

    let task_a = rt.database.get_task(id_a).await.unwrap().unwrap();
    assert!(task_a.worktree.is_none(), "task A should be detached");
    assert!(
        task_a.tmux_window.is_none(),
        "task A tmux should be cleared"
    );

    // Task B should still have the worktree
    let task_b = rt.database.get_task(id_b).await.unwrap().unwrap();
    assert_eq!(task_b.worktree.as_deref(), Some(worktree));
}

#[tokio::test]
async fn exec_finish_happy_path_sends_complete() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
        MockProcessRunner::fail(""),                  // remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main (from worktree)
        MockProcessRunner::ok(),                      // git merge --ff-only (fast-forward)
                                                      // Worktree is preserved; cleanup happens later during archive.
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Test",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Done,
    )
    .await
    .unwrap();
    let id = task.id;

    rt.exec_finish(
        id,
        "/repo".into(),
        "1-test".into(),
        "main".into(),
        "/repo/.worktrees/1-test".into(),
        None,
    )
    .await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::Task(crate::tui::messages::TaskMessage::FinishComplete(tid)) if tid == id),
        "Expected FinishComplete, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_finish_conflict_sends_failed() {
    use crate::process::exit_fail;
    use std::process::Output;

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
        MockProcessRunner::fail(""),                  // remote get-url (no remote)
        Ok(Output {
            status: exit_fail(),
            stdout: b"".to_vec(),
            stderr:
                b"CONFLICT (content): Merge conflict in file.rs\nerror: could not apply abc1234\n"
                    .to_vec(),
        }),
        MockProcessRunner::ok(), // git rebase --abort
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Test",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Done,
    )
    .await
    .unwrap();
    let id = task.id;

    rt.exec_finish(
        id,
        "/repo".into(),
        "1-test".into(),
        "main".into(),
        "/repo/.worktrees/1-test".into(),
        None,
    )
    .await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::Task(crate::tui::messages::TaskMessage::FinishFailed {
        id: tid,
        is_conflict,
        ..
    }) = msg
    else {
        panic!("Expected FinishFailed, got: {msg:?}");
    };
    assert_eq!(tid, id);
    assert!(is_conflict, "Expected is_conflict=true");
}

#[tokio::test]
async fn exec_dispatch_epic_creates_planning_subtask() {
    let (rt, mut app) = test_runtime().await;

    // Create an epic in the DB
    let epic = rt
        .database
        .create_epic("Auth redesign", "Rework login", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    rt.exec_dispatch_epic(&mut app, epic.clone()).await;

    // Planning subtask was created in DB and added to app
    assert_eq!(app.tasks().len(), 1);
    let task = &app.tasks()[0];
    assert_eq!(task.title, "Plan: Auth redesign");
    assert_eq!(task.epic_id, Some(epic.id));
    assert_eq!(task.repo_path, "/repo");
    assert_eq!(task.status, models::TaskStatus::Backlog);

    // Verify description contains epic info
    assert!(task.description.contains("Auth redesign"));
    assert!(task.description.contains("Rework login"));

    // Verify the task is also in the DB
    let db_tasks = rt.database.list_all().await.unwrap();
    assert_eq!(db_tasks.len(), 1);
    assert_eq!(db_tasks[0].title, "Plan: Auth redesign");
}

#[tokio::test]
async fn exec_finish_not_on_main_sends_failed() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"feature-branch\n"), // rev-parse HEAD (not main)
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Test",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Done,
    )
    .await
    .unwrap();
    let id = task.id;

    rt.exec_finish(
        id,
        "/repo".into(),
        "1-test".into(),
        "main".into(),
        "/repo/.worktrees/1-test".into(),
        None,
    )
    .await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::Task(crate::tui::messages::TaskMessage::FinishFailed {
        id: tid,
        is_conflict,
        ..
    }) = msg
    else {
        panic!("Expected FinishFailed, got: {msg:?}");
    };
    assert_eq!(tid, id);
    assert!(!is_conflict, "Expected is_conflict=false for not-on-main");
}

#[tokio::test]
async fn exec_send_notification_calls_notify_send() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // notify-send call
    ]));
    let rt = make_runtime(db, tx, mock.clone());
    rt.exec_send_notification("Task #1: Fix bug", "Ready for review", false);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "notify-send");
    assert!(calls[0].1.contains(&"Task #1: Fix bug".to_string()));
    assert!(calls[0].1.contains(&"Ready for review".to_string()));
}

#[tokio::test]
async fn exec_send_notification_urgent_uses_critical() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let rt = make_runtime(db, tx, mock.clone());
    rt.exec_send_notification("Task #1: Fix bug", "Agent needs your input", true);
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"critical".to_string()));
}

#[tokio::test]
async fn exec_send_notification_failure_does_not_panic() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "command not found",
    )]));
    let rt = make_runtime(db, tx, mock.clone());
    // Should not panic — just logs a warning
    rt.exec_send_notification("Task #1: Fix bug", "Ready for review", false);
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

#[tokio::test]
async fn exec_check_pr_status_sends_merged() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"MERGED\n"), // gh pr view (no review decision line)
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_check_pr_status(TaskId(1), "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        msg,
        Message::Pr(crate::tui::messages::PrMessage::Merged(TaskId(1)))
    ));
}

#[tokio::test]
async fn exec_check_pr_status_open_sends_review_state() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n"), // gh pr view
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_check_pr_status(TaskId(1), "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    match msg {
        Message::Pr(crate::tui::messages::PrMessage::ReviewState {
            id,
            review_decision,
        }) => {
            assert_eq!(id, TaskId(1));
            assert_eq!(review_decision, Some(models::ReviewDecision::Approved));
        }
        other => panic!("Expected PrReviewState, got {:?}", other),
    }
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

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // detect_default_branch (resolved to "main")
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
        // provision_worktree: dir exists so git worktree add is skipped
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l (claude command)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "My Task".into(),
            description: "Do stuff".into(),
            repo_path: repo.to_string(),
            tag: None,
            base_branch: "main".into(),
            wrap_up_mode: None,
        },
        None,
    )
    .await;

    // Task was created in app and DB synchronously
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "My Task");
    assert_eq!(db.list_all().await.unwrap().len(), 1);

    // Repo path was saved
    assert!(app.repo_paths().contains(&repo.to_string()));

    // Dispatch message arrives asynchronously
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

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // detect_default_branch resolves to master
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/master\n"),
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Quick task".into(),
            description: String::new(),
            repo_path: repo.to_string(),
            tag: None,
            // The draft default doesn't matter — quick-dispatch resolves
            // base_branch from the repo's `origin/HEAD`.
            base_branch: "main".into(),
            wrap_up_mode: None,
        },
        None,
    )
    .await;

    let stored = db.list_all().await.unwrap();
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

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db
        .create_epic("My Epic", "epic desc", repo, None, ProjectId(1))
        .await
        .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // detect_default_branch (resolved to "main")
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (claude command)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Epic Task".into(),
            description: "do stuff".into(),
            repo_path: repo.to_string(),
            tag: None,
            base_branch: "main".into(),
            wrap_up_mode: None,
        },
        Some(epic.id),
    )
    .await;

    // Task was created with epic linkage
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].epic_id, Some(epic.id));

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

#[tokio::test]
async fn exec_quick_dispatch_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("not a git repo"), // detect_default_branch
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    // /nonexistent won't have .worktrees dir, so provision_worktree fails
    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Fail Task".into(),
            description: "desc".into(),
            repo_path: "/nonexistent".into(),
            tag: None,
            base_branch: "main".into(),
            wrap_up_mode: None,
        },
        None,
    )
    .await;

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::DispatchFailed(_))
                | Message::System(crate::tui::messages::SystemMessage::Error(_))
        ),
        "Expected DispatchFailed or Error, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_failure_sends_dispatch_failed_and_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "not a git repo",
    )]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Fail Task".into(),
            description: String::new(),
            repo_path: "/nonexistent".into(),
            tag: None,
            base_branch: "main".into(),
            wrap_up_mode: None,
        },
        None,
    )
    .await;

    // The task was created synchronously
    let created_id = app.tasks()[0].id;

    let msg1 = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::Task(crate::tui::messages::TaskMessage::DispatchFailed(id)) if id == created_id),
        "Expected DispatchFailed, got: {msg1:?}"
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
}

#[tokio::test]
async fn exec_resume_sends_resumed_message() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l (claude --continue)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

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

    rt.exec_resume(task);

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
    assert_eq!(tmux_window, format!("task-{id}"));
}

#[tokio::test]
async fn exec_resume_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no tmux session"), // tmux new-window fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

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
    rt.exec_resume(task);

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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no such window"), // tmux kill-window fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    rt.exec_kill_tmux_window("task-99".to_string());

    // Give the spawned task time to complete
    tokio::time::sleep(Duration::from_millis(100)).await;

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
    rt.database
        .patch_task(
            id,
            &db::TaskPatch::new().status(models::TaskStatus::Running),
        )
        .await
        .unwrap();

    rt.exec_patch_sub_status(&mut app, id, models::SubStatus::NeedsInput)
        .await;

    let db_task = rt.database.get_task(id).await.unwrap().unwrap();
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

// -----------------------------------------------------------------------
// Filter preset tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_persist_filter_preset_saves_to_db() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_persist_filter_preset(
        &mut app,
        "my-preset",
        &["/repo1".into(), "/repo2".into()],
        "include",
    )
    .await;
    let presets = rt.database.list_filter_presets().await.unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].0, "my-preset");
    assert_eq!(presets[0].2, "include");
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_delete_filter_preset_removes_from_db() {
    let (rt, mut app) = test_runtime().await;
    rt.database
        .save_filter_preset("doomed", &["/repo".into()], "include")
        .await
        .unwrap();
    rt.exec_delete_filter_preset(&mut app, "doomed").await;
    assert!(rt.database.list_filter_presets().await.unwrap().is_empty());
    assert!(app.error_popup().is_none());
}

// -----------------------------------------------------------------------
// parse_raw_presets tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn parse_raw_presets_converts_all_paths() {
    let raw = vec![(
        "backend".to_string(),
        vec!["/a".to_string(), "/b".to_string()],
        "include".to_string(),
    )];
    let result = parse_raw_presets(raw, None);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "backend");
    assert_eq!(
        result[0].1,
        HashSet::from(["/a".to_string(), "/b".to_string()])
    );
    assert_eq!(result[0].2, RepoFilterMode::Include);
}

#[tokio::test]
async fn parse_raw_presets_filters_against_known_repos() {
    let raw = vec![(
        "backend".to_string(),
        vec!["/a".to_string(), "/b".to_string(), "/gone".to_string()],
        "exclude".to_string(),
    )];
    let known = HashSet::from(["/a".to_string(), "/b".to_string()]);
    let result = parse_raw_presets(raw, Some(&known));
    assert_eq!(
        result[0].1,
        HashSet::from(["/a".to_string(), "/b".to_string()])
    );
    assert_eq!(result[0].2, RepoFilterMode::Exclude);
}

#[tokio::test]
async fn parse_raw_presets_defaults_invalid_mode() {
    let raw = vec![("x".to_string(), vec![], "bogus".to_string())];
    let result = parse_raw_presets(raw, None);
    assert_eq!(result[0].2, RepoFilterMode::Include);
}

#[tokio::test]
async fn parse_raw_presets_empty_input() {
    let result = parse_raw_presets(vec![], None);
    assert!(result.is_empty());
}

#[tokio::test]
async fn parse_raw_presets_multiple_presets() {
    let raw = vec![
        (
            "a".to_string(),
            vec!["/x".to_string()],
            "include".to_string(),
        ),
        (
            "b".to_string(),
            vec!["/y".to_string()],
            "exclude".to_string(),
        ),
    ];
    let result = parse_raw_presets(raw, None);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].2, RepoFilterMode::Include);
    assert_eq!(result[1].2, RepoFilterMode::Exclude);
}

// -----------------------------------------------------------------------
// Repo path tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_delete_repo_path_removes_and_refreshes() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_save_repo_path(&mut app, "/repo1".into()).await;
    rt.exec_save_repo_path(&mut app, "/repo2".into()).await;
    assert_eq!(app.repo_paths().len(), 2);

    rt.exec_delete_repo_path(&mut app, "/repo1").await;
    assert_eq!(app.repo_paths().len(), 1);
    assert!(app.repo_paths().contains(&"/repo2".to_string()));
    assert!(app.error_popup().is_none());
}

// -----------------------------------------------------------------------
// Epic tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_insert_epic_creates_in_db_and_app() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_epic(
        &mut app,
        "My Epic".into(),
        "description".into(),
        "/repo".into(),
        None,
    )
    .await;
    assert_eq!(app.epics().len(), 1);
    assert_eq!(app.epics()[0].title, "My Epic");
    assert_eq!(rt.database.list_epics().await.unwrap().len(), 1);
}

#[tokio::test]
async fn exec_delete_epic_removes_from_db() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("Doomed", "bye", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.exec_delete_epic(&mut app, epic.id).await;
    assert!(rt.database.list_epics().await.unwrap().is_empty());
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_persist_epic_updates_status() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.exec_persist_epic(&mut app, epic.id, Some(models::TaskStatus::Running), None)
        .await;
    let updated = rt.database.get_epic(epic.id).await.unwrap().unwrap();
    assert_eq!(updated.status, models::TaskStatus::Running);
}

#[tokio::test]
async fn exec_persist_epic_noop_when_nothing_to_update() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    // Should return early without error
    rt.exec_persist_epic(&mut app, epic.id, None, None).await;
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_refresh_epics_from_db_syncs_to_app() {
    let (rt, mut app) = test_runtime().await;
    // Insert epic directly into DB, bypassing app
    rt.database
        .create_epic("Direct", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert!(app.epics().is_empty());
    rt.exec_refresh_epics_from_db(&mut app).await;
    assert_eq!(app.epics().len(), 1);
    assert_eq!(app.epics()[0].title, "Direct");
}

// -----------------------------------------------------------------------
// Split mode tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_enter_split_mode_opens_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1\n"), // current_pane_id
        MockProcessRunner::ok_with_stdout(b"%2\n"), // split_window_horizontal
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_enter_split_mode(&mut app);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_enter_split_mode_no_tmux_shows_status() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no server"), // current_pane_id fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_enter_split_mode(&mut app);
    assert_eq!(app.status_message(), Some("Split mode requires tmux"));
}

#[tokio::test]
async fn exec_enter_split_mode_with_task_joins_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1\n"), // current_pane_id
        MockProcessRunner::ok_with_stdout(b"%3\n"), // join_pane: display-message for source pane ID
        MockProcessRunner::ok(),                    // join_pane: join-pane command
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_enter_split_mode_with_task(&mut app, TaskId(1), "task-1");
    let calls = mock.recorded_calls();
    assert!(calls[2].1.contains(&"join-pane".to_string()));
    assert!(app.error_popup().is_none());
    assert!(app.split_active());
    assert_eq!(app.split_pinned_task_id(), Some(TaskId(1)));
}

#[tokio::test]
async fn exec_exit_split_mode_with_restore_breaks_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // break_pane_to_window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_exit_split_mode(&mut app, "%2", Some("task-1"));
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"break-pane".to_string()));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_exit_split_mode_without_restore_kills_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // kill_pane
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_exit_split_mode(&mut app, "%2", None);
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"kill-pane".to_string()));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_check_split_pane_existing_pane_no_message() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // pane_exists → display-message succeeds
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_check_split_pane(&mut app, "%2");
    // No error, no SplitPaneClosed
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_check_split_pane_gone_sends_closed() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no pane"), // pane_exists → display-message fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_check_split_pane(&mut app, "%2");
    // SplitPaneClosed was sent via app.update
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_swap_split_pane_uses_swap_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%5\n"), // pane_id_for_window (new task)
        MockProcessRunner::ok(),                    // swap-pane
        MockProcessRunner::ok(),                    // kill-window (old pane had no task)
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_swap_split_pane(&mut app, TaskId(1), "task-1", Some("%2"), None);
    let calls = mock.recorded_calls();
    // 1st call: display-message to get new pane ID
    assert!(calls[0].1.contains(&"display-message".to_string()));
    // 2nd call: swap-pane
    assert!(calls[1].1.contains(&"swap-pane".to_string()));
    // 3rd call: kill-window (no old task to rename)
    assert!(calls[2].1.contains(&"kill-window".to_string()));
    // No 4th call — focus must NOT be transferred
    assert_eq!(calls.len(), 3, "select-pane must not be called after swap");
    assert!(app.error_popup().is_none());
    assert!(app.split_active());
    assert_eq!(app.split_pinned_task_id(), Some(TaskId(1)));
}

#[tokio::test]
async fn exec_swap_split_pane_renames_old_task_window() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%5\n"), // pane_id_for_window (new task)
        MockProcessRunner::ok(),                    // swap-pane
        MockProcessRunner::ok(),                    // rename-window (old task had a window)
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().await.unwrap();
    let mut app = App::new(tasks, ProjectId(1));

    rt.exec_swap_split_pane(
        &mut app,
        TaskId(1),
        "task-new",
        Some("%2"),
        Some("task-old"),
    );
    let calls = mock.recorded_calls();
    // 3rd call should be rename-window, not kill-window
    assert!(calls[2].1.contains(&"rename-window".to_string()));
    // Verify the rename target and new name
    assert!(calls[2].1.contains(&"task-new".to_string()));
    assert!(calls[2].1.contains(&"task-old".to_string()));
    // No 4th call — focus must NOT be transferred
    assert_eq!(calls.len(), 3, "select-pane must not be called after swap");
    assert!(app.error_popup().is_none());
}

// -----------------------------------------------------------------------
// Async PR pipeline tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_merge_pr_happy_path() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // gh pr merge --squash
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_merge_pr(TaskId(1), "https://github.com/org/repo/pull/42".into());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Pr(crate::tui::messages::PrMessage::Merged(TaskId(1)))
        ),
        "Expected PrMerged, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_merge_pr_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("merge conflict"), // gh pr merge fails
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_merge_pr(TaskId(1), "https://github.com/org/repo/pull/42".into());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Pr(crate::tui::messages::PrMessage::MergeFailed { id: TaskId(1), .. })
        ),
        "Expected MergePrFailed, got: {msg:?}"
    );
}

// -----------------------------------------------------------------------
// Browser / tmux window
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_open_in_browser_calls_xdg_open() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // xdg-open
    ]));
    let rt = make_runtime(db, tx, mock.clone());

    rt.exec_open_in_browser("https://github.com/org/repo/pull/1".into());

    // Give the spawn_blocking time to run
    tokio::time::sleep(Duration::from_millis(100)).await;
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "xdg-open");
    assert!(calls[0]
        .1
        .contains(&"https://github.com/org/repo/pull/1".to_string()));
}

#[tokio::test]
async fn exec_kill_tmux_window_calls_kill() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux kill-window
    ]));
    let rt = make_runtime(db, tx, mock.clone());

    rt.exec_kill_tmux_window("task-1".into());

    tokio::time::sleep(Duration::from_millis(100)).await;
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert!(calls[0].1.contains(&"kill-window".to_string()));
    assert!(calls[0].1.contains(&"task-1".to_string()));
}

#[tokio::test]
async fn exec_kill_tmux_window_failure_is_best_effort() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "no such window",
    )]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_kill_tmux_window("gone-window".into());

    // Give the spawned task time to complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Kill-window failure is best-effort — no error message sent
    assert!(rx.try_recv().is_err(), "Expected no message, but got one");
}

// load_* init helper tests
// -----------------------------------------------------------------------

fn make_app() -> App {
    App::new(vec![], ProjectId(1))
}

#[tokio::test]
async fn load_notifications_pref_defaults_to_false_when_not_set() {
    let db = Database::open_in_memory().await.unwrap();
    let mut app = make_app();
    load_notifications_pref(&db, &mut app).await;
    assert!(!app.notifications_enabled());
}

#[tokio::test]
async fn load_notifications_pref_sets_true_when_enabled() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_bool("notifications_enabled", true)
        .await
        .unwrap();
    let mut app = make_app();
    load_notifications_pref(&db, &mut app).await;
    assert!(app.notifications_enabled());
}

fn make_app_with_two_projects() -> App {
    let mut app = App::new(vec![], ProjectId(1));
    app.update(Message::ProjectsUpdated(vec![
        crate::models::Project {
            id: ProjectId(1),
            name: "Default".into(),
            sort_order: 0,
            is_default: true,
        },
        crate::models::Project {
            id: ProjectId(2),
            name: "B".into(),
            sort_order: 1,
            is_default: false,
        },
    ]));
    app
}

#[tokio::test]
async fn load_per_project_repo_filters_no_op_when_nothing_saved() {
    let db = Database::open_in_memory().await.unwrap();
    let mut app = make_app_with_two_projects();
    load_per_project_repo_filters(&db, &mut app).await;
    assert!(app.repo_filter().is_empty());
    assert_eq!(app.repo_filter_mode(), RepoFilterMode::Include);
}

#[tokio::test]
async fn load_per_project_repo_filters_restores_active_project() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string(
        "repo_filter:1",
        &serde_json::to_string(&["/repo/a"]).unwrap(),
    )
    .await
    .unwrap();
    db.set_setting_string("repo_filter_mode:1", "exclude")
        .await
        .unwrap();
    let mut app = make_app_with_two_projects();
    app.update(Message::RepoPathsUpdated(vec!["/repo/a".into()]));
    load_per_project_repo_filters(&db, &mut app).await;
    // Active project is 1 → its filter is in app.filter
    assert_eq!(app.repo_filter(), &HashSet::from(["/repo/a".to_string()]));
    assert_eq!(app.repo_filter_mode(), RepoFilterMode::Exclude);
}

#[tokio::test]
async fn load_per_project_repo_filters_holds_other_project_filter_in_map() {
    let db = Database::open_in_memory().await.unwrap();
    // Project 2's filter saved; project 1 has nothing.
    db.set_setting_string(
        "repo_filter:2",
        &serde_json::to_string(&["/repo/b"]).unwrap(),
    )
    .await
    .unwrap();
    db.set_setting_string("repo_filter_mode:2", "include")
        .await
        .unwrap();
    let mut app = make_app_with_two_projects();
    app.update(Message::RepoPathsUpdated(vec![
        "/repo/a".into(),
        "/repo/b".into(),
    ]));
    load_per_project_repo_filters(&db, &mut app).await;
    // Active project (1) has empty filter; project 2's slot is staged.
    assert!(app.repo_filter().is_empty());
    assert!(app.has_per_project_filter(ProjectId(2)));

    // Switching to project 2 restores its filter.
    app.update(Message::SelectProject(ProjectId(2)));
    assert_eq!(app.repo_filter(), &HashSet::from(["/repo/b".to_string()]));
}

#[tokio::test]
async fn load_per_project_repo_filters_prunes_stale_paths() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string(
        "repo_filter:1",
        &serde_json::to_string(&["/repo/a", "/gone"]).unwrap(),
    )
    .await
    .unwrap();
    let mut app = make_app_with_two_projects();
    app.update(Message::RepoPathsUpdated(vec!["/repo/a".into()]));
    load_per_project_repo_filters(&db, &mut app).await;
    assert_eq!(app.repo_filter(), &HashSet::from(["/repo/a".to_string()]));
}

#[tokio::test]
async fn load_per_project_repo_filters_migrates_legacy_global_keys() {
    let db = Database::open_in_memory().await.unwrap();
    // Old global keys (pre-555) — should land in default project's slot.
    db.set_setting_string("repo_filter", &serde_json::to_string(&["/repo/a"]).unwrap())
        .await
        .unwrap();
    db.set_setting_string("repo_filter_mode", "exclude")
        .await
        .unwrap();
    let mut app = make_app_with_two_projects();
    app.update(Message::RepoPathsUpdated(vec!["/repo/a".into()]));
    load_per_project_repo_filters(&db, &mut app).await;
    // Default project (1) is active → its filter restored from legacy keys.
    assert_eq!(app.repo_filter(), &HashSet::from(["/repo/a".to_string()]));
    assert_eq!(app.repo_filter_mode(), RepoFilterMode::Exclude);
}

#[tokio::test]
async fn load_per_project_repo_filters_prefers_per_project_over_legacy() {
    let db = Database::open_in_memory().await.unwrap();
    // Both per-project key and legacy key set for default project.
    // Per-project should win (legacy is only fallback for empty slots).
    db.set_setting_string(
        "repo_filter:1",
        &serde_json::to_string(&["/repo/per-project"]).unwrap(),
    )
    .await
    .unwrap();
    db.set_setting_string(
        "repo_filter",
        &serde_json::to_string(&["/repo/legacy"]).unwrap(),
    )
    .await
    .unwrap();
    let mut app = make_app_with_two_projects();
    app.update(Message::RepoPathsUpdated(vec![
        "/repo/per-project".into(),
        "/repo/legacy".into(),
    ]));
    load_per_project_repo_filters(&db, &mut app).await;
    assert_eq!(
        app.repo_filter(),
        &HashSet::from(["/repo/per-project".to_string()])
    );
}

#[tokio::test]
async fn load_filter_presets_returns_none_on_success() {
    let db = Database::open_in_memory().await.unwrap();
    let mut app = make_app();
    let result = load_filter_presets(&db, &mut app);
    assert!(result.await.is_none());
}

#[tokio::test]
async fn load_filter_presets_loads_saved_presets() {
    let db = Database::open_in_memory().await.unwrap();
    db.save_filter_preset("backend", &["/repo/a".into()], "include")
        .await
        .unwrap();
    let mut app = make_app();
    load_filter_presets(&db, &mut app).await;
    assert_eq!(app.filter_presets().len(), 1);
    assert_eq!(app.filter_presets()[0].0, "backend");
}

#[tokio::test]
async fn apply_tmux_focus_warning_returns_none_when_enabled() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
    let result = apply_tmux_focus_warning(&mock);
    assert!(result.is_none());
}

#[tokio::test]
async fn apply_tmux_focus_warning_returns_status_info_when_disabled() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"off\n")]);
    let result = apply_tmux_focus_warning(&mock);
    assert!(matches!(
        result,
        Some(Message::System(
            crate::tui::messages::SystemMessage::StatusInfo(_)
        ))
    ));
}

mod resolve_initial_project_tests {
    use super::*;
    use crate::models::{Project, ProjectId};

    fn make_project(id: ProjectId, is_default: bool) -> Project {
        Project {
            id,
            name: format!("p{id}"),
            sort_order: id.0,
            is_default,
        }
    }

    #[tokio::test]
    async fn falls_back_to_default_when_no_saved_setting() {
        let projects = vec![
            make_project(ProjectId(1), true),
            make_project(ProjectId(2), false),
        ];
        assert_eq!(resolve_initial_project(&projects, None), ProjectId(1));
    }

    #[tokio::test]
    async fn uses_saved_project_when_it_exists() {
        let projects = vec![
            make_project(ProjectId(1), true),
            make_project(ProjectId(2), false),
        ];
        assert_eq!(
            resolve_initial_project(&projects, Some("2".to_string())),
            ProjectId(2)
        );
    }

    #[tokio::test]
    async fn falls_back_to_default_when_saved_project_deleted() {
        let projects = vec![
            make_project(ProjectId(1), true),
            make_project(ProjectId(2), false),
        ];
        assert_eq!(
            resolve_initial_project(&projects, Some("99".to_string())),
            ProjectId(1)
        );
    }

    #[tokio::test]
    async fn falls_back_to_default_when_saved_value_invalid() {
        let projects = vec![
            make_project(ProjectId(1), true),
            make_project(ProjectId(2), false),
        ];
        assert_eq!(
            resolve_initial_project(&projects, Some("not_a_number".to_string())),
            ProjectId(1)
        );
    }
}

// ---------------------------------------------------------------------------
// exec_trigger_epic_feed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_trigger_epic_feed_success() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db
        .create_epic("Security Vulnerabilities", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    let cmd = r#"echo '[{"external_id":"vuln:1","title":"CVE-1","description":"desc","status":"backlog","tag":"fix"}]'"#;
    rt.exec_trigger_epic_feed(
        epic.id,
        "Security Vulnerabilities".to_string(),
        cmd.to_string(),
    );

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("timed out waiting for FeedRefreshed")
        .expect("channel closed");
    assert!(
        matches!(
            msg,
            Message::Feed(crate::tui::messages::FeedMessage::Refreshed { count: 1, .. })
        ),
        "expected FeedRefreshed with count=1, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_trigger_epic_feed_zero_items() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db
        .create_epic("Empty Feed", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    rt.exec_trigger_epic_feed(epic.id, "Empty Feed".to_string(), "echo '[]'".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(
        matches!(
            msg,
            Message::Feed(crate::tui::messages::FeedMessage::Refreshed { count: 0, .. })
        ),
        "empty feed should still succeed with count=0, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_trigger_epic_feed_command_fails() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db
        .create_epic("Failing Feed", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    rt.exec_trigger_epic_feed(epic.id, "Failing Feed".to_string(), "exit 1".to_string());

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(
        matches!(
            msg,
            Message::Feed(crate::tui::messages::FeedMessage::Failed { .. })
        ),
        "non-zero exit should produce FeedFailed, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_trigger_epic_feed_malformed_json() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let epic = db
        .create_epic("Bad JSON Feed", "", "/repo", None, ProjectId(1))
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    rt.exec_trigger_epic_feed(
        epic.id,
        "Bad JSON Feed".to_string(),
        "echo 'not-json'".to_string(),
    );

    let msg = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(
        matches!(
            msg,
            Message::Feed(crate::tui::messages::FeedMessage::Failed { .. })
        ),
        "malformed JSON should produce FeedFailed, got: {msg:?}"
    );
}

// ── exec_open_main_session ──

#[tokio::test]
async fn exec_open_main_session_with_no_dir_shows_error() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_open_main_session(&mut app).await;
    assert!(app.error_popup().is_some());
}

#[tokio::test]
async fn exec_open_main_session_creates_window_when_no_session() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // has_window check (list-windows — window absent)
        MockProcessRunner::ok(), // new-window
        MockProcessRunner::ok(), // send-keys -l
        MockProcessRunner::ok(), // send-keys Enter
        MockProcessRunner::ok(), // select-window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let mut app = make_app();
    app.set_main_session_dir(Some("/home/user".to_string()));

    rt.exec_open_main_session(&mut app).await;

    // Session should be recorded on App.
    assert_eq!(app.main_session(), Some("dispatch-main"));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_open_main_session_attaches_to_existing_alive_session() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"dispatch-main\n"), // has_window → true
        MockProcessRunner::ok(),                               // select-window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let mut app = make_app();
    app.set_main_session_dir(Some("/home/user".to_string()));
    app.set_main_session(Some("dispatch-main".to_string()));

    rt.exec_open_main_session(&mut app).await;

    let calls = mock.recorded_calls();
    // Should NOT have called new-window — only list-windows + select-window.
    assert!(!calls
        .iter()
        .any(|(_, args)| args.contains(&"new-window".to_string())));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_open_main_session_creates_fresh_when_stored_window_is_dead() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    db.set_setting_string("main_session.window", "dispatch-main")
        .await
        .unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // has_window → false (empty list)
        MockProcessRunner::ok(), // has_window check during create path
        MockProcessRunner::ok(), // new-window
        MockProcessRunner::ok(), // send-keys -l
        MockProcessRunner::ok(), // send-keys Enter
        MockProcessRunner::ok(), // select-window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let mut app = make_app();
    app.set_main_session_dir(Some("/home/user".to_string()));
    app.set_main_session(Some("dispatch-main".to_string()));

    rt.exec_open_main_session(&mut app).await;

    // Should have cleared the stale entry and set a fresh one.
    assert_eq!(app.main_session(), Some("dispatch-main"));
    assert!(app.error_popup().is_none());
}

// ── load_main_session ──

#[tokio::test]
async fn load_main_session_sets_dir_from_db() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string("main_session.dir", "/home/user/code")
        .await
        .unwrap();
    let mock = MockProcessRunner::new(vec![]);
    let mut app = make_app();

    load_main_session(&db, &mock, &mut app).await;

    assert_eq!(app.main_session_dir(), Some("/home/user/code"));
}

#[tokio::test]
async fn load_main_session_ignores_empty_dir() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string("main_session.dir", "").await.unwrap();
    let mock = MockProcessRunner::new(vec![]);
    let mut app = make_app();

    load_main_session(&db, &mock, &mut app).await;

    assert_eq!(app.main_session_dir(), None);
}

#[tokio::test]
async fn load_main_session_sets_window_when_alive() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string("main_session.window", "dispatch-main")
        .await
        .unwrap();
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"dispatch-main\n"), // has_window → true
    ]);
    let mut app = make_app();

    load_main_session(&db, &mock, &mut app).await;

    assert_eq!(app.main_session(), Some("dispatch-main"));
}

#[tokio::test]
async fn load_main_session_clears_stale_window() {
    let db = Database::open_in_memory().await.unwrap();
    db.set_setting_string("main_session.window", "dispatch-main")
        .await
        .unwrap();
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // has_window → false
    ]);
    let mut app = make_app();

    load_main_session(&db, &mock, &mut app).await;

    assert_eq!(app.main_session(), None);
    // DB entry should be cleared.
    let stored = db.get_setting_string("main_session.window").await.unwrap();
    assert!(stored.as_deref().unwrap_or("").is_empty());
}

#[tokio::test]
async fn build_learning_injections_partitions_and_records_retrievals() {
    use crate::models::{LearningKind, LearningScope, RetrievalSource};
    use crate::service::embeddings::{serialize_embedding, EmbeddingService};

    let (rt, _app) = test_runtime().await;
    // Seed a task in the default project.
    let task = create_task_returning(
        &*rt.database,
        "title",
        "desc",
        "/repo/a",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();

    // RAG pipeline requires stored embeddings. Seed fake BLOB bytes so both
    // learnings survive the `embedding IS NULL` filter.
    let fake_emb = serialize_embedding(&[0.1f32; 384]);

    // Seed two approved learnings: one repo-scoped non-procedural, one
    // user-scoped procedural. Both should land in the dispatch list for
    // a task in /repo/a.
    let proc_id = rt
        .database
        .create_learning(CreateLearningRow {
            kind: LearningKind::Procedural,
            summary: "Always run tests before committing.",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&fake_emb),
        })
        .await
        .unwrap();
    let repo_id = rt
        .database
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "Use Arc for shared state.",
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo/a"),
            tags: &[],
            source_task_id: None,
            embedding: Some(&fake_emb),
        })
        .await
        .unwrap();

    let emb_svc = EmbeddingService::new_test();
    let injected =
        crate::dispatch::build_and_record_injections(&*rt.database, &task, &emb_svc).await;
    assert_eq!(injected.len(), 2);
    let ids: Vec<_> = injected.iter().map(|l| l.id).collect();
    assert!(ids.contains(&proc_id));
    assert!(ids.contains(&repo_id));

    let rows = rt.database.list_retrievals_for_task(task.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| matches!(r.source, RetrievalSource::PromptInjection)));
}

// ---------------------------------------------------------------------------
// parse_filter_setting tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parse_filter_setting_accepts_valid_json() {
    let mut known = HashSet::new();
    known.insert("/a".to_string());
    known.insert("/b".to_string());
    let raw = Some(r#"["/a","/b","/unknown"]"#.to_string());
    let mode = Some("exclude".to_string());

    let (repos, mode) = parse_filter_setting(raw, mode, &known);

    assert!(repos.contains("/a"));
    assert!(repos.contains("/b"));
    assert!(
        !repos.contains("/unknown"),
        "unknown paths are filtered out"
    );
    assert_eq!(mode, RepoFilterMode::Exclude);
}

#[tokio::test]
async fn parse_filter_setting_returns_default_on_invalid_json() {
    let known: HashSet<String> = HashSet::new();
    let raw = Some("not json at all".to_string());

    let (repos, mode) = parse_filter_setting(raw, None, &known);

    assert!(repos.is_empty(), "bad JSON falls back to empty set");
    assert_eq!(mode, RepoFilterMode::default());
}

#[tokio::test]
async fn parse_filter_setting_returns_default_on_invalid_mode() {
    let known: HashSet<String> = HashSet::new();
    let raw_mode = Some("not-a-mode".to_string());

    let (_, mode) = parse_filter_setting(None, raw_mode, &known);

    assert_eq!(mode, RepoFilterMode::default());
}

#[tokio::test]
async fn parse_filter_setting_empty_when_no_settings() {
    let known: HashSet<String> = HashSet::new();
    let (repos, mode) = parse_filter_setting(None, None, &known);
    assert!(repos.is_empty());
    assert_eq!(mode, RepoFilterMode::default());
}

// ---------------------------------------------------------------------------
// backfill_embeddings tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backfill_fills_missing_embeddings() {
    use crate::db::{CreateLearningRow, LearningStore};
    use crate::models::{LearningKind, LearningScope};
    use crate::service::embeddings::EmbeddingService;

    let db = Arc::new(Database::open_in_memory().await.unwrap());

    // Insert two learnings without embeddings.
    let id1 = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "always use snake_case",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    let id2 = db
        .create_learning(CreateLearningRow {
            kind: LearningKind::Pitfall,
            summary: "avoid unwrap in production",
            detail: Some("use ? or expect with a message"),
            scope: LearningScope::User,
            scope_ref: None,
            tags: &["rust".to_string()],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    // Confirm both are missing embeddings before backfill.
    let missing_before = db.list_learnings_missing_embedding().await.unwrap();
    assert_eq!(
        missing_before.len(),
        2,
        "expected 2 learnings missing embeddings"
    );

    // Run the backfill using the test stub service.
    let emb_svc = EmbeddingService::new_noop();
    let db_for_backfill: Arc<dyn crate::db::LearningStore + Send + Sync> = db.clone();
    super::backfill_embeddings(db_for_backfill, emb_svc)
        .await
        .unwrap();

    // After backfill, no learnings should be missing embeddings.
    let missing_after = db.list_learnings_missing_embedding().await.unwrap();
    assert!(
        missing_after.is_empty(),
        "expected 0 learnings missing embeddings after backfill, got {}",
        missing_after.len()
    );

    // Both learnings should now have non-empty embeddings stored.
    let l1 = db.get_learning(id1).await.unwrap().unwrap();
    let l2 = db.get_learning(id2).await.unwrap().unwrap();
    // Verify via list_all_approved_non_task_learnings which returns embeddings
    let all = db.list_all_approved_non_task_learnings().await.unwrap();
    let emb1 = all.iter().find(|(l, _)| l.id == l1.id).map(|(_, e)| e);
    let emb2 = all.iter().find(|(l, _)| l.id == l2.id).map(|(_, e)| e);
    assert!(
        emb1.is_some_and(|e| !e.is_empty()),
        "learning 1 should have embedding"
    );
    assert!(
        emb2.is_some_and(|e| !e.is_empty()),
        "learning 2 should have embedding"
    );
}

#[tokio::test]
async fn backfill_is_noop_when_no_missing_embeddings() {
    use crate::db::{CreateLearningRow, LearningStore};
    use crate::models::{LearningKind, LearningScope};
    use crate::service::embeddings::{serialize_embedding, EmbeddingService};

    let db = Arc::new(Database::open_in_memory().await.unwrap());

    // Insert a learning that already has an embedding.
    let sentinel = serialize_embedding(&vec![0.1f32; 384]);
    db.create_learning(CreateLearningRow {
        kind: LearningKind::Convention,
        summary: "already embedded",
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: &[],
        source_task_id: None,
        embedding: Some(&sentinel),
    })
    .await
    .unwrap();

    let missing_before = db.list_learnings_missing_embedding().await.unwrap();
    assert!(
        missing_before.is_empty(),
        "precondition: no missing embeddings"
    );

    // Backfill should succeed without doing any work.
    let emb_svc = EmbeddingService::new_noop();
    let db_for_backfill: Arc<dyn crate::db::LearningStore + Send + Sync> = db.clone();
    super::backfill_embeddings(db_for_backfill, emb_svc)
        .await
        .unwrap();

    let missing_after = db.list_learnings_missing_embedding().await.unwrap();
    assert!(
        missing_after.is_empty(),
        "still no missing embeddings after no-op backfill"
    );
}

// ---------------------------------------------------------------------------
// exec_refresh_task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_refresh_task_updates_app_when_task_exists() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Refresh Me".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;
    rt.database
        .patch_task(
            id,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Running)
                .sub_status(models::SubStatus::Active),
        )
        .await
        .unwrap();

    rt.exec_refresh_task(&mut app, id).await;

    assert_eq!(app.tasks()[0].status, models::TaskStatus::Running);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_refresh_task_falls_back_when_task_gone() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Gone Task".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;
    let id = app.tasks()[0].id;
    rt.database.delete_task(id).await.unwrap();

    rt.exec_refresh_task(&mut app, id).await;

    assert!(app.tasks().is_empty());
}

// ---------------------------------------------------------------------------
// exec_refresh_epic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_refresh_epic_updates_app_when_epic_exists() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.exec_refresh_epics_from_db(&mut app).await;
    rt.database
        .patch_epic(
            epic.id,
            &db::EpicPatch::new().status(models::TaskStatus::Running),
        )
        .await
        .unwrap();

    rt.exec_refresh_epic(&mut app, epic.id).await;

    assert_eq!(app.epics()[0].status, models::TaskStatus::Running);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_refresh_epic_falls_back_when_epic_gone() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("Gone Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.exec_refresh_epics_from_db(&mut app).await;
    assert_eq!(app.epics().len(), 1);

    rt.database.delete_epic(epic.id).await.unwrap();

    rt.exec_refresh_epic(&mut app, epic.id).await;

    assert!(app.epics().is_empty());
}

#[tokio::test]
async fn exec_refresh_epic_also_reloads_epic_tasks() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("Feed Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.exec_refresh_epics_from_db(&mut app).await;

    // Insert a task linked to the epic directly in DB (simulates feed-sync)
    rt.database
        .create_task(crate::db::CreateTaskRequest {
            title: "Feed Task",
            description: "from feed",
            repo_path: "/repo",
            plan: None,
            status: models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
            wrap_up_mode: None,
        })
        .await
        .unwrap();

    rt.exec_refresh_epic(&mut app, epic.id).await;

    // The new task should now be visible in app
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "Feed Task");
}

// ---------------------------------------------------------------------------
// exec_refresh_projects_from_db / exec_create_project / exec_rename_project /
// exec_delete_project / exec_reorder_project
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_refresh_projects_from_db_loads_projects() {
    let (rt, mut app) = test_runtime().await;
    assert!(app.projects().is_empty(), "app starts with no projects");

    rt.exec_refresh_projects_from_db(&mut app).await;

    assert!(!app.projects().is_empty());
    assert!(app.projects().iter().any(|p| p.is_default));
}

#[tokio::test]
async fn exec_create_project_adds_to_db_and_refreshes() {
    let (rt, mut app) = test_runtime().await;
    rt.exec_refresh_projects_from_db(&mut app).await;
    let initial_count = app.projects().len();

    rt.exec_create_project(&mut app, "My Project".into()).await;

    assert_eq!(app.projects().len(), initial_count + 1);
    assert!(app.projects().iter().any(|p| p.name == "My Project"));
    assert!(rt
        .database
        .list_projects()
        .await
        .unwrap()
        .iter()
        .any(|p| p.name == "My Project"));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_rename_project_updates_db_and_refreshes() {
    let (rt, mut app) = test_runtime().await;
    let project = rt.database.create_project("Old Name", 10).await.unwrap();
    rt.exec_refresh_projects_from_db(&mut app).await;

    rt.exec_rename_project(&mut app, project.id, "New Name".into())
        .await;

    assert!(app.projects().iter().any(|p| p.name == "New Name"));
    assert!(!app.projects().iter().any(|p| p.name == "Old Name"));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_delete_project_removes_and_refreshes() {
    let (rt, mut app) = test_runtime().await;
    let extra = rt.database.create_project("Extra", 10).await.unwrap();
    rt.exec_refresh_projects_from_db(&mut app).await;
    let initial_count = app.projects().len();

    rt.exec_delete_project(&mut app, extra.id).await;

    assert_eq!(app.projects().len(), initial_count - 1);
    assert!(!app.projects().iter().any(|p| p.id == extra.id));
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_delete_project_switches_active_project_when_deleted_is_active() {
    let (rt, mut app) = test_runtime().await;
    let extra = rt.database.create_project("Extra", 10).await.unwrap();
    rt.exec_refresh_projects_from_db(&mut app).await;
    app.update(Message::SelectProject(extra.id));
    assert_eq!(app.active_project(), extra.id);

    rt.exec_delete_project(&mut app, extra.id).await;

    let default_id = app.projects().iter().find(|p| p.is_default).unwrap().id;
    assert_eq!(app.active_project(), default_id);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_reorder_project_swaps_sort_orders() {
    let (rt, mut app) = test_runtime().await;
    // Default project has sort_order=0; create a second with sort_order=1
    let second = rt.database.create_project("Second", 1).await.unwrap();
    rt.exec_refresh_projects_from_db(&mut app).await;

    let default_id = app.projects().iter().find(|p| p.is_default).unwrap().id;

    // Move default project down (delta=1) → swaps sort_order with second
    rt.exec_reorder_project(&mut app, default_id, 1).await;

    let db_projects = rt.database.list_projects().await.unwrap();
    let db_default = db_projects.iter().find(|p| p.id == default_id).unwrap();
    let db_second = db_projects.iter().find(|p| p.id == second.id).unwrap();
    assert_eq!(db_default.sort_order, 1);
    assert_eq!(db_second.sort_order, 0);
}

// ---------------------------------------------------------------------------
// exec_toggle_epic_auto_dispatch / exec_toggle_epic_group_by_repo
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_toggle_epic_auto_dispatch_sets_flag_to_false() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("AutoDispatch Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert!(epic.auto_dispatch, "default auto_dispatch should be true");

    rt.exec_toggle_epic_auto_dispatch(&mut app, epic.id, false)
        .await;

    let updated = rt.database.get_epic(epic.id).await.unwrap().unwrap();
    assert!(!updated.auto_dispatch);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_toggle_epic_auto_dispatch_sets_flag_to_true() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("AutoDispatch Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.database
        .patch_epic(epic.id, &db::EpicPatch::new().auto_dispatch(false))
        .await
        .unwrap();

    rt.exec_toggle_epic_auto_dispatch(&mut app, epic.id, true)
        .await;

    let updated = rt.database.get_epic(epic.id).await.unwrap().unwrap();
    assert!(updated.auto_dispatch);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_toggle_epic_group_by_repo_sets_flag_to_true() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("GroupByRepo Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    assert!(!epic.group_by_repo, "default group_by_repo should be false");

    rt.exec_toggle_epic_group_by_repo(&mut app, epic.id, true)
        .await;

    let updated = rt.database.get_epic(epic.id).await.unwrap().unwrap();
    assert!(updated.group_by_repo);
    assert!(app.error_popup().is_none());
}

#[tokio::test]
async fn exec_toggle_epic_group_by_repo_sets_flag_to_false() {
    let (rt, mut app) = test_runtime().await;
    let epic = rt
        .database
        .create_epic("GroupByRepo Epic", "desc", "/repo", None, ProjectId(1))
        .await
        .unwrap();
    rt.database
        .patch_epic(epic.id, &db::EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();

    rt.exec_toggle_epic_group_by_repo(&mut app, epic.id, false)
        .await;

    let updated = rt.database.get_epic(epic.id).await.unwrap().unwrap();
    assert!(!updated.group_by_repo);
    assert!(app.error_popup().is_none());
}

// ---------------------------------------------------------------------------
// exec_save_tips_state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_save_tips_state_persists_to_db() {
    let (rt, _app) = test_runtime().await;

    rt.exec_save_tips_state(7, models::TipsShowMode::NewOnly)
        .await;

    let (seen_up_to, show_mode) = rt.database.get_tips_state().await.unwrap();
    assert_eq!(seen_up_to, 7);
    assert_eq!(show_mode, models::TipsShowMode::NewOnly);
}

// ---------------------------------------------------------------------------
// exec_insert_task / exec_quick_dispatch — project_id defaults
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_insert_task_with_epic_uses_epic_project_id() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let proj2 = db.create_project("Project 2", 2).await.unwrap();
    let epic = db
        .create_epic("My Epic", "", "/repo", None, proj2.id)
        .await
        .unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let rt = make_runtime(db.clone(), tx, runner);
    let mut app = App::new(vec![], ProjectId(1));
    rt.exec_refresh_epics_from_db(&mut app).await;

    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Cross-project task".into(),
            description: String::new(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        Some(epic.id),
    )
    .await;

    let tasks = db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].project_id, proj2.id,
        "task should inherit project_id from the epic, not the active project"
    );
}

#[tokio::test]
async fn exec_insert_task_without_epic_uses_active_project() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let proj2 = db.create_project("Project 2", 2).await.unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let rt = make_runtime(db.clone(), tx, runner);
    let mut app = App::new(vec![], proj2.id);

    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Active project task".into(),
            description: String::new(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    )
    .await;

    let tasks = db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].project_id, proj2.id,
        "task should use the TUI active project when no epic is provided"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_with_epic_uses_epic_project_id() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-cross-project-task")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let proj2 = db.create_project("Project 2", 2).await.unwrap();
    let epic = db
        .create_epic("Cross-project Epic", "", repo, None, proj2.id)
        .await
        .unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let mut app = App::new(vec![], ProjectId(1));
    rt.exec_refresh_epics_from_db(&mut app).await;

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Quick task".into(),
            description: String::new(),
            repo_path: repo.to_string(),
            ..Default::default()
        },
        Some(epic.id),
    )
    .await;

    let tasks = db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].project_id, proj2.id,
        "quick dispatch should inherit project_id from the epic, not the active project"
    );
}

// ---------------------------------------------------------------------------
// exec_check_pr_status — closed PR sends no message
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_check_pr_status_closed_sends_no_message() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().await.unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"CLOSED\n"), // gh pr view
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_check_pr_status(TaskId(1), "https://github.com/org/repo/pull/42".to_string());

    // Closed PRs intentionally produce no message
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        rx.try_recv().is_err(),
        "Expected no message for closed PR, but got one"
    );
}
