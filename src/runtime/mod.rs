use anyhow::Result;
use crossterm::{
    event::{self, DisableFocusChange, EnableFocusChange, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::Terminal;
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
pub(crate) const AGENT_TREE_TOGGLE_KEY: &str = "e";

/// The tmux key bound (after the prefix) to jump back to the board window.
pub(crate) const JUMP_BACK_KEY: &str = "space";

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

use crate::db::{RepoConfigRead, TaskRead};
use crate::models::{TaskId, TmuxWindow};
use crate::process::{ProcessRunner, RealProcessRunner};
use crate::service::embeddings::EmbeddingService;
use crate::service::FieldUpdate;
use crate::tui::{
    self, App, Command, EpicFoldState, Message, RepoFilterMode, SectionFoldState,
    COLLAPSED_EPICS_KEY, COLLAPSED_SECTIONS_KEY,
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

/// Publish the store address on the board's own tmux session, under the
/// board's own variable (`BOARD_STORE_ENV`, never the operator's
/// `DISPATCH_SPACETIME_SERVER`, which `dispatch tui` reads: task #28729), so the
/// processes the board starts there — agent windows, the agent-tree and diff
/// panes, a `dispatch` command an agent runs — reach the same store without
/// being told. Best-effort like the rest of the tmux setup: a process that
/// does not inherit it falls back to the managed store's address
/// (`startup::store_server_or_managed`), which is where a board without a named
/// store keeps its own. Skipped when the session is unknown, since an
/// empty target would land on whichever session tmux picks.
fn publish_store_server(session: &str, server: &str, runner: &dyn ProcessRunner) {
    if session.is_empty() {
        return;
    }
    if let Err(e) =
        tmux::set_session_environment(session, crate::startup::BOARD_STORE_ENV, server, runner)
    {
        tracing::warn!("could not publish the store address on the tmux session: {e:#}");
    }
}

/// Publish the port the board serves on, beside the store address, so the
/// companion panes it starts ask this board rather than the default port
/// (`PaneRenderersAskTheBoard`). Best-effort, like the store address.
fn publish_board_port(session: &str, port: u16, runner: &dyn ProcessRunner) {
    if session.is_empty() {
        return;
    }
    if let Err(e) =
        tmux::set_session_environment(session, "DISPATCH_PORT", &port.to_string(), runner)
    {
        tracing::warn!("could not publish the board port on the tmux session: {e:#}");
    }
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
        JUMP_BACK_KEY,
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
    let _ = tmux::unbind_key(JUMP_BACK_KEY, runner);
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
    /// The store the board connected to: the one named, or the managed
    /// store's address once it was brought up.
    store_server: String,
    app: App,
    runtime: TuiRuntime,
    mcp_notify_rx: mpsc::UnboundedReceiver<mcp::McpEvent>,
    msg_rx: mpsc::UnboundedReceiver<Message>,
}

// ---------------------------------------------------------------------------
// The store every process reads and writes through
// ---------------------------------------------------------------------------

/// A `Database` routed through the shared store, and the pieces of the
/// connection behind it.
///
/// Built before anything connects, because the routing is attached at the
/// handle's construction: `with_shared_writer` and the reader attachments
/// consume, deliberately, so which backing a read or write goes to cannot
/// change under a caller. Every reader and the writer's claim chain sit over
/// the same `rows`, so a process cannot read one copy and write another.
pub struct StoreParts {
    pub database: Arc<db::Database>,
    pub rows: Arc<crate::sync::SharedRows>,
    /// The one adapter over `rows`: the board draws from it and `database`'s
    /// shared reads route to it.
    pub board_reads: Arc<crate::sync::SubscriptionBoardReads>,
    pub connector: Arc<dyn crate::sync::StoreConnector>,
    pub settled_identity: Arc<crate::sync::SettledIdentity>,
    /// Reused everywhere a reducer call is needed outside `SharedWriter`
    /// proper — today, only the host-registry mirror (`sync.allium:
    /// RegisterHostOnConnect`/`RegisterHostOnRename`). `register_host` is
    /// deliberately NOT a `SharedWriter` method (see `db::SharedWriter`'s doc
    /// comment), so it needs its own handle to the transport.
    pub reducer_caller: Arc<dyn crate::sync::ReducerCaller>,
    /// Ask the store at an address for its database's identity, before any
    /// connection (`startup.allium`: `StoreIdentityPin.store_database_identity`).
    /// Blocking. A seam so the bootstrap tests can name the database without a
    /// server.
    pub store_identity: fn(&str) -> Option<String>,
}

impl StoreParts {
    /// Route `database` through a fresh connection's rows. `host_id` is this
    /// install's own — known before any connection, because it is minted
    /// locally on first run (`host.allium: MintHostIdentity`) — and the claim
    /// needs it on every write.
    pub fn build(database: db::Database, host_id: &str) -> Self {
        let rows = Arc::new(crate::sync::SharedRows::new());
        let sdk = Arc::new(crate::sync::SpacetimeSdkConnector::new(
            crate::sync::SHARED_DATABASE_NAME,
            rows.clone(),
        ));
        let settled_identity = Arc::new(crate::sync::SettledIdentity::default());
        let reducer_caller: Arc<dyn crate::sync::ReducerCaller> = Arc::new(
            crate::sync::SdkReducerCaller::new(sdk.clone(), settled_identity.clone()),
        );
        let connector: Arc<dyn crate::sync::StoreConnector> = sdk;
        // One adapter over the rows serves the board's drawing, every other
        // shared read, and the writer's claim chain — which must take the
        // task the column shows as next, so it reads the same seam.
        let board_reads = Arc::new(crate::sync::SubscriptionBoardReads::new(rows.clone()));
        let database = Arc::new(database.with_shared_store(db::SharedStorePorts {
            writer: Arc::new(crate::sync::ReducerWriter::new(
                reducer_caller.clone(),
                settled_identity.clone(),
                Arc::new(crate::service::SystemClock),
                host_id.to_string(),
                board_reads.clone(),
            )),
            reader: board_reads.clone(),
            learning_reader: Arc::new(crate::sync::SubscriptionLearningReads::new(rows.clone())),
            usage_reader: Arc::new(crate::sync::SubscriptionUsageReads::new(rows.clone())),
            retired_feed_item_reader: Arc::new(crate::sync::SubscriptionRetiredFeedItemReads::new(
                rows.clone(),
            )),
        }));
        Self {
            store_identity: store_database_identity,
            database,
            rows,
            board_reads,
            connector,
            settled_identity,
            reducer_caller,
        }
    }
}

/// A `dispatch` subcommand's own connection to the shared store.
///
/// Hold it for as long as the command runs: the connection lives in
/// `_session`, and dropping it leaves `database` with nothing to read from.
/// No reconnect loop — a command that loses its store mid-run fails, and the
/// operator runs it again.
pub struct CliStore {
    pub database: Arc<db::Database>,
    _session: crate::sync::SyncSession,
}

/// Route a handle through the store `server` names,
/// and make the first connection — the same one a board makes at startup,
/// with the same failures (`startup.allium`: `AbortWhenNoStoreIsConfigured`,
/// `AbortWhenTheStoreCannotBeReached`). For the subcommands that read or write
/// shared rows (`repo`, `plan`, the agent-tree and diff panes): with the store
/// mandatory, the local database no longer holds them.
pub async fn open_cli_store(db_path: &Path, server: Option<String>) -> Result<CliStore> {
    // Flag, then the operator's environment, then the address the board
    // published on its session, then the record a board left beside this
    // database, then the managed address (`CliCommandsReachTheStoreWithoutManagingIt`).
    // The environment is read here too, so a blank flag falls through to it.
    let server = crate::startup::cli_store_server(
        server,
        std::env::var(crate::startup::STORE_SERVER_ENV).ok(),
        std::env::var(crate::startup::BOARD_STORE_ENV).ok(),
        db_path,
    );
    let (database, host_id) = open_with_cli_identity(db_path).await?;
    let parts = StoreParts::build(database, &host_id);
    // No host-registry push: a short-lived command is not a board, and the
    // board already registers this host on every connect.
    let session = connect_first(server, &parts, None).await?;
    Ok(CliStore {
        database: parts.database,
        _session: session,
    })
}

/// The production [`StoreParts::store_identity`]: the shared database's
/// identity on `server`, over HTTP or TLS to match its address.
fn store_database_identity(server: &str) -> Option<String> {
    crate::startup::fetch_store_database_identity(
        server,
        crate::sync::SHARED_DATABASE_NAME,
        crate::startup::STORE_IDENTITY_TIMEOUT,
    )
}

/// `startup.allium`: `ConnectWhenTheStoreIsTheOneThisInstallUses` /
/// `AbortWhenTheStoreIsNotTheOneThisInstallUses`, run before the first
/// connection, which already writes. Returns the identity found, for
/// [`pin_store_after_connect`].
async fn check_store_identity(
    parts: &StoreParts,
    db_path: &Path,
    server: &str,
    accept_store_switch: bool,
) -> Result<Option<String>> {
    let probe = parts.store_identity;
    let address = server.to_string();
    let pin_db = db_path.to_path_buf();
    // One deadline over the whole probe: its own timeouts are per step, and a
    // name lookup has none. Past it the store counts as unaskable.
    let probed = tokio::time::timeout(
        crate::startup::STORE_IDENTITY_TIMEOUT * 2,
        tokio::task::spawn_blocking(move || {
            (
                probe(&address),
                crate::startup::pinned_store_identity(&pin_db),
            )
        }),
    )
    .await;
    let (found, pinned) = match probed {
        Ok(joined) => joined?,
        Err(_elapsed) => (None, crate::startup::pinned_store_identity(db_path)),
    };
    if found.is_none() {
        tracing::warn!(
            server,
            "could not read the store's database identity; connecting without the store pin check"
        );
    }
    crate::startup::check_store_pin(
        found.as_deref(),
        pinned.as_deref(),
        accept_store_switch,
        server,
    )?;
    Ok(found)
}

/// `startup.allium`: `PinTheStoreOnceItAnswers`. Best-effort: a pin that
/// cannot be written is logged by `pin_store_identity` and the launch goes on.
fn pin_store_after_connect(db_path: &Path, found: Option<&str>) {
    if let Some(identity) = found {
        crate::startup::pin_store_identity(db_path, identity);
    }
}

/// The data directory `--db` names: where `host.json`, `app.log` and the store
/// record live. The database file itself is never opened.
pub fn data_dir_of(db_path: &Path) -> &Path {
    match db_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// The handle every process routes through the store. It holds no data of its
/// own: an in-memory placeholder base for the routed ports (a production path
/// never opens `tasks.db` — `storage.allium: StoreInUseNeverOpensSqlite`),
/// with this install's identity kept in `<data_dir>/host.json`
/// (`host.allium: IdentityLivesInHostFile`).
async fn placeholder_database(data_dir: &Path) -> Result<db::Database> {
    Ok(db::Database::open_in_memory_unattached()
        .await?
        .with_host_file(data_dir))
}

/// A one-shot command's identity: read from the host file, never minted
/// (`cli.allium: CliCommandsNeedAHostFile`).
async fn open_with_cli_identity(db_path: &Path) -> Result<(db::Database, String)> {
    let data_dir = data_dir_of(db_path);
    let dir = data_dir.to_path_buf();
    let identity =
        tokio::task::spawn_blocking(move || crate::host_file::read_for_cli(&dir)).await??;
    // A host file no board has connected with yet holds no user identity, so
    // there is no one for the command to act as: refuse, and write nothing.
    if identity.user_identity.is_none() {
        anyhow::bail!(
            "this machine's identity in {} has no user identity yet: run `dispatch tui` once \
             so a board can connect to the store and complete it",
            crate::host_file::host_file_path(data_dir).display()
        );
    }
    Ok((placeholder_database(data_dir).await?, identity.host_id))
}

/// The first connection, shared by the board and the CLI: connect, settle the
/// identity, apply the initial subscription — or abort with the attempt's
/// reason (`startup.allium`: `AbortWhenTheStoreCannotBeReached`). With a
/// `register_with` caller, the host is mirrored into the shared registry too
/// (`sync.allium: RegisterHostOnConnect`) — the board's case.
async fn connect_first(
    server: String,
    parts: &StoreParts,
    register_with: Option<&dyn crate::sync::ReducerCaller>,
) -> Result<crate::sync::SyncSession> {
    let mut session = crate::sync::SyncSession::open(server, parts.connector.clone());
    session
        .connect_at_startup(&*parts.database, std::time::Instant::now())
        .await
        .map_err(|reason| {
            tracing::error!("first connection to the shared store failed: {reason}");
            crate::startup::StartupAbort::StoreUnavailable { reason }
        })?;
    match register_with {
        Some(caller) => {
            tasks::on_store_connected(&*parts.database, &parts.settled_identity, caller).await
        }
        None => {
            tasks::settle_from_store(&*parts.database, &parts.settled_identity).await;
        }
    }
    Ok(session)
}

/// The store a board connects to: one the operator named, or dispatch's own.
#[derive(Clone)]
enum StoreTarget {
    Named(String),
    Managed(Arc<crate::spacetime::managed_store::ManagedStore>),
}

impl StoreTarget {
    fn is_named(&self) -> bool {
        matches!(self, Self::Named(_))
    }

    fn managed(&self) -> Option<&Arc<crate::spacetime::managed_store::ManagedStore>> {
        match self {
            Self::Named(_) => None,
            Self::Managed(store) => Some(store),
        }
    }
}

/// Stops the managed store this board holds when dropped. Idempotent with the
/// abort paths: `ManagedStore` stops a store once.
struct ManagedStoreGuard(Option<Arc<crate::spacetime::managed_store::ManagedStore>>);

impl Drop for ManagedStoreGuard {
    fn drop(&mut self) {
        if let Some(store) = &self.0 {
            store.stop_on_exit();
        }
    }
}

/// `RecordTheNamedStoreOnceItAnswers`: record `server` beside `db_path` for a
/// named store; a managed board records nothing. True when a record was kept.
/// Best-effort: a board that cannot write it still runs.
fn record_store_server_for(target: &StoreTarget, db_path: &Path, server: &str) -> bool {
    target.is_named() && crate::startup::record_store_server(db_path, server)
}

/// `ForgetAStaleStoreRecordOnAManagedLaunch`: a managed launch clears any
/// record beside `db_path`; a named launch leaves it for its own record to
/// replace.
fn forget_stale_store_record_for(target: &StoreTarget, db_path: &Path) {
    if matches!(target, StoreTarget::Managed(_)) {
        crate::startup::forget_store_server(db_path);
    }
}

/// `ForgetTheStoreRecordWhenTheBoardExits`: removes a named board's record
/// when dropped, so every way out of `run_tui` -- a quit, an abort, an
/// unwinding panic -- takes the record with it. A managed board's guard does
/// nothing.
struct StoreRecordGuard(Option<std::path::PathBuf>);

impl StoreRecordGuard {
    fn for_target(target: &StoreTarget, db_path: &Path) -> Self {
        Self(target.is_named().then(|| db_path.to_path_buf()))
    }
}

impl Drop for StoreRecordGuard {
    fn drop(&mut self) {
        if let Some(db_path) = &self.0 {
            crate::startup::forget_store_server(db_path);
        }
    }
}

/// A later startup failure, with the managed store (if this board holds one)
/// stopped first and the failure's reason kept.
fn abort_managed_startup(
    store: Option<&Arc<crate::spacetime::managed_store::ManagedStore>>,
    error: anyhow::Error,
) -> anyhow::Error {
    let Some(store) = store else { return error };
    // Blocks for up to the stop timeout, on a launch that is already failing.
    tokio::task::block_in_place(|| match error.downcast::<crate::startup::StartupAbort>() {
        Ok(abort) => store.abort_startup(abort).into(),
        Err(other) => {
            store.stop_on_exit();
            other
        }
    })
}

/// A board that is signalled -- SIGHUP is what tmux sends when a later launch
/// retires this board's window (`kill-window`), SIGTERM what `kill` sends --
/// would otherwise die on the default disposition without running its exit,
/// and leave its managed store, or a named board's store record, behind. Catch
/// both, stop the store or forget the record, and go.
fn clean_up_on_termination(
    store: Option<Arc<crate::spacetime::managed_store::ManagedStore>>,
    record_db: Option<std::path::PathBuf>,
) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut hangup), Ok(mut terminate)) = (
        signal(SignalKind::hangup()),
        signal(SignalKind::terminate()),
    ) else {
        tracing::warn!(
            "could not listen for SIGHUP/SIGTERM; a managed store outlives a signalled board"
        );
        return;
    };
    tokio::spawn(async move {
        tokio::select! {
            _ = hangup.recv() => {}
            _ = terminate.recv() => {}
        }
        if let Some(store) = store {
            let _ = tokio::task::spawn_blocking(move || store.stop_on_exit()).await;
        }
        if let Some(db_path) = record_db {
            crate::startup::forget_store_server(&db_path);
        }
        let _ = disable_raw_mode();
        std::process::exit(0);
    });
}

/// Pick the store this launch uses: the one the operator named, or dispatch's
/// own managed one. `cli_on_path` is consulted only when none is named.
fn select_store_target(
    db_path: &Path,
    spacetime_server: Option<String>,
    cli_on_path: impl FnOnce() -> bool,
) -> Result<StoreTarget> {
    Ok(
        match crate::spacetime::managed_store::select_store(spacetime_server, cli_on_path)? {
            crate::spacetime::managed_store::StoreSelection::Named(server) => {
                StoreTarget::Named(server)
            }
            crate::spacetime::managed_store::StoreSelection::Managed => {
                // Fixed, not derived from `--db`: a throwaway database must not
                // start a second store or lose sight of the module hash the first
                // recorded.
                let store_data_dir = data_dir_of(&crate::default_db_path()).join("spacetime");
                let log_dir = data_dir_of(db_path);
                StoreTarget::Managed(Arc::new(
                    crate::spacetime::managed_store::ManagedStore::for_launch(
                        store_data_dir,
                        log_dir,
                    ),
                ))
            }
        },
    )
}

/// `RestartTheManagedStoreWhenItStopsSaving`: every health interval, ask the
/// managed store this board holds whether it is still saving, and tell the
/// board when it had to be restarted. Ends with the board's message channel.
fn watch_managed_store(
    store: Arc<crate::spacetime::managed_store::ManagedStore>,
    msg_tx: mpsc::UnboundedSender<Message>,
) {
    use crate::spacetime::managed_store::{HealthOutcome, MANAGED_STORE_HEALTH_INTERVAL};
    tokio::spawn(async move {
        let mut ticker = interval(MANAGED_STORE_HEALTH_INTERVAL);
        ticker.tick().await; // the first tick is immediate
        loop {
            ticker.tick().await;
            if msg_tx.is_closed() {
                return;
            }
            let checked = store.clone();
            let Ok(outcome) = tokio::task::spawn_blocking(move || checked.check_health()).await
            else {
                continue;
            };
            let text = match outcome {
                HealthOutcome::Healthy => continue,
                HealthOutcome::Restarted => {
                    "The local store had stopped saving (disk full?) and was restarted. \
                     Changes made since it failed were lost."
                        .to_string()
                }
                HealthOutcome::RestartFailed(reason) => {
                    format!("The local store stopped saving and could not be restarted: {reason}")
                }
            };
            tracing::warn!("{text}");
            let _ = msg_tx.send(Message::System(crate::tui::messages::SystemMessage::Error(
                text,
            )));
        }
    });
}

/// Read terminal events on a dedicated blocking thread. Keys go to `key_tx`;
/// resizes and focus changes become system messages on `msg_tx`. The thread
/// idles while `input_paused` is set (e.g. an external editor owns the
/// terminal) and ends when `key_tx` is closed.
fn spawn_input_thread(
    key_tx: mpsc::UnboundedSender<crossterm::event::KeyEvent>,
    msg_tx: mpsc::UnboundedSender<Message>,
    input_paused: Arc<AtomicBool>,
) {
    tokio::task::spawn_blocking(move || loop {
        if input_paused.load(Ordering::Relaxed) {
            std::thread::sleep(INPUT_PAUSE_SLEEP);
            continue;
        }
        if event::poll(EVENT_POLL_INTERVAL).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key_tx.send(key).is_err() => break,
                Ok(Event::Key(_)) => {}
                Ok(Event::Resize(..)) => {
                    let _ = msg_tx.send(Message::System(
                        crate::tui::messages::SystemMessage::TerminalResized,
                    ));
                }
                Ok(Event::FocusGained) => {
                    let _ = msg_tx.send(Message::System(
                        crate::tui::messages::SystemMessage::FocusChanged(true),
                    ));
                }
                Ok(Event::FocusLost) => {
                    let _ = msg_tx.send(Message::System(
                        crate::tui::messages::SystemMessage::FocusChanged(false),
                    ));
                }
                _ => {}
            }
        }
    });
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
    accept_store_switch: bool,
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

    // Which store this launch uses. `src/main.rs` already ran this check on the
    // operator's own terminal before the tmux handoff; it is repeated here
    // because this process is the one that owns a managed store -- the
    // re-exec'd board, the only process that draws and so the only one whose
    // exit can stop it (`BringUpTheManagedStoreOnceTheHostIsNamed`). The
    // process that hands off to tmux never gets this far, so exactly one
    // process starts a store.
    let target = select_store_target(
        db_path,
        spacetime_server,
        crate::spacetime::managed_store::spacetime_cli_on_path,
    )?;
    // Every way out of this function from here on -- a quit, an early `?`, a
    // panic unwinding -- drops this, which stops a managed store this board
    // holds (`StopTheManagedStoreWhenTheBoardExits`).
    let _store_guard = ManagedStoreGuard(target.managed().cloned());
    // A record left by an earlier named board must not outlive this managed
    // launch, and a named board's own record goes when this function does.
    forget_stale_store_record_for(&target, db_path);
    let _record_guard = StoreRecordGuard::for_target(&target, db_path);
    clean_up_on_termination(
        target.managed().cloned(),
        target.is_named().then(|| db_path.to_path_buf()),
    );
    let bootstrapped =
        TuiRuntime::bootstrap_for(db_path, port, paths, target.clone(), accept_store_switch).await;
    let Bootstrap {
        store_server: server,
        mut app,
        mut runtime,
        mut mcp_notify_rx,
        mut msg_rx,
    } = match bootstrapped {
        Ok(bootstrap) => bootstrap,
        // `StopTheManagedStoreWhenStartupAborts`: the abort keeps its reason.
        Err(e) => return Err(abort_managed_startup(target.managed(), e)),
    };

    let mut terminal = enter_tui_terminal()?;
    let tmux_runner = runtime.runner.clone();
    let tmux_wiring = wire_tmux_for_tui(&server, port, &*tmux_runner);

    // Create two channels:
    //    - key_rx: raw crossterm KeyEvents from the blocking poll thread
    //    - msg_rx: higher-level Messages (e.g. from dispatch results)
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<crossterm::event::KeyEvent>();

    // crossterm::event::poll/read are blocking; run them in a dedicated thread
    // so they don't block the async runtime. The thread can be paused (e.g. when
    // opening an external editor) via the input_paused flag.
    let input_paused = Arc::new(AtomicBool::new(false));
    spawn_input_thread(key_tx, runtime.msg_tx.clone(), input_paused);
    if let Some(store) = target.managed() {
        watch_managed_store(store.clone(), runtime.msg_tx.clone());
    }

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
    teardown_tmux_for_tui(
        &tmux_wiring.session,
        tmux_wiring.original_window_name.as_ref(),
        &*tmux_runner,
    );

    leave_tui_terminal(&mut terminal)?;

    result
}

