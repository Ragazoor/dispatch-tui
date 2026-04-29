use super::*;

use crate::db::Database;
use crate::process::MockProcessRunner;

#[test]
fn db_error_formats_consistently() {
    assert_eq!(
        TuiRuntime::db_error("creating task", "disk full"),
        "DB error creating task: disk full"
    );
}

#[test]
fn setup_tmux_for_tui_renames_window_and_binds_key() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // rename_window
        MockProcessRunner::ok(), // bind_key
    ]);
    setup_tmux_for_tui(&mock);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, vec!["rename-window", "-t", "", TUI_WINDOW_NAME]);
    assert_eq!(
        calls[1].1,
        vec![
            "bind-key",
            "g",
            &format!("select-window -t {TUI_WINDOW_NAME}")
        ]
    );
}

#[test]
fn teardown_tmux_for_tui_unbinds_and_restores_name() {
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

#[test]
fn teardown_tmux_for_tui_skips_rename_when_no_original_name() {
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
        feed_runner: crate::feed::FeedRunner::new(db.clone(), feed_tx),
        database: db,
        msg_tx: tx,
        runner,
        editor_session: Arc::new(std::sync::Mutex::new(None)),
    }
}

fn test_runtime() -> (TuiRuntime, App) {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let rt = make_runtime(db.clone(), tx, runner);
    let tasks = db.list_all().unwrap();
    let app = App::new(tasks, 1, Duration::from_secs(300));
    (rt, app)
}

/// Helper: create_task + get_task in one step (replaces removed trait method).
fn create_task_returning(
    db: &dyn db::TaskStore,
    title: &str,
    description: &str,
    repo_path: &str,
    plan: Option<&str>,
    status: models::TaskStatus,
) -> anyhow::Result<models::Task> {
    let id = db.create_task(
        title,
        description,
        repo_path,
        plan,
        status,
        "main",
        None,
        None,
        None,
        1,
    )?;
    db.get_task(id)?
        .ok_or_else(|| anyhow::anyhow!("Task {id} vanished after insert"))
}

#[test]
fn exec_insert_task_adds_to_db_and_app() {
    let (rt, mut app) = test_runtime();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Test".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    );
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "Test");
    assert_eq!(rt.database.list_all().unwrap().len(), 1);
}

#[test]
fn exec_delete_task_removes_from_db() {
    let (rt, mut app) = test_runtime();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Test".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    );
    let id = app.tasks()[0].id;
    rt.exec_delete_task(&mut app, id);
    assert!(rt.database.list_all().unwrap().is_empty());
}

#[test]
fn exec_persist_task_saves_status_to_db() {
    let (rt, mut app) = test_runtime();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Test".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    );
    let mut task = app.tasks()[0].clone();
    task.status = models::TaskStatus::Running;
    task.sub_status = models::SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-test".into());
    rt.exec_persist_task(&mut app, task);
    let db_task = rt.database.get_task(app.tasks()[0].id).unwrap().unwrap();
    assert_eq!(db_task.status, models::TaskStatus::Running);
    assert_eq!(db_task.worktree.as_deref(), Some("/repo/.worktrees/1-test"));
}

#[test]
fn exec_persist_task_preserves_sub_status() {
    let (rt, mut app) = test_runtime();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "PR Task".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    );
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
        .unwrap();
    rt.exec_refresh_from_db(&mut app);
    assert_eq!(app.tasks()[0].sub_status, models::SubStatus::Approved);

    // Persist the in-memory task (simulates handle_pr_review_state saving after PR approval)
    let task = app.tasks()[0].clone();
    rt.exec_persist_task(&mut app, task);

    // sub_status must survive the round-trip to DB
    let db_task = rt.database.get_task(id).unwrap().unwrap();
    assert_eq!(db_task.sub_status, models::SubStatus::Approved);
}

#[test]
fn exec_save_repo_path_updates_app_state() {
    let (rt, mut app) = test_runtime();
    rt.exec_save_repo_path(&mut app, "/repo".into());
    assert!(app.repo_paths().contains(&"/repo".to_string()));
}

