use anyhow::Result;
use crossterm::{
    event::{self, DisableFocusChange, EnableFocusChange, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::Terminal;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;

/// Interval between TUI tick events (captures tmux output, checks staleness, etc.).
const TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Minimum time between rendered frames (~60 fps cap).  Rapid key-repeat events
/// that arrive faster than this are processed but coalesced into a single render.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Sleep duration when the input thread is paused (e.g. while an editor is open).
const INPUT_PAUSE_SLEEP: Duration = Duration::from_millis(100);

/// Poll timeout for crossterm input events.
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long shutdown waits for the split-pane restore a quit issued before
/// abandoning it. Matches `config.quit_restore_timeout` in
/// `docs/specs/split-pane.allium`.
const QUIT_RESTORE_TIMEOUT: Duration = Duration::from_secs(2);

/// Name used for the TUI's tmux window (visible in tmux status bar).
/// The board's own tmux window name. One definition, shared with the launch
/// path that retires it — see [`crate::startup::BOARD_WINDOW_NAME`].
const TUI_WINDOW_NAME: TmuxWindow = crate::startup::BOARD_WINDOW_NAME;

/// Key (after the tmux prefix) that toggles a companion agent-tree pane's
/// visibility in whichever agent window it's pressed in. Matches
/// config.agent_tree_toggle_key in docs/specs/agent-tree.allium.
const AGENT_TREE_TOGGLE_KEY: &str = "e";

/// Command bound to [`AGENT_TREE_TOGGLE_KEY`]. `#{window_name}` is expanded
/// by tmux itself, before invoking the shell, to the name of whichever window
/// was focused when the key was pressed — so `dispatch toggle-agent-tree-pane`
/// is handed the target window without this process ever having to ask tmux
/// which window is focused. `-b` backgrounds the shell job so the keypress
/// doesn't block the tmux client.
const AGENT_TREE_TOGGLE_COMMAND: &str = concat!(
    "run-shell -b \"",
    crate::process::dispatch_program!(),
    " toggle-agent-tree-pane '#{window_name}'\""
);

use crate::db::{HostStore, RepoConfigRead, TaskRead};
use crate::models::{TaskId, TmuxWindow};
use crate::process::{ProcessRunner, RealProcessRunner};
use crate::service::embeddings::EmbeddingService;
use crate::service::FieldUpdate;
use crate::tui::{
    self, App, Command, Message, RepoFilterMode, SectionFoldState, COLLAPSED_SECTIONS_KEY,
};
use crate::{db, dispatch, mcp, models, tmux};

/// Convert `Option<String>` to `FieldUpdate`: `Some(v)` → `Set(v)`, `None` → `Clear`.
fn option_to_field_update(opt: Option<String>) -> FieldUpdate {
    match opt {
        Some(v) => FieldUpdate::Set(v),
        None => FieldUpdate::Clear,
    }
}

/// [`option_to_field_update`] for the typed window field.
fn option_to_tmux_window_update(opt: Option<TmuxWindow>) -> crate::service::TmuxWindowUpdate {
    match opt {
        Some(w) => crate::service::TmuxWindowUpdate::Set(w),
        None => crate::service::TmuxWindowUpdate::Clear,
    }
}

/// Fold `(repo_path, branch)` pairs (as returned by `list_all_base_branches`,
/// ordered by `last_used DESC`) into a per-repo history map, preserving
/// recency order within each repo's `Vec`.
pub(super) fn group_base_branches_by_repo(
    pairs: Vec<(String, String)>,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for (repo, branch) in pairs {
        map.entry(repo).or_default().push(branch);
    }
    map
}

/// Set up tmux for the TUI: rename the current window and bind Prefix+Space
/// to jump back to the TUI window.
/// `session` and `self_pane` are read by the caller rather than here: the
/// session comes from the probe `run_tui` already makes, and reading
/// `$TMUX_PANE` here would leave every test at the mercy of an environment the
/// harness shares across threads.
fn setup_tmux_for_tui(session: &str, self_pane: Option<&str>, runner: &dyn ProcessRunner) {
    // The rename must target this process's own pane, and only that. Every
    // fallback resolves to the session's *active* pane instead — `-t ""` and
    // `current_pane_id` alike (`tmux::self_pane_id`) — so without `$TMUX_PANE`
    // the choice is between renaming the wrong window and renaming none. The
    // name assigned here is the one the next launch retires by, so a wrong
    // rename is worse than none; and both window-creating paths already pass
    // `-n`, which leaves this path rare.
    //
    // The keybindings below are session-wide and do not depend on the pane, so
    // they are set either way.
    if let Some(target) = self_pane {
        // Scoped to this session, not the whole server: the board's window name
        // is unique per session (startup.allium's `config.board_window_name`),
        // so a `TUI` window in a session dispatch does not own must not stop
        // this board adopting the name — a board that never adopts it is one
        // the next launch in this session retires nothing of, starting a rival
        // beside it. The helper leaves a name the window already carries alone.
        let _ = tmux::rename_window_in_session(session, target, &TUI_WINDOW_NAME, runner);
    } else {
        tracing::warn!("no $TMUX_PANE: leaving this window's name alone");
    }
    // `=` anchors the target to an exact name match. tmux otherwise resolves a
    // `-t <name>` by prefix, so a window whose name merely starts with
    // TUI_WINDOW_NAME could absorb this jump. Unlike every other window target
    // in the codebase this one cannot go through `tmux::window_target`: the
    // binding is a string tmux executes later, and a pane ID captured now would
    // be stale by then. The sigil does work for `select-window` specifically
    // (verified against tmux 3.5a; it does not for `send-keys` or
    // `set-option -w`). See `tmux::window_target` for the full picture.
    let _ = tmux::bind_key(
        "space",
        &format!("select-window -t ={TUI_WINDOW_NAME}"),
        runner,
    );
    let _ = tmux::bind_key(AGENT_TREE_TOGGLE_KEY, AGENT_TREE_TOGGLE_COMMAND, runner);
}

/// Tear down tmux TUI state: unbind the keys and restore the original window name.
fn teardown_tmux_for_tui(
    session: &str,
    original_name: Option<&TmuxWindow>,
    runner: &dyn ProcessRunner,
) {
    let _ = tmux::unbind_key("space", runner);
    let _ = tmux::unbind_key(AGENT_TREE_TOGGLE_KEY, runner);
    if let Some(name) = original_name {
        // Session-scoped for the same reason the outbound rename is: resolving
        // `TUI` server-wide could restore the operator's old window name onto a
        // board in a session this process does not own.
        let _ = tmux::rename_window_in_session(session, TUI_WINDOW_NAME.as_str(), name, runner);
    }
}

// ---------------------------------------------------------------------------
// Bootstrap — composition root for TuiRuntime startup
// ---------------------------------------------------------------------------

/// The `$HOME`-derived locations TUI startup needs, resolved once by
/// [`StartupPaths::resolve`] and then passed around.
///
/// Startup never looks these up itself: it is handed them. That is what keeps
/// a run which is not the operator's own session — a test booting the runtime
/// above all — off the operator's real configuration, with no lookup left
/// inside `bootstrap` to reach it. See docs/specs/dispatch.allium:
/// StatusLineDecorator, `SettingsLocationIsAnExplicitStartupInput`.
///
/// The same shape, and the same reason, as `setup::SetupPaths` and
/// `setup::UninstallPaths`.
pub struct StartupPaths {
    /// Claude Code's configuration directory (`~/.claude`), holding the
    /// dispatch-owned statusLine settings file and dispatch's own plugin. The
    /// startup configuration check is handed it via [`StartupPaths::setup_paths`];
    /// nothing downstream looks it up again.
    claude_dir: std::path::PathBuf,
    /// Claude Code's trust store (`~/.claude.json`), read and written by the
    /// trust-gated dispatch arms via `TuiRuntime::claude_json_path`.
    claude_json_path: std::path::PathBuf,
}

impl StartupPaths {
    /// Resolve the real `$HOME`-derived locations used in production. The one
    /// call site is `src/main.rs`; everything downstream takes the resolved
    /// struct.
    ///
    /// The home directory is read once here — the outermost edge of the flow —
    /// and both locations are composed from that one reading, so they cannot
    /// disagree about whether it was available. See docs/specs/dispatch.allium:
    /// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
    pub fn resolve() -> Result<Self> {
        let home = crate::setup::home_dir()?;
        Ok(Self {
            claude_dir: crate::setup::claude_dir_in(&home),
            claude_json_path: crate::setup::user_global_config_path_in(&home),
        })
    }

    /// The configuration locations the startup drift check reads and writes,
    /// composed from the two this struct was handed plus the two that are fixed
    /// per machine rather than per configuration directory.
    ///
    /// This is what keeps `SettingsLocationIsAnExplicitStartupInput` true now
    /// that the settings file is written by the drift check rather than by
    /// `bootstrap`: the check is *handed* the directory too, through the same
    /// one resolved set, and goes looking for none of it. A run that is not the
    /// operator's session is handed locations of its own and so cannot reach
    /// theirs.
    pub fn setup_paths(&self) -> Result<crate::setup::SetupPaths> {
        crate::setup::SetupPaths::under(&self.claude_dir, &self.claude_json_path)
    }
}

/// Everything built by `TuiRuntime::bootstrap` that `run_tui` needs after
/// the composition root returns.
struct Bootstrap {
    app: App,
    runtime: TuiRuntime,
    mcp_notify_rx: mpsc::UnboundedReceiver<mcp::McpEvent>,
    msg_rx: mpsc::UnboundedReceiver<Message>,
}

// ---------------------------------------------------------------------------
// run_tui — entry point for the TUI mode
// ---------------------------------------------------------------------------

/// `paths` carries the operator's `$HOME`-derived locations, resolved by the
/// caller — see [`StartupPaths`].
pub async fn run_tui(
    db_path: &Path,
    port: u16,
    paths: &StartupPaths,
    spacetime_server: Option<String>,
) -> Result<()> {
    // Defence in depth. `src/main.rs` puts the process inside a session before
    // this is reached, so on the real path this never fires — but the board's
    // tmux work (window naming, keybindings, agent panes) is meaningless
    // without one, and a future caller that skips the handoff should be told
    // rather than half-work. Shares the launch path's predicate so the two
    // cannot disagree about what "inside tmux" means.
    if !crate::startup::inside_tmux_session() {
        anyhow::bail!("dispatch tui must be run inside a tmux session (TMUX is not set)");
    }

    let Bootstrap {
        mut app,
        mut runtime,
        mut mcp_notify_rx,
        mut msg_rx,
    } = TuiRuntime::bootstrap(db_path, port, paths, spacetime_server).await?;

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableFocusChange)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set up tmux keybinding: Prefix+Space → jump back to this window.
    // Best-effort: failures don't prevent the TUI from starting.
    let tmux_runner = runtime.runner.clone();
    // One probe for the session and the window name together, rather than one
    // each: they are read for the same purpose and a window renamed between two
    // calls would be described by neither answer. See
    // `tmux::current_window_context`.
    let self_pane = tmux::self_pane_id();
    let here = tmux::current_window_context(self_pane.as_deref(), &*tmux_runner).ok();
    let original_window_name = here
        .as_ref()
        .and_then(|c| TmuxWindow::parse(&c.window_name));
    let session = here.as_ref().map_or("", |c| c.session_name.as_str());
    setup_tmux_for_tui(session, self_pane.as_deref(), &*tmux_runner);

    // Create two channels:
    //    - key_rx: raw crossterm KeyEvents from the blocking poll thread
    //    - msg_rx: higher-level Messages (e.g. from dispatch results)
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<crossterm::event::KeyEvent>();

    // crossterm::event::poll/read are blocking; run them in a dedicated thread
    // so they don't block the async runtime. The thread can be paused (e.g. when
    // opening an external editor) via the input_paused flag.
    let input_paused = Arc::new(AtomicBool::new(false));
    let paused_clone = input_paused.clone();
    let resize_tx = runtime.msg_tx.clone();
    tokio::task::spawn_blocking(move || loop {
        if paused_clone.load(Ordering::Relaxed) {
            std::thread::sleep(INPUT_PAUSE_SLEEP);
            continue;
        }
        if event::poll(EVENT_POLL_INTERVAL).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key_tx.send(key).is_err() => break,
                Ok(Event::Key(_)) => {}
                Ok(Event::Resize(..)) => {
                    let _ = resize_tx.send(Message::System(
                        crate::tui::messages::SystemMessage::TerminalResized,
                    ));
                }
                Ok(Event::FocusGained) => {
                    let _ = resize_tx.send(Message::System(
                        crate::tui::messages::SystemMessage::FocusChanged(true),
                    ));
                }
                Ok(Event::FocusLost) => {
                    let _ = resize_tx.send(Message::System(
                        crate::tui::messages::SystemMessage::FocusChanged(false),
                    ));
                }
                _ => {}
            }
        }
    });

    // Tick interval (2 seconds)
    let mut tick_interval = interval(TICK_INTERVAL);

    tracing::info!(port, db = %db_path.display(), "TUI started, MCP server on port {port}");

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

    // Before anything else touches tmux. A quit with a task pinned issued a
    // break-pane that moves a live agent's pane between windows; the board's own
    // tidying below must not interleave with it, and the process must not go
    // away while it is still running. See `QuitAwaitsSplitPaneRestore` in
    // docs/specs/split-pane.allium.
    //
    // The alternate screen is still up here, so a tmux that has stopped
    // answering leaves the last frame on screen for up to QUIT_RESTORE_TIMEOUT
    // with no feedback. Accepted: the alternative is tearing the terminal down
    // around a rearrangement that is moving a live agent.
    await_split_restores(runtime.take_split_restores(), QUIT_RESTORE_TIMEOUT).await;

    // Tear down tmux keybinding and restore the original window name.
    teardown_tmux_for_tui(session, original_window_name.as_ref(), &*tmux_runner);

    // Cleanup terminal
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
// Embedding backfill — run at startup to embed learnings missing vectors
// ---------------------------------------------------------------------------

