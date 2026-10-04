//! Retiring the previous board window so a fresh board can start.

use super::launch::{StartupAbort, BOARD_WINDOW_NAME};
use crate::process::ProcessRunner;
use crate::tmux;

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
    retire_board_window_with(session, runner, &pane_process_alive)
}

/// [`retire_board_window`] with the "is anything of that pane still running"
/// probe injected, so a test decides what the retired process is doing rather
/// than depending on a real pid.
pub(super) fn retire_board_window_with(
    session: &str,
    runner: &dyn ProcessRunner,
    alive: &dyn Fn(u32) -> bool,
) -> RetireOutcome {
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
            // Read before the kill: tmux forgets the pane with the window.
            let pid = tmux::pane_pid(&pane, runner);
            if let Err(e) = tmux::kill_window_at(&pane, runner) {
                tracing::warn!("could not retire the board's window in '{session}': {e}");
            }
            Some((pane, pid))
        }
        Err(e) => {
            tracing::warn!("could not look for a board window in '{session}': {e}");
            None
        }
    };
    session_state_after_retire(session, pane.as_ref(), runner, alive)
}

/// How long to wait for a retired board to actually be gone.
///
/// `kill-window` removes the window from tmux's listing at once and signals its
/// process; that process's own exit -- which includes stopping the managed
/// store, bounded by `MANAGED_STORE_STOP_TIMEOUT` -- and with it the release of
/// the agent port happens afterwards. So the wait has to outlast that stop, or
/// a relaunch races the old board's exit: it reaches the port claim (or adopts a
/// store mid-shutdown) and tells the operator something is holding on, moments
/// after they had closed it. Comfortably above the stop timeout for that
/// reason; a board still there at the end is `BoardWindowSurvived`.
pub(super) const RETIRED_PANE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Poll step for [`RETIRED_PANE_DEADLINE`]. Short enough that the common case --
/// a board that exits at once -- costs one step rather than the whole budget.
const RETIRED_PANE_POLL_STEP: std::time::Duration = std::time::Duration::from_millis(25);

/// Whether any process still belongs to the session the pane's process leads.
///
/// tmux starts a pane's process as a session leader, so everything the pane
/// ran -- the board, or the shell that wraps it -- shares that session id, and
/// the board is alive exactly while one such process is not a zombie. Reads
/// `/proc`; where there is none this reports nothing running, which leaves the
/// wait to tmux's pane listing alone.
pub(super) fn pane_process_alive(pid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            return false;
        };
        // `pid (comm) state ppid pgrp session ...`; comm may hold spaces and
        // parentheses, so split after the last `)`.
        let Some(rest) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
            return false;
        };
        let mut fields = rest.split_whitespace();
        let state = fields.next();
        let session = fields.nth(2).and_then(|f| f.parse::<u32>().ok());
        session == Some(pid) && state != Some("Z")
    })
}

/// Which [`RetireOutcome`] the session is in now.
///
/// Read back from tmux rather than inferred from whether the close reported
/// success — `AFailedRetireIsNotMistakenForSuccess`. What the caller needs is
/// the state the session is in, and a kill that returned an error may still
/// have taken the window with it.
fn session_state_after_retire(
    session: &str,
    killed_pane: Option<&(String, Option<u32>)>,
    runner: &dyn ProcessRunner,
    alive: &dyn Fn(u32) -> bool,
) -> RetireOutcome {
    if let Some((pane, pid)) = killed_pane {
        await_pane_gone(pane, *pid, runner, alive);
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

/// Wait for the retired board to be gone: its pane out of tmux's listing *and*
/// its process (`pid`, read before the kill) no longer running.
///
/// Makes `SessionReady` mean "retired" rather than "asked to retire"
/// (`RetiredMeansGoneNotSignalled`), so the replacement board does not race the
/// old one's hold on the agent port or on the managed store it is stopping.
/// Bounded and best-effort: anything still there at the deadline is left to the
/// window read-back, which reports `BoardWindowSurvived` for a pane and
/// otherwise lets the launch's own port claim speak.
fn await_pane_gone(
    pane: &str,
    pid: Option<u32>,
    runner: &dyn ProcessRunner,
    alive: &dyn Fn(u32) -> bool,
) {
    let deadline = std::time::Instant::now() + RETIRED_PANE_DEADLINE;
    // Latched: once tmux stops listing the pane it never lists it again, so
    // later polls wait on the process alone and spawn no tmux command.
    let mut pane_listed = true;
    loop {
        pane_listed = pane_listed && tmux::pane_exists(pane, runner);
        if !pane_listed && !pid.is_some_and(alive) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            tracing::warn!("retired board (pane {pane}) still running after the deadline");
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