#[test]
fn exec_save_repo_path_expands_tilde() {
    let (rt, mut app) = test_runtime();
    let home = std::env::var("HOME").unwrap();
    rt.exec_save_repo_path(&mut app, "~/myrepo".into());
    let expected = format!("{home}/myrepo");
    assert!(
        app.repo_paths().contains(&expected),
        "Expected repo_paths to contain '{expected}', got: {:?}",
        app.repo_paths()
    );
    // Verify the DB also has the expanded path, not the tilde version
    let db_paths = rt.database.list_repo_paths().unwrap();
    assert!(db_paths.contains(&expected));
    assert!(!db_paths.iter().any(|p| p.starts_with("~/")));
}

#[test]
fn exec_refresh_from_db_syncs_external_changes() {
    let (rt, mut app) = test_runtime();
    // Insert directly into DB, bypassing app
    rt.database
        .create_task(
            "External",
            "Added via CLI",
            "/repo",
            None,
            models::TaskStatus::Backlog,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    assert!(app.tasks().is_empty());
    rt.exec_refresh_from_db(&mut app);
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "External");
}

#[test]
fn exec_refresh_from_db_returns_commands_from_refresh() {
    let (rt, mut app) = test_runtime();
    // Insert a task directly into DB as Running
    rt.database
        .create_task(
            "Test",
            "Desc",
            "/repo",
            None,
            models::TaskStatus::Running,
            "main",
            None,
            None,
            None,
            1,
        )
        .unwrap();
    // Load it into app
    let cmds = rt.exec_refresh_from_db(&mut app);
    assert!(cmds.is_empty()); // First load — no transition

    // Now update it to Review directly in DB
    let task = rt.database.list_all().unwrap()[0].clone();
    rt.database
        .patch_task(
            task.id,
            &db::TaskPatch::new().status(models::TaskStatus::Review),
        )
        .unwrap();

    app.set_notifications_enabled(true);
    // Refresh should detect the transition and return a SendNotification
    let cmds = rt.exec_refresh_from_db(&mut app);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn exec_delete_task_nonexistent_shows_error() {
    let (rt, mut app) = test_runtime();
    rt.exec_delete_task(&mut app, TaskId(999));
    assert!(app.error_popup().is_some());
}

#[test]
fn exec_jump_to_tmux_calls_select_window() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // for select-window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

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

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    rt.exec_dispatch_agent(task, models::DispatchMode::Dispatch);

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::Dispatched { .. }),
        "Expected Dispatched, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_dispatch_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    rt.exec_dispatch_agent(task.clone(), models::DispatchMode::Dispatch);

    let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::DispatchFailed(id) if id == task.id),
        "Expected DispatchFailed, got: {msg1:?}"
    );

    let msg2 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg2, Message::Error(_)),
        "Expected Error, got: {msg2:?}"
    );
}

#[tokio::test]
async fn exec_capture_tmux_sends_output() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows returns the window name
        MockProcessRunner::ok_with_stdout(b"test-window\n"),
        // window_activity: display-message returns a timestamp
        MockProcessRunner::ok_with_stdout(b"1711700000\n"),
        // capture-pane
        MockProcessRunner::ok_with_stdout(b"Hello from tmux\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    rt.exec_capture_tmux(TaskId(1), "test-window".to_string());

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::TmuxOutput {
        id,
        output,
        activity_ts,
    } = msg
    else {
        panic!("Expected TmuxOutput, got: {msg:?}");
    };
    assert_eq!(id, TaskId(1));
    assert!(output.contains("Hello from tmux"));
    assert_eq!(activity_ts, 1711700000);
}

#[tokio::test]
async fn exec_capture_tmux_window_gone() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // has_window: list-windows returns other window names (not our window)
        MockProcessRunner::ok_with_stdout(b"other-window\n"),
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    rt.exec_capture_tmux(TaskId(1), "gone-window".to_string());

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::WindowGone(TaskId(1))),
        "Expected WindowGone, got: {msg:?}"
    );
}

#[test]
fn exec_jump_to_tmux_failure_shows_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no such window"), // simulate tmux failure
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_jump_to_tmux(&mut app, "nonexistent-window".to_string());

    assert!(app.error_popup().is_some());
}

#[test]
fn exec_cleanup_detaches_when_shared() {
    let (rt, mut app) = test_runtime();

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
    );
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Task B".into(),
            description: "desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    );

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
        .unwrap();
    rt.database
        .patch_task(
            id_b,
            &db::TaskPatch::new()
                .status(models::TaskStatus::Running)
                .worktree(Some(worktree))
                .tmux_window(Some("task-1")),
        )
        .unwrap();

    // Cleanup task A — should detach only (worktree is shared)
    rt.exec_cleanup(id_a, "/repo".into(), worktree.into(), Some("task-1".into()));

    let task_a = rt.database.get_task(id_a).unwrap().unwrap();
    assert!(task_a.worktree.is_none(), "task A should be detached");
    assert!(
        task_a.tmux_window.is_none(),
        "task A tmux should be cleared"
    );

    // Task B should still have the worktree
    let task_b = rt.database.get_task(id_b).unwrap().unwrap();
    assert_eq!(task_b.worktree.as_deref(), Some(worktree));
}