/// Backfills embeddings for any learnings that have no embedding stored.
///
/// Runs at startup in a background task. Failures are logged via `tracing::warn`
/// by the caller; this function propagates errors so the caller can decide.
pub(crate) async fn backfill_embeddings(
    db: Arc<dyn crate::db::LearningStore + Send + Sync>,
    emb_svc: Arc<EmbeddingService>,
) -> Result<()> {
    use crate::service::embeddings::{embed_text_for_learning, serialize_embedding};

    let missing = db.list_learnings_missing_embedding().await?;
    if missing.is_empty() {
        return Ok(());
    }
    tracing::info!("Backfilling embeddings for {} learnings", missing.len());
    let texts: Vec<String> = missing
        .iter()
        .map(|l| embed_text_for_learning(l.kind, &l.summary, &l.tags, l.detail.as_deref()))
        .collect();
    let embeddings = emb_svc.embed_batch(texts).await?;
    for (learning, emb_vec) in missing.iter().zip(embeddings.iter()) {
        let emb_bytes = serialize_embedding(emb_vec);
        db.patch_learning(
            learning.id,
            &crate::db::LearningPatch::new().embedding(&emb_bytes),
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TuiRuntime — shared context for command execution
// ---------------------------------------------------------------------------

struct TuiRuntime {
    // Read-only DB handle: queries only. Task/epic mutations go through
    // `task_svc` / `epic_svc`, which own the `recalculate_epic_status` invariant
    // — calling a mutating method on `database` is a compile error. See the
    // mutation-boundary section of docs/conventions.md.
    database: Arc<dyn db::TaskReadStore>,
    /// Where the board's CARDS come from — see
    /// [`crate::sync::BoardReads`] and `docs/specs/sync.allium`'s
    /// `BoardReadsFromTheSubscription`.
    ///
    /// Deliberately separate from `database` rather than replacing it. The two
    /// answer different questions: this one answers "what is on the board?",
    /// which a shared store can serve, and `database` answers everything else —
    /// settings, the knowledge base, usage — which is local and always will be.
    ///
    /// On an install with no shared store configured this is backed by the same
    /// `database`, so the single-machine board is unchanged rather than
    /// degraded (`sync.allium`'s header says why that matters).
    board_reads: Arc<dyn crate::sync::BoardReads>,
    /// Write-capable handle reserved for the feed subsystem (the manual
    /// `exec_trigger_epic_feed` path), which upserts tasks and recalculates epic
    /// status itself — exactly like `FeedRunner`. This is the one sanctioned
    /// direct-mutation handle on the runtime; general command handlers hold only
    /// the read-only `database` above. See the mutation-boundary section of
    /// docs/conventions.md. In test builds it also backs the `#[cfg(test)]`
    /// `db_write()` accessor used to seed fixtures.
    feed_db: Arc<dyn db::TaskStore>,
    task_svc: Arc<dyn crate::service::TaskServiceApi>,
    epic_svc: Arc<dyn crate::service::EpicServiceApi>,
    todo_svc: Arc<dyn crate::service::TodoServiceApi>,
    learning_svc: Arc<dyn crate::service::LearningServiceApi>,
    msg_tx: mpsc::UnboundedSender<Message>,
    runner: Arc<dyn ProcessRunner>,
    /// Holds the in-flight pop-out editor session, if any. `None` means no
    /// editor is currently open. We enforce "at most one editor at a time"
    /// by refusing to start a new one while this slot is populated.
    editor_session: Arc<std::sync::Mutex<Option<editor::EditorSession>>>,
    feed_runner: Option<crate::feed::FeedRunner>,
    /// Fires the `FeedRunner`'s feed-command cache invalidation. Cloned from
    /// `feed_runner.epic_invalidate_tx()` at construction so both mutation-
    /// carrying MCP events (`Refresh` and `EpicChanged`) can reset the cache
    /// through `invalidate_feed_cache()` — keeping the runner from stranding a
    /// freshly-enabled feed behind `any_feed_cmds == Some(false)`.
    feed_invalidate_tx: Option<tokio::sync::watch::Sender<()>>,
    /// Per-epic feed-cycle claims, shared with `feed_runner` so a manual "r"
    /// refresh and an auto-poll tick serialise against each other
    /// (feeds.allium: SerialisedFeedCycle).
    ///
    /// This MUST be the same `Arc` the `FeedRunner` holds. A separate registry
    /// type-checks and compiles, and silently serialises nothing — always take
    /// it from `FeedRunner::sync_guard()`, never construct one here.
    feed_sync_guard: std::sync::Arc<crate::feed::FeedSyncGuard>,
    /// Shared embedding service for RAG-based learning injection and editor updates.
    emb_svc: Arc<EmbeddingService>,
    /// The revision the board was last refreshed at
    /// ([`crate::sync::BoardReads::revision`]).
    ///
    /// `-1` is "no snapshot yet", so the first tick always refreshes; the value
    /// is otherwise a `u64` widened, and the store is `Relaxed` because this is
    /// a hint, not a lock — a lost race costs one extra refresh.
    ///
    /// **Written by the row pump too, not only by the tick.** A pushed row
    /// already redraws the whole board, so leaving the pump's read unrecorded
    /// made the next tick see a moved revision and do the same two reads again
    /// — a doubled refresh on every teammate edit, forever.
    last_change_count: Arc<AtomicI64>,
    /// Path to the budget snapshot file (`<data_dir>/rate-limits.json`), written
    /// by the statusLine hook of every dispatch-spawned Claude session. Read off
    /// the event loop by `exec_refresh_budget`. See docs/specs/dispatch.allium:
    /// TokenBudgetIndicator.
    budget_snapshot_path: std::path::PathBuf,
    /// Path the trust-gated `TaskCommand` arms (`TrustAndDispatch`,
    /// `CheckTrustAndDispatch`, `QuickDispatch`, `TrustAndQuickDispatch`) read
    /// and write via `dispatch::is_trusted_at`/`dispatch::trust_at`. Defaults
    /// to the real `$HOME/.claude.json` in production, via
    /// [`StartupPaths::resolve`]; a test overrides it with a tempfile so
    /// exercising these arms never touches the machine's actual trust store.
    claude_json_path: std::path::PathBuf,
    /// The tmux work split-mode exits have issued and that has not yet reported
    /// back — the runtime's form of `SplitPane.restores_in_flight` in
    /// `docs/specs/split-pane.allium`. Held rather than dropped because quitting
    /// with a task pinned breaks that agent's pane back out to a standalone
    /// window, and issuing that break is not the same as completing it.
    ///
    /// A list, not one slot: exit is deliberately not gated while an exit is in
    /// flight, so a second exit can be issued while the first is still moving a
    /// pane, and keeping only the newest would quit out from under the older
    /// one.
    split_restores: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

mod budget;
mod commands;
mod editor;
mod epics;
mod learnings;
mod pr;
mod repo_sync;
mod settings;
mod split;
mod tasks;
#[cfg(test)]
mod tests;
mod todos;

impl TuiRuntime {
    fn db_error(action: &str, e: impl std::fmt::Display) -> String {
        format!("DB error {action}: {e}")
    }

    /// Test-only write handle for seeding DB fixtures directly. Backed by the
    /// feed subsystem's write handle; not available in production builds, so
    /// command handlers keep going through the services.
    #[cfg(test)]
    pub(super) fn db_write(&self) -> &Arc<dyn db::TaskStore> {
        &self.feed_db
    }

    fn send_system_error(&self, msg: impl Into<String>) {
        let _ = self
            .msg_tx
            .send(Message::System(crate::tui::messages::SystemMessage::Error(
                msg.into(),
            )));
    }

    /// Build a fully-initialised runtime and its companion `App` from a database
    /// path and MCP port. Encapsulates all startup I/O — database open, embedding
    /// model load, MCP server spawn, and settings hydration — so `run_tui` reads
    /// as a sequence of named steps rather than an inline setup blob.
    ///
    /// The `#[cfg(test)]` / `#[cfg(not(test))]` embedding-service split lives
    /// here so call sites don't branch on `cfg`.
    async fn bootstrap(
        db_path: &Path,
        port: u16,
        paths: &StartupPaths,
        spacetime_server: Option<String>,
    ) -> Result<Bootstrap> {
        // WHERE THIS BOARD'S WRITES GO, decided before the database exists
        // because the answer is a property OF the database handle.
        //
        // The read side picks its backing further down, once the runtime is
        // being assembled, because a read source is a field the runtime holds.
        // A write destination is not: it is the routing inside `Database`
        // itself (`db::SharedWriter`), so it has to be attached at
        // construction — and `with_shared_writer` consumes, deliberately, so
        // that which backing a write goes to cannot change under a caller.
        //
        // Both halves read the same `spacetime_server`, so a board cannot end
        // up reading one store and writing another.
        let shared_store = spacetime_server
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        // A HALF-ROUTED BOARD IS REFUSED, LOUDLY, BEFORE ANYTHING IS BUILT.
        //
        // Phase 6 moved most shared mutations to the store and not all of them
        // (`db::SHARED_WRITES_ARE_COMPLETE` lists what is left). A board
        // pointed at a store meanwhile would write some tables there and some
        // to its own disk, and the two would disagree from the first agent
        // session onward with nothing on screen to say so. Refusing to start is
        // the smaller harm: the operator finds out now, at the moment they set
        // the flag, rather than from a colleague's board a week later.
        if shared_store.is_some() && !db::SHARED_WRITES_ARE_COMPLETE {
            anyhow::bail!(
                "a shared store is configured, but not every change this board makes \
                 reaches one yet — see db::SHARED_WRITES_ARE_COMPLETE for what is left. \
                 Starting would write some of your work to the store and some to this \
                 machine, with nothing to show which. Unset --spacetime-server \
                 (DISPATCH_SPACETIME_SERVER) to start."
            );
        }

        // The rows and the connector are built as ONE option, so nothing below
        // has to reconcile two that are meant to be present together.
        let shared = shared_store.as_ref().map(|_| {
            let rows = Arc::new(crate::sync::SharedRows::new());
            let connector = Arc::new(crate::sync::SpacetimeSdkConnector::new(
                crate::sync::SHARED_DATABASE_NAME,
                rows.clone(),
            ));
            (rows, connector)
        });
        // Filled by the connection loop once the store says who we are. Held
        // here so the writer and the loop share one cell.
        let settled_identity = Arc::new(crate::sync::SettledIdentity::default());

        // Open database and load initial tasks.
        let database = db::Database::open(db_path).await?;
        let database = Arc::new(match &shared {
            Some((rows, connector)) => {
                // The HOST id, unlike the user identity, is known before any
                // connection: it is minted locally on first run and immutable
                // afterwards (`host.allium: MintHostIdentity`). The claim needs
                // it, so it is resolved once here rather than per write.
                let (host_id, _) = database.ensure_host_identity().await?;
                database.with_shared_writer(Arc::new(crate::sync::ReducerWriter::new(
                    Arc::new(crate::sync::SdkReducerCaller::new(connector.clone())),
                    settled_identity.clone(),
                    Arc::new(crate::service::SystemClock),
                    host_id,
                    // The same rows the board draws from, deliberately: the
                    // chain must take the task the column shows as next.
                    rows.clone(),
                )))
            }
            None => database,
        });
        let tasks = database.list_all().await?;

        // Seed the example feed epic for a database that has none. It writes
        // only inside dispatch's own data directory — the one the operator
        // named with `--db` — so it is not a configuration artefact and is not
        // gated on the startup consent prompt (see docs/specs/startup.allium's
        // scope note). It lives here rather than beside that prompt because
        // this is where the board's database connection already is; doing it
        // there would mean opening the same file a second time.
        // Idempotent and best-effort: a failure here must not block startup.
        let data_dir = db_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        if let Err(e) = crate::setup::seed_feed_epics(&database, &data_dir).await {
            tracing::warn!("Example feed epic seeding failed: {e:#}");
        }

        // Provision the managed feed-epic tree from the reviews/CVE config.
        // Idempotent and best-effort: a failure here must not block startup.
        if let Err(e) = crate::service::provision_managed_feeds_from_settings(&*database).await {
            tracing::warn!("Managed feed provisioning failed: {e:#}");
        }

        // Initialise the embedding model (blocks until loaded; may download on first run).
        // Tests bypass run_tui entirely and construct TuiRuntime directly, so
        // the non-test branch is only reached in production.
        #[cfg(not(test))]
        let emb_svc = {
            eprintln!("Loading embedding model...");
            tokio::task::spawn_blocking(EmbeddingService::new)
                .await
                .map_err(|e| anyhow::anyhow!("Embedding thread panicked: {e}"))?
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to initialise embedding model: {e}\n\
                         Clear cache with: rm -rf ~/.cache/huggingface/hub/"
                    )
                })?
        };
        #[cfg(test)]
        let emb_svc = EmbeddingService::new_noop();

        // Backfill embeddings for any learnings that were created before the model
        // was available. Fire-and-forget: partial work is retried on next startup.
        tokio::spawn({
            let db = database.clone();
            let emb = emb_svc.clone();
            async move {
                if let Err(e) = backfill_embeddings(db, emb).await {
                    tracing::warn!("Embedding backfill failed: {e}");
                }
            }
        });

        // Spawn MCP server with notification channel.
        // Handed the operator's config location rather than looking it up — see
        // `SettingsLocationIsAnExplicitStartupInput`. This one runner backs the
        // MCP server, the feed runner and `TaskService`, so every launch path
        // gets the same answer.
        let runner: Arc<dyn ProcessRunner> = Arc::new(RealProcessRunner::with_claude_json(
            paths.claude_json_path.clone(),
        ));
        // Deliberately not derived from `db_path`: the subscription windows are
        // account-global, so a run against a throwaway database must publish
        // and read the same location as every other session. See
        // docs/specs/observability.allium:
        // SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase.
        let budget_snapshot_path = crate::budget_snapshot_path();

        let (mcp_notify_tx, mcp_notify_rx) = mpsc::unbounded_channel::<mcp::McpEvent>();
        let feed_notify_tx = mcp_notify_tx.clone();
        let mcp_deps = mcp::McpDeps {
            db: database.clone(),
            runner: runner.clone(),
            embedding_service: emb_svc.clone(),
            data_dir,
        };
        // Claimed here, before the board takes the screen, so a port another
        // process still holds aborts the launch where the operator can read it
        // — `startup.allium`'s `AbortWhenTheAgentPortIsTaken`. Bound inside the
        // spawned task instead, the failure would land on a stderr the drawn
        // board has already covered, leaving a board no agent can reach.
        let mcp_listener = mcp::bind(port).await.map_err(|e| {
            tracing::error!("agent port {port} unavailable: {e}");
            anyhow::anyhow!(
                "{}",
                crate::startup::StartupAbort::AgentPortUnavailable { port }.message()
            )
        })?;
        tokio::spawn(async move {
            if let Err(e) = mcp::serve_on(mcp_listener, mcp_deps, mcp_notify_tx).await {
                eprintln!("MCP server error: {e}");
            }
        });

        // Create App and hydrate all persisted settings.
        let mut app = App::new(tasks);
        // Mint (or read back) this install's Host identity — see
        // host.allium: MintHostIdentity. A failure here is NOT best-effort:
        // it aborts the launch (startup.allium:
        // AbortWhenTheHostIdentityStoreIsUnusable) rather than leaving
        // `local_host_id` empty and drawing anyway. Tolerating it does not
        // degrade, it corrupts — the claim SQL in db/queries/tasks.rs applies
        // `is_locally_owned`'s two arms (nobody holds this task, or this
        // machine does), and an undetermined identity fails only the second
        // arm, so the claim still succeeds on every never-dispatched task —
        // exactly the ones a first dispatch acts on. A worktree then gets
        // provisioned while the host stamp is skipped for want of an id: a
        // silent, durable violation of core.allium's `HostTracksWorktree`. A
        // board that cannot complete one settings read at startup is not
        // going to stay useful either way, so this fails loudly here instead.
        let (host_id, label) = database.ensure_host_identity().await.map_err(|e| {
            tracing::error!("Failed to read/mint host identity: {e:#}");
            anyhow::anyhow!(
                "{}",
                crate::startup::StartupAbort::HostIdentityUnavailable.message()
            )
        })?;
        app.set_local_host_id(host_id);

        // startup.allium: CheckHostLabel and its remaining children. Runs
        // here — after the port claim above, before the terminal is touched
        // (`EnterAlternateScreen` is in `run_tui`, after this function
        // returns) — so the operator answers on an ordinary terminal and,
        // unlike the configuration-drift check, an unnamed host aborts the
        // whole launch rather than degrading: `TheBoardNeverDrawsForAnUnnamedHost`
        // has no non-fatal counterpart. Blocking (may wait on stdin), so it
        // runs on a blocking thread rather than inline on this async runtime.
        //
        // Skipped outright once the host has a label, which is every launch
        // after the first: `resolve_host_label`'s first act is to return on a
        // label that is already set, so entering it would cost a blocking-pool
        // hop and a `/proc` read (the prompt's default, evaluated eagerly as an
        // argument) only to discard both.
        let resolved = if label.is_none() {
            let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
            tokio::task::spawn_blocking(move || {
                crate::startup::resolve_host_label_interactively(label, interactive)
            })
            .await
            .map_err(|e| anyhow::anyhow!("host-label prompt thread panicked: {e}"))?
        } else {
            Ok(None)
        };
        match resolved {
            // A failed persist here used to be left as a raw propagated
            // error rather than mapped through `StartupAbort` — resolved by
            // startup.allium's `NameHostFromStartupPrompt`, which aborts with
            // the same `host_identity_unavailable` reason a failed read/mint
            // gets (`AbortWhenTheHostIdentityStoreIsUnusable`) rather than a
            // second `StartupAbortReason`: both failures are the same broken
            // settings store and share the same remedy, ensured by two
            // separate rules rather than one rule with a widened guard — see
            // `persist_host_label` below for the mapping.
            Ok(Some(new_label)) => persist_host_label(&*database, &new_label)
                .await
                .map_err(|abort| anyhow::anyhow!("{}", abort.message()))?,
            Ok(None) => {}
            Err(abort) => return Err(anyhow::anyhow!("{}", abort.message())),
        }
        let (repo_paths, base_branch_pairs) = tokio::join!(
            database.list_repo_paths(),
            database.list_all_base_branches()
        );
        app.update(Message::RepoPathsUpdated(repo_paths.unwrap_or_default()));
        app.update(Message::BaseBranchesUpdated(group_base_branches_by_repo(
            base_branch_pairs.unwrap_or_default(),
        )));
        load_notifications_pref(&*database, &mut app).await;
        load_repo_filter(&*database, &mut app).await;
        load_collapsed_sections(&*database, &mut app).await;
        for msg in [
            load_filter_presets(&*database, &mut app).await,
            apply_tmux_focus_warning(&*runner),
        ]
        .into_iter()
        .flatten()
        {
            app.update(msg);
        }

        // WHERE THIS BOARD'S CARDS COME FROM. A configured shared store means
        // the board draws what the subscription delivers; no store configured
        // means it draws SQLite, which is every install today and is a
        // first-class way to run rather than an unconfigured one — see
        // `sync.allium`'s header.
        //
        // There is deliberately no third state. A board pointed at a store does
        // NOT fall back to the local copy when the store is down: the shared
        // tables have exactly one copy, and a fallback would be the read-through
        // cache `crate::sync::rows` exists not to be. A cold start against an
        // unreachable store draws an empty board and says why
        // (`ConnectionIndicator`).
        // Passed in rather than read from the environment here. Every other
        // launch-time knob in this binary is a clap arg with an `env`
        // attribute, resolved once at the entry point and threaded down — so
        // this one is discoverable in `--help`, settable as
        // `--spacetime-server`, and testable without mutating process globals.
        // Resolved at the top of this function, beside the write routing it
        // also decides.
        //
        // Captured before `database` is moved into the runtime below.
        let sync_store: Arc<dyn crate::sync::SyncStore> = database.clone();

        // Build TuiRuntime.
        let (msg_tx, msg_rx) = mpsc::unbounded_channel::<Message>();
        let feed_runner =
            crate::feed::FeedRunner::new(database.clone(), feed_notify_tx, runner.clone());
        let feed_invalidate_tx = Some(feed_runner.epic_invalidate_tx());
        let feed_sync_guard = feed_runner.sync_guard();
        let task_svc = Arc::new(crate::service::TaskService::new(
            database.clone(),
            runner.clone(),
        ));
        let runtime = TuiRuntime {
            task_svc,
            epic_svc: Arc::new(crate::service::EpicService::new(
                database.clone(),
                database.clone(),
            )),
            todo_svc: Arc::new(crate::service::TodoService::new(database.clone())),
            learning_svc: Arc::new(crate::service::LearningService::new(
                database.clone(),
                emb_svc.clone(),
            )),
            feed_runner: Some(feed_runner),
            feed_invalidate_tx,
            feed_sync_guard,
            feed_db: database.clone(),
            board_reads: match &shared {
                Some((rows, _)) => Arc::new(crate::sync::SubscriptionBoardReads::new(rows.clone())),
                None => Arc::new(crate::sync::LocalBoardReads::new(database.clone())),
            },
            database,
            msg_tx,
            runner,
            editor_session: Arc::new(std::sync::Mutex::new(None)),
            emb_svc,
            last_change_count: Arc::new(AtomicI64::new(-1)),
            budget_snapshot_path,
            claude_json_path: paths.claude_json_path.clone(),
            split_restores: std::sync::Mutex::new(Vec::new()),
        };

        // Bring the connection up and keep the board redrawing behind it. Both
        // are spawned rather than awaited: `OpenBoardConnection` deliberately
        // does not block the board, so a slow or unreachable store costs a cold
        // start nothing (see the Phase 4 measurement in the migration plan).
        if let (Some(server), Some((rows, connector))) = (shared_store, shared) {
            drop(runtime.spawn_row_change_pump(rows));
            drop(runtime.spawn_shared_store_connection(
                server,
                connector,
                sync_store,
                settled_identity,
            ));
        }

        // Load initial todo open-count so the board footer shows it immediately.
        runtime.exec_load_todo_count(&mut app).await;

        // RefreshRepoSyncStateOnStartup: the only genuinely new network traffic
        // this feature introduces — one fetch per saved repo path. Fire-and-forget,
        // so a slow or offline network never delays startup.
        let saved_repo_paths = app.repo_paths().to_vec();
        drop(runtime.exec_refresh_all_repo_sync(&saved_repo_paths));

        Ok(Bootstrap {
            app,
            runtime,
            mcp_notify_rx,
            msg_rx,
        })
    }

    /// Invalidate the `FeedRunner`'s `any_feed_cmds` cache so its next tick
    /// re-queries for feed commands. Call after any managed-feed mutation that
    /// may have enabled the first feed on a previously feed-less instance —
    /// otherwise the runner short-circuits on a stale `Some(false)` and never
    /// starts polling until an unrelated event or a restart. Best-effort: a
    /// dropped receiver (no running runner) is a no-op.
    fn invalidate_feed_cache(&self) {
        if let Some(tx) = &self.feed_invalidate_tx {
            let _ = tx.send(());
        }
    }

    async fn create_task(
        &self,
        app: &mut App,
        params: crate::service::CreateTaskParams,
    ) -> Option<models::Task> {
        match self.task_svc.create_task_returning(params).await {
            Ok(task) => Some(task),
            Err(e) => {
                app.update(Message::System(crate::tui::messages::SystemMessage::Error(
                    Self::db_error("creating task", e),
                )));
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// run_loop — select over key events, async messages, and tick timer
// ---------------------------------------------------------------------------

/// One input the TUI event loop reacts to, drawn from any of its four sources
/// (keys, async messages, MCP notifications, the periodic tick). Naming the
/// event explicitly lets the loop body — `apply_loop_event` — be unit-tested
/// without a running `select!` or a real terminal.
///
/// `Message` is the largest variant and stays unboxed here deliberately: it is
/// already passed by value throughout the TUI (it flows through an
/// `UnboundedReceiver<Message>` unboxed), so boxing at this hop would add an
/// allocation per event for no benefit. That only stays affordable while
/// `Message` itself stays small — see the `size_of` guard-rail tests in
/// `src/tui/types.rs`.
#[cfg_attr(test, derive(Debug))]
enum LoopEvent {
    Key(crossterm::event::KeyEvent),
    Message(Message),
    Mcp(mcp::McpEvent),
    Tick,
}

/// Await the next event from any input source. The tick arm is always enabled,
/// so this never resolves on an all-channels-closed condition — the loop exits
/// via `App::should_quit`, not channel closure.
async fn next_loop_event(
    key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    msg_rx: &mut mpsc::UnboundedReceiver<Message>,
    mcp_notify_rx: &mut mpsc::UnboundedReceiver<mcp::McpEvent>,
    tick_interval: &mut tokio::time::Interval,
) -> LoopEvent {
    tokio::select! {
        // Key events from the blocking poll thread.
        Some(key) = key_rx.recv() => LoopEvent::Key(key),
        // Async messages (e.g., from dispatch results).
        Some(msg) = msg_rx.recv() => LoopEvent::Message(msg),
        // MCP event notification.
        Some(event) = mcp_notify_rx.recv() => LoopEvent::Mcp(event),
        // Periodic tick for tmux capture and feed polling.
        _ = tick_interval.tick() => LoopEvent::Tick,
    }
}

/// Apply one loop event to `app`, returning the commands it produced. Mirrors
/// the per-arm `dirty` bookkeeping and MCP-event side effects (refresh spawns,
/// feed-cache invalidation) of the original `select!` body, kept as a separate
/// function so the routing is directly testable.
fn apply_loop_event(app: &mut App, event: LoopEvent, rt: &TuiRuntime) -> Vec<Command> {
    match event {
        // handle_key sets app.dirty unconditionally, same as the Message/Mcp
        // arms below — see the render-dirty-flag section of docs/architecture.md.
        LoopEvent::Key(key) => app.handle_key(key),
        LoopEvent::Message(msg) => {
            // Async messages typically carry visible state changes.
            app.dirty = true;
            app.update(msg)
        }
        LoopEvent::Mcp(event) => {
            // Spawn DB work so this never blocks key-event processing. Results
            // arrive back via msg_rx and are applied on the next iteration.
            app.dirty = true;
            match event {
                mcp::McpEvent::Refresh => {
                    // A broad refresh may follow a managed-feed config save
                    // (set_managed_feed_config) that enabled a feed on a
                    // previously feed-less instance. Invalidate the FeedRunner
                    // cache so the next tick re-queries for feed commands and
                    // starts polling the freshly-provisioned epics rather than
                    // short-circuiting on a stale any_feed_cmds == Some(false).
                    rt.invalidate_feed_cache();
                    drop(rt.spawn_refresh_from_db());
                    vec![]
                }
                mcp::McpEvent::TaskChanged(task_id) => {
                    drop(rt.spawn_refresh_task(task_id));
                    vec![]
                }
                mcp::McpEvent::EpicChanged(epic_id) => {
                    // Invalidate the FeedRunner's cache so the next tick re-queries
                    // for feed commands (e.g. a newly added feed_command becomes visible).
                    rt.invalidate_feed_cache();
                    drop(rt.spawn_refresh_epic(epic_id));
                    vec![]
                }
                mcp::McpEvent::BranchRebased { repo_path } => {
                    // A rebase wrap-up pulled origin/<base> and fast-forwarded
                    // local <base>, so the refs are current and no fetch is
                    // needed. An unresolved repository measures nothing.
                    if !repo_path.is_empty() {
                        drop(rt.exec_refresh_repo_sync(repo_path, false));
                    }
                    vec![]
                }
                mcp::McpEvent::AgentLaunched { repo_path } => {
                    // RefreshRepoSyncStateAfterDispatch: provisioning the agent's
                    // worktree already fetched origin/<base>, so this is a local
                    // ref read at no network cost. The board's own dispatch takes
                    // the same refresh through a command; these are the off-board
                    // launches (dispatch_task, epic auto-dispatch chaining).
                    drop(rt.exec_refresh_repo_sync(repo_path, false));
                    vec![]
                }
                mcp::McpEvent::AutoDispatchFailed {
                    task_id,
                    epic_id,
                    reason,
                } => {
                    // No refresh is spawned here: the chain sends TaskChanged
                    // for the released subtask right behind this, so reloading
                    // the row is already covered.
                    app.update(Message::Task(
                        crate::tui::messages::TaskMessage::AutoDispatchFailed {
                            task_id,
                            epic_id,
                            reason,
                        },
                    ))
                }
            }
        }
        // Handlers set app.dirty themselves when they detect visible changes.
        LoopEvent::Tick => app.update(Message::System(crate::tui::messages::SystemMessage::Tick)),
    }
}

/// Commands executed once, before the event loop's first iteration.
///
/// The list exists so "what runs at startup" is one value a test can read,
/// rather than a sequence of inline calls in `run_loop`. Everything here must be
/// something the tick loop would do anyway, just sooner: startup priming, never
/// startup-only behaviour.
fn startup_commands() -> Vec<Command> {
    // Read the budget snapshot now instead of waiting out the first
    // BUDGET_POLL_TICKS, so a snapshot already on disk shows on the first frame.
    vec![Command::Budget(
        crate::tui::commands::BudgetCommand::Refresh,
    )]
}

async fn run_loop<B: Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    msg_rx: &mut mpsc::UnboundedReceiver<Message>,
    mcp_notify_rx: &mut mpsc::UnboundedReceiver<mcp::McpEvent>,
    tick_interval: &mut tokio::time::Interval,
    rt: &mut TuiRuntime,
) -> Result<()> {
    // Here (not in TuiRuntime::new) so tests that construct TuiRuntime directly
    // don't accidentally spawn background tasks. The invalidation sender is held
    // on `rt.feed_invalidate_tx` (cloned at construction), so it survives the
    // runner being moved into its background task here.
    if let Some(feed_runner) = rt.feed_runner.take() {
        feed_runner.start();
    }

    execute_commands(app, startup_commands(), rt, terminal, key_rx).await?;

    let mut last_render = std::time::Instant::now() - MIN_FRAME_INTERVAL; // allow first frame

    loop {
        // Redraw only when state changed since the last frame AND the frame interval has elapsed.
        // frame_ready coalesces rapid key-repeat events (holding j) into at most ~60 renders/s.
        if frame_ready(last_render.elapsed(), app.dirty) {
            terminal.draw(|frame| tui::ui::render(frame, app))?;
            app.dirty = false;
            last_render = std::time::Instant::now();
        }

        if app.should_quit() {
            break;
        }

        let event = next_loop_event(key_rx, msg_rx, mcp_notify_rx, tick_interval).await;
        let commands = apply_loop_event(app, event, rt);

        execute_commands(app, commands, rt, terminal, key_rx).await?;
    }

    Ok(())
}

/// Wait for the split-pane restores a quit issued, bounded.
///
/// Implements `QuitCompletesWithNothingToRestore`,
/// `QuitCompletesWhenRestoreSettles` and `QuitCompletesOnRestoreTimeout` in
/// `docs/specs/split-pane.allium`. Returns whether every restore reported back.
///
/// One bound covers the whole set, not one each: it is a deadline on the quit,
/// and a per-handle budget would multiply by however many exits overlapped. The
/// bound is a parameter so the abandonment path is testable without a
/// wall-clock wait.
async fn await_split_restores(handles: Vec<tokio::task::JoinHandle<()>>, bound: Duration) -> bool {
    let settled = tokio::time::timeout(bound, async {
        for handle in handles {
            let _ = handle.await;
        }
    })
    .await;
    if settled.is_err() {
        // Logged rather than shown: the event loop has returned, so nothing
        // would draw a status message even though the board is still on screen.
        tracing::warn!(
            timeout_secs = bound.as_secs_f32(),
            "split-pane restore did not report back before quitting; a pinned agent's pane may still be inside the board's tmux window"
        );
    }
    settled.is_ok()
}

// ---------------------------------------------------------------------------
// execute_commands — run side effects for each Command
// ---------------------------------------------------------------------------

async fn execute_commands<B: Backend>(
    app: &mut App,
    cmds: Vec<Command>,
    rt: &TuiRuntime,
    _terminal: &mut Terminal<B>,
    _key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
) -> Result<()> {
    let mut queue = std::collections::VecDeque::from(cmds);
    while let Some(command) = queue.pop_front() {
        let extra = commands::dispatch(command, app, rt).await;
        queue.extend(extra);
    }
    Ok(())
}

/// Persist a newly accepted host label — `host.allium`'s `RenameHost`,
/// handed the operator's answer by `startup.allium`'s
/// `NameHostFromStartupPrompt` — mapping a failed write onto the
/// `host_identity_unavailable` `StartupAbortReason` instead of letting it
/// propagate as a raw, unclassified error. This is the mapping
/// `NameHostFromStartupPrompt`'s own `ensures` describes (its `else` arm);
/// `AbortWhenTheHostIdentityStoreIsUnusable` is the sibling rule that raises
/// the same reason for a failed read/mint instead.
///
/// Factored out of `bootstrap` so this mapping is exercisable directly
/// against a real (possibly corrupted) database: reaching it through the
/// full interactive prompt needs `bootstrap`'s `interactive` flag to read
/// true, which it can only do against a real terminal — `cargo test`'s
/// stdin never reports one.
///
/// One `StartupAbortReason` rather than a second: this failure and a failed
/// `ensure_host_identity` read/mint are the same broken settings store and
/// share the same remedy (repair it), so they share the message
/// `HostIdentityUnavailable` already carries.
async fn persist_host_label(
    db: &dyn db::HostStore,
    label: &str,
) -> std::result::Result<(), crate::startup::StartupAbort> {
    db.rename_host(label).await.map_err(|e| {
        tracing::error!("Failed to persist host label: {e:#}");
        crate::startup::StartupAbort::HostIdentityUnavailable
    })
}

// ---------------------------------------------------------------------------
// init load helpers — extracted from run_tui's startup block
// ---------------------------------------------------------------------------

async fn load_notifications_pref(db: &dyn db::SettingsStore, app: &mut App) {
    let enabled = db
        .get_setting_bool("notifications_enabled")
        .await
        .unwrap_or(None)
        .unwrap_or(false);
    app.set_notifications_enabled(enabled);
}

async fn load_repo_filter(db: &dyn db::SettingsStore, app: &mut App) {
    if let Ok(Some(val)) = db.get_setting_string("repo_filter").await {
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(&val) {
            app.set_repo_filter(paths.into_iter().collect());
        }
    }
    if let Ok(Some(mode_str)) = db.get_setting_string("repo_filter_mode").await {
        if let Ok(mode) = mode_str.parse::<RepoFilterMode>() {
            app.set_repo_filter_mode(mode);
        }
    }
}

/// Restore the folded sub-status sections. Unlike the flat-view toggle, a fold
/// is a standing preference and survives a restart (board-layout.allium:
/// "Collapsed Sections"). An unreadable or absent row simply leaves everything
/// unfolded.
async fn load_collapsed_sections(db: &dyn db::SettingsStore, app: &mut App) {
    if let Ok(Some(val)) = db.get_setting_string(COLLAPSED_SECTIONS_KEY).await {
        app.set_section_folds(SectionFoldState::parse(&val));
    }
}

async fn load_filter_presets(db: &dyn db::SettingsStore, app: &mut App) -> Option<Message> {
    match db.list_filter_presets().await {
        Ok(raw) => {
            let _ = app.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::PresetsLoaded(parse_raw_presets(
                    raw, None,
                )),
            ));
            None
        }
        Err(e) => Some(Message::System(
            crate::tui::messages::SystemMessage::StatusInfo(format!(
                "Failed to load filter presets: {e}"
            )),
        )),
    }
}

fn apply_tmux_focus_warning(runner: &dyn ProcessRunner) -> Option<Message> {
    if !crate::tmux::focus_events_enabled(runner) {
        Some(Message::System(crate::tui::messages::SystemMessage::StatusInfo(
            "tmux focus-events is off \u{2014} split-view focus indicator won't work. Run: tmux set -g focus-events on".to_string(),
        )))
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

/// Returns `true` when the render loop should draw a new frame.
///
/// Both conditions must hold: the app state changed (`dirty`) *and* enough
/// time has elapsed since the last render (`elapsed >= MIN_FRAME_INTERVAL`).
/// The interval coalesces rapid key-repeat events (≥30/s) into at most
/// one render per 16 ms (~60 fps) without adding perceptible latency to
/// single keypresses.
pub(crate) fn frame_ready(elapsed_since_render: Duration, dirty: bool) -> bool {
    dirty && elapsed_since_render >= MIN_FRAME_INTERVAL
}
