//! Obtaining the tmux session the board requires: planning the launch, entering
//! or restarting inside a session.

use std::path::Path;

use super::retire::{retire_board_window, RetireOutcome};
use crate::models::TmuxWindow;
use crate::process::ProcessRunner;
pub use crate::startup_abort::StartupAbort;
use crate::tmux;

/// The tmux session `dispatch tui` creates for itself when run outside one.
/// `startup.allium`'s `config.session_name`.
///
/// Fixed rather than derived: it is the name the operator reattaches to by
/// hand, and a name that varies per invocation cannot be reattached to.
pub const SESSION_NAME: &str = "dispatch";

/// The name the board's own tmux window carries.
/// `startup.allium`'s `config.board_window_name`.
///
/// One definition for two readers who must not disagree: the board's window is
/// given this name as it is created — by [`session_argv`] on the cold path and
/// by `tmux::new_window_in_session_running` on the restart path — and a later
/// launch finds the window to retire by this name. A launch looking under a
/// name the board never adopts would retire nothing and start a rival.
///
/// `runtime::setup_tmux_for_tui` still renames, for the one path that creates
/// no window: a board drawing in a window the operator already had.
pub const BOARD_WINDOW_NAME: TmuxWindow = TmuxWindow::from_static("TUI");

/// Why the command could not put itself inside a tmux session. Both are fatal
/// — `startup.allium`'s `StartupAbortsOnlyOnAnUnusableSubstrate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLaunchFailure {
    /// No tmux on `PATH`.
    TmuxUnavailable,
    /// tmux was reached and refused to start or attach.
    LaunchRejected,
}

impl SessionLaunchFailure {
    /// The operator-facing message. The two cases are worded apart because the
    /// operator's next action differs: install it, or look at why the server
    /// refused (`SessionLauncher`'s `DistinguishesAbsentFromBroken`).
    pub fn message(self) -> String {
        match self {
            Self::TmuxUnavailable => "dispatch tui needs tmux, which is not on PATH. Install it \
                 (e.g. `sudo dnf install tmux`) and run `dispatch tui` again."
                .to_string(),
            Self::LaunchRejected => format!(
                "tmux refused to start or attach to the `{SESSION_NAME}` session. \
                 Check `tmux list-sessions` and the tmux server's state."
            ),
        }
    }
}

/// Everything the launch decision reads, gathered before any of it is acted on.
///
/// Passed in rather than probed inside [`plan_launch`] so the decision is
/// testable without an environment the test harness shares across threads and
/// without a tmux server (the same shape as `setup::home_dir_from_value`).
///
/// Note what is *not* here: any signal about whether a board already running in
/// the session is alive or responsive. `startup.allium`'s
/// `RetiringIsNotConditionalOnLiveness` — there is one launch behaviour, and
/// the planner has no input that could split it in two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchContext {
    /// Not inside tmux. The only question is whether dispatch's own session is
    /// already there.
    Outside {
        /// Whether a session named [`SESSION_NAME`] exists.
        session_exists: bool,
    },
    /// Inside a session. Which session it is, and what this process's own
    /// window is to the board.
    Inside {
        /// The session this process is in, empty when tmux would not name it.
        session: String,
        role: BoardWindowRole,
    },
}

/// What the window this process is running in is, relative to the board.
/// `SessionLauncher`'s `launching_as_the_started_board` and
/// `launching_from_the_board_window`, as one answer rather than two booleans
/// that must never both be true.
///
/// Read from the window's name and how many panes it holds. Both entry paths
/// create the board's window already carrying [`BOARD_WINDOW_NAME`], so the
/// board's own process runs the launch path from inside a window bearing that
/// name — the name alone cannot tell the board from a shell beside one. The
/// board holds its window alone, so the pane count is what separates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardWindowRole {
    /// The board tmux has just started, alone in a window created for it.
    /// Retiring here would close this very window, taking the board with it.
    StartedBoard,
    /// A shell sharing the board's window with a board already there — the
    /// split-pane agent's neighbour, or the operator typing beside the board.
    /// Retiring here would close the shell issuing the command.
    SharedWithBoard,
    /// An ordinary window of the operator's own. The board may draw here, and
    /// any board elsewhere in the session is retired first.
    NotTheBoard,
}