#[tokio::test]
async fn exec_finish_happy_path_sends_complete() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    let id = task.id;

    rt.exec_finish(
        id,
        "/repo".into(),
        "1-test".into(),
        "main".into(),
        "/repo/.worktrees/1-test".into(),
        None,
    );

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::FinishComplete(tid) if tid == id),
        "Expected FinishComplete, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_finish_conflict_sends_failed() {
    use crate::process::exit_fail;
    use std::process::Output;

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    let id = task.id;

    rt.exec_finish(
        id,
        "/repo".into(),
        "1-test".into(),
        "main".into(),
        "/repo/.worktrees/1-test".into(),
        None,
    );

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::FinishFailed {
        id: tid,
        is_conflict,
        ..
    } = msg
    else {
        panic!("Expected FinishFailed, got: {msg:?}");
    };
    assert_eq!(tid, id);
    assert!(is_conflict, "Expected is_conflict=true");
}

#[tokio::test]
async fn exec_dispatch_epic_creates_planning_subtask() {
    let (rt, mut app) = test_runtime();

    // Create an epic in the DB
    let epic = rt
        .database
        .create_epic("Auth redesign", "Rework login", "/repo", None, 1)
        .unwrap();

    rt.exec_dispatch_epic(&mut app, epic.clone());

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
    let db_tasks = rt.database.list_all().unwrap();
    assert_eq!(db_tasks.len(), 1);
    assert_eq!(db_tasks[0].title, "Plan: Auth redesign");
}

#[tokio::test]
async fn exec_finish_not_on_main_sends_failed() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    let id = task.id;

    rt.exec_finish(
        id,
        "/repo".into(),
        "1-test".into(),
        "main".into(),
        "/repo/.worktrees/1-test".into(),
        None,
    );

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::FinishFailed {
        id: tid,
        is_conflict,
        ..
    } = msg
    else {
        panic!("Expected FinishFailed, got: {msg:?}");
    };
    assert_eq!(tid, id);
    assert!(!is_conflict, "Expected is_conflict=false for not-on-main");
}

#[test]
fn exec_send_notification_calls_notify_send() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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

#[test]
fn exec_send_notification_urgent_uses_critical() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
    let rt = make_runtime(db, tx, mock.clone());
    rt.exec_send_notification("Task #1: Fix bug", "Agent needs your input", true);
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"critical".to_string()));
}

#[test]
fn exec_send_notification_failure_does_not_panic() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "command not found",
    )]));
    let rt = make_runtime(db, tx, mock.clone());
    // Should not panic — just logs a warning
    rt.exec_send_notification("Task #1: Fix bug", "Ready for review", false);
}

#[test]
fn exec_persist_setting_writes_to_db() {
    let (rt, mut app) = test_runtime();
    rt.exec_persist_setting(&mut app, "notifications_enabled", true);
    assert_eq!(
        rt.database
            .get_setting_bool("notifications_enabled")
            .unwrap(),
        Some(true)
    );
}

#[tokio::test]
async fn exec_create_pr_happy_path() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_create_pr(
        TaskId(1),
        "/repo".to_string(),
        "1-task".to_string(),
        "main".to_string(),
        "Fix bug".to_string(),
        "Description".to_string(),
    );

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(msg, Message::PrCreated { id: TaskId(1), .. }));
}

#[tokio::test]
async fn exec_create_pr_push_fails() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: no remote"), // git push fails
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_create_pr(
        TaskId(1),
        "/repo".to_string(),
        "1-task".to_string(),
        "main".to_string(),
        "Fix bug".to_string(),
        "Description".to_string(),
    );

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(msg, Message::PrFailed { .. }));
}

#[tokio::test]
async fn exec_check_pr_status_sends_merged() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"MERGED\n"), // gh pr view (no review decision line)
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_check_pr_status(TaskId(1), "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(msg, Message::PrMerged(TaskId(1))));
}