/// Take over the terminal: raw mode, alternate screen, focus events.
fn enter_tui_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableFocusChange)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

/// Hand the terminal back, undoing [`enter_tui_terminal`].
fn leave_tui_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableFocusChange,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// What `run_tui` needs back from the tmux setup to undo it on exit.
struct TmuxWiring {
    session: String,
    original_window_name: Option<TmuxWindow>,
}

/// Set up the tmux keybinding (Prefix+Space → jump back to this window) and
/// publish the store address and board port. Best-effort: failures don't
/// prevent the TUI from starting.
fn wire_tmux_for_tui(server: &str, port: u16, tmux_runner: &dyn ProcessRunner) -> TmuxWiring {
    // One probe for the session and the window name together, rather than one
    // each: they are read for the same purpose and a window renamed between two
    // calls would be described by neither answer. See
    // `tmux::current_window_context`.
    let self_pane = tmux::self_pane_id();
    let here = tmux::current_window_context(self_pane.as_deref(), tmux_runner).ok();
    let original_window_name = here
        .as_ref()
        .and_then(|c| TmuxWindow::parse(&c.window_name));
    let session = here
        .as_ref()
        .map_or(String::new(), |c| c.session_name.clone());
    setup_tmux_for_tui(&session, self_pane.as_deref(), tmux_runner);
    publish_store_server(&session, server, tmux_runner);
    publish_board_port(&session, port, tmux_runner);
    TmuxWiring {
        session,
        original_window_name,
    }
}