impl BoardWindowRole {
    /// The role a window carrying `name` and holding `panes` panes plays.
    ///
    /// Getting this wrong is not a missed optimisation. A board that read its
    /// own window as a previous board's would retire it, closing itself before
    /// it drew — and where that window was the session's only one, taking the
    /// session with it.
    pub fn of(name: &str, panes: u32) -> Self {
        if name != BOARD_WINDOW_NAME.as_str() {
            return Self::NotTheBoard;
        }
        if panes > 1 {
            Self::SharedWithBoard
        } else {
            Self::StartedBoard
        }
    }
}

/// The operator's environment variable that names the shared store, beside the
/// `--spacetime-server` flag. Only the operator sets it; a board never writes it.
pub const STORE_SERVER_ENV: &str = "DISPATCH_SPACETIME_SERVER";

/// The address a board publishes on its own tmux session, so the panes and
/// agent windows it starts reach the same store. Read by subcommands only
/// (`cli_store_server`), never by `dispatch tui`, so an address an earlier
/// board left on the session cannot read as a store the operator named
/// (`startup.allium`: `PublishTheBoardsStoreOnItsSession`, task #28729).
pub const BOARD_STORE_ENV: &str = "DISPATCH_BOARD_STORE";

/// The store a subcommand connects to: the one named, or -- when none is --
/// the managed store's address. A short-lived subcommand (`repo`, `plan`, the
/// agent-tree and diff panes) never starts, publishes to or stops anything;
/// only a board does (`startup.allium`: `ANamedStoreIsNeverManaged` is about
/// the board, and these commands are not one), so with no store named they
/// reach for the address the board's own managed store listens on.
///
/// Blank is none: `DISPATCH_SPACETIME_SERVER=` exported empty is a common
/// shell idiom for "unset", and a connect attempt against an empty address
/// would fail later with a worse message.
pub fn store_server_or_managed(server: Option<String>) -> String {
    crate::spacetime::managed_store::normalize_server(server).unwrap_or_else(|| {
        format!(
            "http://{}",
            crate::spacetime::managed_store::MANAGED_STORE_ADDRESS
        )
    })
}

// ---------------------------------------------------------------------------
// StoreAddressRecord (startup.allium)
//
// A board on a named store keeps that address in a small file beside its
// database, so a command run from a plain terminal reaches the same store
// without the flag or the environment variable.
// ---------------------------------------------------------------------------

/// The record's file name, in the same folder as the database file.
const STORE_RECORD_FILE: &str = "store-server";

/// `file` inside the data directory `data_dir`: where the
/// store record and the store pin live (`TheRecordBelongsToItsDatabase`,
/// `ThePinBelongsToItsDatabase`).
pub(super) fn beside_data_dir(data_dir: &std::path::Path, file: &str) -> std::path::PathBuf {
    data_dir.join(file)
}

fn store_record_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    beside_data_dir(data_dir, STORE_RECORD_FILE)
}

/// The address a board on `data_dir` recorded beside it, else `None`. Blank is
/// `None`. `startup.allium`: `StoreAddressRecord.recorded_store_server`.
pub fn recorded_store_server(data_dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(store_record_path(data_dir)).ok()?;
    crate::spacetime::managed_store::normalize_server(Some(text))
}

/// Record `address` beside `data_dir`, replacing whatever was there. True when
/// the record was kept. `startup.allium`: `StoreAddressRecord.record_store_server`.
pub fn record_store_server(data_dir: &std::path::Path, address: &str) -> bool {
    match std::fs::write(store_record_path(data_dir), format!("{}\n", address.trim())) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("could not record the store address beside the database: {e}");
            false
        }
    }
}

