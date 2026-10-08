use super::*;
use crate::models::test_tmux_window;
use std::cell::Cell;

#[tokio::test]
async fn watch_editor_returns_saved_when_window_gone_and_read_ok() {
    let iterations = Cell::new(0);
    let outcome = watch_editor(
        || {
            let n = iterations.get();
            iterations.set(n + 1);
            n < 3
        },
        || {},
        || Ok("hello".to_string()),
    );
    assert!(matches!(outcome, EditorOutcome::Saved(s) if s == "hello"));
    // Ran 3 alive-checks (returning true) + 1 more that returned false.
    assert_eq!(iterations.get(), 4);
}

#[tokio::test]
async fn watch_editor_returns_cancelled_when_read_fails() {
    let outcome = watch_editor(
        || false,
        || {},
        || Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
    );
    assert!(matches!(outcome, EditorOutcome::Cancelled));
}

#[tokio::test]
async fn watch_editor_stops_polling_once_window_gone() {
    let iterations = Cell::new(0);
    let sleep_calls = Cell::new(0);
    watch_editor(
        || {
            iterations.set(iterations.get() + 1);
            false
        },
        || sleep_calls.set(sleep_calls.get() + 1),
        || Ok(String::new()),
    );
    // Single check, no sleeps.
    assert_eq!(iterations.get(), 1);
    assert_eq!(sleep_calls.get(), 0);
}

// --- window_alive_with_bounded_retry ---

#[test]
fn window_alive_with_bounded_retry_true_when_present() {
    use crate::process::MockProcessRunner;
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"task-42\n")]);
    let mut failures = 0;
    assert!(window_alive_with_bounded_retry(
        &test_tmux_window("task-42"),
        &mock,
        &mut failures
    ));
    assert_eq!(failures, 0);
}

#[test]
fn window_alive_with_bounded_retry_false_when_absent() {
    use crate::process::MockProcessRunner;
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"other\n")]);
    let mut failures = 0;
    assert!(!window_alive_with_bounded_retry(
        &test_tmux_window("task-42"),
        &mock,
        &mut failures
    ));
    assert_eq!(failures, 0);
}

#[test]
fn window_alive_with_bounded_retry_true_on_transient_failure() {
    use crate::process::MockProcessRunner;
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("tmux: command not found"))]);
    let mut failures = 0;
    assert!(
        window_alive_with_bounded_retry(&test_tmux_window("task-42"), &mock, &mut failures),
        "a single query failure should not be treated as the window closing"
    );
    assert_eq!(failures, 1);
}

#[test]
fn window_alive_with_bounded_retry_gives_up_after_max_consecutive_failures() {
    use crate::process::MockProcessRunner;
    let mock = MockProcessRunner::new(
        (0..MAX_CONSECUTIVE_QUERY_FAILURES)
            .map(|_| Err(anyhow::anyhow!("tmux: command not found")))
            .collect(),
    );
    let mut failures = 0;
    for _ in 0..MAX_CONSECUTIVE_QUERY_FAILURES - 1 {
        assert!(window_alive_with_bounded_retry(
            &test_tmux_window("task-42"),
            &mock,
            &mut failures
        ));
    }
    assert!(
        !window_alive_with_bounded_retry(&test_tmux_window("task-42"), &mock, &mut failures),
        "a permanently broken tmux must eventually be treated as closed, \
             or watch_editor's loop would hang forever"
    );
}

#[test]
fn window_alive_with_bounded_retry_resets_count_after_success() {
    use crate::process::MockProcessRunner;
    let mock = MockProcessRunner::new(vec![
        Err(anyhow::anyhow!("tmux: command not found")),
        MockProcessRunner::ok_with_stdout(b"task-42\n"),
    ]);
    let mut failures = 0;
    assert!(window_alive_with_bounded_retry(
        &test_tmux_window("task-42"),
        &mock,
        &mut failures
    ));
    assert_eq!(failures, 1);
    assert!(window_alive_with_bounded_retry(
        &test_tmux_window("task-42"),
        &mock,
        &mut failures
    ));
    assert_eq!(failures, 0, "a successful query should reset the counter");
}

