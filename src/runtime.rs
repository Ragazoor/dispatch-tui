use anyhow::Result;
use crossterm::{
    event::{self, DisableFocusChange, EnableFocusChange, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;

use tempfile::Builder as TempfileBuilder;

/// Interval between TUI tick events (captures tmux output, checks staleness, etc.).
const TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Name used for the TUI's tmux window (visible in tmux status bar).
const TUI_WINDOW_NAME: &str = "TUI";

use crate::db::{AlertStore, PrStore, SettingsStore, TaskCrud};
use crate::editor::{
    format_description_for_editor, format_editor_content, format_epic_for_editor,
    parse_description_editor_output, parse_editor_content, parse_epic_editor_output,
};
use crate::models::TaskId;
use crate::process::{ProcessRunner, RealProcessRunner};
use crate::service::FieldUpdate;
use crate::tui::{
    self, App, Command, Message, PrListKind, RepoFilterMode, ReviewAgentRequest, ReviewBoardMode,
};
use crate::{db, dispatch, mcp, models, tmux};

/// Convert `Option<String>` to `FieldUpdate`: `Some(v)` → `Set(v)`, `None` → `Clear`.
fn option_to_field_update(opt: Option<String>) -> FieldUpdate {
    match opt {
        Some(v) => FieldUpdate::Set(v),
        None => FieldUpdate::Clear,
    }
}

/// Set up tmux for the TUI: rename the current window and bind Prefix+g to jump back.
fn setup_tmux_for_tui(runner: &dyn ProcessRunner) {
    let _ = tmux::rename_window("", TUI_WINDOW_NAME, runner);
    let _ = tmux::bind_key("g", &format!("select-window -t {TUI_WINDOW_NAME}"), runner);
}

/// Tear down tmux TUI state: unbind the key and restore the original window name.
fn teardown_tmux_for_tui(original_name: Option<&str>, runner: &dyn ProcessRunner) {
    let _ = tmux::unbind_key("g", runner);
    if let Some(name) = original_name {
        let _ = tmux::rename_window(TUI_WINDOW_NAME, name, runner);
    }
}

// ---------------------------------------------------------------------------
// run_tui — entry point for the TUI mode
// ---------------------------------------------------------------------------

pub async fn run_tui(db_path: &Path, port: u16, inactivity_timeout: u64) -> Result<()> {
    // 1. Open database and load initial tasks
    let database = Arc::new(db::Database::open(db_path)?);
    let tasks = database.list_all()?;

    // 2. Spawn MCP server with notification channel
    let runner: Arc<dyn ProcessRunner> = Arc::new(RealProcessRunner);
    let mcp_db = database.clone();
    let mcp_runner = runner.clone();
    let (mcp_notify_tx, mut mcp_notify_rx) = mpsc::unbounded_channel::<mcp::McpEvent>();
    tokio::spawn(async move {
        if let Err(e) = mcp::serve(mcp_db, port, mcp_notify_tx, mcp_runner).await {
            eprintln!("MCP server error: {e}");
        }
    });

    // 3. Create App and load saved repo paths
    let mut app = App::new(tasks, Duration::from_secs(inactivity_timeout));
    let paths = database.list_repo_paths().unwrap_or_default();
    app.update(Message::RepoPathsUpdated(paths));
    let usage = database.get_all_usage().unwrap_or_default();
    app.update(Message::RefreshUsage(usage));

    // Warn if tmux focus-events is off (needed for split-view focus indicator)
    if !crate::tmux::focus_events_enabled(&*runner) {
        app.update(Message::StatusInfo(
            "tmux focus-events is off \u{2014} split-view focus indicator won't work. Run: tmux set -g focus-events on".to_string(),
        ));
    }

    // Seed default GitHub query strings (no-op if already set)
    if let Err(e) = database.seed_github_query_defaults() {
        app.update(Message::StatusInfo(format!(
            "Failed to seed GitHub query defaults: {e}"
        )));
    }

    // Load notification preference
    let notif_enabled = database
        .get_setting_bool("notifications_enabled")
        .unwrap_or(None)
        .unwrap_or(false);
    app.set_notifications_enabled(notif_enabled);

    // Load repo filter (intersect with known repo_paths to prune stale entries)
    if let Some(filter_str) = database.get_setting_string("repo_filter").unwrap_or(None) {
        if !filter_str.is_empty() {
            let known: HashSet<&str> = app.repo_paths().iter().map(|s| s.as_str()).collect();
            let paths: Vec<String> = serde_json::from_str(&filter_str).unwrap_or_default();
            let filter: HashSet<String> = paths
                .into_iter()
                .filter(|s| known.contains(s.as_str()))
                .collect();
            app.set_repo_filter(filter);
        }
    }

    // Load repo filter mode
    if let Some(mode_str) = database
        .get_setting_string("repo_filter_mode")
        .unwrap_or(None)
    {
        let mode = mode_str.parse().unwrap_or_default();
        app.set_repo_filter_mode(mode);
    }

    // Load saved filter presets
    match database.list_filter_presets() {
        Ok(raw) => {
            app.update(Message::FilterPresetsLoaded(parse_raw_presets(raw, None)));
        }
        Err(e) => {
            app.update(Message::StatusInfo(format!(
                "Failed to load filter presets: {e}"
            )));
        }
    }

    // Load cached review PRs from database
    match database.load_prs(crate::db::PrKind::Review) {
        Ok(prs) => app.set_review_prs(prs),
        Err(e) => {
            app.update(Message::StatusInfo(format!(
                "Failed to load cached review PRs: {e}"
            )));
        }
    }

    // Load cached bot PRs from database
    match database.load_prs(crate::db::PrKind::Bot) {
        Ok(prs) => app.set_bot_prs(prs),
        Err(e) => {
            app.update(Message::StatusInfo(format!(
                "Failed to load cached bot PRs: {e}"
            )));
        }
    }

    // Load cached security alerts from database
    match database.load_security_alerts() {
        Ok(alerts) => app.set_security_alerts(alerts),
        Err(e) => {
            app.update(Message::StatusInfo(format!(
                "Failed to load cached security alerts: {e}"
            )));
        }
    }

    // 4. Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableFocusChange)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set up tmux keybinding: Prefix+g → jump back to this window.
    // Best-effort: failures don't prevent the TUI from starting.
    let tmux_runner = runner.clone();
    let original_window_name = tmux::current_window_name(&*tmux_runner).ok();
    setup_tmux_for_tui(&*tmux_runner);

    // 5. Create two channels:
    //    - key_rx: raw crossterm KeyEvents from the blocking poll thread
    //    - msg_rx: higher-level Messages (e.g. from dispatch results in Phase 3)
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<crossterm::event::KeyEvent>();
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<Message>();

    // crossterm::event::poll/read are blocking; run them in a dedicated thread
    // so they don't block the async runtime. The thread can be paused (e.g. when
    // opening an external editor) via the input_paused flag.
    let input_paused = Arc::new(AtomicBool::new(false));
    let paused_clone = input_paused.clone();
    let resize_tx = msg_tx.clone();
    tokio::task::spawn_blocking(move || loop {
        if paused_clone.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if key_tx.send(key).is_err() {
                        break;
                    }
                }
                Ok(Event::Resize(..)) => {
                    let _ = resize_tx.send(Message::TerminalResized);
                }
                Ok(Event::FocusGained) => {
                    let _ = resize_tx.send(Message::FocusChanged(true));
                }
                Ok(Event::FocusLost) => {
                    let _ = resize_tx.send(Message::FocusChanged(false));
                }
                _ => {}
            }
        }
    });

    // 6. Tick interval (2 seconds)
    let mut tick_interval = interval(TICK_INTERVAL);

    // 7. Main loop
    tracing::info!(port, db = %db_path.display(), "TUI started, MCP server on port {port}");

    let runtime = TuiRuntime {
        task_svc: crate::service::TaskService::new(database.clone()),
        epic_svc: crate::service::EpicService::new(database.clone()),
        database,
        msg_tx,
        input_paused,
        runner,
    };
    let result = run_loop(
        &mut app,
        &mut terminal,
        &mut key_rx,
        &mut msg_rx,
        &mut mcp_notify_rx,
        &mut tick_interval,
        &runtime,
    )
    .await;

    // Tear down tmux keybinding and restore the original window name.
    teardown_tmux_for_tui(original_window_name.as_deref(), &*tmux_runner);

    // 8. Cleanup terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableFocusChange,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}

// ---------------------------------------------------------------------------
// TerminalSuspend — RAII guard for leaving/re-entering the alternate screen
// ---------------------------------------------------------------------------

struct TerminalSuspend<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
}

impl<'a> TerminalSuspend<'a> {
    fn new(terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<Self> {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            DisableFocusChange,
            LeaveAlternateScreen
        )?;
        terminal.show_cursor()?;
        Ok(TerminalSuspend { terminal })
    }
}

impl Drop for TerminalSuspend<'_> {
    fn drop(&mut self) {
        let _ = enable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableFocusChange
        );
        let _ = self.terminal.hide_cursor();
        let _ = self.terminal.clear();
    }
}

// ---------------------------------------------------------------------------
// InputPausedGuard — RAII guard for pausing input + suspending the terminal
// ---------------------------------------------------------------------------

struct InputPausedGuard<'a> {
    input_paused: Arc<AtomicBool>,
    terminal_guard: Option<TerminalSuspend<'a>>,
}

