//! What happens between the operator typing `dispatch tui` and the board
//! drawing its first frame: obtaining the tmux session the board requires, and
//! resolving any configuration drift before the screen is taken over.
//!
//! See `docs/specs/startup.allium`. Both halves live here because they are one
//! ordered sequence with one shared property — the board draws afterwards
//! either way — and splitting them would leave no single place that states the
//! order.

use std::path::Path;

use crate::models::TmuxWindow;
use crate::process::{ProcessRunner, RealProcessRunner};
use crate::setup::{
    apply_config_update_in, inspect_config_drift_in, ConfigArtefact, ConfigContext, ConfigDrift,
    Confirmer, SetupPaths, StdinConfirmer,
};
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

/// Every condition that stops the board before it draws.
/// `startup.allium`'s `StartupAbortReason`.
///
/// One enumeration rather than a message raised wherever each is discovered, so
/// `StartupAbortsOnlyOnAnUnusableSubstrate` has somewhere to be read off: a new
/// way to abort means a variant here, in front of the invariant that says
/// whether it belongs at startup at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupAbort {
    /// No tmux on `PATH`.
    TmuxUnavailable,
    /// tmux was reached and refused to start or attach.
    LaunchRejected,
    /// The launch came from a pane inside the board's own window.
    BoardAlreadyInThisWindow,
    /// The previous board's window would not close.
    PreviousBoardNotRetired,
    /// tmux would not say which session this process is in.
    SessionUnidentified,
    /// Another process holds the port agents reach the board on.
    AgentPortUnavailable { port: u16 },
    /// This machine has no label and nobody could be asked for one.
    /// `startup.allium`'s `AbortWhenTheHostIsUnnamedAndNoOneCanAnswer`.
    HostUnnamed,
    /// The store this machine's Host row lives in is unusable — either the
    /// row could not be read or minted at all, or the machine is unnamed and
    /// a label it was just given could not be written back. Both are the
    /// same broken substrate with the same remedy (repair the settings
    /// store), which is why one reason covers both rather than a second
    /// value alongside `HostUnnamed` (which means the identity read fine,
    /// the store would accept a label, and there simply isn't one yet).
    /// `startup.allium`'s `AbortWhenTheHostIdentityStoreIsUnusable`.
    HostIdentityUnavailable,
}