// ---------------------------------------------------------------------------
// Embedding backfill — run at startup to embed learnings missing vectors
// ---------------------------------------------------------------------------

/// The pending model load. Production loads the real model on a blocking
/// thread; tests have nothing to wait for.
#[cfg(not(test))]
type EmbeddingLoad = tokio::task::JoinHandle<Result<Arc<EmbeddingService>>>;
#[cfg(test)]
struct EmbeddingLoad;

/// Start loading the embedding model (blocks until loaded; may download on
/// first run). Tests bypass run_tui entirely and construct TuiRuntime
/// directly, so the real load is only reached in production.
#[cfg(not(test))]
fn start_embedding_load() -> EmbeddingLoad {
    eprintln!("Loading embedding model...");
    tokio::task::spawn_blocking(EmbeddingService::new)
}
#[cfg(test)]
fn start_embedding_load() -> EmbeddingLoad {
    EmbeddingLoad
}

/// Wait for the load [`start_embedding_load`] began.
#[cfg(not(test))]
async fn finish_embedding_load(load: EmbeddingLoad) -> Result<Arc<EmbeddingService>> {
    load.await
        .map_err(|e| anyhow::anyhow!("Embedding thread panicked: {e}"))?
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to initialise embedding model: {e}\n\
                 Clear cache with: rm -rf ~/.cache/huggingface/hub/"
            )
        })
}
#[cfg(test)]
async fn finish_embedding_load(_load: EmbeddingLoad) -> Result<Arc<EmbeddingService>> {
    Ok(EmbeddingService::new_noop())
}

