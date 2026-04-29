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

/// Interval between TUI tick events (captures tmux output, checks staleness, etc.).
const TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Name used for the TUI's tmux window (visible in tmux status bar).
const TUI_WINDOW_NAME: &str = "TUI";

use crate::db::{PrWorkflowStore, ProjectCrud, SettingsStore, TaskCrud};
use crate::models::TaskId;
use crate::process::{ProcessRunner, RealProcessRunner};
use crate::service::FieldUpdate;
use crate::tui::{self, App, Command, Message, RepoFilterMode};
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
    if std::env::var("TMUX").is_err() {
        anyhow::bail!("dispatch tui must be run inside a tmux session (TMUX is not set)");
    }

    // 1. Open database and load initial tasks
    let database = Arc::new(db::Database::open(db_path)?);
    let tasks = database.list_all()?;

    // 2. Spawn MCP server with notification channel
    let runner: Arc<dyn ProcessRunner> = Arc::new(RealProcessRunner);
    let mcp_db = database.clone();
    let mcp_runner = runner.clone();
    let (mcp_notify_tx, mut mcp_notify_rx) = mpsc::unbounded_channel::<mcp::McpEvent>();
    let feed_notify_tx = mcp_notify_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = mcp::serve(mcp_db, port, mcp_notify_tx, mcp_runner).await {
            eprintln!("MCP server error: {e}");
        }
    });

    // 3. Create App and load saved repo paths
    let projects = database.list_projects()?;
    let saved_project = database.get_setting_string("last_project").ok().flatten();
    let initial_project_id = resolve_initial_project(&projects, saved_project);
    let mut app = App::new(
        tasks,
        initial_project_id,
        Duration::from_secs(inactivity_timeout),
    );
    app.update(Message::ProjectsUpdated(projects));
    let paths = database.list_repo_paths().unwrap_or_default();
    app.update(Message::RepoPathsUpdated(paths));
    let usage = database.get_all_usage().unwrap_or_default();
    app.update(Message::RefreshUsage(usage));

    // Seed default GitHub query strings (no-op if already set)
    if let Err(e) = database.seed_github_query_defaults() {
        app.update(Message::StatusInfo(format!(
            "Failed to seed GitHub query defaults: {e}"
        )));
    }

    load_notifications_pref(&*database, &mut app);
    load_repo_filter(&*database, &mut app);
    load_repo_filter_mode(&*database, &mut app);
    for msg in [
        load_filter_presets(&*database, &mut app),
        apply_tmux_focus_warning(&*runner),
    ]
    .into_iter()
    .flatten()
    {
        app.update(msg);
    }

    // Prune stale "done" workflow rows on startup (best-effort)
    if let Err(e) = database.prune_done_pr_workflows(chrono::Duration::days(7)) {
        tracing::warn!("Failed to prune done pr workflows on startup: {e}");
    }

    // Load tips and show popup if appropriate
    let tips = crate::tips::embedded_tips();
    let (seen_up_to, show_mode) = database
        .get_tips_state()
        .unwrap_or((0, crate::models::TipsShowMode::Always));
    if let Some(starting_index) = tips_starting_index(&tips, seen_up_to, show_mode) {
        app.update(Message::ShowTips {
            tips,
            starting_index,
            max_seen_id: seen_up_to,
            show_mode,
        });
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
                Ok(Event::Key(key)) if key_tx.send(key).is_err() => break,
                Ok(Event::Key(_)) => {}
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

    let mut runtime = TuiRuntime {
        task_svc: crate::service::TaskService::new(database.clone()),
        epic_svc: crate::service::EpicService::new(database.clone()),
        feed_runner: Some(crate::feed::FeedRunner::new(
            database.clone(),
            feed_notify_tx,
        )),
        database,
        msg_tx,
        runner,
        editor_session: Arc::new(std::sync::Mutex::new(None)),
    };
    let result = run_loop(
        &mut app,
        &mut terminal,
        &mut key_rx,
        &mut msg_rx,
        &mut mcp_notify_rx,
        &mut tick_interval,
        &mut runtime,
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
// TuiRuntime — shared context for command execution
// ---------------------------------------------------------------------------

struct TuiRuntime {
    // Holds the widest TaskStore supertrait because execute_commands dispatches
    // to helpers spanning all four sub-traits (TaskAndEpicStore, PrStore,
    // AlertStore, SettingsStore). See CLAUDE.md §DB trait narrowing for the
    // narrowing discipline applied in TaskService and EpicService.
    database: Arc<dyn db::TaskStore>,
    task_svc: crate::service::TaskService,
    epic_svc: crate::service::EpicService,
    msg_tx: mpsc::UnboundedSender<Message>,
    runner: Arc<dyn ProcessRunner>,
    /// Holds the in-flight pop-out editor session, if any. `None` means no
    /// editor is currently open. We enforce "at most one editor at a time"
    /// by refusing to start a new one while this slot is populated.
    editor_session: Arc<std::sync::Mutex<Option<editor::EditorSession>>>,
    feed_runner: Option<crate::feed::FeedRunner>,
}

mod agents;
mod commands;
mod editor;
mod epics;
mod learnings;
mod pr;
mod settings;
mod split;
mod tasks;
#[cfg(test)]
mod tests;

impl TuiRuntime {
    fn db_error(action: &str, e: impl std::fmt::Display) -> String {
        format!("DB error {action}: {e}")
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
    rt: &mut TuiRuntime,
) -> Result<()> {
    // Here (not in TuiRuntime::new) so tests that construct TuiRuntime directly
    // don't accidentally spawn background tasks.
    if let Some(feed_runner) = rt.feed_runner.take() {
        feed_runner.start();
    }

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
                }
            }

            // Periodic tick for tmux capture and feed polling
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
    cmds: Vec<Command>,
    rt: &TuiRuntime,
    _terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    _key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
) -> Result<()> {
    let mut queue = std::collections::VecDeque::from(cmds);
    while let Some(command) = queue.pop_front() {
        let extra = commands::dispatch(command, app, rt);
        queue.extend(extra);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// init load helpers — extracted from run_tui's startup block
// ---------------------------------------------------------------------------

fn load_notifications_pref(db: &dyn db::SettingsStore, app: &mut App) {
    let enabled = db
        .get_setting_bool("notifications_enabled")
        .unwrap_or(None)
        .unwrap_or(false);
    app.set_notifications_enabled(enabled);
}

/// Intersect the saved repo filter with the app's known repo paths to prune
/// stale entries. No-op when no filter has been saved.
fn load_repo_filter(db: &dyn db::SettingsStore, app: &mut App) {
    let Some(filter_str) = db.get_setting_string("repo_filter").unwrap_or(None) else {
        return;
    };
    if filter_str.is_empty() {
        return;
    }
    let known: HashSet<&str> = app.repo_paths().iter().map(|s| s.as_str()).collect();
    let paths: Vec<String> = serde_json::from_str(&filter_str).unwrap_or_default();
    let filter: HashSet<String> = paths
        .into_iter()
        .filter(|s| known.contains(s.as_str()))
        .collect();
    app.set_repo_filter(filter);
}

fn load_repo_filter_mode(db: &dyn db::SettingsStore, app: &mut App) {
    if let Some(mode_str) = db.get_setting_string("repo_filter_mode").unwrap_or(None) {
        app.set_repo_filter_mode(mode_str.parse().unwrap_or_default());
    }
}

fn load_filter_presets(db: &dyn db::SettingsStore, app: &mut App) -> Option<Message> {
    match db.list_filter_presets() {
        Ok(raw) => {
            let _ = app.update(Message::FilterPresetsLoaded(parse_raw_presets(raw, None)));
            None
        }
        Err(e) => Some(Message::StatusInfo(format!(
            "Failed to load filter presets: {e}"
        ))),
    }
}

fn apply_tmux_focus_warning(runner: &dyn ProcessRunner) -> Option<Message> {
    if !crate::tmux::focus_events_enabled(runner) {
        Some(Message::StatusInfo(
            "tmux focus-events is off \u{2014} split-view focus indicator won't work. Run: tmux set -g focus-events on".to_string(),
        ))
    } else {
        None
    }
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

/// Determines which index to start the tips overlay at, or `None` if tips
/// should not be shown. Pure function — enables unit testing of startup logic.
pub fn tips_starting_index(
    tips: &[crate::tips::Tip],
    seen_up_to: u32,
    show_mode: crate::models::TipsShowMode,
) -> Option<usize> {
    use crate::models::TipsShowMode;

    if tips.is_empty() {
        return None;
    }

    match show_mode {
        TipsShowMode::Never => None,
        TipsShowMode::NewOnly | TipsShowMode::Always => {
            if let Some(idx) = tips.iter().position(|t| t.id > seen_up_to) {
                Some(idx)
            } else if show_mode == TipsShowMode::Always {
                // No new tips but show anyway — pick a time-based pseudo-random index
                let idx = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize)
                    % tips.len();
                Some(idx)
            } else {
                None // NewOnly + no new tips
            }
        }
    }
}

/// Returns the project id to open at startup.
/// Prefers the saved `last_project` setting; falls back to the default project.
fn resolve_initial_project(
    projects: &[crate::models::Project],
    saved: Option<String>,
) -> crate::models::ProjectId {
    let default_id = projects
        .iter()
        .find(|p| p.is_default)
        .map(|p| p.id)
        .expect("no default project — database invariant violated");

    let Some(raw) = saved else {
        return default_id;
    };
    let Ok(id) = raw.parse::<crate::models::ProjectId>() else {
        return default_id;
    };
    if projects.iter().any(|p| p.id == id) {
        id
    } else {
        default_id
    }
}