/// Remove the record beside `data_dir`. A record that is not there is not a
/// failure. `startup.allium`: `StoreAddressRecord.forget_store_server`.
pub fn forget_store_server(data_dir: &std::path::Path) -> bool {
    match std::fs::remove_file(store_record_path(data_dir)) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            tracing::warn!("could not remove the store address record: {e}");
            false
        }
    }
}

/// The store a short-lived subcommand connects to, first answer wins: the
/// flag, the operator's environment variable, the address the board published
/// on its session (`board`), the record beside `data_dir`, the managed
/// address. Blank counts as absent at every step. `cli.allium`:
/// `CliCommandsReachTheStoreWithoutManagingIt`.
pub fn cli_store_server(
    flag: Option<String>,
    env: Option<String>,
    board: Option<String>,
    data_dir: &std::path::Path,
) -> String {
    use crate::spacetime::managed_store::normalize_server;
    let named = normalize_server(flag)
        .or_else(|| normalize_server(env))
        .or_else(|| normalize_server(board))
        .or_else(|| recorded_store_server(data_dir));
    store_server_or_managed(named)
}

/// The operator-facing wording lives here, beside the session and window
/// names it cites; the enum itself is in `src/startup_abort.rs` so the layers
/// that discover an abort (`host_file`, `spacetime::managed_store`) need not
/// depend on `startup`.
impl StartupAbort {
    /// The operator-facing message. Each names the next action, because that is
    /// what differs between them — install tmux, look at the server, move
    /// window, close a board.
    pub fn message(&self) -> String {
        match self {
            Self::TmuxUnavailable => SessionLaunchFailure::TmuxUnavailable.message(),
            Self::LaunchRejected => SessionLaunchFailure::LaunchRejected.message(),
            Self::BoardAlreadyInThisWindow => String::from(
                "This window is already the dispatch board. Restarting it from a pane \
                 inside it would close the shell you are typing in. Run `dispatch tui` \
                 from another tmux window, or from outside tmux.",
            ),
            Self::PreviousBoardNotRetired => format!(
                "The board already running in the `{SESSION_NAME}` session could not be \
                 closed, so a new one was not started — two boards in one session would \
                 fight over the agent port and the tmux keybindings. Close the `{name}` \
                 window by hand (`tmux kill-window -t {SESSION_NAME}:{name}`) and try again.",
                name = BOARD_WINDOW_NAME.as_str(),
            ),
            Self::SessionUnidentified => String::from(
                "tmux would not say which session this window belongs to, so dispatch \
                 cannot tell which board to replace. Check that the tmux server is \
                 healthy (`tmux list-sessions`), or run `dispatch tui` from outside \
                 tmux.",
            ),
            Self::AgentPortUnavailable { port } => format!(
                "Port {port} is already in use, so agents would have no way to reach this \
                 board. Another dispatch board is probably still holding it — close it, \
                 or start this one with `--port <n>`."
            ),
            Self::HostUnnamed => String::from(
                "This machine has not been named yet, and nothing could ask for a name \
                 (no terminal to prompt on). Run `dispatch tui` interactively once to name \
                 it; every scripted launch after that will proceed on its own.",
            ),
            Self::HostIdentityUnavailable => String::from(
                "This machine's identity could not be read or created — dispatch could not \
                 read or write its identity file, host.json, in its data directory. Check that \
                 host.json and the directory are readable and writable (a damaged host.json is \
                 never overwritten: repair or remove it), then run `dispatch tui` again; there \
                 is no name to type here, the problem is lower down than that.",
            ),
            Self::SpacetimeCliMissing => String::from(
                "The `spacetime` command is not installed, and dispatch needs it to run its \
                 own local store. Install it from https://spacetimedb.com/install and run \
                 `dispatch tui` again, or point dispatch at a store that is already running \
                 with `--spacetime-server <url>` or DISPATCH_SPACETIME_SERVER.",
            ),
            Self::ManagedStorePortTaken { address } => format!(
                "Something other than a SpacetimeDB store is listening on {address}, where \
                 dispatch runs its own local store. Stop that program and run `dispatch tui` \
                 again, or point dispatch at a store elsewhere with `--spacetime-server <url>` \
                 or DISPATCH_SPACETIME_SERVER."
            ),
            Self::ManagedStoreDidNotStart { reason, stopped } => format!(
                "dispatch started its local SpacetimeDB store and it did not come up: \
                 {reason}. {} Run `dispatch tui` again once the cause is fixed.",
                if *stopped {
                    "Nothing was left running."
                } else {
                    "The store did not stop in time and may still be running; the next launch \
                     will adopt it or you can stop it yourself."
                }
            ),
            Self::ModuleNeedsManualMigration { reason } => format!(
                "This dispatch build's database module cannot replace the one the local store \
                 runs without deleting data, and dispatch never deletes your data to make a \
                 launch go through: {reason}. Run the dispatch build that matches the store, \
                 or migrate or clear the `dispatch` database yourself, then run `dispatch \
                 tui` again."
            ),
            Self::ModulePublishFailed { reason } => format!(
                "dispatch could not publish its database module to the local store: {reason}. \
                 The board has not started, and the store was stopped. Run `dispatch tui` \
                 again once the cause is fixed."
            ),
            Self::StoreSwitched {
                address,
                pinned,
                found,
            } => format!(
                "The store at {address} holds a different database ({found}) from the one this \
                 install last used ({pinned}). The board has not started, so nothing was written \
                 to it, and a local store dispatch started for this launch was stopped. If the \
                 store was chosen by mistake -- an old DISPATCH_SPACETIME_SERVER, \
                 say -- unset it and run `dispatch tui` again. If you mean to move to this \
                 database, run `dispatch tui --accept-store-switch` once."
            ),
            Self::StoreUnavailable { reason } => format!(
                "Could not connect to the shared store: {reason}. The board draws only what \
                 the store holds, so it has not started. Check that the server is running \
                 and the address is right, then run `dispatch tui` again."
            ),
        }
    }
}