#[tokio::test]
async fn editor_session_drop_removes_tempfile() {
    use tempfile::NamedTempFile;

    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    // Consume the NamedTempFile without deleting so only EditorSession
    // owns the file.
    let (_file, persisted) = tmp.keep().unwrap();
    assert_eq!(persisted, path);
    assert!(path.exists());

    let session = EditorSession {
        window_name: test_tmux_window("test-window"),
        temp_path: Some(path.clone()),
        cleanup_runner: None,
    };
    drop(session);
    assert!(!path.exists(), "tempfile should be removed on drop");
}

#[tokio::test]
async fn editor_session_drop_kills_tmux_window_when_runner_set() {
    use crate::process::MockProcessRunner;

    let mock = Arc::new(
        MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&["edit-window"]),
    );
    let session = EditorSession {
        window_name: test_tmux_window("edit-window"),
        temp_path: None,
        cleanup_runner: Some(mock.clone()),
    };
    drop(session);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    // Targeted by the window's resolved pane ID — see `tmux::window_target`.
    assert_eq!(
        calls[0].1,
        vec!["kill-window", "-t", &mock.pane_id_of("edit-window")]
    );
}

#[test]
fn emit_pop_out_error_surfaces_error_popup() {
    let mut app = App::new(vec![]);
    emit_pop_out_error(&mut app, "boom".to_string());
    let msg = app.error_popup().unwrap_or_default();
    assert!(
        msg.contains("boom"),
        "expected 'boom' in error popup, got {msg:?}"
    );
}

#[tokio::test]
async fn saved_text_extracts_from_saved() {
    assert_eq!(
        saved_text(EditorOutcome::Saved("x".into())),
        Some("x".into())
    );
}

#[tokio::test]
async fn saved_text_returns_none_for_cancelled() {
    assert_eq!(saved_text(EditorOutcome::Cancelled), None);
}

#[tokio::test]
async fn editor_already_open_msg_is_stable() {
    // Pinned so a future rename is a deliberate act, not an accident.
    assert_eq!(
        EDITOR_ALREADY_OPEN_MSG,
        "Editor already open — close it first"
    );
}

// --- TuiRuntime-level tests -------------------------------------------
//
// The watcher task inside exec_pop_out_editor is async (spawn_blocking);
// these tests cover the synchronous parts: the guard and the
// finalize-result dispatch. The watcher itself is covered by the pure
// watch_editor tests above.

use crate::models::TaskStatus;
use crate::process::MockProcessRunner;
use crate::store::{CreateTaskRequest, RepoConfigRead, Store, TaskRead};
use crate::tui::{App, EditKind};
use tokio::sync::mpsc::unbounded_channel;

/// Build a `TuiRuntime` for these editor tests.
///
/// One fixture rather than a literal per test: every `TuiRuntime` field
/// addition would otherwise be a nine-site edit, and one of those fields —
/// `feed_sync_guard` — has to be the `FeedRunner`'s own registry or the two
/// feed surfaces silently stop serialising against each other. Wiring that
/// correctly once beats warning about it nine times.
fn editor_runtime(
    db: Arc<Store>,
    runner: Arc<dyn ProcessRunner>,
    msg_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::Message>,
) -> TuiRuntime {
    editor_runtime_on_host(db, runner, msg_tx, "test-host")
}