impl StartupAbort {
    /// The operator-facing message. Each names the next action, because that is
    /// what differs between them — install tmux, look at the server, move
    /// window, close a board.
    pub fn message(self) -> String {
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
                 reach or write its settings. Check that the database and its directory are \
                 reachable and writable, then run `dispatch tui` again; there is no name to \
                 type here, the problem is lower down than that.",
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

/// The three states retiring the board's window can leave a session in.
/// `startup.allium`'s `RetireOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetireOutcome {
    /// The session is there and holds no board window: start the fresh one.
    SessionReady,
    /// The board's window was the session's last, so tmux dropped the session
    /// with it. There is nothing to attach to; the create path applies.
    SessionDiscarded,
    /// The window is still there. Starting a board now would make two, so the
    /// launch stops — `startup.allium`'s `AbortWhenThePreviousBoardWillNotClose`.
    BoardWindowSurvived,
}

/// Close the board's window in `session`, whatever it currently holds.
/// `startup.allium`'s `RetireTheBoardWindow`.
///
/// The outcome distinguishes the three states the session can be left in, each
/// of which the launch path follows differently — see [`RetireOutcome`].
///
/// A session with no board window is already in the state this aims at
/// (`RetiringAnAbsentBoardWindowSucceeds`) and reports `SessionReady`.
pub fn retire_board_window(session: &str, runner: &dyn ProcessRunner) -> RetireOutcome {
    // An empty name would reach tmux as the bare target `=`, which is not a
    // session anybody named. Nothing is asked and the launch is stopped rather
    // than guessing which session was meant.
    // A backstop, not the reporting path: `plan_launch` refuses an unnamed
    // session with `StartupAbort::SessionUnidentified`, whose message says what
    // is actually wrong. This is here so a future caller that skips the planner
    // cannot send tmux the bare target `=`, which is not a session anybody
    // named.
    if session.is_empty() {
        tracing::warn!("cannot retire a board window without a session name");
        return RetireOutcome::BoardWindowSurvived;
    }
    let pane = match tmux::pane_id_of_window_in_session(session, &BOARD_WINDOW_NAME, runner) {
        // Nothing to close, and this lookup just answered the question the
        // read-back would ask again — so only the session's own existence is
        // still open.
        Ok(None) => {
            return if tmux::session_exists(session, runner) {
                RetireOutcome::SessionReady
            } else {
                RetireOutcome::SessionDiscarded
            }
        }
        Ok(Some(pane)) => {
            if let Err(e) = tmux::kill_window_at(&pane, runner) {
                tracing::warn!("could not retire the board's window in '{session}': {e}");
            }
            Some(pane)
        }
        Err(e) => {
            tracing::warn!("could not look for a board window in '{session}': {e}");
            None
        }
    };
    session_state_after_retire(session, pane.as_deref(), runner)
}

/// How long to wait for a retired board's pane to actually disappear.
///
/// `kill-window` removes the window and signals its process; that process's own
/// exit — and with it the release of the agent port — happens afterwards. A
/// launch that raced the exit reached the port claim first and told the operator
/// another board was holding the port, moments after they had closed it.
const RETIRED_PANE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// Poll step for [`RETIRED_PANE_DEADLINE`]. Short enough that the common case —
/// a board that exits at once — costs one step rather than the whole budget.
const RETIRED_PANE_POLL_STEP: std::time::Duration = std::time::Duration::from_millis(25);

/// Which [`RetireOutcome`] the session is in now.
///
/// Read back from tmux rather than inferred from whether the close reported
/// success — `AFailedRetireIsNotMistakenForSuccess`. What the caller needs is
/// the state the session is in, and a kill that returned an error may still
/// have taken the window with it.
fn session_state_after_retire(
    session: &str,
    killed_pane: Option<&str>,
    runner: &dyn ProcessRunner,
) -> RetireOutcome {
    if let Some(pane) = killed_pane {
        await_pane_gone(pane, runner);
    }
    if !tmux::session_exists(session, runner) {
        return RetireOutcome::SessionDiscarded;
    }
    match tmux::pane_id_of_window_in_session(session, &BOARD_WINDOW_NAME, runner) {
        Ok(None) => RetireOutcome::SessionReady,
        // A window still there, or a lookup that cannot say otherwise. Both
        // stop the launch: starting a board on either reading risks a second
        // one beside a live board.
        _ => RetireOutcome::BoardWindowSurvived,
    }
}

/// Wait, briefly, for `pane` to leave tmux's listing.
///
/// Makes `SessionReady` mean "retired" rather than "asked to retire", so the
/// replacement board does not race the old one's hold on the agent port.
/// Bounded and best-effort: a pane still there at the deadline is left to the
/// window read-back above, which reports `BoardWindowSurvived` and stops the
/// launch with a message about the board rather than about the port.
fn await_pane_gone(pane: &str, runner: &dyn ProcessRunner) {
    let deadline = std::time::Instant::now() + RETIRED_PANE_DEADLINE;
    while tmux::pane_exists(pane, runner) {
        if std::time::Instant::now() >= deadline {
            tracing::warn!("pane {pane} still present after being retired");
            return;
        }
        std::thread::sleep(RETIRED_PANE_POLL_STEP);
    }
}

/// Retire any board in `session` so this process can draw in its own window.
/// `startup.allium`'s `LaunchBoardInsideExistingSession`.
///
/// The inside-tmux counterpart of [`restart_in_session`]: this process already
/// has a window, so there is nothing to enter and nothing to attach to. Only
/// one retire outcome stops it — a board window that would not close, which
/// would leave two boards in one session.
pub fn retire_before_drawing(
    session: &str,
    runner: &dyn ProcessRunner,
) -> Result<(), StartupAbort> {
    match retire_board_window(session, runner) {
        // `SessionDiscarded` cannot arise here: a session whose only window was
        // the board's has no other window for this process to be running in.
        // Grouped with the ready case rather than argued about, so this stays
        // correct if that ever changes.
        RetireOutcome::SessionReady | RetireOutcome::SessionDiscarded => Ok(()),
        RetireOutcome::BoardWindowSurvived => Err(StartupAbort::PreviousBoardNotRetired),
    }
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

// ---------------------------------------------------------------------------
// The configuration check — startup.allium's CheckStartupConfig rules
// ---------------------------------------------------------------------------

/// How the startup configuration check ended.
/// `startup.allium`'s `StartupConfigOutcome`.
///
/// Every value is non-fatal: the board draws after all four
/// (`ConfigurationDriftNeverBlocksTheBoard`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupConfigOutcome {
    /// Nothing was out of date; nothing was said.
    AlreadyCurrent,
    /// The operator agreed and the artefacts were written.
    Updated,
    /// The operator was asked and said no.
    Declined,
    /// No one could be asked, so nothing was written.
    ReportedOnly,
}

/// The line shown to the operator when configuration is stale, whether they
/// are about to be asked or merely told.
///
/// Names every stale artefact: `ConfigUpdatePrompt`'s `NamesWhatWillChange`
/// forbids asking for approval of an unnamed set of writes.
fn describe_drift(drift: &ConfigDrift) -> String {
    format!(
        "Dispatch configuration is out of date: {}.",
        list_artefacts(&drift.items)
    )
}

/// The artefact labels as the operator reads them. One formatting of the list,
/// used by the drift line and by the partial-failure warning, so the two cannot
/// punctuate the same set differently.
fn list_artefacts(items: &[ConfigArtefact]) -> String {
    items
        .iter()
        .map(|a| a.label())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve configuration drift on the way to the board, against the real
/// `$HOME`-derived locations.
///
/// Synchronous and blocking: it reads a dozen files, walks the installed plugin
/// tree, spawns a tmux subprocess, and may block on stdin waiting for an
/// answer. Callers on an async runtime must run it on a blocking thread.
pub fn resolve_startup_config(
    paths: &SetupPaths,
    data_dir: &std::path::Path,
    port: u16,
    interactive: bool,
) -> StartupConfigOutcome {
    let confirmer = StdinConfirmer;
    resolve_startup_config_in(
        paths,
        data_dir,
        port,
        &RealProcessRunner::default(),
        interactive.then_some(&confirmer as &dyn Confirmer),
    )
}

/// Injectable core of [`resolve_startup_config`]. The three-way branch is
/// `startup.allium`'s `CheckStartupConfigWhenNothingIsStale` /
/// `PromptToUpdateStaleConfig` / `ReportStaleConfigWhenNoOneCanAnswer`.
///
/// Returns an outcome rather than a `Result` on purpose. Every internal failure
/// is folded into the report it already prints, so there is no `Err` for a
/// caller to propagate and therefore no way to make configuration drift fatal —
/// `ConfigurationDriftNeverBlocksTheBoard` holds by type rather than by every
/// call site remembering to swallow the error.
///
/// `confirmer` is `None` when nobody can answer — a script, a CI job, stdin
/// redirected from nowhere. The absence is the input, not a flag beside one:
/// there is then no code path that could read a queued "yes" out of silence.
pub(crate) fn resolve_startup_config_in(
    paths: &SetupPaths,
    data_dir: &std::path::Path,
    port: u16,
    runner: &dyn ProcessRunner,
    confirmer: Option<&dyn Confirmer>,
) -> StartupConfigOutcome {
    let ctx = ConfigContext {
        paths,
        port,
        runner,
        data_dir,
        confirmer,
    };
    let drift = inspect_config_drift_in(&ctx);

    if drift.is_clean() {
        // Silent on purpose. A line reporting four current artefacts on every
        // launch trains the operator to skip the region of the screen where the
        // one line that matters appears.
        return StartupConfigOutcome::AlreadyCurrent;
    }

    let Some(confirmer) = confirmer else {
        // Treating silence as consent is how a scripted run rewrites the home
        // directory of whoever happened to start it.
        eprintln!(
            "{} Nothing was written — run `dispatch tui` from a terminal to update it.",
            describe_drift(&drift)
        );
        return StartupConfigOutcome::ReportedOnly;
    };

    // One prompt for the whole report: the artefacts are named in it, but they
    // are not separable decisions — a half-updated installation is a
    // configuration nobody chose and nothing tests.
    eprintln!("{}", describe_drift(&drift));
    match confirmer.confirm("Update it now?") {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "Configuration left as it is. The board will start; \
                 you will be asked again next launch."
            );
            return StartupConfigOutcome::Declined;
        }
        Err(e) => {
            // An unreadable stdin is an operator who could not answer, not one
            // who said yes.
            eprintln!("Warning: could not read an answer ({e:#}). Nothing was written.");
            return StartupConfigOutcome::ReportedOnly;
        }
    }