impl std::fmt::Display for StartupAbort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for StartupAbort {}

impl From<SessionLaunchFailure> for StartupAbort {
    fn from(failure: SessionLaunchFailure) -> Self {
        match failure {
            SessionLaunchFailure::TmuxUnavailable => Self::TmuxUnavailable,
            SessionLaunchFailure::LaunchRejected => Self::LaunchRejected,
        }
    }
}

/// What the launch path decided, computed without touching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// This process is the board tmux just started, in a window created for
    /// it. Draw, retiring nothing — the only board that could be retired here
    /// is this one. `startup.allium`'s `DrawInTheWindowThisBoardWasStartedIn`.
    DrawInThisWindow,
    /// Inside a session the operator already had — retire any board window in
    /// it, then carry on and draw in this process.
    /// `startup.allium`'s `LaunchBoardInsideExistingSession`.
    ContinueHere { session: String },
    /// The launch cannot proceed and the operator is told why.
    /// `startup.allium`'s `RefuseToLaunchFromInsideTheBoardWindow` and
    /// `RefuseWhenTheSessionCannotBeIdentified`.
    Refuse(StartupAbort),
    /// No session of dispatch's own yet — replace this process with the same
    /// invocation running inside a session created for it.
    /// `startup.allium`'s `LaunchBoardBySupplyingASession`.
    EnterSession { session: String, argv: Vec<String> },
    /// The session is already there — retire the board in it, start a fresh one
    /// from this invocation, and attach.
    /// `startup.allium`'s `RestartTheBoardInTheExistingSession`.
    RestartInSession { session: String, argv: Vec<String> },
}

