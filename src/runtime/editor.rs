//! Pop-out editor: spawn `$EDITOR` in a separate tmux window while the TUI
//! keeps running, then apply the edit when the editor window closes.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::Builder as TempfileBuilder;

use super::{TuiRuntime, TUI_WINDOW_NAME};
use crate::editor::{
    apply_epic_editor_fields, apply_task_editor_fields, format_description_for_editor,
    format_editor_content, format_epic_for_editor, parse_editor_content, parse_epic_editor_output,
};
use crate::process::ProcessRunner;
use crate::service::{UpdateEpicParams, UpdateTaskParams};
use crate::tui::{App, Command, EditKind, EditorOutcome, Message};
use crate::{models, tmux};

/// Interval between `has_window` polls while waiting for the editor to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Message shown when a second editor is requested while one is already open.
pub const EDITOR_ALREADY_OPEN_MSG: &str = "Editor already open — close it first";

/// Tracks a live editor session.
///
/// The tempfile is kept alive here so that the watcher task can read it after
/// the editor closes. Dropping this struct deletes the tempfile and
/// best-effort kills the tmux window, covering TUI shutdown while an editor
/// is still open.
pub struct EditorSession {
    pub window_name: String,
    /// The temp path owning the file on disk. `Some` until the watcher task
    /// reads and consumes it.
    pub temp_path: Option<PathBuf>,
    /// Process runner used by `Drop` to best-effort kill the tmux window.
    /// `None` in tests that construct sessions without a real runner.
    cleanup_runner: Option<Arc<dyn ProcessRunner>>,
}

impl Drop for EditorSession {
    fn drop(&mut self) {
        if let Some(path) = self.temp_path.take() {
            let _ = std::fs::remove_file(&path);
        }
        if let Some(runner) = self.cleanup_runner.take() {
            let _ = tmux::kill_window(&self.window_name, &*runner);
        }
    }
}

/// Poll `is_window_alive` until it returns `false`, then read the tempfile.
/// Returns `Cancelled` if the read fails (tempfile was deleted or unreadable),
/// otherwise `Saved(content)`.
///
/// Extracted as a pure function so the polling behaviour is testable without
/// any tmux/tokio involvement.
pub fn watch_editor<FA, FS, FR>(
    mut is_window_alive: FA,
    sleep: FS,
    read_tempfile: FR,
) -> EditorOutcome
where
    FA: FnMut() -> bool,
    FS: Fn(),
    FR: FnOnce() -> io::Result<String>,
{
    while is_window_alive() {
        sleep();
    }
    match read_tempfile() {
        Ok(text) => EditorOutcome::Saved(text),
        Err(_) => EditorOutcome::Cancelled,
    }
}

/// Build the initial content and tempfile prefix for a given [`EditKind`].
///
/// For `GithubQueries` / `SecurityQueries` variants this reads from the
/// database settings layer. Returns `(prefix, content)`.
fn initial_content_for(kind: &EditKind) -> (String, String) {
    match kind {
        EditKind::TaskEdit(task) => {
            let prefix = format!("task-{}-", task.id.0);
            let content = format_editor_content(task);
            (prefix, content)
        }
        EditKind::EpicEdit(epic) => {
            let prefix = format!("epic-{}-", epic.id.0);
            let content = format_epic_for_editor(epic);
            (prefix, content)
        }
        EditKind::Description { .. } => (
            "description-".to_string(),
            format_description_for_editor(""),
        ),
    }
}

/// Generate a unique tmux window name for a new editor session.
fn new_window_name() -> String {
    // Nanoseconds since the process began are plenty unique for a single
    // dispatch run; collisions would require the same nanosecond tick.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("dispatch-edit-{nanos}")
}