#[tokio::test]
async fn exec_check_pr_status_open_sends_review_state() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n"), // gh pr view
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_check_pr_status(TaskId(1), "https://github.com/org/repo/pull/42".to_string());

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    match msg {
        Message::PrReviewState {
            id,
            review_decision,
        } => {
            assert_eq!(id, TaskId(1));
            assert_eq!(review_decision, Some(models::ReviewDecision::Approved));
        }
        other => panic!("Expected PrReviewState, got {:?}", other),
    }
}

#[test]
fn exec_persist_string_setting_writes_to_db() {
    let (rt, mut app) = test_runtime();
    rt.exec_persist_string_setting(&mut app, "repo_filter", "/repo1\n/repo2");
    assert_eq!(
        rt.database.get_setting_string("repo_filter").unwrap(),
        Some("/repo1\n/repo2".to_string())
    );
}

#[tokio::test]
async fn exec_quick_dispatch_creates_task_and_dispatches() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    // Pre-create worktree directory so provision_worktree skips git worktree add
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-my-task")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        // provision_worktree: dir exists so git worktree add is skipped
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l (claude command)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "My Task".into(),
            description: "Do stuff".into(),
            repo_path: repo.to_string(),
            tag: None,
            base_branch: "main".into(),
        },
        None,
    );

    // Task was created in app and DB synchronously
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].title, "My Task");
    assert_eq!(db.list_all().unwrap().len(), 1);

    // Repo path was saved
    assert!(app.repo_paths().contains(&repo.to_string()));

    // Dispatch message arrives asynchronously
    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            msg,
            Message::Dispatched {
                switch_focus: true,
                ..
            }
        ),
        "Expected Dispatched, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_with_epic_dispatches_successfully() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-epic-task")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let epic = db
        .create_epic("My Epic", "epic desc", repo, None, 1)
        .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l (claude command)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Epic Task".into(),
            description: "do stuff".into(),
            repo_path: repo.to_string(),
            tag: None,
            base_branch: "main".into(),
        },
        Some(epic.id),
    );

    // Task was created with epic linkage
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].epic_id, Some(epic.id));

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::Dispatched { .. }),
        "Expected Dispatched, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("not a git repo"), // detect_default_branch
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    // /nonexistent won't have .worktrees dir, so provision_worktree fails
    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Fail Task".into(),
            description: "desc".into(),
            repo_path: "/nonexistent".into(),
            tag: None,
            base_branch: "main".into(),
        },
        None,
    );

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::DispatchFailed(_) | Message::Error(_)),
        "Expected DispatchFailed or Error, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_quick_dispatch_failure_sends_dispatch_failed_and_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "not a git repo",
    )]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_quick_dispatch(
        &mut app,
        tui::TaskDraft {
            title: "Fail Task".into(),
            description: String::new(),
            repo_path: "/nonexistent".into(),
            tag: None,
            base_branch: "main".into(),
        },
        None,
    );

    // The task was created synchronously
    let created_id = app.tasks()[0].id;

    let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::DispatchFailed(id) if id == created_id),
        "Expected DispatchFailed, got: {msg1:?}"
    );
    let msg2 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg2, Message::Error(_)),
        "Expected Error, got: {msg2:?}"
    );
}

#[tokio::test]
async fn exec_resume_sends_resumed_message() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    task.worktree = Some("/repo/.worktrees/1-resume-me".into());
    let id = task.id;

    rt.exec_resume(task);

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Message::Resumed {
        id: tid,
        tmux_window,
    } = msg
    else {
        panic!("Expected Resumed, got: {msg:?}");
    };
    assert_eq!(tid, id);
    assert_eq!(tmux_window, format!("task-{id}"));
}

#[tokio::test]
async fn exec_resume_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    .unwrap();
    rt.exec_resume(task);

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::Error(_)),
        "Expected Error, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_kill_tmux_window_failure_does_not_send_error() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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