impl<'a> InputPausedGuard<'a> {
    fn new(
        input_paused: &Arc<AtomicBool>,
        terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<Self> {
        input_paused.store(true, Ordering::Relaxed);
        while key_rx.try_recv().is_ok() {}
        let terminal_guard = TerminalSuspend::new(terminal)?;
        Ok(InputPausedGuard {
            input_paused: Arc::clone(input_paused),
            terminal_guard: Some(terminal_guard),
        })
    }
}

impl Drop for InputPausedGuard<'_> {
    fn drop(&mut self) {
        // Restore terminal first, then resume input (matches original ordering)
        drop(self.terminal_guard.take());
        self.input_paused.store(false, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// TuiRuntime — shared context for command execution
// ---------------------------------------------------------------------------

struct TuiRuntime {
    database: Arc<dyn db::TaskStore>,
    task_svc: crate::service::TaskService,
    epic_svc: crate::service::EpicService,
    msg_tx: mpsc::UnboundedSender<Message>,
    input_paused: Arc<AtomicBool>,
    runner: Arc<dyn ProcessRunner>,
}

impl TuiRuntime {
    fn db_error(action: &str, e: impl std::fmt::Display) -> String {
        format!("DB error {action}: {e}")
    }

    /// Load GitHub query strings for a given settings key, split by newline.
    /// Returns an empty vec if the setting is missing.
    fn load_github_queries(&self, key: &str) -> Vec<String> {
        self.database
            .get_setting_string(key)
            .ok()
            .flatten()
            .map(|s| {
                s.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn create_task(
        &self,
        app: &mut App,
        params: crate::service::CreateTaskParams,
    ) -> Option<models::Task> {
        match self.task_svc.create_task_returning(params) {
            Ok(task) => Some(task),
            Err(e) => {
                app.update(Message::Error(Self::db_error("creating task", e)));
                None
            }
        }
    }

    fn exec_insert_task(
        &self,
        app: &mut App,
        draft: tui::TaskDraft,
        epic_id: Option<models::EpicId>,
    ) {
        use crate::service::CreateTaskParams;
        let params = CreateTaskParams {
            title: draft.title,
            description: draft.description,
            repo_path: draft.repo_path,
            plan_path: None,
            epic_id: epic_id.map(|e| e.0),
            sort_order: None,
            tag: draft.tag,
            base_branch: Some(draft.base_branch),
        };
        if let Some(task) = self.create_task(app, params) {
            app.update(Message::TaskCreated { task });
        }
    }

    fn exec_quick_dispatch(
        &self,
        app: &mut App,
        title: String,
        description: String,
        repo_path: String,
        epic_id: Option<models::EpicId>,
    ) {
        let Some(task) = self.create_task(
            app,
            crate::service::CreateTaskParams {
                title: title.clone(),
                description: description.clone(),
                repo_path: repo_path.clone(),
                plan_path: None,
                epic_id: epic_id.map(|e| e.0),
                sort_order: None,
                tag: None,
                base_branch: None,
            },
        ) else {
            return;
        };
        app.update(Message::TaskCreated { task: task.clone() });
        let expanded = models::expand_tilde(&repo_path);
        let _ = self.database.save_repo_path(&expanded);
        let paths = self.database.list_repo_paths().unwrap_or_default();
        app.update(Message::RepoPathsUpdated(paths));
        let epic_ctx = dispatch::EpicContext::from_db(&task, &*self.database);
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let id = task.id;
            match dispatch::quick_dispatch_agent(&task, &*runner, epic_ctx.as_ref()) {
                Ok(result) => {
                    let _ = tx.send(Message::Dispatched {
                        id,
                        worktree: result.worktree_path,
                        tmux_window: result.tmux_window,
                        switch_focus: true,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!("Quick dispatch failed: {e:#}")));
                }
            }
        });
    }

    fn exec_persist_task(&self, app: &mut App, task: models::Task) {
        use crate::service::UpdateTaskParams;
        if let Err(e) = self.task_svc.update_task(UpdateTaskParams {
            task_id: task.id.0,
            status: Some(task.status),
            plan_path: None,
            title: None,
            description: None,
            repo_path: None,
            sort_order: task.sort_order,
            pr_url: Some(option_to_field_update(task.pr_url.clone())),
            tag: None,
            sub_status: Some(task.sub_status),
            epic_id: None,
            worktree: Some(option_to_field_update(task.worktree.clone())),
            tmux_window: Some(option_to_field_update(task.tmux_window.clone())),
            base_branch: None,
        }) {
            app.update(Message::Error(Self::db_error("persisting task", e)));
        }
    }

    fn exec_patch_sub_status(
        &self,
        app: &mut App,
        id: models::TaskId,
        sub_status: models::SubStatus,
    ) {
        use crate::service::UpdateTaskParams;
        if let Err(e) = self.task_svc.update_task(UpdateTaskParams {
            task_id: id.0,
            status: None,
            plan_path: None,
            title: None,
            description: None,
            repo_path: None,
            sort_order: None,
            pr_url: None,
            tag: None,
            sub_status: Some(sub_status),
            epic_id: None,
            worktree: None,
            tmux_window: None,
            base_branch: None,
        }) {
            app.update(Message::Error(Self::db_error("patching sub_status", e)));
        }
    }

    fn exec_delete_task(&self, app: &mut App, id: TaskId) {
        if let Err(e) = self.task_svc.delete_task(id.0) {
            app.update(Message::Error(Self::db_error("deleting task", e)));
        }
    }

    fn spawn_dispatch<F>(&self, task: models::Task, dispatch_fn: F, label: &'static str)
    where
        F: FnOnce(&models::Task, &dyn ProcessRunner) -> Result<models::DispatchResult>
            + Send
            + 'static,
    {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            let id = task.id;
            tracing::info!(task_id = id.0, label, "dispatching");
            match dispatch_fn(&task, &*runner) {
                Ok(result) => {
                    // receiver dropped = app shutting down; nothing to log
                    let _ = tx.send(Message::Dispatched {
                        id,
                        worktree: result.worktree_path,
                        tmux_window: result.tmux_window,
                        switch_focus: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::DispatchFailed(id));
                    let _ = tx.send(Message::Error(format!("{label} failed: {e:#}")));
                }
            }
        });
    }

    fn exec_dispatch_agent(&self, task: models::Task, mode: models::DispatchMode) {
        let epic_ctx = dispatch::EpicContext::from_db(&task, &*self.database);
        let label = match mode {
            models::DispatchMode::Dispatch => "Dispatch",
            models::DispatchMode::Brainstorm => "Brainstorm",
            models::DispatchMode::Plan => "Plan",
        };
        self.spawn_dispatch(
            task,
            move |t, r| match mode {
                models::DispatchMode::Dispatch => dispatch::dispatch_agent(t, r, epic_ctx.as_ref()),
                models::DispatchMode::Brainstorm => {
                    dispatch::brainstorm_agent(t, r, epic_ctx.as_ref())
                }
                models::DispatchMode::Plan => dispatch::plan_agent(t, r, epic_ctx.as_ref()),
            },
            label,
        );
    }

    fn exec_capture_tmux(&self, id: TaskId, window: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Ok(false) = tmux::has_window(&window, &*runner) {
                let _ = tx.send(Message::WindowGone(id));
                return;
            }

            // Activity timestamp for staleness detection (fall back to 0 on error
            // so we never falsely mark an agent as stale).
            let activity_ts = tmux::window_activity(&window, &*runner).unwrap_or(0);

            match tmux::capture_pane(&window, 5, &*runner) {
                Ok(output) => {
                    let _ = tx.send(Message::TmuxOutput {
                        id,
                        output,
                        activity_ts,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!(
                        "tmux capture failed for window {window}: {e}"
                    )));
                }
            }
        });
    }

    /// Suspend the TUI, open content in $EDITOR, return edited text (or None).
    fn run_editor(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
        prefix: &str,
        content: &str,
    ) -> Result<Option<String>> {
        let mut tmp = TempfileBuilder::new()
            .prefix(prefix)
            .suffix(".md")
            .tempfile()?;
        std::io::Write::write_all(tmp.as_file_mut(), content.as_bytes())?;

        let _guard = InputPausedGuard::new(&self.input_paused, terminal, key_rx)?;
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
        let status = std::process::Command::new(&editor).arg(tmp.path()).status();
        drop(_guard);

        match status {
            Ok(exit) if exit.success() => Ok(std::fs::read_to_string(tmp.path()).ok()),
            Ok(exit) => {
                tracing::warn!(?exit, "editor exited with non-zero status");
                Ok(None)
            }
            Err(e) => {
                tracing::warn!("failed to spawn editor: {e}");
                Ok(None)
            }
        }
    }

    fn exec_edit_in_editor(
        &self,
        app: &mut App,
        task: models::Task,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<()> {
        let task_id = task.id;
        let content = format_editor_content(&task);
        let Some(edited) =
            self.run_editor(terminal, key_rx, &format!("task-{task_id}-"), &content)?
        else {
            return Ok(());
        };

        let fields = parse_editor_content(&edited);
        let title = if fields.title.is_empty() {
            task.title.clone()
        } else {
            fields.title
        };
        let description = if fields.description.is_empty() {
            task.description.clone()
        } else {
            fields.description
        };
        let repo_path = if fields.repo_path.is_empty() {
            task.repo_path.clone()
        } else {
            fields.repo_path
        };
        let new_status = models::TaskStatus::parse(&fields.status).unwrap_or(task.status);
        let plan = if fields.plan.is_empty() {
            None
        } else {
            Some(fields.plan)
        };
        let tag = if fields.tag.is_empty() {
            None
        } else {
            models::TaskTag::parse(&fields.tag)
        };
        let base_branch = if fields.base_branch.is_empty() {
            None
        } else {
            Some(fields.base_branch.clone())
        };

        if let Err(e) = self.task_svc.update_task(crate::service::UpdateTaskParams {
            task_id: task_id.0,
            status: Some(new_status),
            plan_path: plan.clone(),
            title: Some(title.clone()),
            description: Some(description.clone()),
            repo_path: Some(repo_path.clone()),
            sort_order: None,
            pr_url: None,
            tag,
            sub_status: None,
            epic_id: None,
            worktree: None,
            tmux_window: None,
            base_branch: base_branch.clone(),
        }) {
            app.update(Message::Error(Self::db_error("updating task", e)));
        }
        app.update(Message::TaskEdited(tui::TaskEdit {
            id: task_id,
            title,
            description,
            repo_path,
            status: new_status,
            plan_path: plan,
            tag,
            base_branch,
        }));
        Ok(())
    }

    fn exec_description_editor(
        &self,
        app: &mut App,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<()> {
        let content = format_description_for_editor("");
        let result = self.run_editor(terminal, key_rx, "description-", &content)?;
        match result {
            Some(text) => {
                let description = parse_description_editor_output(&text);
                app.update(Message::DescriptionEditorResult(description));
            }
            None => {
                app.update(Message::CancelInput);
            }
        }
        Ok(())
    }

    fn exec_save_repo_path(&self, app: &mut App, path: String) {
        let path = models::expand_tilde(&path);
        if let Err(e) = self.database.save_repo_path(&path) {
            app.update(Message::Error(Self::db_error("saving repo path", e)));
        }
        match self.database.list_repo_paths() {
            Ok(paths) => {
                app.update(Message::RepoPathsUpdated(paths));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("listing repo paths", e)));
            }
        }
    }

    fn exec_refresh_from_db(&self, app: &mut App) -> Vec<Command> {
        let mut cmds = Vec::new();
        // Re-read all tasks from SQLite to pick up MCP/CLI updates
        match self.database.list_all() {
            Ok(tasks) => {
                cmds = app.update(Message::RefreshTasks(tasks));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing tasks", e)));
            }
        }
        // Also refresh epics
        self.exec_refresh_epics_from_db(app);
        self.exec_refresh_usage_from_db(app);
        cmds
    }

    fn exec_send_notification(&self, title: &str, body: &str, urgent: bool) {
        let urgency = if urgent { "critical" } else { "normal" };
        if let Err(e) = self
            .runner
            .run("notify-send", &["-u", urgency, title, body])
        {
            tracing::warn!("notify-send failed: {e}");
        }
    }

    fn exec_persist_setting(&self, app: &mut App, key: &str, value: bool) {
        if let Err(e) = self.database.set_setting_bool(key, value) {
            app.update(Message::Error(Self::db_error("persisting setting", e)));
        }
    }

    fn exec_persist_string_setting(&self, app: &mut App, key: &str, value: &str) {
        if let Err(e) = self.database.set_setting_string(key, value) {
            app.update(Message::Error(Self::db_error("persisting setting", e)));
        }
    }

    fn exec_persist_filter_preset(
        &self,
        app: &mut App,
        name: &str,
        repo_paths: &[String],
        mode: &str,
    ) {
        if let Err(e) = self.database.save_filter_preset(name, repo_paths, mode) {
            app.update(Message::Error(Self::db_error("saving filter preset", e)));
        }
    }

    fn exec_delete_filter_preset(&self, app: &mut App, name: &str) {
        if let Err(e) = self.database.delete_filter_preset(name) {
            app.update(Message::Error(Self::db_error("deleting filter preset", e)));
        }
    }

    fn exec_delete_repo_path(&self, app: &mut App, path: &str) {
        if let Err(e) = self.database.delete_repo_path(path) {
            app.update(Message::Error(Self::db_error("deleting repo path", e)));
            return;
        }
        match self.database.list_repo_paths() {
            Ok(paths) => {
                app.update(Message::RepoPathsUpdated(paths));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("listing repo paths", e)));
            }
        }
        // Refresh presets since delete_repo_path cleans them
        if let Ok(raw) = self.database.list_filter_presets() {
            let known: HashSet<String> = app.repo_paths().iter().cloned().collect();
            let presets = parse_raw_presets(raw, Some(&known));
            app.update(Message::FilterPresetsLoaded(presets));
        }
    }