impl TuiRuntime {
    /// Entry point for `Command::PopOutEditor`. Opens the editor in a new
    /// tmux window, spawns a watcher task, and emits a
    /// [`Message::EditorResult`] when the editor exits.
    pub(super) fn exec_pop_out_editor(&self, app: &mut App, kind: EditKind) {
        // Enforce "one editor at a time".
        let mut guard = match self.editor_session.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.is_some() {
            app.update(Message::StatusInfo(EDITOR_ALREADY_OPEN_MSG.to_string()));
            return;
        }

        let (prefix, content) = initial_content_for(&kind);

        // Write tempfile.
        let mut tmp = match TempfileBuilder::new()
            .prefix(&prefix)
            .suffix(".md")
            .tempfile()
        {
            Ok(f) => f,
            Err(e) => {
                app.update(Message::Error(Self::db_error(
                    "creating editor tempfile",
                    e,
                )));
                return;
            }
        };
        if let Err(e) = std::io::Write::write_all(tmp.as_file_mut(), content.as_bytes()) {
            app.update(Message::Error(Self::db_error("writing editor tempfile", e)));
            return;
        }

        let (_file, temp_path) = match tmp.keep() {
            Ok(p) => p,
            Err(e) => {
                app.update(Message::Error(Self::db_error(
                    "persisting editor tempfile",
                    e.error,
                )));
                return;
            }
        };

        let window_name = new_window_name();
        let editor_cmd = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
        let cwd = std::env::temp_dir();
        let cwd_str = cwd.to_string_lossy().into_owned();
        let temp_str = temp_path.to_string_lossy().into_owned();

        if let Err(e) = tmux::new_window_running(
            &window_name,
            &cwd_str,
            &[&editor_cmd, &temp_str],
            &*self.runner,
        ) {
            let _ = std::fs::remove_file(&temp_path);
            app.update(Message::Error(format!("Failed to open editor window: {e}")));
            return;
        }

        // Best-effort: switch tmux focus to the editor window. Failing to
        // switch isn't fatal — the window still exists.
        let _ = tmux::select_window(&window_name, &*self.runner);

        *guard = Some(EditorSession {
            window_name: window_name.clone(),
            temp_path: Some(temp_path.clone()),
            cleanup_runner: Some(self.runner.clone()),
        });
        drop(guard);

        // Spawn the watcher on a blocking thread so it doesn't tie up the
        // async runtime.
        let runner = self.runner.clone();
        let msg_tx = self.msg_tx.clone();
        let session = self.editor_session.clone();
        let window = window_name;
        let path = temp_path;
        let kind_for_result = kind;
        tokio::task::spawn_blocking(move || {
            let outcome = watch_editor(
                || tmux::has_window(&window, &*runner).unwrap_or(false),
                || std::thread::sleep(POLL_INTERVAL),
                || std::fs::read_to_string(&path),
            );

            // Restore focus to the TUI window. Best-effort.
            let _ = tmux::select_window(TUI_WINDOW_NAME, &*runner);

            clear_session_slot(&session);
            // Clean up the tempfile explicitly now that we have the contents;
            // Drop on the session would also do it, but we want it gone before
            // the handler runs so retries don't pick up a stale file.
            let _ = std::fs::remove_file(&path);

            let _ = msg_tx.send(Message::EditorResult {
                kind: kind_for_result,
                outcome,
            });
        });
    }

    /// Apply the editor result for the given [`EditKind`].
    pub(super) fn exec_finalize_editor_result(
        &self,
        app: &mut App,
        kind: EditKind,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        match kind {
            EditKind::TaskEdit(task) => self.finalize_task_edit(app, task, outcome),
            EditKind::EpicEdit(epic) => self.finalize_epic_edit(app, epic, outcome),
            EditKind::Description { .. } => {
                // Description edits are handled in the App before they reach
                // this point; seeing one here means a logic bug, but we no-op
                // to avoid panicking in production.
                tracing::warn!("FinalizeEditorResult received Description kind; ignoring");
                vec![]
            }
        }
    }

    fn finalize_task_edit(
        &self,
        app: &mut App,
        task: models::Task,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        let Some(text) = saved_text(outcome) else {
            return vec![];
        };
        let fields = parse_editor_content(&text);
        let applied = apply_task_editor_fields(&task, fields);

        let task_id = task.id;
        let plan = applied.plan_path.clone();
        let params = UpdateTaskParams::for_task(task_id.0)
            .status(applied.status)
            .plan_path(plan.clone())
            .title(applied.title.clone())
            .description(applied.description.clone())
            .repo_path(applied.repo_path.clone())
            .tag(applied.tag)
            .base_branch(applied.base_branch.clone());

        if let Err(e) = self.task_svc.update_task(params) {
            app.update(Message::Error(Self::db_error("updating task", e)));
        }
        app.update(Message::TaskEdited(crate::tui::TaskEdit {
            id: task_id,
            title: applied.title,
            description: applied.description,
            repo_path: applied.repo_path,
            status: applied.status,
            plan_path: plan,
            tag: applied.tag,
            base_branch: applied.base_branch,
        }))
    }