#[test]
fn exec_patch_sub_status_updates_db() {
    let (rt, mut app) = test_runtime();
    rt.exec_insert_task(
        &mut app,
        tui::TaskDraft {
            title: "Test".into(),
            description: "Desc".into(),
            repo_path: "/repo".into(),
            ..Default::default()
        },
        None,
    );
    let id = app.tasks()[0].id;

    // Move task to Running first
    rt.database
        .patch_task(
            id,
            &db::TaskPatch::new().status(models::TaskStatus::Running),
        )
        .unwrap();

    rt.exec_patch_sub_status(&mut app, id, models::SubStatus::NeedsInput);

    let db_task = rt.database.get_task(id).unwrap().unwrap();
    assert_eq!(db_task.sub_status, models::SubStatus::NeedsInput);
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_patch_sub_status_shows_error_for_missing_task() {
    let (rt, mut app) = test_runtime();
    rt.exec_patch_sub_status(&mut app, TaskId(999), models::SubStatus::Active);
    assert!(app.error_popup().is_some());
}

// -----------------------------------------------------------------------
// Filter preset tests
// -----------------------------------------------------------------------

#[test]
fn exec_persist_filter_preset_saves_to_db() {
    let (rt, mut app) = test_runtime();
    rt.exec_persist_filter_preset(
        &mut app,
        "my-preset",
        &["/repo1".into(), "/repo2".into()],
        "include",
    );
    let presets = rt.database.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].0, "my-preset");
    assert_eq!(presets[0].2, "include");
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_delete_filter_preset_removes_from_db() {
    let (rt, mut app) = test_runtime();
    rt.database
        .save_filter_preset("doomed", &["/repo".into()], "include")
        .unwrap();
    rt.exec_delete_filter_preset(&mut app, "doomed");
    assert!(rt.database.list_filter_presets().unwrap().is_empty());
    assert!(app.error_popup().is_none());
}

// -----------------------------------------------------------------------
// parse_raw_presets tests
// -----------------------------------------------------------------------

#[test]
fn parse_raw_presets_converts_all_paths() {
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

#[test]
fn parse_raw_presets_filters_against_known_repos() {
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

#[test]
fn parse_raw_presets_defaults_invalid_mode() {
    let raw = vec![("x".to_string(), vec![], "bogus".to_string())];
    let result = parse_raw_presets(raw, None);
    assert_eq!(result[0].2, RepoFilterMode::Include);
}

#[test]
fn parse_raw_presets_empty_input() {
    let result = parse_raw_presets(vec![], None);
    assert!(result.is_empty());
}

#[test]
fn parse_raw_presets_multiple_presets() {
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

#[test]
fn exec_delete_repo_path_removes_and_refreshes() {
    let (rt, mut app) = test_runtime();
    rt.exec_save_repo_path(&mut app, "/repo1".into());
    rt.exec_save_repo_path(&mut app, "/repo2".into());
    assert_eq!(app.repo_paths().len(), 2);

    rt.exec_delete_repo_path(&mut app, "/repo1");
    assert_eq!(app.repo_paths().len(), 1);
    assert!(app.repo_paths().contains(&"/repo2".to_string()));
    assert!(app.error_popup().is_none());
}

// -----------------------------------------------------------------------
// Epic tests
// -----------------------------------------------------------------------

#[test]
fn exec_insert_epic_creates_in_db_and_app() {
    let (rt, mut app) = test_runtime();
    rt.exec_insert_epic(
        &mut app,
        "My Epic".into(),
        "description".into(),
        "/repo".into(),
        None,
    );
    assert_eq!(app.epics().len(), 1);
    assert_eq!(app.epics()[0].title, "My Epic");
    assert_eq!(rt.database.list_epics().unwrap().len(), 1);
}

#[test]
fn exec_delete_epic_removes_from_db() {
    let (rt, mut app) = test_runtime();
    let epic = rt
        .database
        .create_epic("Doomed", "bye", "/repo", None, 1)
        .unwrap();
    rt.exec_delete_epic(&mut app, epic.id);
    assert!(rt.database.list_epics().unwrap().is_empty());
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_persist_epic_updates_status() {
    let (rt, mut app) = test_runtime();
    let epic = rt
        .database
        .create_epic("Epic", "desc", "/repo", None, 1)
        .unwrap();
    rt.exec_persist_epic(&mut app, epic.id, Some(models::TaskStatus::Running), None);
    let updated = rt.database.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.status, models::TaskStatus::Running);
}

#[test]
fn exec_persist_epic_noop_when_nothing_to_update() {
    let (rt, mut app) = test_runtime();
    let epic = rt
        .database
        .create_epic("Epic", "desc", "/repo", None, 1)
        .unwrap();
    // Should return early without error
    rt.exec_persist_epic(&mut app, epic.id, None, None);
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_refresh_epics_from_db_syncs_to_app() {
    let (rt, mut app) = test_runtime();
    // Insert epic directly into DB, bypassing app
    rt.database
        .create_epic("Direct", "desc", "/repo", None, 1)
        .unwrap();
    assert!(app.epics().is_empty());
    rt.exec_refresh_epics_from_db(&mut app);
    assert_eq!(app.epics().len(), 1);
    assert_eq!(app.epics()[0].title, "Direct");
}

#[test]
fn exec_refresh_usage_from_db_syncs_to_app() {
    let (rt, mut app) = test_runtime();
    // Just verify it doesn't error with empty DB
    rt.exec_refresh_usage_from_db(&mut app);
    assert!(app.error_popup().is_none());
}

// -----------------------------------------------------------------------
// Split mode tests
// -----------------------------------------------------------------------

#[test]
fn exec_enter_split_mode_opens_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1\n"), // current_pane_id
        MockProcessRunner::ok_with_stdout(b"%2\n"), // split_window_horizontal
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_enter_split_mode(&mut app);
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_enter_split_mode_no_tmux_shows_status() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no server"), // current_pane_id fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_enter_split_mode(&mut app);
    assert_eq!(app.status_message(), Some("Split mode requires tmux"));
}

#[test]
fn exec_enter_split_mode_with_task_joins_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1\n"), // current_pane_id
        MockProcessRunner::ok_with_stdout(b"%3\n"), // join_pane: display-message for source pane ID
        MockProcessRunner::ok(),                    // join_pane: join-pane command
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_enter_split_mode_with_task(&mut app, TaskId(1), "task-1");
    let calls = mock.recorded_calls();
    assert!(calls[2].1.contains(&"join-pane".to_string()));
    assert!(app.error_popup().is_none());
    assert!(app.split_active());
    assert_eq!(app.split_pinned_task_id(), Some(TaskId(1)));
}

#[test]
fn exec_exit_split_mode_with_restore_breaks_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // break_pane_to_window
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_exit_split_mode(&mut app, "%2", Some("task-1"));
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"break-pane".to_string()));
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_exit_split_mode_without_restore_kills_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // kill_pane
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_exit_split_mode(&mut app, "%2", None);
    let calls = mock.recorded_calls();
    assert!(calls[0].1.contains(&"kill-pane".to_string()));
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_check_split_pane_existing_pane_no_message() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // pane_exists → display-message succeeds
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_check_split_pane(&mut app, "%2");
    // No error, no SplitPaneClosed
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_check_split_pane_gone_sends_closed() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("no pane"), // pane_exists → display-message fails
    ]));
    let rt = make_runtime(db.clone(), tx, mock);
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

    rt.exec_check_split_pane(&mut app, "%2");
    // SplitPaneClosed was sent via app.update
    assert!(app.error_popup().is_none());
}

