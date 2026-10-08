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
    TaskEditApplied,
};
#[cfg(test)]
use crate::embeddings::EmbeddingService;
use crate::models::TmuxWindow;
use crate::process::ProcessRunner;
use crate::service::{UpdateEpicParams, UpdateTaskParams};
use crate::tui::{App, Command, EditKind, EditorOutcome, Message};
use crate::{models, tmux};

/// Interval between `has_window` polls while waiting for the editor to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Consecutive tmux query failures [`window_alive_with_bounded_retry`]
/// tolerates before giving up and reporting "not alive".
///
/// `tmux::has_window_or_assume_present` (query failure -> alive) is the right
/// default for the other liveness call sites (`exec_check_window`) because
/// those are periodic re-checks — a false
/// "alive" there just delays detection by one tick. `watch_editor`'s loop
/// below has no other exit condition, so treating a *permanently* broken
/// tmux as alive forever would hang it indefinitely; bounding the retries
/// keeps the "don't overreact to one blip" behaviour while still
/// terminating on a sustained failure.
const MAX_CONSECUTIVE_QUERY_FAILURES: u32 = 5;

/// Liveness check for [`watch_editor`]'s poll loop. On a successful query,
/// reports the real state and resets `consecutive_failures`. On a query
/// error, assumes "still alive" for up to [`MAX_CONSECUTIVE_QUERY_FAILURES`]
/// in a row, then gives up and reports "not alive".
fn window_alive_with_bounded_retry(
    window: &TmuxWindow,
    runner: &dyn ProcessRunner,
    consecutive_failures: &mut u32,
) -> bool {
    match tmux::has_window(window, runner) {
        Ok(alive) => {
            *consecutive_failures = 0;
            alive
        }
        Err(_) => {
            *consecutive_failures += 1;
            *consecutive_failures < MAX_CONSECUTIVE_QUERY_FAILURES
        }
    }
}

/// Message shown when a second editor is requested while one is already open.
pub const EDITOR_ALREADY_OPEN_MSG: &str = "Editor already open — close it first";

/// Tracks a live editor session.
///
/// The tempfile is kept alive here so that the watcher task can read it after
/// the editor closes. Dropping this struct deletes the tempfile and
/// best-effort kills the tmux window, covering TUI shutdown while an editor
/// is still open.
pub struct EditorSession {
    pub window_name: TmuxWindow,
    /// The temp path owning the file on disk. `Some` until the watcher task
    /// reads and consumes it.
    pub temp_path: Option<PathBuf>,
    /// Process runner used by `Drop` to best-effort kill the tmux window.
    /// `None` in tests that construct sessions without a real runner.
    cleanup_runner: Option<Arc<dyn ProcessRunner>>,
}

#[cfg(test)]
impl EditorSession {
    /// Test-only constructor for an *occupied* session slot: no tempfile and no
    /// cleanup runner, so `Drop` is a no-op. Lets tests outside this module
    /// (notably `runtime::tests`) exercise the "one editor at a time" guard in
    /// `exec_pop_out_editor` without spawning a real editor window.
    pub(super) fn occupied_for_test(window_name: &TmuxWindow) -> Self {
        Self {
            window_name: window_name.clone(),
            temp_path: None,
            cleanup_runner: None,
        }
    }
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

/// Surface a pop-out editor failure as a status error and let the caller
/// `return`. Funnels the several early-return error paths in
/// `exec_pop_out_editor` through one place instead of repeating the
/// `app.update(Message::System(...))` boilerplate at each site.
fn emit_pop_out_error(app: &mut App, message: String) {
    app.update(Message::System(crate::tui::messages::SystemMessage::Error(
        message,
    )));
}

/// Generate a unique tmux window name for a new editor session.
fn new_window_name() -> TmuxWindow {
    // Nanoseconds since the process began are plenty unique for a single
    // dispatch run; collisions would require the same nanosecond tick.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    TmuxWindow::for_editor(nanos)
}

impl TuiRuntime {
    /// Entry point for `EditorCommand::PopOut`. Opens the editor in a new
    /// tmux window, spawns a watcher task, and emits an
    /// [`EditorMessage::Result`] when the editor exits.
    pub(super) fn exec_pop_out_editor(&self, app: &mut App, kind: EditKind) {
        // Enforce "one editor at a time".
        let mut guard = match self.editor_session.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.is_some() {
            app.update(Message::System(
                crate::tui::messages::SystemMessage::StatusInfo(
                    EDITOR_ALREADY_OPEN_MSG.to_string(),
                ),
            ));
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
                emit_pop_out_error(app, Self::db_error("creating editor tempfile", e));
                return;
            }
        };
        if let Err(e) = std::io::Write::write_all(tmp.as_file_mut(), content.as_bytes()) {
            emit_pop_out_error(app, Self::db_error("writing editor tempfile", e));
            return;
        }