    fn finalize_epic_edit(
        &self,
        app: &mut App,
        epic: models::Epic,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        let Some(text) = saved_text(outcome) else {
            return vec![];
        };
        let fields = parse_epic_editor_output(&text);
        let applied = apply_epic_editor_fields(&epic, fields);

        let epic_id = epic.id;
        if let Err(e) = self.epic_svc.update_epic(UpdateEpicParams {
            epic_id: epic_id.0,
            title: Some(applied.title.clone()),
            description: Some(applied.description.clone()),
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: Some(applied.repo_path.clone()),
            auto_dispatch: None,
            feed_command: Some(applied.feed_command.clone()),
            feed_interval_secs: Some(applied.feed_interval_secs),
            project_id: None,
        }) {
            app.update(Message::Error(Self::db_error("updating epic", e)));
        }
        let mut updated = epic;
        updated.title = applied.title;
        updated.description = applied.description;
        updated.repo_path = applied.repo_path;
        if let crate::service::FieldUpdate::Set(ref cmd) = applied.feed_command {
            updated.feed_command = Some(cmd.clone());
        } else {
            updated.feed_command = None;
        }
        updated.feed_interval_secs = applied.feed_interval_secs;
        app.update(Message::EpicEdited(updated))
    }
}

/// Extract the saved text from an [`EditorOutcome`], returning `None` if
/// cancelled.
fn saved_text(outcome: EditorOutcome) -> Option<String> {
    match outcome {
        EditorOutcome::Saved(text) => Some(text),
        EditorOutcome::Cancelled => None,
    }
}