    fn exec_insert_epic(
        &self,
        app: &mut App,
        title: String,
        description: String,
        repo_path: String,
    ) {
        match self.epic_svc.create_epic(crate::service::CreateEpicParams {
            title,
            description,
            repo_path,
            sort_order: None,
        }) {
            Ok(epic) => {
                app.update(Message::EpicCreated(epic));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("creating epic", e)));
            }
        }
    }

    fn exec_edit_epic_in_editor(
        &self,
        app: &mut App,
        epic: models::Epic,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<()> {
        let epic_id = epic.id;
        let content = format_epic_for_editor(&epic);
        let Some(edited) =
            self.run_editor(terminal, key_rx, &format!("epic-{epic_id}-"), &content)?
        else {
            return Ok(());
        };

        let fields = parse_epic_editor_output(&edited);
        let title = if fields.title.is_empty() {
            epic.title.clone()
        } else {
            fields.title
        };
        let description = if fields.description.is_empty() {
            epic.description.clone()
        } else {
            fields.description
        };
        let repo_path = if fields.repo_path.is_empty() {
            epic.repo_path.clone()
        } else {
            fields.repo_path
        };

        if let Err(e) = self.epic_svc.update_epic(crate::service::UpdateEpicParams {
            epic_id: epic_id.0,
            title: Some(title.clone()),
            description: Some(description.clone()),
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: Some(repo_path.clone()),
            auto_dispatch: None,
        }) {
            app.update(Message::Error(Self::db_error("updating epic", e)));
        }
        let mut updated = epic;
        updated.title = title;
        updated.description = description;
        updated.repo_path = repo_path;
        app.update(Message::EpicEdited(updated));
        Ok(())
    }

    fn exec_edit_github_queries(
        &self,
        app: &mut App,
        mode: ReviewBoardMode,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    ) -> Result<Vec<Command>> {
        let key = match mode {
            ReviewBoardMode::Reviewer => "github_queries_review",
            ReviewBoardMode::Author => "github_queries_my_prs",
            ReviewBoardMode::Dependabot => "github_queries_bot",
        };

        let current = self
            .database
            .get_setting_string(key)
            .ok()
            .flatten()
            .unwrap_or_default();

        let header = format!(
            "# GitHub queries for: {}\n# One search query per line. Blank lines and lines starting with # are ignored.\n# See: https://docs.github.com/en/search-github/searching-on-github/searching-issues-and-pull-requests\n\n",
            match mode {
                ReviewBoardMode::Reviewer => "Review PRs",
                ReviewBoardMode::Author => "My PRs",
                ReviewBoardMode::Dependabot => "Bot PRs",
            }
        );
        let content = format!("{header}{current}\n");

        let Some(edited) = self.run_editor(terminal, key_rx, "github-queries-", &content)? else {
            return Ok(vec![]);
        };

        // Strip comments and blank lines
        let queries: String = edited
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");

        if let Err(e) = self.database.set_setting_string(key, &queries) {
            app.update(Message::Error(Self::db_error("saving github queries", e)));
            return Ok(vec![]);
        }

        // Trigger a refresh for the affected category
        let refresh_msg = match mode {
            ReviewBoardMode::Reviewer => Message::RefreshReviewPrs,
            ReviewBoardMode::Author => Message::RefreshReviewPrs,
            ReviewBoardMode::Dependabot => Message::RefreshBotPrs,
        };
        Ok(app.update(refresh_msg))
    }

    fn exec_delete_epic(&self, app: &mut App, id: models::EpicId) {
        if let Err(e) = self.epic_svc.delete_epic(id.0) {
            app.update(Message::Error(Self::db_error("deleting epic", e)));
        }
    }

    fn exec_persist_epic(
        &self,
        app: &mut App,
        id: models::EpicId,
        status: Option<models::TaskStatus>,
        sort_order: Option<i64>,
    ) {
        // Only call service if there's something to update
        if status.is_none() && sort_order.is_none() {
            return;
        }
        if let Err(e) = self.epic_svc.update_epic(crate::service::UpdateEpicParams {
            epic_id: id.0,
            title: None,
            description: None,
            status,
            plan_path: None,
            sort_order,
            repo_path: None,
            auto_dispatch: None,
        }) {
            app.update(Message::Error(Self::db_error("updating epic", e)));
        }
    }

    fn exec_toggle_epic_auto_dispatch(
        &self,
        app: &mut App,
        id: models::EpicId,
        auto_dispatch: bool,
    ) {
        if let Err(e) = self.epic_svc.update_epic(crate::service::UpdateEpicParams {
            epic_id: id.0,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: Some(auto_dispatch),
        }) {
            app.update(Message::Error(Self::db_error("toggling auto dispatch", e)));
        }
    }

    fn exec_refresh_epics_from_db(&self, app: &mut App) {
        match self.database.list_epics() {
            Ok(epics) => {
                app.update(Message::RefreshEpics(epics));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing epics", e)));
            }
        }
    }

    fn exec_refresh_usage_from_db(&self, app: &mut App) {
        match self.database.get_all_usage() {
            Ok(usage) => {
                app.update(Message::RefreshUsage(usage));
            }
            Err(e) => {
                app.update(Message::Error(Self::db_error("refreshing usage", e)));
            }
        }
    }

    fn exec_cleanup(
        &self,
        id: TaskId,
        repo_path: String,
        worktree: String,
        tmux_window: Option<String>,
    ) {
        let shared = self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .unwrap_or(false);

        if shared {
            // Other active tasks share this worktree — just detach this task
            tracing::info!(task_id = id.0, "worktree shared, detaching only");
            if let Err(e) = self.task_svc.update_task(crate::service::UpdateTaskParams {
                task_id: id.0,
                status: None,
                plan_path: None,
                title: None,
                description: None,
                repo_path: None,
                sort_order: None,
                pr_url: None,
                tag: None,
                sub_status: None,
                epic_id: None,
                worktree: Some(FieldUpdate::Clear),
                tmux_window: Some(FieldUpdate::Clear),
                base_branch: None,
            }) {
                let _ = self
                    .msg_tx
                    .send(Message::Error(format!("Detach failed: {e:#}")));
            }
            return;
        }

        // No other active tasks — full cleanup
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                dispatch::cleanup_task(&repo_path, &worktree, tmux_window.as_deref(), &*runner)
            {
                let _ = tx.send(Message::Error(format!("Cleanup failed: {e:#}")));
            }
        });
    }

    fn exec_finish(
        &self,
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        worktree: String,
        tmux_window: Option<String>,
    ) {
        let shared = self
            .database
            .has_other_tasks_with_worktree(&worktree, id)
            .unwrap_or(false);

        if shared {
            tracing::info!(
                task_id = id.0,
                "worktree shared, detaching only (no rebase)"
            );
            if let Err(e) = self.task_svc.update_task(crate::service::UpdateTaskParams {
                task_id: id.0,
                status: None,
                plan_path: None,
                title: None,
                description: None,
                repo_path: None,
                sort_order: None,
                pr_url: None,
                tag: None,
                sub_status: None,
                epic_id: None,
                worktree: Some(FieldUpdate::Clear),
                tmux_window: Some(FieldUpdate::Clear),
                base_branch: None,
            }) {
                let _ = self
                    .msg_tx
                    .send(Message::Error(format!("Detach failed: {e:#}")));
            }
            let _ = self.msg_tx.send(Message::FinishComplete(id));
            return;
        }

        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::finish_task(
                &repo_path,
                &worktree,
                &branch,
                &base_branch,
                tmux_window.as_deref(),
                &*runner,
            ) {
                Ok(()) => {
                    let _ = tx.send(Message::FinishComplete(id));
                }
                Err(e) => {
                    let is_conflict = matches!(e, dispatch::FinishError::RebaseConflict(_));
                    let _ = tx.send(Message::FinishFailed {
                        id,
                        error: e.to_string(),
                        is_conflict,
                    });
                }
            }
        });
    }

    fn exec_resume(&self, task: models::Task) {
        let tx = self.msg_tx.clone();
        let id = task.id;
        let worktree_path = task.worktree.clone().unwrap_or_default();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            tracing::info!(task_id = id.0, "resuming task");
            match dispatch::resume_agent(id, &worktree_path, &*runner) {
                Ok(result) => {
                    let _ = tx.send(Message::Resumed {
                        id,
                        tmux_window: result.tmux_window,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!("Resume failed: {e:#}")));
                }
            }
        });
    }

    fn exec_jump_to_tmux(&self, app: &mut App, window: String) {
        if let Err(e) = tmux::select_window(&window, &*self.runner) {
            app.update(Message::Error(format!("Jump failed: {e:#}")));
        }
    }

    fn exec_enter_split_mode(&self, app: &mut App) {
        let dispatch_pane = match tmux::current_pane_id(&*self.runner) {
            Ok(id) => id,
            Err(_) => {
                app.update(Message::StatusInfo("Split mode requires tmux".to_string()));
                return;
            }
        };
        match tmux::split_window_horizontal(&dispatch_pane, &*self.runner) {
            Ok(pane_id) => {
                app.update(Message::SplitPaneOpened {
                    pane_id,
                    task_id: None,
                });
            }
            Err(e) => {
                app.update(Message::Error(format!("Split failed: {e:#}")));
            }
        }
    }

    fn exec_enter_split_mode_with_task(&self, app: &mut App, task_id: TaskId, window: &str) {
        let dispatch_pane = match tmux::current_pane_id(&*self.runner) {
            Ok(id) => id,
            Err(_) => {
                app.update(Message::StatusInfo("Split mode requires tmux".to_string()));
                return;
            }
        };
        match tmux::join_pane(window, &dispatch_pane, &*self.runner) {
            Ok(pane_id) => {
                app.update(Message::SplitPaneOpened {
                    pane_id,
                    task_id: Some(task_id),
                });
            }
            Err(e) => {
                app.update(Message::Error(format!("Split with task failed: {e:#}")));
            }
        }
    }

    fn exec_exit_split_mode(&self, app: &mut App, pane_id: &str, restore_window: Option<&str>) {
        if let Some(window_name) = restore_window {
            if let Err(e) = tmux::break_pane_to_window(pane_id, window_name, &*self.runner) {
                app.update(Message::Error(format!("Break pane failed: {e:#}")));
                return;
            }
        } else if let Err(e) = tmux::kill_pane(pane_id, &*self.runner) {
            app.update(Message::Error(format!("Kill pane failed: {e:#}")));
            return;
        }
        app.update(Message::SplitPaneClosed);
    }

    fn exec_swap_split_pane(
        &self,
        app: &mut App,
        task_id: TaskId,
        new_window: &str,
        old_pane_id: Option<&str>,
        old_window: Option<&str>,
    ) {
        let Some(right_pane) = old_pane_id else {
            // No right pane to swap into — shouldn't happen, but handle gracefully
            return;
        };

        // 1. Get the new task's pane ID before swapping (pane IDs follow content)
        let new_pane_id = match tmux::pane_id_for_window(new_window, &*self.runner) {
            Ok(id) => id,
            Err(e) => {
                app.update(Message::Error(format!("Cannot get pane ID: {e:#}")));
                return;
            }
        };

        // 2. Atomically swap pane contents — no layout change, no resize, no flicker
        let source = format!("{new_window}.0");
        if let Err(e) = tmux::swap_pane(&source, right_pane, &*self.runner) {
            app.update(Message::Error(format!("Swap pane failed: {e:#}")));
            return;
        }

        // 3. The standalone window now holds the old pane's content.
        //    Rename it back to the old task's window name, or kill it if there was no task.
        if let Some(old_name) = old_window {
            // The window kept its name (new_window). Rename it to the old task's name.
            if let Err(e) = tmux::rename_window(new_window, old_name, &*self.runner) {
                app.update(Message::Error(format!("Rename window failed: {e:#}")));
                return;
            }
        } else {
            // Old pane was empty (no task) — kill the standalone window holding it
            if let Err(e) = tmux::kill_window(new_window, &*self.runner) {
                app.update(Message::Error(format!("Kill window failed: {e:#}")));
                return;
            }
        }

        app.update(Message::SplitPaneOpened {
            pane_id: new_pane_id.clone(),
            task_id: Some(task_id),
        });

        // Focus the right pane so the user can interact with the agent
        if let Err(e) = tmux::select_pane(&new_pane_id, &*self.runner) {
            tracing::warn!("select-pane failed: {e:#}");
        }
    }

    fn exec_check_split_pane(&self, app: &mut App, pane_id: &str) {
        if !tmux::pane_exists(pane_id, &*self.runner) {
            app.update(Message::SplitPaneClosed);
        }
    }

    fn exec_respawn_split_pane(&self, app: &mut App, pane_id: &str) {
        if !tmux::pane_exists(pane_id, &*self.runner) {
            app.update(Message::SplitPaneClosed);
            return;
        }
        if let Err(e) = tmux::respawn_pane(pane_id, &*self.runner) {
            tracing::warn!("respawn-pane failed: {e:#}");
            app.update(Message::SplitPaneClosed);
        }
    }

    fn exec_dispatch_epic(&self, app: &mut App, epic: models::Epic) {
        let title = format!("Plan: {}", epic.title);
        let description = format!(
            "Planning subtask for epic: {}\n\n{}",
            epic.title, epic.description
        );

        // Create the planning subtask via service
        let task = match self
            .task_svc
            .create_task_returning(crate::service::CreateTaskParams {
                title: title.clone(),
                description: description.clone(),
                repo_path: epic.repo_path.clone(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
            }) {
            Ok(task) => task,
            Err(e) => {
                app.update(Message::Error(Self::db_error("creating planning task", e)));
                return;
            }
        };

        app.update(Message::TaskCreated { task: task.clone() });

        // Dispatch the planning subtask asynchronously
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        let epic_id = epic.id;
        let epic_title = epic.title.clone();
        let epic_description = epic.description.clone();

        tokio::task::spawn_blocking(move || {
            let id = task.id;
            tracing::info!(
                task_id = id.0,
                epic_id = epic_id.0,
                "dispatching epic planning agent"
            );
            match dispatch::epic_planning_agent(
                &task,
                epic_id,
                &epic_title,
                &epic_description,
                &*runner,
            ) {
                Ok(result) => {
                    let _ = tx.send(Message::Dispatched {
                        id,
                        worktree: result.worktree_path,
                        tmux_window: result.tmux_window,
                        switch_focus: true,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::Error(format!(
                        "Epic planning dispatch failed: {e:#}"
                    )));
                }
            }
        });
    }

    fn exec_kill_tmux_window(&self, window: String) {
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(e) = tmux::kill_window(&window, &*runner) {
                tracing::warn!(%window, "failed to kill tmux window (best-effort): {e:#}");
            }
        });
    }

    fn exec_create_pr(
        &self,
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        title: String,
        description: String,
    ) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::create_pr(
                &repo_path,
                &branch,
                &title,
                &description,
                &base_branch,
                &*runner,
            ) {
                Ok(result) => {
                    let _ = tx.send(Message::PrCreated {
                        id,
                        pr_url: result.pr_url,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::PrFailed {
                        id,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    fn exec_check_pr_status(&self, id: TaskId, pr_url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || {
            match dispatch::check_pr_status(&pr_url, &*runner) {
                Ok(status) => {
                    if status.state == dispatch::PrState::Merged {
                        let _ = tx.send(Message::PrMerged(id));
                    } else if status.state == dispatch::PrState::Open {
                        let _ = tx.send(Message::PrReviewState {
                            id,
                            review_decision: status.review_decision,
                        });
                    }
                    // Closed PRs: no message
                }
                Err(e) => {
                    tracing::warn!(task_id = id.0, "PR status check failed: {e}");
                }
            }
        });
    }

    fn exec_merge_pr(&self, id: TaskId, pr_url: String) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();

        tokio::task::spawn_blocking(move || match dispatch::merge_pr(&pr_url, &*runner) {
            Ok(()) => {
                let _ = tx.send(Message::PrMerged(id));
            }
            Err(e) => {
                let _ = tx.send(Message::MergePrFailed {
                    id,
                    error: e.to_string(),
                });
            }
        });
    }

    fn exec_fetch_prs(&self, kind: PrListKind) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        let queries = self.load_github_queries(kind.settings_key());

        if queries.is_empty() && kind == PrListKind::Bot {
            let _ = tx.send(Message::PrsFetchFailed(
                kind,
                "Bot queries not configured — press [e] to add your org filter".to_string(),
            ));
            return;
        }

        tokio::task::spawn_blocking(move || {
            tracing::info!(kind = kind.label(), "fetching PRs via gh");
            match crate::github::fetch_prs(&*runner, &queries) {
                Ok(prs) => {
                    tracing::info!(
                        kind = kind.label(),
                        count = prs.len(),
                        "PRs fetched successfully"
                    );
                    let _ = tx.send(Message::PrsLoaded(kind, prs));
                }
                Err(e) => {
                    tracing::warn!(kind = kind.label(), error = %e, "PR fetch failed");
                    let _ = tx.send(Message::PrsFetchFailed(kind, e));
                }
            }
        });
    }

    fn exec_persist_prs(&self, app: &mut App, kind: PrListKind, prs: Vec<crate::models::ReviewPr>) {
        let result = self.database.save_prs(kind.to_pr_kind(), &prs);
        if let Err(e) = result {
            app.update(Message::Error(Self::db_error(
                &format!("persisting {} PRs", kind.label()),
                e,
            )));
        }
    }

    fn exec_batch_approve_prs(&self, urls: Vec<String>) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let mut approved = 0usize;
            for url in &urls {
                tracing::info!(url, "approving PR");
                match runner.run("gh", &["pr", "review", "--approve", url]) {
                    Ok(output) if output.status.success() => approved += 1,
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!(url, error = %stderr, "failed to approve PR");
                    }
                    Err(e) => tracing::warn!(url, error = %e, "failed to run gh"),
                }
            }
            tracing::info!(approved, total = urls.len(), "batch approve complete");
            let _ = tx.send(Message::RefreshBotPrs);
            let _ = tx.send(Message::StatusInfo(format!(
                "Approved {approved}/{} PRs",
                urls.len()
            )));
        });
    }

    fn exec_batch_merge_prs(&self, urls: Vec<String>) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let mut merged_urls = Vec::new();
            for url in &urls {
                tracing::info!(url, "merging PR");
                match runner.run("gh", &["pr", "merge", "--merge", url]) {
                    Ok(output) if output.status.success() => {
                        merged_urls.push(url.clone());
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!(url, error = %stderr, "failed to merge PR");
                    }
                    Err(e) => tracing::warn!(url, error = %e, "failed to run gh"),
                }
            }
            let merged = merged_urls.len();
            tracing::info!(merged, total = urls.len(), "batch merge complete");
            if !merged_urls.is_empty() {
                let _ = tx.send(Message::BotPrsMerged(merged_urls));
            }
            let _ = tx.send(Message::RefreshBotPrs);
            let _ = tx.send(Message::StatusInfo(format!(
                "Merged {merged}/{} PRs",
                urls.len()
            )));
        });
    }

    fn exec_open_in_browser(&self, url: String) {
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = runner.run("xdg-open", &[&url]) {
                tracing::warn!("Failed to open browser: {e}");
            }
        });
    }

    fn exec_fetch_security_alerts(&self) {
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            tracing::info!("fetching security alerts via gh");
            match crate::github::fetch_security_alerts(&*runner) {
                Ok(alerts) => {
                    tracing::info!(count = alerts.len(), "security alerts fetched successfully");
                    let _ = tx.send(Message::SecurityAlertsLoaded(alerts));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "security alert fetch failed");
                    let _ = tx.send(Message::SecurityAlertsFetchFailed(e));
                }
            }
        });
    }

    fn exec_persist_security_alerts(
        &self,
        app: &mut App,
        alerts: Vec<crate::models::SecurityAlert>,
    ) {
        if let Err(e) = self.database.save_security_alerts(&alerts) {
            app.update(Message::Error(Self::db_error(
                "persisting security alerts",
                e,
            )));
        }
    }

    fn exec_dispatch_fix_agent(&self, req: tui::FixAgentRequest) {
        // repo is already resolved to a local path by the TUI
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            let github_repo = req.github_repo.clone();
            let number = req.number;
            let kind = req.kind;
            match dispatch::dispatch_fix_agent(req, &*runner) {
                Ok(result) => {
                    let _ = tx.send(Message::FixAgentDispatched {
                        github_repo,
                        number,
                        kind,
                        tmux_window: result.tmux_window,
                        worktree: result.worktree_path,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::FixAgentFailed {
                        github_repo,
                        number,
                        kind,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    fn exec_dispatch_review_agent(&self, req: ReviewAgentRequest) {
        // repo is already resolved to a local path by the TUI
        let tx = self.msg_tx.clone();
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            match crate::dispatch::dispatch_review_agent(&req, &*runner) {
                Ok(result) => {
                    let _ = tx.send(Message::ReviewAgentDispatched {
                        github_repo: req.github_repo,
                        number: req.number,
                        tmux_window: result.tmux_window,
                        worktree: result.worktree_path,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Message::ReviewAgentFailed {
                        github_repo: req.github_repo,
                        number: req.number,
                        error: format!("{e:#}"),
                    });
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// run_loop — select over key events, async messages, and tick timer
// ---------------------------------------------------------------------------

async fn run_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    msg_rx: &mut mpsc::UnboundedReceiver<Message>,
    mcp_notify_rx: &mut mpsc::UnboundedReceiver<mcp::McpEvent>,
    tick_interval: &mut tokio::time::Interval,
    rt: &TuiRuntime,
) -> Result<()> {
    loop {
        // Draw the current frame
        terminal.draw(|frame| tui::ui::render(frame, app))?;

        if app.should_quit() {
            break;
        }

        let commands = tokio::select! {
            // Key events from the blocking poll thread
            Some(key) = key_rx.recv() => {
                app.handle_key(key)
            }

            // Async messages (e.g., from dispatch results)
            Some(msg) = msg_rx.recv() => {
                app.update(msg)
            }

            // MCP event notification
            Some(event) = mcp_notify_rx.recv() => {
                match event {
                    mcp::McpEvent::Refresh => rt.exec_refresh_from_db(app),
                    mcp::McpEvent::MessageSent { to_task_id } => {
                        app.update(Message::MessageReceived(to_task_id));
                        rt.exec_refresh_from_db(app)
                    }
                    mcp::McpEvent::ReviewReady { repo, number } => {
                        app.update(Message::ReviewStatusUpdated {
                            repo,
                            number,
                            status: crate::models::ReviewAgentStatus::FindingsReady,
                        });
                        rt.exec_refresh_from_db(app)
                    }
                }
            }

            // Periodic tick for tmux capture
            _ = tick_interval.tick() => {
                app.update(Message::Tick)
            }
        };

        execute_commands(app, commands, rt, terminal, key_rx).await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// execute_commands — run side effects for each Command
// ---------------------------------------------------------------------------

async fn execute_commands(
    app: &mut App,
    commands: Vec<Command>,
    rt: &TuiRuntime,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
) -> Result<()> {
    let mut queue = std::collections::VecDeque::from(commands);
    while let Some(command) = queue.pop_front() {
        match command {
            Command::PersistTask(task) => rt.exec_persist_task(app, task),
            Command::PersistReviewAgent {
                pr_kind,
                github_repo,
                number,
                tmux_window,
                worktree,
            } => {
                if let Err(e) =
                    rt.database
                        .set_pr_agent(pr_kind, &github_repo, number, &tmux_window, &worktree)
                {
                    let extra = app.update(Message::Error(format!(
                        "Failed to persist review agent: {e}"
                    )));
                    queue.extend(extra);
                }
            }
            Command::PersistFixAgent {
                github_repo,
                number,
                kind,
                tmux_window,
                worktree,
            } => {
                if let Err(e) =
                    rt.database
                        .set_alert_agent(&github_repo, number, kind, &tmux_window, &worktree)
                {
                    let extra =
                        app.update(Message::Error(format!("Failed to persist fix agent: {e}")));
                    queue.extend(extra);
                }
            }
            Command::InsertTask { draft, epic_id } => rt.exec_insert_task(app, draft, epic_id),
            Command::DeleteTask(id) => rt.exec_delete_task(app, id),
            Command::DispatchAgent { task, mode } => rt.exec_dispatch_agent(task, mode),
            Command::CaptureTmux { id, window } => rt.exec_capture_tmux(id, window),
            Command::EditTaskInEditor(task) => {
                rt.exec_edit_in_editor(app, task, terminal, key_rx)?
            }
            Command::OpenDescriptionEditor { .. } => {
                rt.exec_description_editor(app, terminal, key_rx)?
            }
            Command::SaveRepoPath(path) => rt.exec_save_repo_path(app, path),
            Command::RefreshFromDb => {
                let extra = rt.exec_refresh_from_db(app);
                queue.extend(extra);
            }
            Command::Cleanup {
                id,
                repo_path,
                worktree,
                tmux_window,
            } => rt.exec_cleanup(id, repo_path, worktree, tmux_window),
            Command::Resume { task } => rt.exec_resume(task),
            Command::JumpToTmux { window } => rt.exec_jump_to_tmux(app, window),
            Command::QuickDispatch { draft, epic_id } => rt.exec_quick_dispatch(
                app,
                draft.title,
                draft.description,
                draft.repo_path,
                epic_id,
            ),
            Command::KillTmuxWindow { window } => rt.exec_kill_tmux_window(window),
            Command::Finish {
                id,
                repo_path,
                branch,
                base_branch,
                worktree,
                tmux_window,
            } => rt.exec_finish(id, repo_path, branch, base_branch, worktree, tmux_window),
            // Epic commands
            Command::InsertEpic(draft) => {
                rt.exec_insert_epic(app, draft.title, draft.description, draft.repo_path)
            }
            Command::EditEpicInEditor(epic) => {
                rt.exec_edit_epic_in_editor(app, epic, terminal, key_rx)?
            }
            Command::DeleteEpic(id) => rt.exec_delete_epic(app, id),
            Command::PersistEpic {
                id,
                status,
                sort_order,
            } => rt.exec_persist_epic(app, id, status, sort_order),
            Command::RefreshEpicsFromDb => rt.exec_refresh_epics_from_db(app),
            Command::DispatchEpic { epic } => rt.exec_dispatch_epic(app, epic),
            Command::ToggleEpicAutoDispatch { id, auto_dispatch } => {
                rt.exec_toggle_epic_auto_dispatch(app, id, auto_dispatch)
            }
            Command::SendNotification {
                title,
                body,
                urgent,
            } => rt.exec_send_notification(&title, &body, urgent),
            Command::PersistSetting { key, value } => rt.exec_persist_setting(app, &key, value),
            Command::CreatePr {
                id,
                repo_path,
                branch,
                base_branch,
                title,
                description,
            } => rt.exec_create_pr(id, repo_path, branch, base_branch, title, description),
            Command::CheckPrStatus { id, pr_url } => rt.exec_check_pr_status(id, pr_url),
            Command::MergePr { id, pr_url } => rt.exec_merge_pr(id, pr_url),
            Command::PersistStringSetting { key, value } => {
                rt.exec_persist_string_setting(app, &key, &value)
            }
            Command::FetchPrs(kind) => rt.exec_fetch_prs(kind),
            Command::PersistPrs(kind, prs) => rt.exec_persist_prs(app, kind, prs),
            Command::BatchApprovePrs(urls) => rt.exec_batch_approve_prs(urls),
            Command::BatchMergePrs(urls) => rt.exec_batch_merge_prs(urls),
            Command::OpenInBrowser { url } => rt.exec_open_in_browser(url),
            Command::PersistFilterPreset {
                name,
                repo_paths,
                mode,
            } => {
                rt.exec_persist_filter_preset(app, &name, &repo_paths, mode.as_str());
            }
            Command::DeleteFilterPreset(name) => rt.exec_delete_filter_preset(app, &name),
            Command::DeleteRepoPath(path) => rt.exec_delete_repo_path(app, &path),
            Command::PatchSubStatus { id, sub_status } => {
                rt.exec_patch_sub_status(app, id, sub_status)
            }
            Command::DispatchReviewAgent(req) => rt.exec_dispatch_review_agent(req),
            Command::FetchSecurityAlerts => rt.exec_fetch_security_alerts(),
            Command::PersistSecurityAlerts(alerts) => rt.exec_persist_security_alerts(app, alerts),
            Command::DispatchFixAgent(req) => {
                rt.exec_dispatch_fix_agent(req);
            }
            Command::EditGithubQueries(mode) => {
                let extra = rt.exec_edit_github_queries(app, mode, terminal, key_rx)?;
                queue.extend(extra);
            }
            Command::UpdateAgentStatus {
                repo,
                number,
                status,
            } => {
                let status_str = status.map(|s| s.as_db_str().to_string());
                if let Err(e) =
                    rt.database
                        .update_agent_status(&repo, number, status_str.as_deref())
                {
                    tracing::warn!("Failed to update agent status for {repo}#{number}: {e}");
                }
            }
            Command::ReReview {
                repo,
                number,
                tmux_window,
            } => {
                let runner = rt.runner.clone();
                let tx = rt.msg_tx.clone();
                let db = rt.database.clone();
                tokio::task::spawn_blocking(move || {
                    let cmd = format!("/review-pr {number}");
                    if let Err(e) = crate::tmux::send_keys(&tmux_window, &cmd, &*runner) {
                        tracing::warn!("Failed to send re-review to {tmux_window}: {e}");
                        return;
                    }
                    if let Err(e) = db.update_agent_status(&repo, number, Some("reviewing")) {
                        tracing::warn!("Failed to update agent status: {e}");
                    }
                    let _ = tx.send(Message::ReviewStatusUpdated {
                        repo,
                        number,
                        status: crate::models::ReviewAgentStatus::Reviewing,
                    });
                });
            }
            // Split mode
            Command::EnterSplitMode => rt.exec_enter_split_mode(app),
            Command::EnterSplitModeWithTask { task_id, window } => {
                rt.exec_enter_split_mode_with_task(app, task_id, &window)
            }
            Command::ExitSplitMode {
                pane_id,
                restore_window,
            } => rt.exec_exit_split_mode(app, &pane_id, restore_window.as_deref()),
            Command::SwapSplitPane {
                task_id,
                new_window,
                old_pane_id,
                old_window,
            } => rt.exec_swap_split_pane(
                app,
                task_id,
                &new_window,
                old_pane_id.as_deref(),
                old_window.as_deref(),
            ),
            Command::FocusSplitPane { pane_id } => {
                if let Err(e) = tmux::select_pane(&pane_id, &*rt.runner) {
                    tracing::warn!("select-pane failed: {e:#}");
                }
            }
            Command::CheckSplitPaneExists { pane_id } => rt.exec_check_split_pane(app, &pane_id),
            Command::RespawnSplitPane { pane_id } => rt.exec_respawn_split_pane(app, &pane_id),
        }
    }

    Ok(())
}

/// Convert raw DB preset tuples into typed presets.
///
/// When `known_repos` is `Some`, each preset's paths are filtered to only
/// include paths present in the set. When `None`, all paths are kept.
fn parse_raw_presets(
    raw: Vec<(String, Vec<String>, String)>,
    known_repos: Option<&HashSet<String>>,
) -> Vec<(String, HashSet<String>, RepoFilterMode)> {
    raw.into_iter()
        .map(|(name, paths, mode_str)| {
            let set: HashSet<String> = if let Some(known) = known_repos {
                paths.into_iter().filter(|p| known.contains(p)).collect()
            } else {
                paths.into_iter().collect()
            };
            let mode = mode_str.parse().unwrap_or_default();
            (name, set, mode)
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
        TuiRuntime {
            task_svc: crate::service::TaskService::new(db.clone()),
            epic_svc: crate::service::EpicService::new(db.clone()),
            database: db,
            msg_tx: tx,
            input_paused: Arc::new(AtomicBool::new(false)),
            runner,
        }
    }

    fn test_runtime() -> (TuiRuntime, App) {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, _rx) = mpsc::unbounded_channel();
        let runner: Arc<dyn ProcessRunner> = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db.clone(), tx, runner);
        let tasks = db.list_all().unwrap();
        let app = App::new(tasks, Duration::from_secs(300));
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
        let id = db.create_task(title, description, repo_path, plan, status, "main")?;
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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
            MockProcessRunner::fail(""),                   // remote get-url (no remote)
            Ok(Output {
                status: exit_fail(),
                stdout: b"".to_vec(),
                stderr: b"CONFLICT (content): Merge conflict in file.rs\nerror: could not apply abc1234\n".to_vec(),
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
            .create_epic("Auth redesign", "Rework login", "/repo")
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

    #[test]
    fn startup_loads_cached_review_prs() {
        use crate::models::{CiStatus, ReviewDecision, ReviewPr};
        use chrono::Utc;

        let (rt, mut app) = test_runtime();

        // Pre-populate the database with a cached review PR
        let pr = ReviewPr {
            number: 42,
            title: "Fix bug".to_string(),
            author: "alice".to_string(),
            repo: "acme/app".to_string(),
            url: "https://github.com/acme/app/pull/42".to_string(),
            is_draft: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            additions: 10,
            deletions: 5,
            review_decision: ReviewDecision::ReviewRequired,
            labels: vec![],
            body: String::new(),
            head_ref: String::new(),
            ci_status: CiStatus::None,
            reviewers: vec![],
            tmux_window: None,
            worktree: None,
            agent_status: None,
        };
        rt.database
            .save_prs(crate::db::PrKind::Review, &[pr])
            .unwrap();

        // Simulate what run_tui does: load cached reviews
        let cached = rt.database.load_prs(crate::db::PrKind::Review).unwrap();
        app.set_review_prs(cached);

        assert_eq!(app.review_prs().len(), 1);
        assert_eq!(app.review_prs()[0].number, 42);
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
        let mut app = App::new(tasks, Duration::from_secs(300));

        rt.exec_quick_dispatch(
            &mut app,
            "My Task".into(),
            "Do stuff".into(),
            repo.to_string(),
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
        let epic = db.create_epic("My Epic", "epic desc", repo).unwrap();
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
        let mut app = App::new(tasks, Duration::from_secs(300));

        rt.exec_quick_dispatch(
            &mut app,
            "Epic Task".into(),
            "do stuff".into(),
            repo.to_string(),
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
        let mut app = App::new(tasks, Duration::from_secs(300));

        // /nonexistent won't have .worktrees dir, so provision_worktree fails
        rt.exec_quick_dispatch(
            &mut app,
            "Fail Task".into(),
            "desc".into(),
            "/nonexistent".into(),
            None,
        );

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
        );
        assert_eq!(app.epics().len(), 1);
        assert_eq!(app.epics()[0].title, "My Epic");
        assert_eq!(rt.database.list_epics().unwrap().len(), 1);
    }

    #[test]
    fn exec_delete_epic_removes_from_db() {
        let (rt, mut app) = test_runtime();
        let epic = rt.database.create_epic("Doomed", "bye", "/repo").unwrap();
        rt.exec_delete_epic(&mut app, epic.id);
        assert!(rt.database.list_epics().unwrap().is_empty());
        assert!(app.error_popup().is_none());
    }

    #[test]
    fn exec_persist_epic_updates_status() {
        let (rt, mut app) = test_runtime();
        let epic = rt.database.create_epic("Epic", "desc", "/repo").unwrap();
        rt.exec_persist_epic(&mut app, epic.id, Some(models::TaskStatus::Running), None);
        let updated = rt.database.get_epic(epic.id).unwrap().unwrap();
        assert_eq!(updated.status, models::TaskStatus::Running);
    }

    #[test]
    fn exec_persist_epic_noop_when_nothing_to_update() {
        let (rt, mut app) = test_runtime();
        let epic = rt.database.create_epic("Epic", "desc", "/repo").unwrap();
        // Should return early without error
        rt.exec_persist_epic(&mut app, epic.id, None, None);
        assert!(app.error_popup().is_none());
    }

    #[test]
    fn exec_refresh_epics_from_db_syncs_to_app() {
        let (rt, mut app) = test_runtime();
        // Insert epic directly into DB, bypassing app
        rt.database.create_epic("Direct", "desc", "/repo").unwrap();
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
    // PR persistence tests
    // -----------------------------------------------------------------------

    #[test]
    fn exec_persist_review_prs_saves_to_db() {
        use crate::models::{CiStatus, ReviewDecision, ReviewPr};
        use chrono::Utc;

        let (rt, mut app) = test_runtime();
        let pr = ReviewPr {
            number: 1,
            title: "Fix".into(),
            author: "alice".into(),
            repo: "acme/app".into(),
            url: "https://github.com/acme/app/pull/1".into(),
            is_draft: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            additions: 5,
            deletions: 2,
            review_decision: ReviewDecision::ReviewRequired,
            labels: vec![],
            body: String::new(),
            head_ref: String::new(),
            ci_status: CiStatus::None,
            reviewers: vec![],
            tmux_window: None,
            worktree: None,
            agent_status: None,
        };
        rt.exec_persist_prs(&mut app, PrListKind::Review, vec![pr]);
        assert_eq!(
            rt.database
                .load_prs(crate::db::PrKind::Review)
                .unwrap()
                .len(),
            1
        );
        assert!(app.error_popup().is_none());
    }

    #[test]
    fn exec_persist_my_prs_saves_to_db() {
        use crate::models::{CiStatus, ReviewDecision, ReviewPr};
        use chrono::Utc;

        let (rt, mut app) = test_runtime();
        let pr = ReviewPr {
            number: 2,
            title: "Feature".into(),
            author: "bob".into(),
            repo: "acme/app".into(),
            url: "https://github.com/acme/app/pull/2".into(),
            is_draft: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            additions: 10,
            deletions: 0,
            review_decision: ReviewDecision::Approved,
            labels: vec![],
            body: String::new(),
            head_ref: String::new(),
            ci_status: CiStatus::None,
            reviewers: vec![],
            tmux_window: None,
            worktree: None,
            agent_status: None,
        };
        rt.exec_persist_prs(&mut app, PrListKind::Authored, vec![pr]);
        assert_eq!(
            rt.database.load_prs(crate::db::PrKind::My).unwrap().len(),
            1
        );
        assert!(app.error_popup().is_none());
    }

    #[test]
    fn exec_persist_bot_prs_saves_to_db() {
        use crate::models::{CiStatus, ReviewDecision, ReviewPr};
        use chrono::Utc;

        let (rt, mut app) = test_runtime();
        let pr = ReviewPr {
            number: 3,
            title: "Bump deps".into(),
            author: "dependabot[bot]".into(),
            repo: "acme/app".into(),
            url: "https://github.com/acme/app/pull/3".into(),
            is_draft: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            additions: 1,
            deletions: 1,
            review_decision: ReviewDecision::ReviewRequired,
            labels: vec![],
            body: String::new(),
            head_ref: String::new(),
            ci_status: CiStatus::None,
            reviewers: vec![],
            tmux_window: None,
            worktree: None,
            agent_status: None,
        };
        rt.exec_persist_prs(&mut app, PrListKind::Bot, vec![pr]);
        assert_eq!(
            rt.database.load_prs(crate::db::PrKind::Bot).unwrap().len(),
            1
        );
        assert!(app.error_popup().is_none());
    }

    #[test]
    fn exec_persist_security_alerts_saves_to_db() {
        use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
        use chrono::Utc;

        let (rt, mut app) = test_runtime();
        let alert = SecurityAlert {
            number: 1,
            repo: "acme/app".into(),
            severity: AlertSeverity::High,
            kind: AlertKind::Dependabot,
            title: "CVE-2024-1234".into(),
            package: Some("lodash".into()),
            vulnerable_range: Some("< 4.17.21".into()),
            fixed_version: Some("4.17.21".into()),
            cvss_score: Some(7.5),
            url: "https://github.com/acme/app/security/dependabot/1".into(),
            created_at: Utc::now(),
            state: "open".into(),
            description: "Prototype pollution".into(),
            tmux_window: None,
            worktree: None,
            agent_status: None,
        };
        rt.exec_persist_security_alerts(&mut app, vec![alert]);
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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        let mut app = App::new(tasks, Duration::from_secs(300));

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
            MockProcessRunner::ok(),                    // select-pane (focus right pane)
        ]));
        let rt = make_runtime(db.clone(), tx, mock.clone());
        let tasks = db.list_all().unwrap();
        let mut app = App::new(tasks, Duration::from_secs(300));

        rt.exec_swap_split_pane(&mut app, TaskId(1), "task-1", Some("%2"), None);
        let calls = mock.recorded_calls();
        // 1st call: display-message to get new pane ID
        assert!(calls[0].1.contains(&"display-message".to_string()));
        // 2nd call: swap-pane
        assert!(calls[1].1.contains(&"swap-pane".to_string()));
        // 3rd call: kill-window (no old task to rename)
        assert!(calls[2].1.contains(&"kill-window".to_string()));
        // 4th call: select-pane to focus the right pane
        assert!(calls[3].1.contains(&"select-pane".to_string()));
        assert!(calls[3].1.contains(&"%5".to_string()));
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
            MockProcessRunner::ok(),                    // select-pane (focus right pane)
        ]));
        let rt = make_runtime(db.clone(), tx, mock.clone());
        let tasks = db.list_all().unwrap();
        let mut app = App::new(tasks, Duration::from_secs(300));

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
        // 4th call: select-pane to focus the right pane
        assert!(calls[3].1.contains(&"select-pane".to_string()));
        assert!(calls[3].1.contains(&"%5".to_string()));
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
            MockProcessRunner::ok(), // gh pr merge --merge
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
    // load_github_queries comment filtering
    // -----------------------------------------------------------------------

    #[test]
    fn load_github_queries_strips_comment_lines() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        db.set_setting_string(
            "github_queries_bot",
            "# All comments\n# Another comment\n# Final",
        )
        .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock);
        assert!(rt.load_github_queries("github_queries_bot").is_empty());
    }

    #[test]
    fn load_github_queries_strips_mixed_comments_and_queries() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        db.set_setting_string(
            "github_queries_bot",
            "# header comment\nis:pr is:open author:app/dependabot org:myorg\n# mid comment\nis:pr is:open author:app/renovate org:myorg",
        )
        .unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock);
        let queries = rt.load_github_queries("github_queries_bot");
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0], "is:pr is:open author:app/dependabot org:myorg");
        assert_eq!(queries[1], "is:pr is:open author:app/renovate org:myorg");
    }

    // -----------------------------------------------------------------------
    // Fetch PR tests (no queries configured → empty results)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn exec_fetch_review_prs_no_queries_returns_empty() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        // No mock calls needed — empty queries short-circuits in fetch_prs
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_fetch_prs(PrListKind::Review);

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg {
            Message::PrsLoaded(PrListKind::Review, prs) => assert!(prs.is_empty()),
            other => panic!("Expected PrsLoaded(Review, _), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn exec_fetch_my_prs_no_queries_returns_empty() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_fetch_prs(PrListKind::Authored);

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg {
            Message::PrsLoaded(PrListKind::Authored, prs) => assert!(prs.is_empty()),
            other => panic!("Expected PrsLoaded(Authored, _), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn exec_fetch_bot_prs_no_queries_sends_not_configured() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Mock runner must never be called — not-configured short-circuits before fetch
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_fetch_prs(PrListKind::Bot);

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::PrsFetchFailed(PrListKind::Bot, _)),
            "Expected PrsFetchFailed(Bot, _) when queries not configured, got: {msg:?}"
        );
    }

    #[tokio::test]
    async fn exec_fetch_review_prs_gh_failure() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        // Configure a query so fetch_prs actually calls gh
        db.set_setting_string("github_queries_review", "is:pr review-requested:@me")
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::fail("gh auth failure"), // gh api graphql fails
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_fetch_prs(PrListKind::Review);

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::PrsFetchFailed(PrListKind::Review, _)),
            "Expected PrsFetchFailed(Review, _), got: {msg:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Batch operations
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn exec_batch_approve_prs_approves_all() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // gh pr review --approve (PR 1)
            MockProcessRunner::ok(), // gh pr review --approve (PR 2)
        ]));
        let rt = make_runtime(db, tx, mock.clone());

        rt.exec_batch_approve_prs(vec![
            "https://github.com/org/repo/pull/1".into(),
            "https://github.com/org/repo/pull/2".into(),
        ]);

        // Should send RefreshBotPrs then StatusInfo
        let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg1, Message::RefreshBotPrs),
            "Expected RefreshBotPrs, got: {msg1:?}"
        );

        let msg2 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg2 {
            Message::StatusInfo(s) => assert!(s.contains("Approved 2/2")),
            other => panic!("Expected StatusInfo, got: {other:?}"),
        }

        // Verify gh was called correctly
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "gh");
        assert!(calls[0].1.contains(&"--approve".to_string()));
    }

    #[tokio::test]
    async fn exec_batch_approve_prs_partial_failure() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok(),                // PR 1 succeeds
            MockProcessRunner::fail("not allowed"), // PR 2 fails
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_batch_approve_prs(vec![
            "https://github.com/org/repo/pull/1".into(),
            "https://github.com/org/repo/pull/2".into(),
        ]);

        let _refresh = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg {
            Message::StatusInfo(s) => assert!(s.contains("Approved 1/2")),
            other => panic!("Expected StatusInfo with 1/2, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn exec_batch_merge_prs_merges_all() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // gh pr merge --merge (PR 1)
            MockProcessRunner::ok(), // gh pr merge --merge (PR 2)
        ]));
        let rt = make_runtime(db, tx, mock.clone());

        rt.exec_batch_merge_prs(vec![
            "https://github.com/org/repo/pull/1".into(),
            "https://github.com/org/repo/pull/2".into(),
        ]);

        // First: BotPrsMerged with all successfully merged URLs
        let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match &msg1 {
            Message::BotPrsMerged(urls) => {
                assert!(urls.contains(&"https://github.com/org/repo/pull/1".to_string()));
                assert!(urls.contains(&"https://github.com/org/repo/pull/2".to_string()));
            }
            other => panic!("Expected BotPrsMerged, got: {other:?}"),
        }

        // Second: RefreshBotPrs
        let msg2 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg2, Message::RefreshBotPrs),
            "Expected RefreshBotPrs, got: {msg2:?}"
        );

        // Third: StatusInfo
        let msg3 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg3 {
            Message::StatusInfo(s) => assert!(s.contains("Merged 2/2")),
            other => panic!("Expected StatusInfo, got: {other:?}"),
        }

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].1.contains(&"--merge".to_string()));
    }

    #[tokio::test]
    async fn exec_batch_merge_prs_partial_failure() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::fail("checks pending"), // PR 1 fails
            MockProcessRunner::ok(),                   // PR 2 succeeds
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_batch_merge_prs(vec![
            "https://github.com/org/repo/pull/1".into(),
            "https://github.com/org/repo/pull/2".into(),
        ]);

        // First: BotPrsMerged with only the successful URL (PR 2)
        let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match &msg1 {
            Message::BotPrsMerged(urls) => {
                assert_eq!(urls.len(), 1);
                assert!(urls.contains(&"https://github.com/org/repo/pull/2".to_string()));
            }
            other => panic!("Expected BotPrsMerged, got: {other:?}"),
        }

        // Second: RefreshBotPrs
        let _refresh = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();

        // Third: StatusInfo
        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg {
            Message::StatusInfo(s) => assert!(s.contains("Merged 1/2")),
            other => panic!("Expected StatusInfo with 1/2, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn exec_batch_merge_prs_all_fail_emits_no_bot_prs_merged() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::fail("network error"), // PR 1 fails
            MockProcessRunner::fail("network error"), // PR 2 fails
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_batch_merge_prs(vec![
            "https://github.com/org/repo/pull/1".into(),
            "https://github.com/org/repo/pull/2".into(),
        ]);

        // First message should be RefreshBotPrs (no BotPrsMerged when nothing merged)
        let msg1 = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg1, Message::RefreshBotPrs),
            "Expected RefreshBotPrs (no BotPrsMerged when nothing merged), got: {msg1:?}"
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
    // Security alerts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn exec_fetch_security_alerts_gh_failure() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::fail("auth error"), // gh api graphql fails
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_fetch_security_alerts();

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::SecurityAlertsFetchFailed(_)),
            "Expected SecurityAlertsFetchFailed, got: {msg:?}"
        );
    }

    #[tokio::test]
    async fn exec_fetch_security_alerts_empty_result() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({
            "data": {
                "viewer": {
                    "repositories": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": []
                    }
                }
            }
        });
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(json.to_string().as_bytes()),
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_fetch_security_alerts();

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match msg {
            Message::SecurityAlertsLoaded(alerts) => assert!(alerts.is_empty()),
            other => panic!("Expected SecurityAlertsLoaded, got: {other:?}"),
        }
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

    // -----------------------------------------------------------------------
    // Dispatch fix/review agents
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn exec_dispatch_fix_agent_sends_dispatched() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_str().unwrap();
        // provision_and_dispatch uses worktree_name = "fix-vuln-{number}"
        std::fs::create_dir_all(format!("{repo}/.worktrees/fix-vuln-1")).unwrap();

        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"\n"), // has_window (list-windows, no match)
            MockProcessRunner::ok(),                  // git worktree prune
            MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"), // symbolic-ref
            MockProcessRunner::ok(),                  // git fetch origin main
            // worktree dir exists, skip git worktree add
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_dispatch_fix_agent(tui::FixAgentRequest {
            repo: repo.to_string(),
            github_repo: "acme/app".into(),
            number: 1,
            kind: models::AlertKind::Dependabot,
            title: "CVE-2024-1234".into(),
            description: "Fix this vuln".into(),
            package: Some("lodash".into()),
            fixed_version: Some("4.17.21".into()),
        });

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::FixAgentDispatched { number: 1, .. }),
            "Expected FixAgentDispatched, got: {msg:?}"
        );
    }

    #[tokio::test]
    async fn exec_dispatch_fix_agent_failure() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
            "tmux not running",
        )]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_dispatch_fix_agent(tui::FixAgentRequest {
            repo: "/nonexistent".into(),
            github_repo: "acme/app".into(),
            number: 1,
            kind: models::AlertKind::Dependabot,
            title: "CVE".into(),
            description: "desc".into(),
            package: None,
            fixed_version: None,
        });

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::FixAgentFailed { number: 1, .. }),
            "Expected FixAgentFailed, got: {msg:?}"
        );
    }

    #[tokio::test]
    async fn exec_dispatch_review_agent_sends_dispatched() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_str().unwrap();
        // provision_and_dispatch uses worktree_name = "review-{number}"
        std::fs::create_dir_all(format!("{repo}/.worktrees/review-42")).unwrap();

        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"\n"), // has_window (no match)
            MockProcessRunner::ok(),                  // git worktree prune
            MockProcessRunner::ok(),                  // git fetch origin fix-branch
            // worktree dir exists, skip git worktree add
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux send-keys -l
            MockProcessRunner::ok(), // tmux send-keys Enter
        ]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_dispatch_review_agent(tui::ReviewAgentRequest {
            repo: repo.to_string(),
            github_repo: "acme/app".into(),
            number: 42,
            head_ref: "fix-branch".into(),
            is_dependabot: false,
        });

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::ReviewAgentDispatched { number: 42, .. }),
            "Expected ReviewAgentDispatched, got: {msg:?}"
        );
    }

    #[tokio::test]
    async fn exec_dispatch_review_agent_failure() {
        let db: Arc<dyn db::TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
            "tmux not running",
        )]));
        let rt = make_runtime(db, tx, mock);

        rt.exec_dispatch_review_agent(tui::ReviewAgentRequest {
            repo: "/nonexistent".into(),
            github_repo: "acme/app".into(),
            number: 42,
            head_ref: "fix-branch".into(),
            is_dependabot: false,
        });

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(msg, Message::ReviewAgentFailed { number: 42, .. }),
            "Expected ReviewAgentFailed, got: {msg:?}"
        );
    }
}