#[test]
fn exec_swap_split_pane_uses_swap_pane() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%5\n"), // pane_id_for_window (new task)
        MockProcessRunner::ok(),                    // swap-pane
        MockProcessRunner::ok(),                    // kill-window (old pane had no task)
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

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

#[test]
fn exec_swap_split_pane_renames_old_task_window() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%5\n"), // pane_id_for_window (new task)
        MockProcessRunner::ok(),                    // swap-pane
        MockProcessRunner::ok(),                    // rename-window (old task had a window)
    ]));
    let rt = make_runtime(db.clone(), tx, mock.clone());
    let tasks = db.list_all().unwrap();
    let mut app = App::new(tasks, 1, Duration::from_secs(300));

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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // gh pr merge --squash
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_merge_pr(TaskId(1), "https://github.com/org/repo/pull/42".into());

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::PrMerged(TaskId(1))),
        "Expected PrMerged, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_merge_pr_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::fail("merge conflict"), // gh pr merge fails
    ]));
    let rt = make_runtime(db, tx, mock);

    rt.exec_merge_pr(TaskId(1), "https://github.com/org/repo/pull/42".into());

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::MergePrFailed { id: TaskId(1), .. }),
        "Expected MergePrFailed, got: {msg:?}"
    );
}

// -----------------------------------------------------------------------
// Browser / tmux window
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_open_in_browser_calls_xdg_open() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
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

// -----------------------------------------------------------------------
// Brainstorm / Plan modes (via exec_dispatch_agent)
// -----------------------------------------------------------------------