/// Seed the example feed epic and provision the managed feed-epic tree.
/// Idempotent and best-effort: a failure is logged and never blocks startup.
async fn seed_and_provision_feeds(database: &Arc<db::Database>, data_dir: &Path) {
    if let Err(e) = crate::setup::seed_feed_epics(database, data_dir).await {
        tracing::warn!("Example feed epic seeding failed: {e:#}");
    }
    if let Err(e) = crate::service::provision_managed_feeds_from_settings(&**database).await {
        tracing::warn!("Managed feed provisioning failed: {e:#}");
    }
}

/// Fire-and-forget: partial work is retried on the next startup.
fn spawn_embedding_backfill(database: Arc<db::Database>, emb: Arc<EmbeddingService>) {
    tokio::spawn(async move {
        if let Err(e) = backfill_embeddings(database, emb).await {
            tracing::warn!("Embedding backfill failed: {e}");
        }
    });
}

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
    /// Deliberately separate from `database` rather than replacing it. This one
    /// answers "what is on the board?" — the reads the row-change pump and the
    /// revision guard refresh — and `database` answers everything else. Both
    /// read the same subscription rows; see `crate::sync::board_reads`.
    board_reads: Arc<dyn crate::sync::BoardReads>,
    /// This machine's own `Host.id` — minted locally on first run, immutable
    /// afterwards (`host.allium: MintHostIdentity`). Needed by
    /// `exec_check_status_if_owned`/the feed-tick ownership check to compare
    /// against `core/PollOwner.host`, the same value `ReducerWriter`'s own
    /// `host` field carries.
    host_id: String,
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
pub(crate) mod poll_ownership;
mod pr;
mod repo_sync;
mod settings;
mod split;
mod tasks;
#[cfg(test)]
mod tests;

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
    async fn bootstrap_for(
        db_path: &Path,
        port: u16,
        paths: &StartupPaths,
        target: StoreTarget,
        accept_store_switch: bool,
    ) -> Result<Bootstrap> {
        Self::bootstrap_inner(
            db_path,
            port,
            paths,
            target,
            StoreParts::build,
            accept_store_switch,
        )
        .await
    }

    /// [`Self::bootstrap`], with the store's wiring supplied by the caller.
    ///
    /// Production passes [`StoreParts::build`]. The tests pass a stand-in
    /// whose connector accepts without a server and whose database is left
    /// unrouted, so the startup wiring can be exercised against SQLite until
    /// Phase 12b (#4975) supplies an in-memory store.
    #[cfg(test)]
    async fn bootstrap_with(
        db_path: &Path,
        port: u16,
        paths: &StartupPaths,
        server: String,
        build_store: fn(db::Database, &str) -> StoreParts,
        accept_store_switch: bool,
    ) -> Result<Bootstrap> {
        Self::bootstrap_inner(
            db_path,
            port,
            paths,
            StoreTarget::Named(server),
            build_store,
            accept_store_switch,
        )
        .await
    }

    /// The composition root itself. A managed store is brought up between the
    /// host being named and the first connection
    /// (`BringUpTheManagedStoreOnceTheHostIsNamed`).
    async fn bootstrap_inner(
        db_path: &Path,
        port: u16,
        paths: &StartupPaths,
        target: StoreTarget,
        build_store: fn(db::Database, &str) -> StoreParts,
        accept_store_switch: bool,
    ) -> Result<Bootstrap> {
        // ONE read of the host identity, used twice: the board needs its own
        // host id and the writer needs it for the claim. The abort is the
        // strict one on purpose — see the long note at the `set_local_host_id`
        // call below for why a board that cannot read its own identity must
        // not draw.
        //
        // The agent port is claimed first, so a launch that is going to abort
        // on a port somebody else holds mints no host file
        // (`CheckHostLabelAfterStartupConfigResolves`: `requires:
        // agent_port_available`, then `FirstRun`). Claimed here, before the
        // board takes the screen, so a port another
        // process still holds aborts the launch where the operator can read it
        // — `startup.allium`'s `AbortWhenTheAgentPortIsTaken`. Bound inside the
        // spawned task instead, the failure would land on a stderr the drawn
        // board has already covered, leaving a board no agent can reach.
        let mcp_listener = claim_agent_port(port).await?;
        let data_dir = data_dir_of(db_path).to_path_buf();
        let identity = {
            let dir = data_dir.clone();
            tokio::task::spawn_blocking(move || crate::host_file::resolve_for_launch(&dir))
                .await??
        };
        let (host_id, host_label) = (identity.host_id, identity.label);
        let database = placeholder_database(&data_dir).await?;
        // Routed at construction: which backing a read or write goes to cannot
        // change under a caller. Nothing connects yet — that is
        // `connect_first` below, once the host is named.
        let parts = build_store(database, &host_id);
        let database = parts.database.clone();

        // `data_dir` (above) is the directory the operator named with `--db`:
        // the example feed epic's script lives there (seeded once the store is
        // up, below).

        // Initialise the embedding model (blocks until loaded; may download on first run).
        // Tests bypass run_tui entirely and construct TuiRuntime directly, so
        // the non-test branch is only reached in production.
        //
        // Started now and awaited after the first store connection: the two
        // are independent, so a cold start costs the slower of them rather
        // than both.
        let emb_load = start_embedding_load();

        // Spawn MCP server with notification channel.
        // Handed the operator's config location rather than looking it up — see
        // `SettingsLocationIsAnExplicitStartupInput`. This one runner backs the
        // MCP server, the feed runner and `TaskService`, so every launch path
        // gets the same answer.
        let runner: Arc<dyn ProcessRunner> = Arc::new(RealProcessRunner::with_claude_json(
            paths.claude_json_path.clone(),
        ));
        let (mcp_notify_tx, mcp_notify_rx) = mpsc::unbounded_channel::<mcp::McpEvent>();
        let feed_notify_tx = mcp_notify_tx.clone();

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
        let label = host_label;

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
        name_the_host(label, &database).await?;

        // THE FIRST CONNECTION, before anything reads a shared row and before
        // the board draws. startup.allium: ConnectToTheStoreOnceTheHostIsNamed
        // and AbortWhenTheStoreCannotBeReached; sync.allium:
        // OpenBoardConnection. Everything below — the tasks the board opens
        // with, the example feed epic, the managed feeds, the repo paths and
        // every setting the loaders read — is a shared row, and would read
        // nothing from a store that had not answered yet.
        let server = resolve_store_server(&target).await?;
        // Before connecting: the first connection already writes, so a store
        // holding a different database from the one this install last used is
        // refused here (`AbortWhenTheStoreIsNotTheOneThisInstallUses`, task
        // #28710).
        let found = check_store_identity(&parts, db_path, &server, accept_store_switch).await?;
        let session = connect_first(server.clone(), &parts, Some(&*parts.reducer_caller)).await?;
        pin_store_after_connect(db_path, found.as_deref());
        // Only an address that answered is worth recording; failing to write
        // it is a warning, not an abort.
        record_store_server_for(&target, db_path, &server);
        let store_server = server;
        let sync_store: Arc<dyn crate::sync::SyncStore> = database.clone();

        let emb_svc = finish_embedding_load(emb_load).await?;

        let mcp_deps = mcp::McpDeps {
            db: database.clone(),
            runner: runner.clone(),
            embedding_service: emb_svc.clone(),
            data_dir: data_dir.clone(),
        };

        let tasks = database.list_all().await?;

        // Seed the example feed epic for a store that has none, and provision
        // the managed feed-epic tree from the reviews/CVE config. Both create
        // shared rows, which needs the identity settled just above
        // (sync.allium: CreatesRequireASettledIdentity) — which is why they
        // run here rather than when the database opens. Idempotent and
        // best-effort: a failure here must not block startup.
        seed_and_provision_feeds(&database, &data_dir).await;

        // Backfill embeddings for any learnings that were created before the model
        // was available. Fire-and-forget: partial work is retried on next startup.
        // After the connection, because the learnings it backfills are the
        // store's.
        spawn_embedding_backfill(database.clone(), emb_svc.clone());

        // Serve agents now that their reads have something to answer from.
        // The port was claimed above, before the board could take the screen;
        // a connection that arrived in between waited in the listen backlog.
        tokio::spawn(async move {
            if let Err(e) = mcp::serve_on(mcp_listener, mcp_deps, mcp_notify_tx).await {
                eprintln!("MCP server error: {e}");
            }
        });

        let mut app = hydrate_app(tasks, &host_id, &database, &*runner).await;
        app.set_store_server(Some(store_server.clone()));

        // WHERE THIS BOARD'S CARDS COME FROM: the subscription, always. There
        // is deliberately no fallback to the local copy when the store is down:
        // the shared tables have exactly one copy, and a fallback would be the
        // read-through cache `crate::sync::rows` exists not to be. A board whose
        // store drops mid-session draws empty and says why
        // (`ConnectionIndicator`).

        let (runtime, msg_rx) = Self::build_runtime(
            &database,
            &runner,
            &emb_svc,
            &parts,
            &host_id,
            feed_notify_tx,
            paths,
        );

        runtime.start_background_tasks(&parts, session, sync_store, app.repo_paths());

        Ok(Bootstrap {
            store_server,
            app,
            runtime,
            mcp_notify_rx,
            msg_rx,
        })
    }

    /// Start the tasks that keep the board live behind the first connection.
    fn start_background_tasks(
        &self,
        parts: &StoreParts,
        session: crate::sync::SyncSession,
        sync_store: Arc<dyn crate::sync::SyncStore>,
        saved_repo_paths: &[String],
    ) {
        // Keep the board redrawing behind the connection, and keep the
        // connection up: the first attempt is already answered (above), so the
        // loop starts from `connected` and only ever handles later drops.
        drop(self.spawn_row_change_pump(parts.rows.clone()));
        drop(self.spawn_shared_store_connection(
            session,
            sync_store,
            parts.settled_identity.clone(),
            parts.reducer_caller.clone(),
        ));

        // RefreshRepoSyncStateOnStartup: the only genuinely new network traffic
        // this feature introduces — one fetch per saved repo path. Fire-and-forget,
        // so a slow or offline network never delays startup.
        drop(self.exec_refresh_all_repo_sync(saved_repo_paths));
    }

    /// Wire the services, the feed runner and the message channel around an
    /// already-connected store. Nothing here touches the network or the terminal.
    fn build_runtime(
        database: &Arc<db::Database>,
        runner: &Arc<dyn ProcessRunner>,
        emb_svc: &Arc<EmbeddingService>,
        parts: &StoreParts,
        host_id: &str,
        feed_notify_tx: mpsc::UnboundedSender<mcp::McpEvent>,
        paths: &StartupPaths,
    ) -> (TuiRuntime, mpsc::UnboundedReceiver<Message>) {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel::<Message>();
        // Hoisted above `FeedRunner::new` so both it and the runtime's own
        // `board_reads` field share one handle — `FeedTick`'s host-scoping
        // (feeds.allium: FeedTick) needs to read `core/PollOwner`, which is
        // exactly what this seam answers.
        let board_reads: Arc<dyn crate::sync::BoardReads> = parts.board_reads.clone();
        let feed_runner = crate::feed::FeedRunner::new(
            database.clone(),
            feed_notify_tx,
            runner.clone(),
            board_reads.clone(),
            host_id.to_string(),
        );
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
            learning_svc: Arc::new(crate::service::LearningService::new(
                database.clone(),
                emb_svc.clone(),
            )),
            feed_runner: Some(feed_runner),
            feed_invalidate_tx,
            feed_sync_guard,
            feed_db: database.clone(),
            board_reads,
            host_id: host_id.to_string(),
            database: database.clone(),
            msg_tx,
            runner: runner.clone(),
            editor_session: Arc::new(std::sync::Mutex::new(None)),
            emb_svc: emb_svc.clone(),
            last_change_count: Arc::new(AtomicI64::new(-1)),
            // Deliberately not derived from `db_path`: the subscription windows
            // are account-global, so a run against a throwaway database must
            // publish and read the same location as every other session. See
            // docs/specs/observability.allium:
            // SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase.
            budget_snapshot_path: crate::budget_snapshot_path(),
            claude_json_path: paths.claude_json_path.clone(),
            split_restores: std::sync::Mutex::new(Vec::new()),
        };
        (runtime, msg_rx)
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
///
/// No registry push here: the startup prompt runs before the first
/// connection (`startup.allium: ConnectToTheStoreOnceTheHostIsNamed`), and
/// `sync.allium: RegisterHostOnConnect` pushes the new label as that
/// connection settles. A rename surface reachable while connected would push
/// itself, via [`crate::sync::push_host_registration`]
/// (`sync.allium: RegisterHostOnRename`).
async fn persist_host_label(
    db: &dyn db::HostStore,
    label: &str,
) -> std::result::Result<(), crate::startup::StartupAbort> {
    db.rename_host(label).await.map_err(|e| {
        tracing::error!("Failed to persist host label: {e:#}");
        crate::startup::StartupAbort::HostIdentityUnavailable
    })?;
    Ok(())
}