    let failed = apply_config_update_in(&drift, &ctx);
    if !failed.is_empty() {
        eprintln!(
            "Warning: could not update {}. The board will start; \
             this is retried next launch.",
            list_artefacts(&failed)
        );
    }
    StartupConfigOutcome::Updated
}

/// The host-label gate: `startup.allium`'s `CheckHostLabel` and its
/// label-side rules (`ContinueWhenTheHostIsAlreadyNamed`,
/// `PromptForHostLabelWhenUnnamed`, `NameHostFromStartupPrompt`,
/// `AbortWhenTheHostIsUnnamedAndNoOneCanAnswer`). `CheckHostLabel`'s fourth
/// arm (`AbortWhenTheHostIdentityStoreIsUnusable`) fires earlier — in
/// `TuiRuntime::bootstrap`, before this function is ever reached — so it has
/// no counterpart here. The broken-settings-store condition that rule names
/// has a second face (a persist that does not take), but that face is not
/// this rule either: it fires *after* this function returns `Ok(Some(label))`,
/// raised directly by `startup.allium`'s `NameHostFromStartupPrompt` in the
/// caller's persist step — see `persist_host_label` in `src/runtime/mod.rs`.
///
/// Pure given its inputs: reading the current label from the database and
/// persisting a newly accepted one via `host.allium: RenameHost` are the
/// caller's job — this function only decides, so it can be tested without a
/// database. `current_label` is the host's label as read from settings right
/// now; `None` means unnamed. Returns:
///
/// - `Ok(None)` — the host is already named; nothing to persist
///   (`ContinueWhenTheHostIsAlreadyNamed`).
/// - `Ok(Some(label))` — the operator answered the prompt; the caller must
///   persist `label` via `RenameHost` before drawing
///   (`PromptForHostLabelWhenUnnamed` / `NameHostFromStartupPrompt`), and
///   abort through `AbortWhenTheHostIdentityStoreIsUnusable`'s reason if that
///   write fails rather than treat `Ok` as "resolved".
/// - `Err(StartupAbort::HostUnnamed)` — nobody could be asked, or the one
///   asked could not answer (`AbortWhenTheHostIsUnnamedAndNoOneCanAnswer`).
///
/// `confirmer` is `None` on exactly the same non-interactive condition
/// `resolve_startup_config_in` uses for `operator_can_answer` — a script, a
/// CI job, stdin redirected from nowhere. Unlike that function, an absent or
/// failing confirmer here is fatal
/// (`TheBoardNeverDrawsForAnUnnamedHost`): there is no `reported_only`
/// counterpart for a host with no name.
///
/// The prompt is retried on a blank answer rather than giving up
/// (`HostLabelPrompt`'s `ThereIsNoWayPast`): in practice this never loops
/// against the real `StdinConfirmer`, whose `prompt_text` already substitutes
/// the (always non-empty) hostname default for empty input, but a confirmer
/// that returns blank text directly must still be re-asked rather than
/// treated as a way past the gate.
pub(crate) fn resolve_host_label(
    current_label: Option<&str>,
    hostname: &str,
    confirmer: Option<&dyn Confirmer>,
) -> Result<Option<String>, StartupAbort> {
    if current_label.is_some() {
        return Ok(None);
    }

    let Some(confirmer) = confirmer else {
        return Err(StartupAbort::HostUnnamed);
    };

    eprintln!(
        "This machine has not been named yet. The name is shown on shared boards \
         so teammates can tell whose worktree a task belongs to."
    );

    loop {
        let answer = confirmer
            .prompt_text("Name for this machine", hostname)
            .map_err(|_| StartupAbort::HostUnnamed)?;
        let trimmed = answer.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
        eprintln!("A name is required — this machine cannot stay unnamed.");
    }
}

/// Real, blocking entry point for the host-label gate: constructs the
/// stdin-backed prompter and this machine's hostname, then delegates to
/// [`resolve_host_label`]. Mirrors [`resolve_startup_config`]'s split from
/// [`resolve_startup_config_in`] — callers on an async runtime must run this
/// on a blocking thread, since it may block on stdin waiting for an answer.
pub fn resolve_host_label_interactively(
    current_label: Option<String>,
    interactive: bool,
) -> Result<Option<String>, StartupAbort> {
    let confirmer = StdinConfirmer;
    resolve_host_label(
        current_label.as_deref(),
        &machine_hostname(),
        interactive.then_some(&confirmer as &dyn Confirmer),
    )
}