/// [`editor_runtime`], with this board's host id overridable — needed by
/// tests that drive `core/PollOwner`-dependent behaviour (`epics.allium:
/// EditEpic`'s take-over prompt). The in-memory store claims as `test-host`,
/// so a runtime on any other host sees that claim as a foreign owner.
pub(super) fn editor_runtime_on_host(
    db: Arc<Store>,
    runner: Arc<dyn ProcessRunner>,
    msg_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::Message>,
    host_id: &str,
) -> TuiRuntime {
    let board_reads: Arc<dyn crate::sync::BoardReads> = db.clone();
    let db: Arc<dyn crate::store::TaskStore> = db;
    let (feed_tx, _) = unbounded_channel();
    let feed_board_reads = board_reads.clone();
    let feed_runner = crate::feed::FeedRunner::new(
        db.clone(),
        feed_tx,
        runner.clone(),
        feed_board_reads,
        "test-host".into(),
    );
    let feed_sync_guard = feed_runner.sync_guard();
    TuiRuntime {
        task_svc: Arc::new(crate::service::TaskService::new(db.clone(), runner.clone())),
        epic_svc: Arc::new(crate::service::EpicService::new(db.clone(), db.clone())),
        feed_runner: Some(feed_runner),
        // Never started by these fixtures — see the field's doc comment.
        feed_sync_guard,
        feed_invalidate_tx: None,
        learning_svc: Arc::new(crate::service::MockLearningService),
        feed_db: db.clone(),
        board_reads,
        host_id: host_id.to_string(),
        database: db,
        msg_tx,
        runner,
        editor_session: Arc::new(Mutex::new(None)),
        emb_svc: EmbeddingService::new_noop(),
        last_change_count: Arc::new(std::sync::atomic::AtomicI64::new(-1)),
        budget_snapshot_path: std::path::PathBuf::from("/nonexistent-test-path/rate-limits.json"),
        claude_json_path: std::path::PathBuf::from("/nonexistent-test-path/.claude.json"),
        split_restores: std::sync::Mutex::new(Vec::new()),
    }
}

async fn runtime_with_runner(runner: Arc<dyn ProcessRunner>) -> (TuiRuntime, App) {
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db, runner.clone(), tx);
    let app = App::new(vec![]);
    (rt, app)
}

#[tokio::test]
async fn exec_pop_out_editor_is_noop_when_session_occupied() {
    let mock = Arc::new(MockProcessRunner::new(vec![]));
    let (rt, mut app) = runtime_with_runner(mock.clone()).await;

    // Pre-populate the session slot.
    *rt.editor_session.lock().unwrap() = Some(EditorSession {
        window_name: test_tmux_window("already-open"),
        temp_path: None,
        cleanup_runner: None,
    });

    rt.exec_pop_out_editor(&mut app, EditKind::Description { is_epic: false });

    // No tmux calls should have been issued.
    assert_eq!(mock.recorded_calls().len(), 0);
    // A status message should surface the "already open" notice.
    let msg = app.status_message().unwrap_or_default();
    assert!(
        msg.contains("Editor already open"),
        "expected 'Editor already open' in status, got {msg:?}"
    );
}

/// The pop-out editor and the agent-tree editor pane resolve one setting,
/// so they must resolve it the same way: `crate::editor::editor_from_env`.
/// Asserted as argv *elements* — a multi-word `$EDITOR` ("vim -p") passed
/// as a single string would be looked up as a binary of that name and fail
/// to launch, which is the bug this pins.
///
/// Written against whatever the ambient environment resolves to rather than
/// a fixed editor name: `std::env::set_var` is `unsafe` and races the other
/// harness threads, so the environment is read here, never written.
#[tokio::test]
async fn exec_pop_out_editor_launches_the_resolved_editor_argv() {
    let mock = Arc::new(
        MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
            MockProcessRunner::ok(), // new-window
            MockProcessRunner::ok(), // select-window (focus the editor)
            MockProcessRunner::ok(), // watcher: list-windows -> none, so it exits
            MockProcessRunner::ok(), // select-window (focus back to the TUI)
        ])
        // Window-name lookups are answered out of band, so the session's
        // best-effort kill-window on drop cannot exhaust the queue above.
        .with_windows(&[]),
    );
    let (rt, mut app) = runtime_with_runner(mock.clone()).await;

    rt.exec_pop_out_editor(&mut app, EditKind::Description { is_epic: false });

    // calls[0] is `new_window_running`'s duplicate-name `list-windows`
    // query; calls[1] is the create. Both issue synchronously, before the
    // watcher thread starts.
    let calls = mock.recorded_calls();
    assert_eq!(calls[1].0, "tmux");
    let args = &calls[1].1;
    assert_eq!(args[0], "new-window");
    let sep = args
        .iter()
        .position(|a| a == "--")
        .unwrap_or_else(|| panic!("no exec separator in {args:?}"));
    let expected = crate::editor::editor_from_env();
    assert_eq!(
        args[sep + 1..args.len() - 1],
        expected[..],
        "the editor must reach tmux as the resolved argv; got {args:?}"
    );
    assert!(
        args[args.len() - 1].ends_with(".md"),
        "the tempfile must be the final argv element; got {args:?}"
    );
}