/// Claim the agent port before the board takes the screen.
async fn claim_agent_port(port: u16) -> Result<tokio::net::TcpListener> {
    mcp::bind(port).await.map_err(|e| {
        tracing::error!("agent port {port} unavailable: {e}");
        anyhow::anyhow!(
            "{}",
            crate::startup::StartupAbort::AgentPortUnavailable { port }.message()
        )
    })
}

/// Resolve the host's label, prompting if it is unset, and persist a new one.
async fn name_the_host(label: Option<String>, database: &db::Database) -> Result<()> {
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
        // `persist_host_label` for the mapping.
        Ok(Some(new_label)) => persist_host_label(database, &new_label).await?,
        Ok(None) => {}
        Err(abort) => return Err(anyhow::anyhow!("{}", abort.message())),
    }
    Ok(())
}

/// The store address to connect to, bringing a managed store up first.
async fn resolve_store_server(target: &StoreTarget) -> Result<String> {
    match target {
        StoreTarget::Named(server) => Ok(server.clone()),
        StoreTarget::Managed(store) => {
            // Blocking: probes, a process start, a publish. Off the async
            // threads so the runtime keeps turning meanwhile.
            let store = store.clone();
            eprintln!("Starting the local store...");
            let ready = tokio::task::spawn_blocking(move || store.bring_up())
                .await
                .map_err(|e| anyhow::anyhow!("managed store thread panicked: {e}"))??;
            Ok(ready.server)
        }
    }
}