/// Decide how to obtain the session the board needs, and what to do about any
/// board already in it.
pub fn plan_launch(ctx: LaunchContext, argv: Vec<String>) -> LaunchPlan {
    match ctx {
        // The case every board arrives in: tmux started it in a window created
        // for it, and the launch that created that window already retired
        // whatever came before.
        LaunchContext::Inside {
            role: BoardWindowRole::StartedBoard,
            ..
        } => LaunchPlan::DrawInThisWindow,
        LaunchContext::Inside {
            role: BoardWindowRole::SharedWithBoard,
            ..
        } => LaunchPlan::Refuse(StartupAbort::BoardAlreadyInThisWindow),
        // No session name leaves nothing to scope the retire to, and an
        // unscoped one closes a window dispatch may not own.
        LaunchContext::Inside { session, .. } if session.is_empty() => {
            LaunchPlan::Refuse(StartupAbort::SessionUnidentified)
        }
        LaunchContext::Inside { session, .. } => LaunchPlan::ContinueHere { session },
        LaunchContext::Outside {
            session_exists: true,
        } => LaunchPlan::RestartInSession {
            session: SESSION_NAME.to_string(),
            argv,
        },
        LaunchContext::Outside { .. } => LaunchPlan::EnterSession {
            session: SESSION_NAME.to_string(),
            argv,
        },
    }
}

/// Read the launch context from the running process and the tmux server.
///
/// The impure counterpart of [`plan_launch`]: every probe lives here, so the
/// decision itself stays a pure function of what was read.
pub fn read_launch_context(runner: &dyn ProcessRunner) -> LaunchContext {
    if !inside_tmux_session() {
        return LaunchContext::Outside {
            session_exists: tmux::session_exists(SESSION_NAME, runner),
        };
    }
    // `$TMUX_PANE` is the only answer that is about this process: an untargeted
    // `display-message` reports the session's *active* window, so a launch from
    // a sibling window would read the board's window as its own and conclude it
    // is the board. See `tmux::self_pane_id`.
    launch_context_inside(tmux::self_pane_id().as_deref(), runner)
}

/// [`read_launch_context`]'s inside-tmux half, with the environment read
/// already done.
///
/// Separated so the composition — probe, then role — is exercised by a test
/// against a real tmux server rather than re-implemented there, and so no test
/// depends on an environment the harness shares across threads.
pub fn launch_context_inside(pane: Option<&str>, runner: &dyn ProcessRunner) -> LaunchContext {
    // One probe answers both questions, so a launch pays for a single
    // subprocess. Inside a session, the session in hand is the one used, so its
    // existence is not a question.
    match tmux::current_window_context(pane, runner) {
        Ok(ctx) => LaunchContext::Inside {
            session: ctx.session_name,
            role: BoardWindowRole::of(&ctx.window_name, ctx.window_panes),
        },
        // A probe that cannot be answered is not the board's window: refusing on
        // a failed probe would block the operator over a tmux hiccup, while
        // proceeding at worst retires a window that was about to be replaced
        // anyway. The empty session name is the half that is NOT waved through —
        // `plan_launch` refuses on it, because an unscoped retire closes a
        // window dispatch may not own.
        Err(e) => {
            tracing::warn!("could not read this tmux window's context: {e}");
            LaunchContext::Inside {
                session: String::new(),
                role: BoardWindowRole::NotTheBoard,
            }
        }
    }
}

/// The exact tmux command line that creates-or-attaches `session` and runs
/// `argv` inside it.
///
/// `-A` is what upholds `SessionLauncher`'s `NeverCreatesASecondSession`:
/// create-or-attach is one indivisible tmux operation, so no second board can
/// appear between a check and a create.
///
/// The caller reaches here having read `session_exists` as false, and `-A`
/// covers the gap between that reading and this command: a session that
/// appeared in between is attached to rather than duplicated. That attach runs
/// no board — tmux ignores `argv` for an existing session — but the same gap
/// is what `RestartTheBoardInTheExistingSession` handles on the reading the
/// planner actually saw, and losing this race is rarer than the rival session
/// dropping `-A` would create.
pub fn session_argv(session: &str, argv: &[String]) -> Vec<String> {
    let mut out = vec![
        "tmux".to_string(),
        "new-session".to_string(),
        "-A".to_string(),
        "-s".to_string(),
        session.to_string(),
        // The board's window carries its name from creation rather than
        // adopting it later. A board that dies before `setup_tmux_for_tui`
        // renames anything still leaves a window the next launch can find and
        // retire; without this, that launch retires nothing and starts a rival.
        "-n".to_string(),
        BOARD_WINDOW_NAME.as_str().to_string(),
    ];
    out.push("--".to_string());
    out.extend(argv.iter().cloned());
    out
}

