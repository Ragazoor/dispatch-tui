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

mod agents;
mod epics;
mod pr;
mod security;
mod settings;
mod split;
mod tasks;
#[cfg(test)]
mod tests;

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

        // Drain any keystrokes buffered in the OS terminal while the editor was
        // running. The polling thread checks input_paused every 100ms and then
        // polls for up to 50ms, so allow 200ms for it to flush OS-buffered events
        // before we clear the channel. Without this, editor keystrokes (e.g. `:wq`)
        // arrive in key_rx and get processed by whatever InputMode is active next.
        std::thread::sleep(Duration::from_millis(200));
        while key_rx.try_recv().is_ok() {}

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
            Command::InsertEpic(draft) => rt.exec_insert_epic(
                app,
                draft.title,
                draft.description,
                draft.repo_path,
                draft.parent_epic_id,
            ),
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
            Command::ApproveBotPr(url) => rt.exec_approve_bot_pr(url),
            Command::MergeBotPr(url) => rt.exec_merge_bot_pr(url),
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