/// Best-effort machine hostname, used only as the host-label prompt's
/// pre-filled default (never the Host's id — see host.allium:
/// MintHostIdentity's guidance on why the id is generated, not derived).
/// Reads `/proc/sys/kernel/hostname` directly rather than shelling out to
/// `hostname(1)`: cheaper, and every target this binary ships for is Linux
/// (see CLAUDE.md: "POSIX-only"). Falls back to the `HOSTNAME` environment
/// variable, then to a fixed placeholder — this must never fail startup, and
/// must never be empty, since an empty default would make the prompt's
/// accept-with-enter path indistinguishable from a blank answer.
pub(crate) fn machine_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown-host".to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::process::MockProcessRunner;
    use crate::setup::FakeConfirmer;

    /// A `Confirmer` whose every method fails, standing in for a launch with
    /// no one to answer it. Shared by the config-drift and host-label gates —
    /// both ask the same question of the same trait, so a second copy would
    /// only be one more place to add the next `Confirmer` method.
    struct FailingConfirmer;
    impl Confirmer for FailingConfirmer {
        fn confirm(&self, _: &str) -> anyhow::Result<bool> {
            Err(anyhow::anyhow!("stdin closed"))
        }
        fn confirm_dangerous(&self, _: &str) -> anyhow::Result<bool> {
            Err(anyhow::anyhow!("stdin closed"))
        }
        fn prompt_text(&self, _: &str, _: &str) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("stdin closed"))
        }
    }

    // -- Launch planning (LaunchBoardInsideExistingSession /
    //    LaunchBoardBySupplyingASession) --

    /// Outside tmux, with no session of dispatch's own yet.
    fn cold() -> LaunchContext {
        LaunchContext::Outside {
            session_exists: false,
        }
    }

    /// Inside `session`, in a window playing `role`.
    fn inside(session: &str, role: BoardWindowRole) -> LaunchContext {
        LaunchContext::Inside {
            session: session.to_string(),
            role,
        }
    }

    fn argv() -> Vec<String> {
        vec!["/bin/dispatch".to_string(), "tui".to_string()]
    }

    #[test]
    fn plan_launch_inside_tmux_continues_in_this_process() {
        let plan = plan_launch(inside("work", BoardWindowRole::NotTheBoard), argv());
        assert_eq!(
            plan,
            LaunchPlan::ContinueHere {
                session: "work".to_string()
            },
            "an operator who already had a session keeps it"
        );
    }

    #[test]
    fn plan_launch_does_not_refuse_the_board_its_own_launch() {
        // The regression this pins: both entry paths create the board's window
        // already carrying BOARD_WINDOW_NAME, so the board's own process runs
        // this path from inside a window named TUI. A predicate that read only
        // the window's name would have every board refuse itself, and the
        // operator would attach to a session whose board had just exited with
        // "This window is already the dispatch board."
        let plan = plan_launch(inside(SESSION_NAME, BoardWindowRole::NotTheBoard), argv());
        assert_eq!(
            plan,
            LaunchPlan::ContinueHere {
                session: SESSION_NAME.to_string()
            }
        );
    }

    #[test]
    fn plan_launch_draws_without_retiring_when_this_process_is_the_started_board() {
        // The bug this pins was only visible against a real tmux: on the
        // inside-tmux path the board retired the window named TUI in its own
        // session — which is the window it is itself running in. It closed
        // itself before drawing, and where that window was the session's only
        // one, tmux discarded the session and the whole server went with it.
        let plan = plan_launch(inside(SESSION_NAME, BoardWindowRole::StartedBoard), argv());
        assert_eq!(
            plan,
            LaunchPlan::DrawInThisWindow,
            "ALaunchNeverRetiresItsOwnWindow"
        );
    }

    #[test]
    fn plan_launch_refuses_a_session_tmux_will_not_name() {
        let plan = plan_launch(inside("", BoardWindowRole::NotTheBoard), argv());
        assert_eq!(
            plan,
            LaunchPlan::Refuse(StartupAbort::SessionUnidentified),
            "with no session name there is nothing to scope a retire to, and an \
             unscoped one closes a window in a session dispatch may not own"
        );
    }

    #[test]
    fn board_window_role_reads_one_name_two_ways() {
        // The board holds its window alone, so a second pane in it is somebody
        // else. The name alone cannot tell them apart: both entry paths create
        // the board's window already carrying it, so the board's own process
        // arrives here too — and a name-only reading would have every board
        // refuse, or worse retire, its own launch.
        assert_eq!(
            BoardWindowRole::of("TUI", 1),
            BoardWindowRole::StartedBoard,
            "one pane in the board's window is the board itself, mid-launch"
        );
        assert_eq!(
            BoardWindowRole::of("TUI", 2),
            BoardWindowRole::SharedWithBoard,
            "a second pane is the operator's shell, or the split-pane agent"
        );
        assert_eq!(
            BoardWindowRole::of("task-42", 2),
            BoardWindowRole::NotTheBoard,
            "a window that is not the board's is neither"
        );
    }

    #[test]
    fn plan_launch_from_inside_the_board_window_refuses() {
        let plan = plan_launch(
            inside(SESSION_NAME, BoardWindowRole::SharedWithBoard),
            argv(),
        );
        assert_eq!(
            plan,
            LaunchPlan::Refuse(StartupAbort::BoardAlreadyInThisWindow),
            "retiring the board's window from a shell inside it would close that shell"
        );
    }

    #[test]
    fn plan_launch_outside_tmux_enters_the_dispatch_session() {
        let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
        let plan = plan_launch(cold(), argv.clone());
        assert_eq!(
            plan,
            LaunchPlan::EnterSession {
                session: SESSION_NAME.to_string(),
                argv,
            },
            "the command supplies its own session rather than reporting it has none"
        );
    }

    #[test]
    fn plan_launch_outside_tmux_restarts_the_board_in_an_existing_session() {
        let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
        let plan = plan_launch(
            LaunchContext::Outside {
                session_exists: true,
            },
            argv.clone(),
        );
        assert_eq!(
            plan,
            LaunchPlan::RestartInSession {
                session: SESSION_NAME.to_string(),
                argv,
            },
            "EveryLaunchProducesAFreshBoard: a second launch restarts, never reattaches"
        );
    }

    #[test]
    fn plan_launch_restarts_without_asking_whether_the_old_board_is_alive() {
        // RetiringIsNotConditionalOnLiveness. The context the planner is given
        // carries no liveness signal at all, which is the point: there is no
        // input here that could make a running board take a different path
        // from an exited one.
        let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
        match plan_launch(
            LaunchContext::Outside {
                session_exists: true,
            },
            argv.clone(),
        ) {
            LaunchPlan::RestartInSession { argv: carried, .. } => assert_eq!(carried, argv),
            other => panic!("expected RestartInSession, got {other:?}"),
        }
    }

    #[test]
    fn plan_launch_carries_the_operators_arguments_through() {
        let argv = vec![
            "/bin/dispatch".to_string(),
            "--db".to_string(),
            "/tmp/scratch.db".to_string(),
            "tui".to_string(),
            "--port".to_string(),
            "9999".to_string(),
        ];
        match plan_launch(cold(), argv.clone()) {
            LaunchPlan::EnterSession { argv: carried, .. } => assert_eq!(
                carried, argv,
                "ArgvIsCarriedThrough: the board must come up with the db and port asked for"
            ),
            other => panic!("expected EnterSession, got {other:?}"),
        }
    }

    // -- The tmux command line (NeverCreatesASecondSession) --

    #[test]
    fn session_argv_creates_or_attaches_in_one_operation() {
        let argv = session_argv(
            "dispatch",
            &["/bin/dispatch".to_string(), "tui".to_string()],
        );
        assert_eq!(
            argv,
            vec![
                "tmux",
                "new-session",
                "-A",
                "-s",
                "dispatch",
                "-n",
                "TUI",
                "--",
                "/bin/dispatch",
                "tui"
            ],
            "-A makes create-or-attach indivisible, so no second board can slip in"
        );
    }

    #[test]
    fn session_argv_names_the_boards_window_up_front() {
        let argv = session_argv("dispatch", &["/bin/dispatch".to_string()]);
        let n = argv.iter().position(|a| a == "-n").expect("-n is passed");
        assert_eq!(
            argv[n + 1],
            BOARD_WINDOW_NAME.as_str(),
            "a board that dies before doing anything must still leave a window \
             the next launch can find and retire"
        );
    }

    #[test]
    fn session_argv_separates_the_inner_command_from_tmux_flags() {
        let argv = session_argv(
            "dispatch",
            &["/bin/dispatch".to_string(), "--db".to_string()],
        );
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert!(
            argv[sep + 1..].iter().all(|a| a != "-s"),
            "everything after -- belongs to the inner command"
        );
        assert_eq!(argv[sep + 1], "/bin/dispatch");
    }

    // -- Retiring the previous board (RetireTheBoardWindow) --

    #[test]
    fn retire_board_window_kills_the_board_window_and_nothing_else() {
        let mock = MockProcessRunner::new(vec![
            // list-panes -s: the session holds the board plus two agents.
            MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n1 %1 task-42\n1 %2 task-43\n"),
            MockProcessRunner::ok(), // kill-window
            // pane_exists: %0 is gone the moment it is asked about.
            MockProcessRunner::ok_with_stdout(b"%1\n%2\n"),
            MockProcessRunner::ok(), // has-session
            // list-panes -s, read back: the board's window is gone, the
            // agents' are not.
            MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n1 %2 task-43\n"),
        ])
        .with_queued_window_lookup();

        let outcome = retire_board_window("dispatch", &mock);

        assert_eq!(
            outcome,
            RetireOutcome::SessionReady,
            "the agent windows keep the session alive"
        );
        let kills: Vec<_> = mock
            .recorded_calls()
            .into_iter()
            .filter(|(_, args)| args.first().is_some_and(|a| a == "kill-window"))
            .collect();
        assert_eq!(kills.len(), 1, "exactly one window is retired");
        assert_eq!(
            kills[0].1,
            vec!["kill-window", "-t", "%0"],
            "RestartingCostsOnlyTheBoard: the agents' windows are not targets"
        );
    }

    #[test]
    fn retire_board_window_does_nothing_when_there_is_no_board_window() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n"), // list-panes -s
            MockProcessRunner::ok(),                              // has-session
        ])
        .with_queued_window_lookup();

        let outcome = retire_board_window("dispatch", &mock);

        assert_eq!(outcome, RetireOutcome::SessionReady);
        assert!(
            !mock
                .recorded_calls()
                .iter()
                .any(|(_, args)| args.first().is_some_and(|a| a == "kill-window")),
            "RetiringAnAbsentBoardWindowSucceeds: nothing to close is not an error"
        );
        assert_eq!(
            mock.recorded_calls().len(),
            2,
            "the lookup already answered what a read-back would ask again"
        );
    }

    #[test]
    fn retire_board_window_waits_for_the_retired_pane_to_go() {
        // `kill-window` removes the window and signals the board; the board's
        // own exit — and the release of the agent port with it — happens
        // afterwards. A launch that raced it told the operator another board
        // was holding the port, moments after they had closed it.
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n1 %1 task-42\n"), // list-panes -s
            MockProcessRunner::ok(),                                        // kill-window
            MockProcessRunner::ok_with_stdout(b"%0\n%1\n"), // pane_exists: still there
            MockProcessRunner::ok_with_stdout(b"%1\n"),     // pane_exists: gone
            MockProcessRunner::ok(),                        // has-session
            MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n"), // read back
        ])
        .with_queued_window_lookup();

        assert_eq!(
            retire_board_window("dispatch", &mock),
            RetireOutcome::SessionReady,
            "SessionReady must mean retired, not merely asked to retire"
        );
        assert_eq!(
            mock.recorded_calls()
                .iter()
                .filter(|(_, args)| args.contains(&"-a".to_string()))
                .count(),
            2,
            "the pane is re-checked until it is gone"
        );
    }

    #[test]
    fn retire_board_window_reports_the_session_gone_when_the_board_was_its_last_window() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n"), // list-panes -s
            MockProcessRunner::ok(),                          // kill-window
            MockProcessRunner::ok_with_stdout(b""),           // pane_exists: gone
            MockProcessRunner::fail("no such session"),       // has-session
        ])
        .with_queued_window_lookup();

        assert_eq!(
            retire_board_window("dispatch", &mock),
            RetireOutcome::SessionDiscarded,
            "tmux discards a session with no windows, and the launch path must notice"
        );
    }

    #[test]
    fn retire_board_window_reports_a_window_that_would_not_close() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n"), // list-panes -s
            MockProcessRunner::fail("can't kill window"),     // kill-window
            MockProcessRunner::ok_with_stdout(b""),           // pane_exists: gone
            MockProcessRunner::ok(),                          // has-session
            MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n"), // list-panes -s, read back
        ])
        .with_queued_window_lookup();

        assert_eq!(
            retire_board_window("dispatch", &mock),
            RetireOutcome::BoardWindowSurvived,
            "AFailedRetireIsNotMistakenForSuccess: starting a board here would make two"
        );
    }

    #[test]
    fn retire_board_window_refuses_an_empty_session_name() {
        let mock = MockProcessRunner::new(vec![]).with_queued_window_lookup();

        assert_eq!(
            retire_board_window("", &mock),
            RetireOutcome::BoardWindowSurvived,
            "an unreadable session name must not become a bare `=` target that \
             matches whatever tmux feels like"
        );
        assert!(
            mock.recorded_calls().is_empty(),
            "nothing is asked of tmux without a session to ask about"
        );
    }

    // -- Attaching after a restart --

    #[test]
    fn attach_argv_targets_the_session_exactly() {
        assert_eq!(
            attach_argv("dispatch"),
            vec!["tmux", "attach-session", "-t", "=dispatch"],
            "a prefix match would attach to somebody else's session"
        );
    }

    #[test]
    fn current_invocation_uses_the_resolved_executable_not_argv_zero() {
        let argv = current_invocation(
            Path::new("/usr/local/bin/dispatch"),
            vec!["tui".to_string()].into_iter(),
        );
        assert_eq!(argv[0], "/usr/local/bin/dispatch");
        assert_eq!(argv[1], "tui");
    }

    // -- Launch failure classification (DistinguishesAbsentFromBroken) --

    #[test]
    fn a_missing_tmux_is_reported_as_unavailable() {
        assert_eq!(
            classify_launch_error(std::io::ErrorKind::NotFound),
            SessionLaunchFailure::TmuxUnavailable
        );
    }

    #[test]
    fn any_other_exec_failure_is_reported_as_rejected() {
        assert_eq!(
            classify_launch_error(std::io::ErrorKind::PermissionDenied),
            SessionLaunchFailure::LaunchRejected
        );
    }

    #[test]
    fn the_two_launch_failures_are_worded_apart() {
        let unavailable = SessionLaunchFailure::TmuxUnavailable.message();
        let rejected = SessionLaunchFailure::LaunchRejected.message();
        assert_ne!(unavailable, rejected);
        assert!(
            unavailable.contains("not on PATH"),
            "a missing tmux must tell the operator to install it: {unavailable}"
        );
        assert!(
            rejected.contains(SESSION_NAME),
            "a refusal must name the session it was refused for: {rejected}"
        );
    }

    // -- Drift messaging (NamesWhatWillChange) --

    #[test]
    fn describe_drift_names_every_stale_artefact() {
        let drift = ConfigDrift {
            items: vec![ConfigArtefact::Plugin, ConfigArtefact::StatusLine],
        };
        let msg = describe_drift(&drift);
        assert!(msg.contains(ConfigArtefact::Plugin.label()), "{msg}");
        assert!(msg.contains(ConfigArtefact::StatusLine.label()), "{msg}");
    }

    #[test]
    fn every_artefact_has_a_label_and_they_are_distinct() {
        let labels: Vec<&str> = ConfigArtefact::ALL.iter().map(|a| a.label()).collect();
        assert_eq!(
            labels.len(),
            ConfigArtefact::ALL.len(),
            "ConfigArtefact::ALL must list every variant"
        );
        for (i, a) in labels.iter().enumerate() {
            assert!(!a.is_empty(), "an unnamed artefact cannot be consented to");
            for b in &labels[i + 1..] {
                assert_ne!(
                    a, b,
                    "two artefacts sharing a label are one line in the prompt"
                );
            }
        }
    }

    // -- The configuration check: the four StartupConfigOutcome values --

    /// Setup paths under a temp root, so no test can reach the operator's real
    /// configuration directory.
    fn layout(root: &Path) -> SetupPaths {
        let claude_dir = root.join(".claude");
        SetupPaths {
            claude_dir: claude_dir.clone(),
            mcp_path: root.join(".claude.json"),
            legacy_mcp_path: claude_dir.join(".mcp.json"),
            tmux_conf_path: root.join(".tmux.conf"),
            statusline_path: crate::setup::statusline::settings_path(&claude_dir),
            budget_snapshot_path: root.join("data").join("rate-limits.json"),
        }
    }

    /// Where the fixtures put `<data_dir>/scripts/`. Beside the configuration
    /// directory rather than inside it, mirroring the real layout.
    fn data_dir(root: &Path) -> std::path::PathBuf {
        root.join("data")
    }

    /// A tmux runner reporting focus-events already on, so a test that is not
    /// about tmux needs to queue only the one process result.
    fn focus_events_on() -> MockProcessRunner {
        MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")])
    }

    /// Bring every artefact current, so a test can assert on a clean check.
    fn make_current(paths: &SetupPaths, data_dir: &Path, port: u16) {
        let runner = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"off\n"),
            MockProcessRunner::ok(),
        ]);
        let confirmer = FakeConfirmer::never();
        let ctx = ConfigContext {
            paths,
            port,
            runner: &runner,
            data_dir,
            confirmer: Some(&confirmer),
        };
        let drift = crate::setup::inspect_config_drift_in(&ctx);
        let failed = crate::setup::apply_config_update_in(&drift, &ctx);
        assert!(failed.is_empty(), "fixture setup must not fail: {failed:?}");
    }

    #[test]
    fn a_current_installation_resolves_silently() {
        let root = tempfile::tempdir().unwrap();
        let paths = layout(root.path());
        make_current(&paths, &data_dir(root.path()), 3142);

        // never(): AbsentWhenNothingIsStale — no prompt may fire.
        let confirmer = FakeConfirmer::never();
        let outcome = resolve_startup_config_in(
            &paths,
            &data_dir(root.path()),
            3142,
            &focus_events_on(),
            Some(&confirmer),
        );

        assert_eq!(outcome, StartupConfigOutcome::AlreadyCurrent);
        assert_eq!(confirmer.confirm_call_count(), 0);
    }

    #[test]
    fn stale_configuration_is_written_once_the_operator_agrees() {
        let root = tempfile::tempdir().unwrap();
        let paths = layout(root.path());
        let runner = MockProcessRunner::new(vec![
            // The inspect's read, then the apply's read and set-option.
            MockProcessRunner::ok_with_stdout(b"off\n"),
            MockProcessRunner::ok_with_stdout(b"off\n"),
            MockProcessRunner::ok(),
        ]);
        let confirmer = FakeConfirmer::new(vec![true], vec![]);

        let outcome = resolve_startup_config_in(
            &paths,
            &data_dir(root.path()),
            3142,
            &runner,
            Some(&confirmer),
        );

        assert_eq!(outcome, StartupConfigOutcome::Updated);
        assert_eq!(
            confirmer.confirm_call_count(),
            1,
            "one prompt for the whole report, not one per artefact"
        );
        assert!(
            paths.mcp_path.exists(),
            "consent is the one path that writes"
        );
    }

    #[test]
    fn declining_writes_nothing_and_still_resolves() {
        let root = tempfile::tempdir().unwrap();
        let paths = layout(root.path());
        let confirmer = FakeConfirmer::new(vec![false], vec![]);

        let outcome = resolve_startup_config_in(
            &paths,
            &data_dir(root.path()),
            3142,
            &focus_events_on(),
            Some(&confirmer),
        );

        assert_eq!(outcome, StartupConfigOutcome::Declined);
        assert!(
            !paths.mcp_path.exists(),
            "ConfigurationIsNeverWrittenWithoutConsent: a declined prompt writes nothing"
        );
        assert!(!paths.claude_dir.exists(), "not even the directory");
    }

    #[test]
    fn a_non_interactive_launch_reports_without_writing() {
        let root = tempfile::tempdir().unwrap();
        let paths = layout(root.path());

        // No confirmer at all: nobody can answer.
        let outcome = resolve_startup_config_in(
            &paths,
            &data_dir(root.path()),
            3142,
            &focus_events_on(),
            None,
        );

        assert_eq!(outcome, StartupConfigOutcome::ReportedOnly);
        assert!(
            !paths.mcp_path.exists(),
            "treating silence as consent is how a script rewrites someone else's home directory"
        );
        assert!(!paths.claude_dir.exists());
    }

    #[test]
    fn a_non_interactive_launch_with_nothing_stale_is_already_current() {
        let root = tempfile::tempdir().unwrap();
        let paths = layout(root.path());
        make_current(&paths, &data_dir(root.path()), 3142);

        let outcome = resolve_startup_config_in(
            &paths,
            &data_dir(root.path()),
            3142,
            &focus_events_on(),
            None,
        );

        assert_eq!(
            outcome,
            StartupConfigOutcome::AlreadyCurrent,
            "SilentWhenClean holds whether or not anyone could be asked"
        );
    }

    /// An unreadable stdin is an operator who could not answer, not one who
    /// said yes. Without this the `Err` arm could plausibly be written as
    /// "proceed", which would write configuration nobody approved.
    #[test]
    fn an_unanswerable_prompt_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let paths = layout(root.path());

        let outcome = resolve_startup_config_in(
            &paths,
            &data_dir(root.path()),
            3142,
            &focus_events_on(),
            Some(&FailingConfirmer),
        );

        assert_eq!(outcome, StartupConfigOutcome::ReportedOnly);
        assert!(!paths.claude_dir.exists());
    }

    // -- The host-label gate (CheckHostLabel and its label-side children;
    //    see the identity arm's own tests in `mod bootstrap` in
    //    src/runtime/tests/misc.rs) --

    #[test]
    fn an_already_named_host_is_not_prompted() {
        let confirmer = FakeConfirmer::never();

        let outcome = resolve_host_label(Some("my-laptop"), "fallback-hostname", Some(&confirmer));

        assert_eq!(
            outcome,
            Ok(None),
            "ContinueWhenTheHostIsAlreadyNamed: nothing to persist when already named"
        );
        assert_eq!(confirmer.text_call_count(), 0);
    }

    #[test]
    fn an_already_named_host_is_not_prompted_even_non_interactively() {
        // A named host must never abort, whether or not anyone could answer —
        // TheBoardNeverDrawsForAnUnnamedHost only ever blocks an UNNAMED host.
        let outcome = resolve_host_label(Some("my-laptop"), "fallback-hostname", None);

        assert_eq!(outcome, Ok(None));
    }

    #[test]
    fn an_unnamed_host_is_prompted_with_the_hostname_prefilled() {
        let confirmer = FakeConfirmer::with_text(vec![], vec![], vec!["some-laptop".to_string()]);

        let outcome = resolve_host_label(None, "some-laptop", Some(&confirmer));

        assert_eq!(
            outcome,
            Ok(Some("some-laptop".to_string())),
            "PromptForHostLabelWhenUnnamed: the operator's answer is handed back to persist"
        );
        assert_eq!(confirmer.text_call_count(), 1);
    }

    #[test]
    fn accepting_the_prefilled_hostname_names_the_host() {
        // StdinConfirmer::prompt_text substitutes the default for empty input,
        // so a confirmer honouring that contract returns the hostname back —
        // exercised here via a fake standing in for "operator pressed enter".
        let confirmer =
            FakeConfirmer::with_text(vec![], vec![], vec!["my-machine-hostname".to_string()]);

        let outcome = resolve_host_label(None, "my-machine-hostname", Some(&confirmer));

        assert_eq!(outcome, Ok(Some("my-machine-hostname".to_string())));
    }

    #[test]
    fn a_blank_answer_is_asked_again_rather_than_accepted() {
        // ThereIsNoWayPast: a confirmer that hands back whitespace is asked
        // again rather than being treated as a way past the gate.
        let confirmer = FakeConfirmer::with_text(
            vec![],
            vec![],
            vec!["   ".to_string(), "real-name".to_string()],
        );

        let outcome = resolve_host_label(None, "fallback-hostname", Some(&confirmer));

        assert_eq!(outcome, Ok(Some("real-name".to_string())));
        assert_eq!(
            confirmer.text_call_count(),
            2,
            "a blank answer must not resolve the gate on the first ask"
        );
    }

    #[test]
    fn a_non_interactive_launch_aborts_on_an_unnamed_host() {
        // AbortWhenTheHostIsUnnamedAndNoOneCanAnswer: no confirmer at all —
        // a script, a CI job, stdin redirected from nowhere.
        let outcome = resolve_host_label(None, "fallback-hostname", None);

        assert_eq!(outcome, Err(StartupAbort::HostUnnamed));
    }

    #[test]
    fn an_unanswerable_host_label_prompt_aborts_rather_than_proceeding_unnamed() {
        // Unlike the configuration prompt's unreadable-stdin case (which
        // degrades to ReportedOnly), a host that cannot be named must abort —
        // TheBoardNeverDrawsForAnUnnamedHost has no non-fatal counterpart.
        let outcome = resolve_host_label(None, "fallback-hostname", Some(&FailingConfirmer));

        assert_eq!(outcome, Err(StartupAbort::HostUnnamed));
    }

    #[test]
    fn host_unnamed_is_worded_distinctly_and_names_the_remedy() {
        let msg = StartupAbort::HostUnnamed.message();
        assert!(
            msg.contains("dispatch tui"),
            "the remedy — one interactive launch — must be named: {msg}"
        );
        for other in [
            StartupAbort::TmuxUnavailable,
            StartupAbort::LaunchRejected,
            StartupAbort::BoardAlreadyInThisWindow,
            StartupAbort::PreviousBoardNotRetired,
            StartupAbort::SessionUnidentified,
            StartupAbort::AgentPortUnavailable { port: 1234 },
            StartupAbort::HostIdentityUnavailable,
        ] {
            assert_ne!(msg, other.message());
        }
    }

    /// startup.allium: `AbortWhenTheHostIdentityStoreIsUnusable`. This is a
    /// different failure from `HostUnnamed` — either the identity could not
    /// be read or minted at all, or an unnamed machine's label could not be
    /// written back, so there is no label question worth asking and "run
    /// `dispatch tui` interactively to name it" would not help. The message
    /// must say so distinctly rather than reusing `HostUnnamed`'s remedy.
    #[test]
    fn host_identity_unavailable_is_worded_distinctly_and_names_the_remedy() {
        let msg = StartupAbort::HostIdentityUnavailable.message();
        assert!(
            !msg.contains("dispatch tui interactively"),
            "nothing is asking for a name, so the remedy must not be the \
             one-time naming prompt: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("identity") || msg.to_lowercase().contains("settings"),
            "the message must explain that the host's identity itself could \
             not be determined, not that it merely lacks a label: {msg}"
        );
        for other in [
            StartupAbort::TmuxUnavailable,
            StartupAbort::LaunchRejected,
            StartupAbort::BoardAlreadyInThisWindow,
            StartupAbort::PreviousBoardNotRetired,
            StartupAbort::SessionUnidentified,
            StartupAbort::AgentPortUnavailable { port: 1234 },
            StartupAbort::HostUnnamed,
        ] {
            assert_ne!(msg, other.message());
        }
    }
}