/// This invocation, rebuilt so it can be handed to tmux: the resolved binary
/// path followed by the arguments the operator passed.
///
/// `argv[0]` comes from the running executable rather than from `args()`,
/// which may hold a relative path that no longer resolves once tmux has
/// changed the working directory.
pub fn current_invocation(exe: &Path, args: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = vec![exe.display().to_string()];
    out.extend(args);
    out
}

/// Replace this process with one inside `session`.
///
/// Only ever returns on failure — on success there is no "after".
#[cfg(unix)]
pub fn enter_session(session: &str, argv: &[String]) -> SessionLaunchFailure {
    exec_tmux(&session_argv(session, argv))
}

/// Replace this process with `full`, which always names tmux first.
///
/// Only ever returns on failure. Shared by both entry paths so the exec and the
/// error classification have one definition rather than one per path.
#[cfg(unix)]
fn exec_tmux(full: &[String]) -> SessionLaunchFailure {
    use std::os::unix::process::CommandExt;

    // Both callers build from literals, so the empty case is unreachable; the
    // let-else is how that is stated without an unwrap.
    let Some((program, args)) = full.split_first() else {
        return SessionLaunchFailure::LaunchRejected;
    };
    let err = std::process::Command::new(program).args(args).exec();
    classify_launch_error(err.kind())
}

/// The tmux command line that attaches this process to an existing session.
///
/// `=` forces an exact session match, for the reason [`tmux::session_exists`]
/// gives: a prefix match would attach the operator to a session they named for
/// something else.
pub fn attach_argv(session: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "attach-session".to_string(),
        "-t".to_string(),
        format!("={session}"),
    ]
}

/// Retire the board in `session`, start a fresh one from `argv`, and attach.
/// `startup.allium`'s `RestartTheBoardInTheExistingSession`.
///
/// Only ever returns on failure — on success this process has been replaced by
/// the tmux client.
#[cfg(unix)]
pub fn restart_in_session(
    session: &str,
    argv: &[String],
    runner: &dyn ProcessRunner,
) -> StartupAbort {
    // The session may not survive its board window: tmux discards a session
    // whose last window closes. `RecreateSessionRetiredWithItsLastWindow` —
    // there is then nothing to attach to and the cold path applies, which
    // creates the session and runs the board in it.
    match retire_board_window(session, runner) {
        RetireOutcome::SessionReady => {}
        RetireOutcome::SessionDiscarded => return enter_session(session, argv).into(),
        RetireOutcome::BoardWindowSurvived => return StartupAbort::PreviousBoardNotRetired,
    }
    let command: Vec<&str> = argv.iter().map(String::as_str).collect();
    if let Err(e) =
        tmux::new_window_in_session_running(session, &BOARD_WINDOW_NAME, &command, runner)
    {
        tracing::error!("could not start the board in session '{session}': {e}");
        return StartupAbort::LaunchRejected;
    }
    exec_tmux(&attach_argv(session)).into()
}

/// Which failure an exec error reports. Split out so the mapping is testable
/// without actually failing to exec.
pub fn classify_launch_error(kind: std::io::ErrorKind) -> SessionLaunchFailure {
    match kind {
        std::io::ErrorKind::NotFound => SessionLaunchFailure::TmuxUnavailable,
        _ => SessionLaunchFailure::LaunchRejected,
    }
}

/// Whether this process is already inside a tmux session.
/// `SessionLauncher.inside_tmux_session`.
///
/// The one definition, shared by the launch handoff in `src/main.rs` and by
/// `runtime::run_tui`'s guard. An empty `TMUX` counts as outside: a shell
/// spells "unset" both by omitting the variable and by setting it to nothing,
/// and only one of those spellings counting would put the two readers on
/// different sides of the same environment (the same reasoning as
/// `setup::home_dir_from_value`).
pub fn inside_tmux_session() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}