async fn seed_task(db: &dyn crate::store::TaskStore) -> models::Task {
    let id = db
        .create_task(CreateTaskRequest {
            title: "Original title",
            description: "Original desc",
            repo_path: "/orig/repo",
            plan: Some("docs/plan.md"),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,

            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    db.get_task(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn finalize_task_edit_persists_changes() {
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await;

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    let mut app = App::new(vec![task.clone()]);

    let edited_text = "--- TITLE ---\nNew title\n\
            --- DESCRIPTION ---\nNew description\n\
            --- REPO_PATH ---\n/new/repo\n\
            --- STATUS ---\nrunning\n\
            --- PLAN ---\n\n\
            --- TAG ---\nbug\n\
            --- BASE_BRANCH ---\n\n";

    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    // The DB row should reflect the edits.
    let updated = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(updated.title, "New title");
    assert_eq!(updated.description, "New description");
    assert_eq!(updated.repo_path, "/new/repo");
    assert_eq!(updated.status, TaskStatus::Running);
    // Empty BASE_BRANCH → preserved prior value at the runtime layer
    // (service treats None as "don't touch" rather than "clear").
    assert_eq!(updated.base_branch, "main");
}

#[tokio::test]
async fn finalize_task_edit_persists_url() {
    use crate::models::{TaskUrl, UrlType};
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await; // Backlog → no was_pr_finalisation path
    assert!(task.url.is_none());

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    let mut app = App::new(vec![task.clone()]);

    let edited_text = "--- TITLE ---\n\n\
            --- URL ---\nhttps://github.com/o/r/pull/9\n\
            --- URL_TYPE ---\npr\n";

    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    let updated = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(
        updated.url,
        Some(TaskUrl::new("https://github.com/o/r/pull/9", UrlType::Pr))
    );
    // In-memory snapshot updated too.
    assert_eq!(app.tasks()[0].url, updated.url);
}

#[tokio::test]
async fn finalize_task_edit_clears_url_when_section_emptied() {
    use crate::models::{TaskUrl, UrlType};
    use crate::service::{UpdateTaskParams, UrlUpdate};
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await;

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    // Pre-set a url on the task.
    rt.task_svc
        .update_task(
            UpdateTaskParams::for_task(task.id).url(UrlUpdate::Set(TaskUrl::new(
                "https://github.com/o/r/pull/1",
                UrlType::Pr,
            ))),
        )
        .await
        .unwrap();
    let task = db.get_task(task.id).await.unwrap().unwrap();
    assert!(task.url.is_some());
    let mut app = App::new(vec![task.clone()]);

    // URL section present but empty → clear.
    let edited_text = "--- TITLE ---\n\n--- URL ---\n\n--- URL_TYPE ---\n\n";
    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    let updated = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(updated.url, None);
    assert_eq!(app.tasks()[0].url, None);
}

#[tokio::test]
async fn finalize_task_edit_clears_plan_when_section_emptied() {
    // Regression: blanking the PLAN section must clear plan_path in the
    // DB, not just the in-memory snapshot. The editor expresses "clear"
    // via FieldUpdate::Clear, which must reach the DB patch.
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await; // seeded with plan docs/plan.md
    assert!(task.plan_path.is_some(), "precondition: task has a plan");

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    let mut app = App::new(vec![task.clone()]);

    // PLAN section present but empty → clear.
    let edited_text = "--- TITLE ---\n\n--- PLAN ---\n\n";
    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    let updated = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(updated.plan_path, None, "DB plan_path should be cleared");
    assert_eq!(app.tasks()[0].plan_path, None);
}

#[tokio::test]
async fn finalize_task_edit_clears_tag_when_section_emptied() {
    // Regression: blanking the TAG section must clear the tag in the DB,
    // not just the in-memory snapshot.
    use crate::models::TaskTag;
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await;

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    // Pre-set a tag on the task.
    rt.task_svc
        .update_task(UpdateTaskParams::for_task(task.id).tag(Some(Some(TaskTag::Bug))))
        .await
        .unwrap();
    let task = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(task.tag, Some(TaskTag::Bug), "precondition: task has a tag");
    let mut app = App::new(vec![task.clone()]);

    // TAG section present but empty → clear.
    let edited_text = "--- TITLE ---\n\n--- TAG ---\n\n";
    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    let updated = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(updated.tag, None, "DB tag should be cleared");
    assert_eq!(app.tasks()[0].tag, None);
}

#[tokio::test]
async fn finalize_task_edit_persists_new_repo_path_to_known_list() {
    // Edits that change repo_path must also add the new path to the
    // saved repo_paths list, so sibling feed items (e.g. other
    // Dependabot PRs in the same repo) can be auto-resolved.
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await;
    // Precondition: known repo_paths does not contain the new path.
    assert!(
        !db.list_repo_paths()
            .await
            .unwrap()
            .iter()
            .any(|p| p == "/new/repo"),
        "precondition: /new/repo should not be in known list yet"
    );

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    let mut app = App::new(vec![task.clone()]);

    let edited_text = "--- TITLE ---\n\n\
            --- DESCRIPTION ---\n\n\
            --- REPO_PATH ---\n/new/repo\n\
            --- STATUS ---\n\n\
            --- PLAN ---\n\n\
            --- TAG ---\n\n\
            --- BASE_BRANCH ---\n\n";

    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    let paths = db.list_repo_paths().await.unwrap();
    assert!(
        paths.iter().any(|p| p == "/new/repo"),
        "expected /new/repo in known repo_paths, got {paths:?}"
    );
}

#[tokio::test]
async fn finalize_task_edit_unchanged_repo_path_does_not_save() {
    // When repo_path is unchanged (empty section preserves the prior
    // value), we must not re-save it. Avoids spurious writes when
    // editing unrelated fields.
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await;

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    let mut app = App::new(vec![task.clone()]);

    // Title change only — REPO_PATH section is empty so the editor
    // applier preserves the prior /orig/repo value.
    let edited_text = "--- TITLE ---\nNew title\n\
            --- DESCRIPTION ---\n\n\
            --- REPO_PATH ---\n\n\
            --- STATUS ---\n\n\
            --- PLAN ---\n\n\
            --- TAG ---\n\n\
            --- BASE_BRANCH ---\n\n";

    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Saved(edited_text.into()),
    )
    .await;

    // /orig/repo was never in the known list, and a no-op edit must
    // not add it.
    let paths = db.list_repo_paths().await.unwrap();
    assert!(
        !paths.iter().any(|p| p == "/orig/repo"),
        "unchanged repo_path must not be added to known list, got {paths:?}"
    );
}

#[tokio::test]
async fn finalize_task_edit_cancelled_does_not_change_db() {
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let db = Arc::new(Store::open_in_memory().await.unwrap());
    let task = seed_task(&*db).await;

    let (tx, _rx) = unbounded_channel();
    let rt = editor_runtime(db.clone(), runner.clone(), tx);
    let mut app = App::new(vec![task.clone()]);

    rt.exec_finalize_editor_result(
        &mut app,
        EditKind::TaskEdit(Box::new(task.clone())),
        EditorOutcome::Cancelled,
    )
    .await;

    let still = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(still.title, task.title);
    assert_eq!(still.description, task.description);
}

#[tokio::test]
async fn finalize_description_kind_is_noop() {
    // Description edits are finalized inside App::update (not here).
    // If a FinalizeEditorResult with Description leaks through, it
    // should not crash or produce commands.
    let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
    let (rt, mut app) = runtime_with_runner(runner).await;
    let cmds = rt
        .exec_finalize_editor_result(
            &mut app,
            EditKind::Description { is_epic: false },
            EditorOutcome::Saved("ignored".into()),
        )
        .await;
    assert!(cmds.is_empty());
}