/// Best-effort clear of the session slot. Logs if the mutex is poisoned but
/// keeps going so the watcher doesn't leave the slot stuck populated.
fn clear_session_slot(slot: &Arc<Mutex<Option<EditorSession>>>) {
    match slot.lock() {
        Ok(mut g) => {
            // Take the session out and drop it outside the lock so Drop
            // side-effects (tempfile removal, kill-window) don't run while
            // holding the mutex.
            let taken = g.take();
            drop(g);
            drop(taken);
        }
        Err(poisoned) => {
            let mut g = poisoned.into_inner();
            g.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn watch_editor_returns_saved_when_window_gone_and_read_ok() {
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

    #[test]
    fn watch_editor_returns_cancelled_when_read_fails() {
        let outcome = watch_editor(
            || false,
            || {},
            || Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
        );
        assert!(matches!(outcome, EditorOutcome::Cancelled));
    }

    #[test]
    fn watch_editor_stops_polling_once_window_gone() {
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

    #[test]
    fn editor_session_drop_removes_tempfile() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // Consume the NamedTempFile without deleting so only EditorSession
        // owns the file.
        let (_file, persisted) = tmp.keep().unwrap();
        assert_eq!(persisted, path);
        assert!(path.exists());

        let session = EditorSession {
            window_name: "test-window".to_string(),
            temp_path: Some(path.clone()),
            cleanup_runner: None,
        };
        drop(session);
        assert!(!path.exists(), "tempfile should be removed on drop");
    }

    #[test]
    fn editor_session_drop_kills_tmux_window_when_runner_set() {
        use crate::process::MockProcessRunner;

        let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::ok()]));
        let session = EditorSession {
            window_name: "edit-window".to_string(),
            temp_path: None,
            cleanup_runner: Some(mock.clone()),
        };
        drop(session);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1, vec!["kill-window", "-t", "edit-window"]);
    }

    #[test]
    fn saved_text_extracts_from_saved() {
        assert_eq!(
            saved_text(EditorOutcome::Saved("x".into())),
            Some("x".into())
        );
    }

    #[test]
    fn saved_text_returns_none_for_cancelled() {
        assert_eq!(saved_text(EditorOutcome::Cancelled), None);
    }

    #[test]
    fn editor_already_open_msg_is_stable() {
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

    use crate::db::Database;
    use crate::models::TaskStatus;
    use crate::process::MockProcessRunner;
    use crate::tui::{App, EditKind};
    use tokio::sync::mpsc::unbounded_channel;

    fn runtime_with_runner(runner: Arc<dyn ProcessRunner>) -> (TuiRuntime, App) {
        let db: Arc<dyn crate::db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, _rx) = unbounded_channel();
        let (feed_tx, _) = unbounded_channel();
        let rt = TuiRuntime {
            task_svc: crate::service::TaskService::new(db.clone()),
            epic_svc: crate::service::EpicService::new(db.clone()),
            feed_runner: crate::feed::FeedRunner::new(db.clone(), feed_tx),
            database: db,
            msg_tx: tx,
            runner,
            editor_session: Arc::new(Mutex::new(None)),
        };
        let app = App::new(vec![], 1, Duration::from_secs(300));
        (rt, app)
    }

    #[test]
    fn exec_pop_out_editor_is_noop_when_session_occupied() {
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let (rt, mut app) = runtime_with_runner(mock.clone());

        // Pre-populate the session slot.
        *rt.editor_session.lock().unwrap() = Some(EditorSession {
            window_name: "already-open".into(),
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

    fn seed_task(db: &dyn crate::db::TaskStore) -> models::Task {
        let id = db
            .create_task(
                "Original title",
                "Original desc",
                "/orig/repo",
                Some("docs/plan.md"),
                TaskStatus::Backlog,
                "main",
                None,
                None,
                None,
                1,
            )
            .unwrap();
        db.get_task(id).unwrap().unwrap()
    }

    #[test]
    fn finalize_task_edit_persists_changes() {
        let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
        let db: Arc<dyn crate::db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let task = seed_task(&*db);

        let (tx, _rx) = unbounded_channel();
        let (feed_tx, _) = unbounded_channel();
        let rt = TuiRuntime {
            task_svc: crate::service::TaskService::new(db.clone()),
            epic_svc: crate::service::EpicService::new(db.clone()),
            feed_runner: crate::feed::FeedRunner::new(db.clone(), feed_tx),
            database: db.clone(),
            msg_tx: tx,
            runner,
            editor_session: Arc::new(Mutex::new(None)),
        };
        let mut app = App::new(vec![task.clone()], 1, Duration::from_secs(300));

        let edited_text = "--- TITLE ---\nNew title\n\
            --- DESCRIPTION ---\nNew description\n\
            --- REPO_PATH ---\n/new/repo\n\
            --- STATUS ---\nrunning\n\
            --- PLAN ---\n\n\
            --- TAG ---\nbug\n\
            --- BASE_BRANCH ---\n\n";

        rt.exec_finalize_editor_result(
            &mut app,
            EditKind::TaskEdit(task.clone()),
            EditorOutcome::Saved(edited_text.into()),
        );

        // The DB row should reflect the edits.
        let updated = db.get_task(task.id).unwrap().unwrap();
        assert_eq!(updated.title, "New title");
        assert_eq!(updated.description, "New description");
        assert_eq!(updated.repo_path, "/new/repo");
        assert_eq!(updated.status, TaskStatus::Running);
        // Empty BASE_BRANCH → preserved prior value at the runtime layer
        // (service treats None as "don't touch" rather than "clear").
        assert_eq!(updated.base_branch, "main");
    }

    #[test]
    fn finalize_task_edit_cancelled_does_not_change_db() {
        let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
        let db: Arc<dyn crate::db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let task = seed_task(&*db);

        let (tx, _rx) = unbounded_channel();
        let (feed_tx, _) = unbounded_channel();
        let rt = TuiRuntime {
            task_svc: crate::service::TaskService::new(db.clone()),
            epic_svc: crate::service::EpicService::new(db.clone()),
            feed_runner: crate::feed::FeedRunner::new(db.clone(), feed_tx),
            database: db.clone(),
            msg_tx: tx,
            runner,
            editor_session: Arc::new(Mutex::new(None)),
        };
        let mut app = App::new(vec![task.clone()], 1, Duration::from_secs(300));

        rt.exec_finalize_editor_result(
            &mut app,
            EditKind::TaskEdit(task.clone()),
            EditorOutcome::Cancelled,
        );

        let still = db.get_task(task.id).unwrap().unwrap();
        assert_eq!(still.title, task.title);
        assert_eq!(still.description, task.description);
    }

    #[test]
    fn finalize_description_kind_is_noop() {
        // Description edits are finalized inside App::update (not here).
        // If a FinalizeEditorResult with Description leaks through, it
        // should not crash or produce commands.
        let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
        let (rt, mut app) = runtime_with_runner(runner);
        let cmds = rt.exec_finalize_editor_result(
            &mut app,
            EditKind::Description { is_epic: false },
            EditorOutcome::Saved("ignored".into()),
        );
        assert!(cmds.is_empty());
    }
}