        let (_file, temp_path) = match tmp.keep() {
            Ok(p) => p,
            Err(e) => {
                emit_pop_out_error(app, Self::db_error("persisting editor tempfile", e.error));
                return;
            }
        };

        let window_name = new_window_name();
        // Same resolution as the agent-tree editor pane — one `$EDITOR` means
        // one thing (docs/specs/core.allium: `editor_fallback`). Never empty,
        // so `new_window_running`'s empty-command guard is unreachable here.
        let editor = crate::editor::editor_from_env();
        let cwd = std::env::temp_dir();
        let cwd_str = cwd.to_string_lossy().into_owned();
        let temp_str = temp_path.to_string_lossy().into_owned();

        let mut command: Vec<&str> = editor.iter().map(String::as_str).collect();
        command.push(&temp_str);

        if let Err(e) = tmux::new_window_running(&window_name, &cwd_str, &command, &*self.runner) {
            let _ = std::fs::remove_file(&temp_path);
            emit_pop_out_error(app, format!("Failed to open editor window: {e}"));
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
            let mut consecutive_failures = 0;
            let outcome = watch_editor(
                || window_alive_with_bounded_retry(&window, &*runner, &mut consecutive_failures),
                || std::thread::sleep(POLL_INTERVAL),
                || std::fs::read_to_string(&path),
            );

            // Restore focus to the TUI window. Best-effort.
            let _ = tmux::select_window(&TUI_WINDOW_NAME, &*runner);

            clear_session_slot(&session);
            // Clean up the tempfile explicitly now that we have the contents;
            // Drop on the session would also do it, but we want it gone before
            // the handler runs so retries don't pick up a stale file.
            let _ = std::fs::remove_file(&path);

            let _ = msg_tx.send(Message::Editor(
                crate::tui::messages::EditorMessage::Result {
                    kind: kind_for_result,
                    outcome,
                },
            ));
        });
    }

    /// Apply the editor result for the given [`EditKind`].
    pub(super) async fn exec_finalize_editor_result(
        &self,
        app: &mut App,
        kind: EditKind,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        match kind {
            EditKind::TaskEdit(task) => self.finalize_task_edit(app, *task, outcome).await,
            EditKind::EpicEdit(epic) => self.finalize_epic_edit(app, *epic, outcome).await,
            EditKind::Description { .. } => {
                tracing::warn!("FinalizeEditorResult received Description kind; ignoring");
                vec![]
            }
        }
    }

    async fn finalize_task_edit(
        &self,
        app: &mut App,
        task: models::Task,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        let Some(text) = saved_text(outcome) else {
            return vec![];
        };
        let mut fields = parse_editor_content(&text);
        let parse_errors = std::mem::take(&mut fields.errors);
        let applied = apply_task_editor_fields(&task, fields);
        emit_parse_errors(app, &parse_errors);

        let task_id = task.id;
        let prior_repo_path = task.repo_path.clone();

        // Single source of truth: destructure `TaskEditApplied` exhaustively
        // (no `..`) so adding an editable field is a compile error (E0027)
        // here rather than a silently-dropped field. The `UpdateTaskParams`
        // patch and the in-memory `TaskEdit` event are both derived from these
        // bindings. `resolved_plan_path`/`resolved_url` are the post-edit
        // values `editor.rs` already computed — consumed here, not re-derived.
        let TaskEditApplied {
            title,
            description,
            repo_path,
            status,
            plan_path,
            resolved_plan_path,
            tag,
            base_branch,
            wrap_up_mode,
            url,
            resolved_url,
            phoenix,
        } = applied;

        let mut params = UpdateTaskParams::for_task(task_id)
            .status(status)
            .plan_path(plan_path)
            .title(title.clone())
            .description(description.clone())
            .repo_path(repo_path.clone())
            .tag(Some(tag))
            .base_branch(base_branch.clone())
            .wrap_up_mode(wrap_up_mode)
            .phoenix(phoenix);
        // Only forward a url change when the edit actually altered it.
        if let Some(url_update) = url {
            params = params.url(url_update);
        }

        if let Err(e) = self.task_svc.update_task(params).await {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("updating task", e),
            )));
        }

        // Persist non-empty edited repo_path to the known list so sibling
        // feed items (e.g. other Dependabot PRs in the same repo) can be
        // resolved on the next feed sync.
        if !repo_path.is_empty() && repo_path != prior_repo_path {
            self.exec_save_repo_path(app, repo_path.clone()).await;
        }

        app.update(Message::Task(crate::tui::messages::TaskMessage::Edited(
            crate::tui::TaskEdit {
                id: task_id,
                title,
                description,
                repo_path,
                status,
                plan_path: resolved_plan_path,
                tag,
                base_branch,
                wrap_up_mode,
                url: resolved_url,
                phoenix,
            },
        )))
    }

    async fn finalize_epic_edit(
        &self,
        app: &mut App,
        epic: models::Epic,
        outcome: EditorOutcome,
    ) -> Vec<Command> {
        let Some(text) = saved_text(outcome) else {
            return vec![];
        };
        let mut fields = parse_epic_editor_output(&text);
        let parse_errors = std::mem::take(&mut fields.errors);
        let applied = apply_epic_editor_fields(&epic, fields);
        emit_parse_errors(app, &parse_errors);

        let epic_id = epic.id;
        // Captured before `epic` is consumed by `updated = epic` below —
        // needed to tell "changed" from "left alone" for the take-over
        // prompt (epics.allium: EditEpic).
        let original_feed_command = epic.feed_command.clone();
        if let Err(e) = self
            .epic_svc
            .update_epic(UpdateEpicParams {
                title: Some(applied.title.clone()),
                description: Some(applied.description.clone()),
                feed_command: Some(applied.feed_command.clone()),
                feed_interval_secs: Some(applied.feed_interval_secs),
                ..UpdateEpicParams::for_epic(epic_id)
            })
            .await
        {
            app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                Self::db_error("updating epic", e),
            )));
            // Return before the optimistic update below. The write was refused
            // — a sub-floor feed_interval_secs is the reachable case
            // (epics.allium: EditEpic) — so applying it locally anyway would
            // report an error while showing the user evidence it succeeded, and
            // the board would only self-correct on the next DB refresh.
            return vec![];
        }
        let mut updated = epic;
        updated.title = applied.title;
        updated.description = applied.description;
        if let crate::service::FieldUpdate::Set(ref cmd) = applied.feed_command {
            updated.feed_command = Some(cmd.clone());
        } else {
            updated.feed_command = None;
        }
        updated.feed_interval_secs = applied.feed_interval_secs;
        let feed_command_changed =
            updated.feed_command.is_some() && updated.feed_command != original_feed_command;
        let mut cmds = app.update(Message::Epic(crate::tui::messages::EpicMessage::Edited(
            updated,
        )));

        // Take-over prompt (epics.allium: EditEpic, feeds.allium:
        // OverrideFeedOwner): only when this edit actually CHANGES
        // feed_command to a non-empty value, and only when a different host
        // already owns this epic's feed. No prompt for a brand-new feed epic
        // (no PollOwner row yet — the next FeedTick claims it normally), an
        // edit that leaves feed_command untouched or clears it, or an epic
        // this host already owns.
        if feed_command_changed {
            match self
                .board_reads
                .poll_owner(crate::models::PollScopeId::Epic(epic_id))
                .await
            {
                Ok(Some(other_host)) if other_host != self.host_id => {
                    cmds.extend(app.update(Message::Epic(
                        crate::tui::messages::EpicMessage::FeedOwnerTakeoverOffered {
                            epic_id,
                            other_host,
                        },
                    )));
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(
                        epic_id = epic_id.0,
                        "failed to read feed poll ownership, skipping the take-over prompt: {e:#}"
                    );
                }
            }
        }
        cmds
    }
}

/// Surface accumulated editor parse errors as a status message. No-op when
/// the slice is empty so callers don't need to guard the call themselves.
fn emit_parse_errors(app: &mut App, errors: &[crate::editor::EditorParseError]) {
    if errors.is_empty() {
        return;
    }
    let summary = errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    app.update(Message::System(
        crate::tui::messages::SystemMessage::StatusInfo(format!(
            "Edit accepted with parse errors — {summary}"
        )),
    ));
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
mod tests;

#[cfg(test)]
mod epic_edit_tests;