#[tokio::test]
async fn exec_brainstorm_sends_dispatched_message() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-brainstorm-task")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Brainstorm Task",
        "desc",
        repo,
        None,
        models::TaskStatus::Backlog,
    )
    .unwrap();
    rt.exec_dispatch_agent(task, models::DispatchMode::Brainstorm);

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::Dispatched { .. }),
        "Expected Dispatched, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_brainstorm_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: not a git repository",
    )]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Fail",
        "desc",
        "/nonexistent",
        None,
        models::TaskStatus::Backlog,
    )
    .unwrap();
    rt.exec_dispatch_agent(task.clone(), models::DispatchMode::Brainstorm);

    let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::DispatchFailed(id) if id == task.id),
        "Expected DispatchFailed, got: {msg1:?}"
    );
}

#[tokio::test]
async fn exec_plan_sends_dispatched_message() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{repo}/.worktrees/1-plan-task")).unwrap();

    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Plan Task",
        "desc",
        repo,
        None,
        models::TaskStatus::Backlog,
    )
    .unwrap();
    rt.exec_dispatch_agent(task, models::DispatchMode::Plan);

    let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg, Message::Dispatched { .. }),
        "Expected Dispatched, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_plan_sends_error_on_failure() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: not a git repository",
    )]));
    let rt = make_runtime(db.clone(), tx, mock);

    let task = create_task_returning(
        &*db,
        "Fail",
        "desc",
        "/nonexistent",
        None,
        models::TaskStatus::Backlog,
    )
    .unwrap();
    rt.exec_dispatch_agent(task.clone(), models::DispatchMode::Plan);

    let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(msg1, Message::DispatchFailed(id) if id == task.id),
        "Expected DispatchFailed, got: {msg1:?}"
    );
}

// load_* init helper tests
// -----------------------------------------------------------------------

fn make_app() -> App {
    App::new(vec![], 1, Duration::from_secs(300))
}

#[test]
fn load_notifications_pref_defaults_to_false_when_not_set() {
    let db = Database::open_in_memory().unwrap();
    let mut app = make_app();
    load_notifications_pref(&db, &mut app);
    assert!(!app.notifications_enabled());
}

#[test]
fn load_notifications_pref_sets_true_when_enabled() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_bool("notifications_enabled", true).unwrap();
    let mut app = make_app();
    load_notifications_pref(&db, &mut app);
    assert!(app.notifications_enabled());
}

#[test]
fn load_repo_filter_mode_defaults_to_include_when_not_set() {
    let db = Database::open_in_memory().unwrap();
    let mut app = make_app();
    load_repo_filter_mode(&db, &mut app);
    assert_eq!(app.repo_filter_mode(), RepoFilterMode::Include);
}

#[test]
fn load_repo_filter_mode_restores_exclude() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_string("repo_filter_mode", "exclude")
        .unwrap();
    let mut app = make_app();
    load_repo_filter_mode(&db, &mut app);
    assert_eq!(app.repo_filter_mode(), RepoFilterMode::Exclude);
}

#[test]
fn load_repo_filter_no_op_when_not_set() {
    let db = Database::open_in_memory().unwrap();
    let mut app = make_app();
    load_repo_filter(&db, &mut app);
    assert!(app.repo_filter().is_empty());
}

#[test]
fn load_repo_filter_restores_saved_filter() {
    let db = Database::open_in_memory().unwrap();
    db.save_repo_path("/repo/a").unwrap();
    db.save_repo_path("/repo/b").unwrap();
    // register paths in app so filter intersection works
    let mut app = App::new(vec![], 1, Duration::from_secs(300));
    app.update(Message::RepoPathsUpdated(vec![
        "/repo/a".into(),
        "/repo/b".into(),
    ]));
    let filter = serde_json::to_string(&["/repo/a"]).unwrap();
    db.set_setting_string("repo_filter", &filter).unwrap();
    load_repo_filter(&db, &mut app);
    assert_eq!(app.repo_filter(), &HashSet::from(["/repo/a".to_string()]));
}

#[test]
fn load_repo_filter_prunes_stale_paths() {
    let db = Database::open_in_memory().unwrap();
    // Only /repo/a is in the app's known paths; /gone is stale
    let mut app = App::new(vec![], 1, Duration::from_secs(300));
    app.update(Message::RepoPathsUpdated(vec!["/repo/a".into()]));
    let filter = serde_json::to_string(&["/repo/a", "/gone"]).unwrap();
    db.set_setting_string("repo_filter", &filter).unwrap();
    load_repo_filter(&db, &mut app);
    assert_eq!(app.repo_filter(), &HashSet::from(["/repo/a".to_string()]));
}