/// Create the `App` and hydrate all persisted settings.
async fn hydrate_app(
    tasks: Vec<models::Task>,
    host_id: &str,
    database: &db::Database,
    runner: &dyn ProcessRunner,
) -> App {
    let mut app = App::new(tasks);
    app.set_local_host_id(host_id.to_string());
    let (repo_paths, base_branch_pairs) = tokio::join!(
        database.list_repo_paths(),
        database.list_all_base_branches()
    );
    app.update(Message::RepoPathsUpdated(repo_paths.unwrap_or_default()));
    app.update(Message::BaseBranchesUpdated(group_base_branches_by_repo(
        base_branch_pairs.unwrap_or_default(),
    )));
    load_notifications_pref(database, &mut app).await;
    load_repo_filter(database, &mut app).await;
    load_collapsed_sections(database, &mut app).await;
    load_collapsed_epics(database, &mut app).await;
    if let Some(msg) = apply_tmux_focus_warning(runner) {
        app.update(msg);
    }
    app
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

/// Restore the folded epic groups, on the same terms as
/// `load_collapsed_sections` (board-layout.allium: "Epic Folding").
async fn load_collapsed_epics(db: &dyn db::SettingsStore, app: &mut App) {
    if let Ok(Some(val)) = db.get_setting_string(COLLAPSED_EPICS_KEY).await {
        app.set_epic_folds(EpicFoldState::parse(&val));
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