#[test]
fn load_filter_presets_returns_none_on_success() {
    let db = Database::open_in_memory().unwrap();
    let mut app = make_app();
    let result = load_filter_presets(&db, &mut app);
    assert!(result.is_none());
}

#[test]
fn load_filter_presets_loads_saved_presets() {
    let db = Database::open_in_memory().unwrap();
    db.save_filter_preset("backend", &["/repo/a".into()], "include")
        .unwrap();
    let mut app = make_app();
    load_filter_presets(&db, &mut app);
    assert_eq!(app.filter_presets().len(), 1);
    assert_eq!(app.filter_presets()[0].0, "backend");
}

#[test]
fn apply_tmux_focus_warning_returns_none_when_enabled() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
    let result = apply_tmux_focus_warning(&mock);
    assert!(result.is_none());
}

#[test]
fn apply_tmux_focus_warning_returns_status_info_when_disabled() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"off\n")]);
    let result = apply_tmux_focus_warning(&mock);
    assert!(matches!(result, Some(Message::StatusInfo(_))));
}

mod resolve_initial_project_tests {
    use super::*;
    use crate::models::{Project, ProjectId};

    fn make_project(id: ProjectId, is_default: bool) -> Project {
        Project {
            id,
            name: format!("p{id}"),
            sort_order: id,
            is_default,
        }
    }

    #[test]
    fn falls_back_to_default_when_no_saved_setting() {
        let projects = vec![make_project(1, true), make_project(2, false)];
        assert_eq!(resolve_initial_project(&projects, None), 1);
    }

    #[test]
    fn uses_saved_project_when_it_exists() {
        let projects = vec![make_project(1, true), make_project(2, false)];
        assert_eq!(resolve_initial_project(&projects, Some("2".to_string())), 2);
    }

    #[test]
    fn falls_back_to_default_when_saved_project_deleted() {
        let projects = vec![make_project(1, true), make_project(2, false)];
        assert_eq!(
            resolve_initial_project(&projects, Some("99".to_string())),
            1
        );
    }

    #[test]
    fn falls_back_to_default_when_saved_value_invalid() {
        let projects = vec![make_project(1, true), make_project(2, false)];
        assert_eq!(
            resolve_initial_project(&projects, Some("not_a_number".to_string())),
            1
        );
    }
}

// ---------------------------------------------------------------------------
// exec_trigger_epic_feed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exec_trigger_epic_feed_success() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let epic = db
        .create_epic("Security Vulnerabilities", "", "/repo", None, 1)
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    let cmd = r#"echo '[{"external_id":"vuln:1","title":"CVE-1","description":"desc","status":"backlog"}]'"#;
    rt.exec_trigger_epic_feed(
        epic.id,
        "Security Vulnerabilities".to_string(),
        cmd.to_string(),
    );

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for FeedRefreshed")
        .expect("channel closed");
    assert!(
        matches!(msg, Message::FeedRefreshed { count: 1, .. }),
        "expected FeedRefreshed with count=1, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_trigger_epic_feed_zero_items() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let epic = db.create_epic("Empty Feed", "", "/repo", None, 1).unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    rt.exec_trigger_epic_feed(epic.id, "Empty Feed".to_string(), "echo '[]'".to_string());

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(
        matches!(msg, Message::FeedRefreshed { count: 0, .. }),
        "empty feed should still succeed with count=0, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_trigger_epic_feed_command_fails() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let epic = db
        .create_epic("Failing Feed", "", "/repo", None, 1)
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    rt.exec_trigger_epic_feed(epic.id, "Failing Feed".to_string(), "exit 1".to_string());

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(
        matches!(msg, Message::FeedFailed { .. }),
        "non-zero exit should produce FeedFailed, got: {msg:?}"
    );
}

#[tokio::test]
async fn exec_trigger_epic_feed_malformed_json() {
    let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
    let epic = db
        .create_epic("Bad JSON Feed", "", "/repo", None, 1)
        .unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let rt = make_runtime(db, tx, Arc::new(MockProcessRunner::new(vec![])));

    rt.exec_trigger_epic_feed(
        epic.id,
        "Bad JSON Feed".to_string(),
        "echo 'not-json'".to_string(),
    );

    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(
        matches!(msg, Message::FeedFailed { .. }),
        "malformed JSON should produce FeedFailed, got: {msg:?}"
    );
}
