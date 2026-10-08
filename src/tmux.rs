use anyhow::{bail, Context, Result};
use std::process::Output;

use crate::models::TmuxWindow;
use crate::process::{stderr_str, stdout_str, ProcessRunner, SUBPROCESS_TIMEOUT};

// ---------------------------------------------------------------------------
// Shared checked-run helper
// ---------------------------------------------------------------------------

/// Build the consistent `"tmux {context} failed with status {status}[: {stderr}]"`
/// error for a failed [`Output`].
fn checked_error(context: &str, output: &Output) -> anyhow::Error {
    let stderr = stderr_str(output);
    if stderr.is_empty() {
        anyhow::anyhow!("tmux {context} failed with status {}", output.status)
    } else {
        anyhow::anyhow!(
            "tmux {context} failed with status {}: {}",
            output.status,
            stderr
        )
    }
}

/// Turn a raw [`Output`] into a [`Result`], via [`checked_error`] on failure.
/// Shared by [`run_checked`] and [`run_checked_timeout`], which differ only in
/// which `ProcessRunner` method produced the `Output`.
fn check_output(output: Output, context: &str) -> Result<Output> {
    if !output.status.success() {
        return Err(checked_error(context, &output));
    }
    Ok(output)
}

/// Run `tmux` with `args`, returning the raw [`Output`] on success and a
/// consistent checked-run error (see [`checked_error`]) otherwise.
fn run_checked(runner: &dyn ProcessRunner, args: &[&str], context: &str) -> Result<Output> {
    check_output(runner.run("tmux", args)?, context)
}

/// Like [`run_checked`], but returns trimmed stdout as a `String` instead of
/// the raw `Output` — for calls whose stdout is the actual result.
fn run_checked_stdout(runner: &dyn ProcessRunner, args: &[&str], context: &str) -> Result<String> {
    let output = run_checked(runner, args, context)?;
    Ok(stdout_str(&output))
}

/// Like [`run_checked`], but bounds the subprocess with [`SUBPROCESS_TIMEOUT`]
/// so a hung tmux server cannot park the calling thread forever.
///
/// Used by [`new_window`], [`set_window_dispatch_dir`] and
/// [`ensure_split_hook`] — and so by every one of their callers, including
/// `provision_worktree`'s `post_add` step and `resume_agent`
/// (`src/dispatch/agents.rs`) — which is what closes #4202. Every other tmux
/// call in this module still goes through the unbounded [`run_checked`]; if
/// one of those turns out to need the same treatment it should get its own
/// pass rather than folding in here.
fn run_checked_timeout(runner: &dyn ProcessRunner, args: &[&str], context: &str) -> Result<Output> {
    check_output(
        runner.run_with_timeout("tmux", args, SUBPROCESS_TIMEOUT)?,
        context,
    )
}

// ---------------------------------------------------------------------------
// Exact window-name targeting
// ---------------------------------------------------------------------------

/// `list-panes` output format for [`window_target`]: fixed-width fields first so
/// the window name — which may contain spaces — is the parseable remainder.
pub(crate) const WINDOW_PANE_FORMAT: &str = "#{pane_active} #{pane_id} #{window_name}";

/// Opening of the `-f` filter [`window_filter`] builds. Split out so
/// [`window_name_in_lookup`] can invert it.
const WINDOW_FILTER_PREFIX: &str = "#{==:#{window_name},";

/// The `(pane_id, window_name)` of each **active** pane in a listing formatted
/// with [`WINDOW_PANE_FORMAT`].
///
/// One row per window, because exactly one pane per window is active — which is
/// what lets a second match mean "two windows share this name" rather than "two
/// panes in one window". Shared by the server-wide [`window_target`] and the
/// session-scoped [`pane_id_of_window_in_session`] so the format's field order
/// and that assumption are written down once.
fn active_pane_rows(listing: &str) -> impl Iterator<Item = (&str, &str)> {
    listing.lines().filter_map(|line| {
        let mut parts = line.splitn(3, ' ');
        let active = parts.next()?;
        let pane_id = parts.next()?;
        let name = parts.next()?.trim_end();
        (active == "1").then_some((pane_id, name))
    })
}

/// A `list-panes -f` filter selecting panes whose window name equals `window`.
/// `#{==:…}` compares in tmux, so no prefix matching is involved.
fn window_filter(window: &str) -> String {
    format!("{WINDOW_FILTER_PREFIX}{window}}}")
}

/// The window name a [`window_target`] lookup is asking about, given its argv —
/// the inverse of [`window_filter`]. `None` when `args` is not such a lookup.
///
/// Exists for `MockProcessRunner`, which answers the lookup without a tmux
/// server and so needs to know which window is being asked for. Keeping the
/// construction and the inversion adjacent is what stops them drifting apart.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn window_name_in_lookup<'a>(args: &[&'a str]) -> Option<&'a str> {
    match args {
        ["list-panes", "-a", "-f", filter, "-F", format] if *format == WINDOW_PANE_FORMAT => {
            filter.strip_prefix(WINDOW_FILTER_PREFIX)?.strip_suffix('}')
        }
        _ => None,
    }
}

/// Whether `target` is already unambiguous and must reach tmux untouched:
/// a pane ID (`%N`), or the empty string, which is tmux's "current window" and
/// is part of [`rename_window`]'s documented contract.
///
/// These are exactly the two strings [`TmuxWindow::parse`] rejects, and the
/// pane-ID half is [`crate::models::is_pane_id`] so the two cannot drift: a
/// string is either a window name or a resolved target, never both.
fn is_resolved_target(target: &str) -> bool {
    target.is_empty() || crate::models::is_pane_id(target)
}

/// Resolve a tmux window *name* to the pane ID of that window's active pane,
/// matching the name **exactly**. Pane IDs pass through unchanged.
///
/// # Why every window target goes through here
///
/// tmux resolves a `-t <window-name>` target by exact match and then by
/// **prefix**. Dispatch names windows `task-<id>`, so once ids reach the
/// thousands one task's name is a prefix of another's — `task-378` and
/// `task-3782`. When the intended window is absent (killed, crashed, cleaned
/// up) and a longer-named sibling is alive, tmux silently redirects the
/// operation to the sibling: `send-keys` types the agent command into another
/// task's live Claude session, and `kill-window` destroys another task's agent.
/// Both are the same class of defect as issue #3781. See the
/// `TmuxWindowTargetedExactly` invariant in docs/specs/dispatch.allium.
///
/// # Why a pane ID rather than tmux's `=` sigil
///
/// tmux's documented exact-match sigil (`-t '=task-4'`) is not a general
/// answer. Verified against tmux 3.5a: `send-keys` rejects it outright
/// (`can't find pane: =task-42`, even when that window exists), `set-option -w`
/// rejects it (`no such window`), and `display-message -p` accepts it while
/// printing nothing and **exiting zero** — the worst outcome, a silently empty
/// pane ID. It only works for the target-*window* commands. A pane ID, by
/// contrast, is accepted by every command this module issues and cannot be
/// prefix-matched, so one mechanism covers all of them.
///
/// # Errors
///
/// Absent name, or two windows sharing it. Ambiguity is refused rather than
/// resolved arbitrarily: tmux already refuses it for `kill-window` and
/// `select-window`, but silently picks one for `set-option -w`. Refusing
/// uniformly is what [`set_window_dispatch_dir`]'s stderr sniff for
/// "ambiguous" used to approximate for that one call.
///
/// `pub(crate)` rather than private: [`crate::notify::notify_tmux`] resolves
/// a window once and reuses the resolved pane id for both its `capture-pane`
/// and `send-keys` calls, rather than letting each of those helpers
/// independently re-run this lookup's `list-panes` subprocess for the exact
/// same window on every notification.
///
/// # Why this one still takes `&str`
///
/// It accepts *either* a window name *or* an already-resolved target
/// ([`is_resolved_target`] — a pane ID, or `""` for the current window), and
/// those two are precisely what [`TmuxWindow::parse`] rejects. Typing the
/// parameter would either swallow that distinction or force callers to fake a
/// name for a pane they already hold. The same reasoning keeps
/// [`rename_window`]'s `target`, [`pane_ids_with_option`],
/// [`pane_ids_with_option_value`] and the `split_window_*_running` pair on
/// `&str` — each is documented as taking a name *or* a pane id. Every helper
/// whose parameter can only ever be a name takes a [`TmuxWindow`].
pub(crate) fn window_target(window: &str, runner: &dyn ProcessRunner) -> Result<String> {
    if is_resolved_target(window) {
        return Ok(window.to_string());
    }
    // A failed query means there are no windows to match — no server running,
    // or tmux unreachable. Same soft-fail as `list_all_window_names`, whose
    // `-a` rationale this shares: works inside or outside tmux, and finds
    // windows living in a session other than the current one.
    let filter = window_filter(window);
    let output = runner.run(
        "tmux",
        &["list-panes", "-a", "-f", &filter, "-F", WINDOW_PANE_FORMAT],
    )?;
    let listing = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        String::new()
    };

    // The `name == window` check is not redundant with the `-f` filter: the
    // filter interpolates the name into a tmux format string, so a name
    // containing `,` or `}` could confuse `#{==:…}`. Re-comparing here makes
    // correctness independent of that — a crafted name can at worst produce a
    // miss (which fails safe), never a match on the wrong window.
    //
    // Filtering on the *active* pane yields exactly one row per window, so a
    // second match means two windows share the name — not two panes in one.
    let mut matches = active_pane_rows(&listing)
        .filter(|&(_, name)| name == window)
        .map(|(pane_id, _)| pane_id.to_string());

    let Some(pane_id) = matches.next() else {
        bail!("no tmux window named '{window}'");
    };
    if matches.next().is_some() {
        bail!(
            "multiple tmux windows named '{}' exist — close the duplicate windows before dispatching",
            window
        );
    }
    Ok(pane_id)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Refuse `name` when a live tmux window already carries it.
///
/// The single enforcement point for `TmuxWindowNamesAreUnique`
/// (docs/specs/dispatch.allium). Every operation that assigns a name to a
/// window calls this first, so the guarantee sits at the four tmux primitives
/// rather than at their callers — which is what makes it backstop the callers
/// nobody has written yet. tmux is happy to let two windows share a name;
/// the spec says why it must not, and what a duplicate costs.
///
/// A failed existence query reads as "no live window" and the caller proceeds:
/// a tmux hiccup must not block a dispatch, and the operation itself then
/// fails anyway if the server really is unreachable.
fn refuse_duplicate_window_name(name: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<()> {
    if has_window(name, runner).unwrap_or(false) {
        bail!(
            "a tmux window named '{name}' already exists — a second window under that name \
             would make every operation on it ambiguous, leaving that task unreachable"
        );
    }
    Ok(())
}

/// Create a new tmux window with the given name, starting in `working_dir`.
///
/// Refuses a name a live window already holds rather than reattaching to it:
/// the caller here is starting a *fresh* agent, so adopting an unknown live
/// session would hand it a prompt meant for a session that was never created.
/// See [`refuse_duplicate_window_name`].
///
/// `env` is set on the window's own process via tmux's `-e KEY=VALUE`
/// (supported since tmux 3.0), not `set-environment` — the latter only
/// applies to processes tmux spawns *after* the call, so it cannot reach the
/// shell `new-window` itself is about to start. Empty for a window with no
/// launch-time environment to set.
pub fn new_window(
    name: &TmuxWindow,
    working_dir: &str,
    env: &[(&str, &str)],
    runner: &dyn ProcessRunner,
) -> Result<()> {
    refuse_duplicate_window_name(name, runner)?;
    let mut args: Vec<&str> = vec!["new-window", "-d", "-n", name.as_str(), "-c", working_dir];
    let env_pairs: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    for pair in &env_pairs {
        args.push("-e");
        args.push(pair);
    }
    run_checked_timeout(runner, &args, "new-window")?;
    Ok(())
}

/// Create a new tmux window running the given command as separate argv
/// elements (no shell wrapping). When the command exits, the window closes.
///
/// `-d` keeps current focus; callers use [`select_window`] afterwards to
/// switch to the new window if desired.
pub fn new_window_running(
    name: &TmuxWindow,
    working_dir: &str,
    command: &[&str],
    runner: &dyn ProcessRunner,
) -> Result<()> {
    if command.is_empty() {
        bail!("new_window_running: command must not be empty");
    }
    // After the empty-command guard, so an invalid call is rejected on its own
    // terms without paying for a subprocess.
    refuse_duplicate_window_name(name, runner)?;
    let mut args: Vec<&str> = vec![
        "new-window",
        "-d",
        "-n",
        name.as_str(),
        "-c",
        working_dir,
        "--",
    ];
    args.extend(command.iter().copied());
    run_checked(runner, &args, "new-window")?;
    Ok(())
}

/// Send literal text to a tmux window, then press Enter.
///
/// Uses `-l` to prevent tmux from interpreting escape sequences in the text.
/// Enter is sent as a separate `send-keys` call without `-l`.
///
/// The window name is resolved by [`window_target`] first — an absent window
/// must fail here, never fall through to a prefix-matched sibling, because the
/// payload is typed into whatever Claude session receives it.
pub fn send_keys(window: &TmuxWindow, keys: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let target = window_target(window.as_str(), runner)?;
    send_keys_at(&target, keys, runner)
}

/// [`send_keys`] against a target [`window_target`] has already resolved.
///
/// `pub(crate)` for [`crate::notify::notify_tmux`] alone, which resolves one
/// window and drives both this and [`capture_pane`] from that single pane
/// id rather than paying for the `list-panes` lookup twice per notification.
/// Everything else goes through the [`TmuxWindow`]-typed wrappers, so the
/// resolution step cannot be skipped by accident.
pub(crate) fn send_keys_at(target: &str, keys: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(
        runner,
        &["send-keys", "-t", target, "-l", keys],
        "send-keys -l",
    )?;
    run_checked(
        runner,
        &["send-keys", "-t", target, "Enter"],
        "send-keys Enter",
    )?;
    Ok(())
}

/// Return true if a tmux window with the given name currently exists,
/// searching across all sessions. Built on [`list_all_window_names`], which
/// issues the identical `list-windows -a` query — same `-a` rationale: works
/// whether the caller is inside or outside tmux, and finds windows living in
/// a session other than the current/attached one.
pub fn has_window(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<bool> {
    Ok(list_all_window_names(runner)?
        .iter()
        .any(|n| n == window.as_str()))
}

/// Whether `window` should be treated as alive: `true` when [`has_window`]
/// finds it, but also `true` when the query itself fails.
///
/// A query failure (tmux not reachable, transient error) is deliberately
/// mapped to "present" rather than "absent" — the caller of this helper uses
/// the result to decide whether to treat a task's agent as crashed, and a
/// false "absent" would trigger a spurious re-dispatch or crash notification
/// from a hiccup that has nothing to do with the window's actual state. See
/// `has_window`'s other callers (`kill_window_if_present`) for the opposite
/// default, which applies where the gated action is itself destructive.
pub fn has_window_or_assume_present(window: &TmuxWindow, runner: &dyn ProcessRunner) -> bool {
    has_window(window, runner).unwrap_or(true)
}

/// Kill `window` if a live check finds it present.
///
/// Unlike [`has_window_or_assume_present`], a query failure here is logged
/// and treated as "nothing to kill" rather than propagated or assumed
/// present — attempting a `kill-window` against a query we couldn't
/// validate risks a hard failure that would abort the rest of the
/// caller's cleanup (e.g. removing the git worktree). Skipping the kill is
/// the safe choice when we can't tell whether the window still exists.
pub fn kill_window_if_present(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<()> {
    match has_window(window, runner) {
        Ok(true) => kill_window(window, runner),
        Ok(false) => Ok(()),
        Err(e) => {
            tracing::warn!("could not check tmux window '{window}' before kill: {e}");
            Ok(())
        }
    }
}

/// Run a server-wide `list-*` query and collect its non-empty output lines.
///
/// Shared by [`list_all_window_names`] and [`list_all_pane_ids`] so the
/// non-obvious half of the contract — a failed call means "no server running",
/// not an error — lives in one place.
fn list_all(args: &[&str], runner: &dyn ProcessRunner) -> Result<Vec<String>> {
    let output = runner.run("tmux", args)?;
    if !output.status.success() {
        return Ok(vec![]);
    }
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(lines)
}

/// List the names of all tmux windows across all sessions.
///
/// Uses `-a` so the query works whether the caller is inside or outside tmux.
/// Returns an empty vec (not an error) when no tmux server is running.
pub fn list_all_window_names(runner: &dyn ProcessRunner) -> Result<Vec<String>> {
    list_all(&["list-windows", "-a", "-F", "#{window_name}"], runner)
}

/// Kill the tmux window with the given name.
///
/// The name is resolved by [`window_target`] first, so an absent window fails
/// instead of destroying a prefix-matched sibling's agent. That makes this safe
/// on its own; [`kill_window_if_present`] remains the wrapper for callers whose
/// cleanup must not abort when the window is simply already gone.
pub fn kill_window(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<()> {
    let target = window_target(window.as_str(), runner)?;
    kill_window_at(&target, runner)
}

/// [`kill_window`] against a target already resolved to a pane ID.
///
/// For the caller that resolved the window itself because a server-wide
/// [`window_target`] would have been the wrong question — see
/// [`pane_id_of_window_in_session`].
pub fn kill_window_at(target: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(runner, &["kill-window", "-t", target], "kill-window")?;
    Ok(())
}

/// The process id of the first process in `pane` (`#{pane_pid}`), read so a
/// caller can later tell whether that pane's process has actually exited —
/// tmux's own listing forgets the pane the moment its window is killed, long
/// before the process in it has gone. `None` when tmux cannot say.
///
/// `pane` is an explicit pane id: `display-message` without `-t` answers about
/// the session's active pane, not the caller's.
pub fn pane_pid(pane: &str, runner: &dyn ProcessRunner) -> Option<u32> {
    let output = runner
        .run(
            "tmux",
            &["display-message", "-p", "-t", pane, "#{pane_pid}"],
        )
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

// ---------------------------------------------------------------------------
// Session-scoped operations — startup.allium's restart path
// ---------------------------------------------------------------------------

/// Whether a session of this exact name exists.
///
/// `=` forces an exact match: tmux otherwise resolves a session target by
/// prefix, and "is dispatch's session there?" answered by a session the
/// operator named `dispatch-notes` would send the launch down the restart path
/// against somebody else's windows.
///
/// A query that cannot run at all — no server, no tmux — reads as "no", which
/// is the same answer a running server with no such session gives, and leads
/// the launch to the create path where a real tmux problem surfaces properly.
pub fn session_exists(session: &str, runner: &dyn ProcessRunner) -> bool {
    let target = format!("={session}");
    runner
        .run("tmux", &["has-session", "-t", &target])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The pane ID of `window`'s active pane **within `session`**, or `None` when
/// that session has no window of that name.
///
/// # Why not [`window_target`]
///
/// [`window_target`] searches the whole server, which is right for a
/// `task-<id>` window that lives in exactly one place. The board's window name
/// is not like that: it is a fixed name, and a window the operator happens to
/// have called the same thing in an unrelated session is not dispatch's board.
/// Since the caller is about to *close* what this finds, the scope has to be
/// the session it was asked about and no wider.
///
/// The name is compared exactly, for the reason [`window_target`] gives at
/// length: tmux's own `-t` resolution falls back to prefix matching.
///
/// A failed query reads as "no such window". The session may be gone, or the
/// server with it; either way there is nothing to retire, which is exactly what
/// `None` says.
pub fn pane_id_of_window_in_session(
    session: &str,
    window: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<Option<String>> {
    let target = format!("={session}");
    let output = runner.run(
        "tmux",
        &["list-panes", "-s", "-t", &target, "-F", WINDOW_PANE_FORMAT],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let found = active_pane_rows(&listing)
        .find(|(_, name)| *name == window.as_str())
        .map(|(pane_id, _)| pane_id.to_string());
    Ok(found)
}

/// Refuse `name` when a window in `session` already holds it, ignoring `except`
/// — the pane of a window that is allowed to keep the name it already has.
///
/// The session-scoped counterpart of [`refuse_duplicate_window_name`], for the
/// one name whose uniqueness is per session rather than per server: the board's
/// (startup.allium's `config.board_window_name`, named as the exception in
/// dispatch.allium's `TmuxWindowNamesAreUnique`).
///
/// A failed lookup reads as "no duplicate" and the caller proceeds, the same
/// soft-fail the server-wide version gives: a tmux hiccup must not block a
/// launch, and the operation itself then fails anyway if the server really is
/// unreachable.
fn refuse_duplicate_window_name_in_session(
    session: &str,
    name: &TmuxWindow,
    except: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<bool> {
    match pane_id_of_window_in_session(session, name, runner) {
        Ok(Some(pane)) if Some(pane.as_str()) == except => Ok(false),
        Ok(Some(_)) => bail!("tmux session '{session}' already has a window named '{name}'"),
        Ok(None) => Ok(true),
        Err(e) => {
            tracing::warn!("could not check '{name}' in session '{session}': {e}");
            Ok(true)
        }
    }
}

/// Create a window in `session` running `command` as separate argv elements,
/// and leave it selected.
///
/// Unlike [`new_window_running`] this targets a named session rather than the
/// caller's own, and deliberately omits `-d`: the caller is about to attach to
/// `session`, and a window created in the background would put the operator in
/// front of whatever window happened to be current instead of the board.
///
/// The window is named up front rather than left for the board to rename
/// itself. A board that dies before it renames anything still leaves a window
/// the next launch can find and retire.
pub fn new_window_in_session_running(
    session: &str,
    name: &TmuxWindow,
    command: &[&str],
    runner: &dyn ProcessRunner,
) -> Result<()> {
    if command.is_empty() {
        bail!("new_window_in_session_running: command must not be empty");
    }
    // Scoped to `session`, deliberately, unlike `refuse_duplicate_window_name`:
    // a server-wide check would let a window in a session dispatch does not own
    // refuse the replacement board, on a path that has already retired the
    // previous one.
    refuse_duplicate_window_name_in_session(session, name, None, runner)?;
    let target = format!("={session}:");
    let mut args: Vec<&str> = vec!["new-window", "-t", &target, "-n", name.as_str(), "--"];
    args.extend(command.iter().copied());
    run_checked(runner, &args, "new-window")?;
    Ok(())
}

/// Where the calling process is: its session, its window's name, and how many
/// panes that window holds.
///
/// The three are read in one `display-message` because the launch path needs
/// all of them to decide anything, and asking separately would let them
/// disagree — a window renamed between two probes would be described by neither
/// answer. See `startup::read_launch_context`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentWindowContext {
    pub session_name: String,
    pub window_name: String,
    pub window_panes: u32,
}

/// Format for [`current_window_context`]: the two fixed-shape fields first, so
/// the window name — which may contain spaces — is the parseable remainder.
/// Same reasoning as [`WINDOW_PANE_FORMAT`].
const CURRENT_WINDOW_FORMAT: &str = "#{window_panes} #{session_name} #{window_name}";

/// Read [`CurrentWindowContext`] for the window this process is in.
///
/// `pane` is the caller's own pane — [`self_pane_id`] on the real path, an
/// explicit id in tests. Without it tmux answers about the session's *active*
/// window instead, which is a different window whenever the caller is not the
/// focused one, and the answer looks perfectly reasonable.
///
/// A session name containing a space would take the window name's place here.
/// That is not reachable through dispatch, which names its own session from
/// `startup::SESSION_NAME`, and the cost of the misread is a launch that
/// declines to treat the window as the board's — the safe direction.
pub fn current_window_context(
    pane: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<CurrentWindowContext> {
    let line = run_checked_stdout(
        runner,
        &display_message_args(pane, CURRENT_WINDOW_FORMAT),
        "display-message",
    )?;
    let mut parts = line.splitn(3, ' ');
    let panes = parts.next().unwrap_or_default();
    let session = parts.next().unwrap_or_default();
    let window = parts.next().unwrap_or_default();
    let Ok(window_panes) = panes.parse::<u32>() else {
        bail!("tmux reported an unparseable window context: {line:?}");
    };
    Ok(CurrentWindowContext {
        session_name: session.to_string(),
        window_name: window.trim_end().to_string(),
        window_panes,
    })
}

/// [`rename_window`] whose duplicate-name refusal is scoped to `session`.
///
/// For the board's own window alone: its name is unique per session rather than
/// per server (dispatch.allium's `TmuxWindowNamesAreUnique` names it as the one
/// exception), so a window of the same name in a session dispatch does not own
/// must not stop the board adopting it. Every other rename keeps the
/// server-wide refusal.
///
/// A name the window already carries is left alone rather than reassigned: the
/// two window-creating paths now pass `-n`, so this is the common case and a
/// self-rename would only cost a subprocess and trip the refusal.
pub fn rename_window_in_session(
    session: &str,
    target: &str,
    new_name: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let target = window_target(target, runner)?;
    if session.is_empty() {
        // Nothing to scope the check to. The rename proceeds unchecked, which
        // is the soft-fail `rename_window` already gives a failed existence
        // query: the rename itself fails if the name really is taken.
        tracing::warn!("renaming '{target}' to '{new_name}' without a session to check against");
    } else if !refuse_duplicate_window_name_in_session(session, new_name, Some(&target), runner)? {
        // The window already carries this name. Both window-creating paths pass
        // `-n`, so this is the common case, and reassigning would only cost a
        // subprocess.
        return Ok(());
    }
    run_checked(
        runner,
        &["rename-window", "-t", &target, new_name.as_str()],
        "rename-window",
    )?;
    Ok(())
}

/// Whether a [`kill_window`] error means the window was simply already gone,
/// as opposed to a kill that was attempted and failed.
///
/// For a best-effort teardown the two are worlds apart: an absent window is the
/// state the caller wanted, while a failed kill leaves a real window behind. The
/// distinction was previously invisible, so every teardown of an
/// already-closed window warned.
///
/// Matching on the message is sound here in a way it would not be for an
/// external tool's output: the wording is produced by [`window_target`] in this
/// module, and `window_target_treats_a_failed_lookup_as_not_found` pins it.
/// Prefer [`kill_window_if_present`] when an extra tmux round-trip is
/// acceptable; use this when it is not.
pub fn is_window_absent_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("no tmux window named")
}

/// Switch the active tmux window to the one with the given name.
pub fn select_window(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<()> {
    let target = window_target(window.as_str(), runner)?;
    run_checked(runner, &["select-window", "-t", &target], "select-window")?;
    Ok(())
}

/// Store the worktree path as a per-window user option so the session-level
/// `after-split-window` hook (installed by [`ensure_split_hook`]) can look it
/// up when a split happens in this window.
///
/// A prefix-matched target here would leak one task's worktree path onto
/// another task's window, sending that window's future splits into the wrong
/// worktree — so the name is resolved by [`window_target`] first. That resolver
/// also owns the duplicate-name refusal this function used to approximate by
/// sniffing tmux's stderr for "ambiguous"; `set-option -w` does not actually
/// report ambiguity, it silently picks one of the duplicates.
pub fn set_window_dispatch_dir(
    window: &TmuxWindow,
    working_dir: &str,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let target = window_target(window.as_str(), runner)?;
    run_checked_timeout(
        runner,
        &[
            "set-option",
            "-w",
            "-t",
            &target,
            "@dispatch_dir",
            working_dir,
        ],
        "set-option",
    )?;
    Ok(())
}

/// Install a single session-level `after-split-window` hook that reads the
/// `@dispatch_dir` window option. If the option is set on the window being split
/// *and* the new pane did not already start in that directory, the pane is
/// respawned there; otherwise nothing happens.
///
/// This is idempotent — calling it multiple times replaces the same hook.
///
/// # Why this hook exists (do not delete it)
///
/// It is load-bearing, not a convenience. tmux never inherits the split pane's
/// directory: a `split-window` invoked by an *external CLI client* — which is how
/// dispatch, or any script, shells out to tmux — starts the new pane in the
/// **invoking process's** cwd, and one the user triggers from an attached client
/// starts it in the **session's** cwd. Neither is the worktree. Without this hook
/// every user split inside an agent window lands outside it. Any refactor that
/// removes the hook must first replace that guarantee by other means. Full
/// history (issue #231, commit 8bf36803) and the behavioural contract live in the
/// `AgentWindowSplitStartsInTaskWorktree` rule in docs/specs/split-pane.allium.
///
/// # Why the hook is a *fallback*
///
/// Dispatch's own splits name their start directory
/// ([`split_window_horizontal_running`]'s `start_dir` → `split-window -c`), so
/// they are correct at creation. The `#{pane_start_path}` half of the guard makes
/// the hook skip them, which matters: `respawn-pane` would restart the companion
/// process those panes were created to run.
///
/// # Why the correction is not `send-keys`
///
/// It used to be — `cd <dir>` + Enter typed at the new pane — and that is only
/// correct if a shell happens to be reading. The agent-tree companion pane exits
/// on `q`, so any worktree path containing that letter closed the pane the moment
/// it opened. See `SplitDirectoryIsNeverKeystrokes` in the same spec.
///
/// # Why `respawn-pane` must carry `-t #{pane_id}`
///
/// The target is mandatory, not decorative. `run-shell -bC` loses the enclosing
/// command's target context, so an untargeted command falls back to the session's
/// **active** pane. Because dispatch opens the agent-tree companion pane by
/// splitting the agent window in the background (`spawn_agent_tree_pane`) while
/// the board is still focused, the untargeted form of the old hook typed
/// `cd <worktree>` into the board TUI, where `c` fired the Copy-Task keybinding
/// (#3781). `#{pane_id}` is expanded in the hook's own context — the newly
/// created pane. Pane routing is only observable against a real tmux server, so
/// it is covered by tests/tmux_split_hook.rs rather than by this file's
/// mock-level test.
/// The tmux format [`ensure_split_hook`] tests before correcting a pane: true
/// when the window carries `@dispatch_dir` *and* the new pane did not already
/// start there.
///
/// Named rather than inlined so that a test can evaluate the real guard against
/// a real pane (`display-message -p`) instead of restating it — whether a pane is
/// selected for correction is the whole point of the hook, and a restated copy
/// could agree with itself while disagreeing with what ships.
pub const SPLIT_NEEDS_CORRECTION: &str =
    "#{&&:#{@dispatch_dir},#{!=:#{pane_start_path},#{@dispatch_dir}}}";

pub fn ensure_split_hook(runner: &dyn ProcessRunner) -> Result<()> {
    // if-shell -F only format-expands its test argument, NOT the branch command.
    // respawn-pane doesn't expand formats either, so we wrap it in run-shell -C
    // which does expand #{…} before executing the tmux command.
    //
    // The innermost quotes around #{@dispatch_dir} are load-bearing: without
    // them a worktree path containing a space silently resolves to $HOME.
    let hook_cmd = format!(
        "if-shell -F '{SPLIT_NEEDS_CORRECTION}' \
         'run-shell -bC \"respawn-pane -k -t #{{pane_id}} -c \\\"#{{@dispatch_dir}}\\\"\"'"
    );
    run_checked_timeout(
        runner,
        &["set-hook", "after-split-window", &hook_cmd],
        "set-hook",
    )?;
    Ok(())
}

/// Read back a window's `@dispatch_dir` — the worktree path
/// [`set_window_dispatch_dir`] stored on it. `None` when the option is unset,
/// which is the normal answer for the board and the editor windows.
///
/// Lets the callers that hold only a window name (the agent-tree toggle and
/// resync paths) name a start directory for a split, without threading the
/// worktree through tmux and back out of the database. The window option is
/// already the single source of truth the correction hook reads.
///
/// `window` goes through [`window_target`] like every other name-taking helper
/// here, so a prefix-matched sibling window cannot answer for it.
pub fn window_dispatch_dir(
    window: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<Option<String>> {
    let target = window_target(window.as_str(), runner)?;
    // -q so an unset option is an empty answer rather than an error.
    let dir = run_checked_stdout(
        runner,
        &["show-options", "-wqv", "-t", &target, "@dispatch_dir"],
        "show-options",
    )?;
    Ok((!dir.is_empty()).then_some(dir))
}

/// Check whether tmux has `focus-events` enabled globally.
///
/// Returns `false` if the option is off or if the query fails (e.g. not
/// running inside tmux).
pub fn focus_events_enabled(runner: &dyn ProcessRunner) -> bool {
    let Ok(output) = runner.run("tmux", &["show-options", "-gv", "focus-events"]) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim() == "on"
}

/// Enable tmux `focus-events` globally.
///
/// This is idempotent — calling it when already enabled is a no-op.
pub fn set_focus_events(runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(
        runner,
        &["set-option", "-g", "focus-events", "on"],
        "set-option focus-events",
    )?;
    Ok(())
}

/// Path to the user's `~/.tmux.conf` under a given home directory.
///
/// Owned here so callers don't re-derive the `$HOME`-relative location, and
/// takes the home directory rather than reading it so this is not a second
/// reader with its own idea of what an unavailable `$HOME` means — see
/// `crate::setup::home_dir` and docs/specs/dispatch.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`. This one *writes* the file
/// it names, so an empty `$HOME` resolving here would drop a `.tmux.conf` into
/// whatever directory the process happened to be started from.
pub(crate) fn tmux_conf_path_in(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".tmux.conf")
}

/// [`tmux_conf_path_in`] against the operator's own home directory.
pub(crate) fn tmux_conf_path() -> Result<std::path::PathBuf> {
    Ok(tmux_conf_path_in(&crate::setup::home_dir()?))
}

/// The setting that marks `~/.tmux.conf` as already configured. Written down
/// once: the reader below and the writer beneath it both use it, so the drift
/// check and the write cannot disagree about what "already there" means
/// (`startup.allium`'s `OneDefinitionOfOutOfDate`).
const FOCUS_EVENTS_SETTING: &str = "focus-events on";

/// Whether this `~/.tmux.conf` content already carries the focus-events line.
fn contains_focus_events(content: &str) -> bool {
    content.contains(FOCUS_EVENTS_SETTING)
}

/// Whether `~/.tmux.conf` already carries the focus-events line.
///
/// The read-only half of [`write_focus_events_to_tmux_conf_at`]. An unreadable
/// or absent file does not carry it.
pub fn tmux_conf_has_focus_events(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|existing| contains_focus_events(&existing))
}

pub(crate) fn write_focus_events_to_tmux_conf_at(path: &std::path::Path) -> Result<()> {
    let existing = if path.exists() {
        std::fs::read_to_string(path).context("failed to read .tmux.conf")?
    } else {
        String::new()
    };
    if contains_focus_events(&existing) {
        return Ok(());
    }
    let line = format!("set -g {FOCUS_EVENTS_SETTING}\n");
    let addition = if existing.ends_with('\n') || existing.is_empty() {
        line
    } else {
        format!("\n{line}")
    };
    std::fs::write(path, existing + &addition).context("failed to write .tmux.conf")?;
    Ok(())
}

/// The pane this process is running in, as tmux told it in `$TMUX_PANE`.
///
/// # Why this exists
///
/// `display-message -p` with no `-t` does **not** report the calling pane. It
/// reports the session's *active* pane — verified against tmux 3.5a: run from a
/// background window, `#{pane_id}` comes back as the active window's pane and
/// `#W` as the active window's name. Every "where am I?" query is therefore
/// wrong for a process that is not in the focused window, silently and with a
/// plausible answer.
///
/// tmux sets `$TMUX_PANE` in every pane it starts, which is the one answer that
/// is about the caller. `None` when it is unset or malformed — outside tmux, or
/// an environment something has rewritten — and callers then fall back to the
/// untargeted query, which is no worse than what they had.
pub fn self_pane_id() -> Option<String> {
    std::env::var("TMUX_PANE")
        .ok()
        .filter(|p| crate::models::is_pane_id(p))
}

/// `display-message -p <format>`, targeted at `pane` when one is known.
///
/// Split out so the targeting is unit-testable without an environment the test
/// harness shares across threads.
pub(crate) fn display_message_args<'a>(pane: Option<&'a str>, format: &'a str) -> Vec<&'a str> {
    match pane {
        Some(p) => vec!["display-message", "-p", "-t", p, format],
        None => vec!["display-message", "-p", format],
    }
}

/// Return the name of the window `pane` is in — [`self_pane_id`] on the real
/// path. With `None`, tmux answers about the session's *active* window instead.
#[cfg(any(test, feature = "test-support"))]
pub fn current_window_name(pane: Option<&str>, runner: &dyn ProcessRunner) -> Result<String> {
    run_checked_stdout(runner, &display_message_args(pane, "#W"), "display-message")
}

/// Rename a tmux window. `target` may be a window name, a pane ID, or `""` to
/// rename the current window.
///
/// Only `target` is resolved by [`window_target`] — never `new_name`, which is
/// a name being assigned and by definition need not exist yet.
///
/// # Errors
///
/// Absent or ambiguous `target`, as everywhere else — and, uniquely to this
/// helper, a `new_name` a live window already holds. tmux is happy to let two
/// windows share a name; `TmuxWindowNamesAreUnique` in
/// docs/specs/dispatch.allium is why it must not, and what it costs. The
/// check lives here, at the one operation that assigns a name to a window
/// that already has one, so it backstops every caller — including the ones
/// nobody has written yet.
///
/// A failed existence query reads as "no such window" and the rename proceeds:
/// same soft-fail default [`list_all_window_names`] gives every other caller,
/// and the rename itself then fails if the server really is unreachable.
pub fn rename_window(
    target: &str,
    new_name: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let target = window_target(target, runner)?;
    // After resolving `target`, so an absent target is still reported as such
    // rather than being masked by whatever the new name happens to hit.
    refuse_duplicate_window_name(new_name, runner)?;
    run_checked(
        runner,
        &["rename-window", "-t", &target, new_name.as_str()],
        "rename-window",
    )?;
    Ok(())
}

/// Bind a tmux key (requires the tmux prefix first) to a command string.
pub fn bind_key(key: &str, command: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(runner, &["bind-key", key, command], "bind-key")?;
    Ok(())
}

/// Set `key=value` in `session`'s environment, so every process tmux starts
/// in that session from now on — a pane split, a new window — inherits it.
/// Processes already running are untouched; that is `set-environment`'s
/// contract, and why [`new_window`] uses `-e` for a window's own shell.
///
/// `=` anchors the target to the exact session name, as it does for
/// `select-window` in the board's keybinding: tmux otherwise matches a
/// `-t <name>` by prefix.
pub fn set_session_environment(
    session: &str,
    key: &str,
    value: &str,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let target = format!("={session}");
    run_checked(
        runner,
        &["set-environment", "-t", &target, key, value],
        "set-environment",
    )?;
    Ok(())
}

/// Remove a tmux key binding (previously registered with `bind-key`).
pub fn unbind_key(key: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(runner, &["unbind-key", key], "unbind-key")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Split mode operations
// ---------------------------------------------------------------------------

/// Return the tmux pane ID of the session's **active** pane (e.g. "%42").
///
/// **This is the session's *active* pane, not necessarily the caller's** — see
/// [`self_pane_id`] for why those differ and when it matters. Callers that need
/// their own pane read [`self_pane_id`] first and fall back to this. The
/// environment is deliberately not read here: these helpers take what they need
/// as arguments so a test is not at the mercy of an environment the harness
/// shares across threads.
pub fn current_pane_id(runner: &dyn ProcessRunner) -> Result<String> {
    run_checked_stdout(
        runner,
        &["display-message", "-p", "#{pane_id}"],
        "display-message",
    )
}

/// Create a horizontal split (right pane) at 40% width, keeping focus on the
/// left pane. Returns the new pane's ID.
pub fn split_window_horizontal(target_pane: &str, runner: &dyn ProcessRunner) -> Result<String> {
    run_checked_stdout(
        runner,
        &[
            "split-window",
            "-h",
            "-d",
            "-l",
            "40%",
            "-t",
            target_pane,
            "-P",
            "-F",
            "#{pane_id}",
        ],
        "split-window",
    )
}

/// Create a horizontal split (left pane) at `size_pct`% width, running the
/// given command as separate argv elements (no shell wrapping) in the new
/// pane, following the same "create + immediately run a command" shape
/// [`new_window_running`] establishes for window creation. Keeps focus on
/// the target pane. Returns the new pane's ID.
///
/// Sibling of [`split_window_horizontal`] (hardcoded 40%, no command, used
/// by the board's own split-pane feature) — this one is for spawning a
/// companion process (e.g. `dispatch agent-tree <task_id>`) narrower than
/// that split, since the target pane's own output still needs the room.
///
/// `target` may be a pane ID or a window *name*: `spawn_agent_tree_pane`
/// (src/dispatch/agents.rs) passes the agent's `task-<id>` window. Names go
/// through [`window_target`], so the companion pane cannot be opened inside a
/// prefix-matched sibling's window; pane IDs pass through untouched.
///
/// `start_dir` becomes `split-window -c`, the new pane's working directory.
/// Naming it is what makes the pane correct at creation instead of leaving it to
/// the `after-split-window` correction hook, which would restart `command` —
/// see [`ensure_split_hook`]. `None` leaves the directory to tmux, which picks
/// the invoking process's cwd for an external CLI client like this one.
pub fn split_window_horizontal_running(
    target: &str,
    size_pct: u8,
    command: &[&str],
    start_dir: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    if command.is_empty() {
        bail!("split_window_horizontal_running: command must not be empty");
    }
    let target_pane = window_target(target, runner)?;
    let size_arg = format!("{size_pct}%");
    let mut args: Vec<&str> = vec![
        "split-window",
        "-h",
        "-b",
        "-d",
        "-l",
        &size_arg,
        "-t",
        &target_pane,
        "-P",
        "-F",
        "#{pane_id}",
    ];
    if let Some(dir) = start_dir {
        args.extend(["-c", dir]);
    }
    // `--` last: everything after it is the pane's command, not an option.
    args.push("--");
    args.extend(command.iter().copied());
    run_checked_stdout(runner, &args, "split-window")
}

/// Create a pane **below `target`, inside `target`'s own column**, taking
/// `size_pct`% of that column's height, running `command` as separate argv
/// elements (no shell) with `cwd` as its start directory. Keeps focus where it
/// is. Returns the new pane's ID.
///
/// The third split helper in this module, and each difference is load-bearing.
/// [`split_window_horizontal`] (40%, right, no command) serves the board's
/// split-pane feature; [`split_window_horizontal_running`] (left, `size_pct`,
/// command) opens the agent-tree companion pane. This one opens the DIFF pane
/// *from* that companion pane, where:
///
/// * There is deliberately **no `-f`**. The new pane subdivides the companion
///   pane's narrow column rather than spanning the window, so the agent's own
///   pane keeps every column it had and opening a diff never takes room from
///   the thing the user is supervising. The editor pane this replaced did span
///   the window, and that is the one geometric difference between them — see
///   `SplitAgentTreeDiffPane` in docs/specs/agent-tree.allium.
/// * `-c` is passed explicitly rather than relying on [`ensure_split_hook`]'s
///   `@dispatch_dir` `cd`: that hook *types* `cd <dir>` into the new pane, which
///   works for a shell and would land in the diff renderer's own input here.
/// * Focus stays put (`-d`) so the user can keep browsing the tree; reaching
///   the diff to scroll it is tmux's own pane navigation.
pub fn split_window_below_running(
    target: &str,
    size_pct: u8,
    cwd: &str,
    command: &[&str],
    runner: &dyn ProcessRunner,
) -> Result<String> {
    if command.is_empty() {
        bail!("split_window_below_running: command must not be empty");
    }
    let target_pane = window_target(target, runner)?;
    let size_arg = format!("{size_pct}%");
    let mut args: Vec<&str> = vec![
        "split-window",
        "-v",
        "-d",
        "-l",
        &size_arg,
        "-t",
        &target_pane,
        "-c",
        cwd,
        "-P",
        "-F",
        "#{pane_id}",
        "--",
    ];
    args.extend(command.iter().copied());
    run_checked_stdout(runner, &args, "split-window")
}

/// Replace what is running in `pane_id` with `command` (argv, no shell), started
/// in `cwd`. `-k` kills the pane's current process first.
///
/// The pane object itself survives, which is what makes this the way the editor
/// pane shows a second file: it keeps its geometry and its pane options, so
/// nothing has to be re-marked, and focus is untouched. Sibling of
/// [`respawn_pane`], which respawns a plain shell in place.
pub fn respawn_pane_running(
    pane_id: &str,
    cwd: &str,
    command: &[&str],
    runner: &dyn ProcessRunner,
) -> Result<()> {
    if command.is_empty() {
        bail!("respawn_pane_running: command must not be empty");
    }
    let mut args: Vec<&str> = vec!["respawn-pane", "-k", "-c", cwd, "-t", pane_id, "--"];
    args.extend(command.iter().copied());
    run_checked(runner, &args, "respawn-pane")?;
    Ok(())
}

/// Pane option marking a pane **dispatch itself created** in an agent window,
/// valued with what that pane is for: [`PANE_ROLE_AGENT_TREE`] or
/// [`PANE_ROLE_DIFF`].
///
/// Lives here rather than with either creator because unrelated modules must
/// agree on it forever: `dispatch::agents` writes the tree role and reads it back
/// to toggle and resync that pane, `agent_tree_diff_pane` writes and reads the
/// diff role to split and kill its pane, and `dispatch::companion_pane_ids` reads the
/// option's mere *presence* to drain every dispatch-created pane when an agent
/// window is pinned into the board. The *policy* (when to split, when to replace)
/// stays with each creator; the vocabulary is shared infrastructure.
///
/// One option with a role value, rather than one option per pane kind, is what
/// makes that third question answerable in a single lookup — and answerable for
/// a future role without teaching the drain about it. See
/// docs/specs/agent-tree.allium's `HideAgentTreePane` ("How the companion pane
/// is identified") and `OneEditorPanePerAgentWindow`.
pub const PANE_ROLE_OPTION: &str = "@dispatch_pane_role";

/// [`PANE_ROLE_OPTION`] value for the `dispatch agent-tree` companion pane.
pub const PANE_ROLE_AGENT_TREE: &str = "agent_tree";

/// [`PANE_ROLE_OPTION`] value for the `dispatch agent-diff` pane opened from
/// that companion pane.
pub const PANE_ROLE_DIFF: &str = "diff";

/// Set a pane-scoped tmux user option (`@name`). The pane-level sibling of
/// [`set_window_dispatch_dir`]'s `set-option -w`.
///
/// Takes a pane **id** only, never a window name: a pane option is how dispatch
/// marks a pane it created, and a marker written to the wrong pane is worse than
/// no marker at all.
pub fn set_pane_option(
    pane_id: &str,
    option: &str,
    value: &str,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    run_checked(
        runner,
        &["set-option", "-p", "-t", pane_id, option, value],
        "set-option",
    )?;
    Ok(())
}

/// Move a tmux window into the current window as a right pane (40% width).
/// Returns the new pane's ID.
pub fn join_pane(
    source_window: &TmuxWindow,
    target_pane: &str,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    // Resolving the source window by exact name *is* the pane-ID lookup this
    // used to do with a separate `display-message`: it returns the window's
    // active pane, which is the pane join-pane moves, and pane IDs are
    // preserved across the move (join-pane has no -P/-F to print the result).
    // Passing the ID rather than the name also keeps a prefix-matched sibling
    // from being torn out of its own window and into the board.
    let pane_id = window_target(source_window.as_str(), runner)?;

    run_checked(
        runner,
        &[
            "join-pane",
            "-h",
            "-d",
            "-s",
            &pane_id,
            "-t",
            target_pane,
            "-l",
            "40%",
        ],
        "join-pane",
    )?;
    Ok(pane_id)
}

/// Break a pane out into its own tmux window with the given name.
///
/// Refuses a name a live window already holds, leaving the pane exactly where
/// it is. Why staying put beats every alternative here is argued in
/// split-pane.allium's `RefuseExitSplitModeOntoLiveWindow`.
pub fn break_pane_to_window(
    pane_id: &str,
    window_name: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    refuse_duplicate_window_name(window_name, runner)?;
    run_checked(
        runner,
        &[
            "break-pane",
            "-d",
            "-s",
            pane_id,
            "-n",
            window_name.as_str(),
        ],
        "break-pane",
    )?;
    Ok(())
}

/// Kill a specific tmux pane by ID.
pub fn kill_pane(pane_id: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(runner, &["kill-pane", "-t", pane_id], "kill-pane")?;
    Ok(())
}

/// Replace the content of a pane with a fresh shell, preserving the pane itself.
pub fn respawn_pane(pane_id: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(
        runner,
        &["respawn-pane", "-k", "-t", pane_id],
        "respawn-pane",
    )?;
    Ok(())
}

/// Get the pane ID of a window's active pane, matching the window name exactly.
/// Errors when `window` does not exist.
///
/// This is the public face of [`window_target`]. It replaced a
/// `display-message -p -t <window> '#{pane_id}'` call, which was wrong in two
/// compounding ways. It prefix-matched the window name, so it could hand back a
/// *different* task's pane ID — one that then propagated into swaps, splits and
/// the split-pane's tracked pane. And for a window that genuinely did not exist
/// it exited 0 printing an empty string rather than failing, so the miss
/// resolved to `Ok("")` — and `swap-pane -s ''` also exits 0, so the empty id
/// propagated silently until some later command failed with a misleading
/// message. Resolving through a `list-panes` row removes both: there is no row
/// to misattribute, and no row at all means no window. Verified against tmux
/// 3.5a.
pub fn pane_id_for_window(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Result<String> {
    window_target(window.as_str(), runner)
}

/// Atomically swap the contents of two panes without changing the layout.
/// `-d` keeps focus on the current pane.
///
/// Pass pane **ids**, not `<window>.<index>` targets: such a target is wrong in
/// both halves. The index shifts with the user's `pane-base-index` and is
/// renumbered by a `-b` split, so it can miss or hit the wrong pane; the window
/// name prefix-matches (see [`window_target`]), so it can address the wrong
/// window entirely. Use [`pane_id_for_window`], [`pane_ids_with_option`] or
/// [`pane_ids_with_option_value`] to resolve one.
pub fn swap_pane(source: &str, target: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(
        runner,
        &["swap-pane", "-d", "-s", source, "-t", target],
        "swap-pane",
    )?;
    Ok(())
}

/// Move tmux focus to the specified pane.
pub fn select_pane(pane_id: &str, runner: &dyn ProcessRunner) -> Result<()> {
    run_checked(runner, &["select-pane", "-t", pane_id], "select-pane")?;
    Ok(())
}

// An `inactive_pane_id` used to live here: "the window's single inactive pane",
// which every caller used to mean "the agent-tree companion pane". It was removed
// with its last caller (#3856). The premise held only for a two-pane window whose
// focus had not moved, so with focus in the companion pane it named the *agent's*
// pane and the toggle killed a live claude session. Identify a pane by what it is
// rather than by whether it happens to be focused: [`pane_ids_with_option`] for
// "any pane dispatch created", [`pane_ids_with_option_value`] for one role. See
// docs/specs/agent-tree.allium's HideAgentTreePane.

/// Split one `list-panes` row of the form `<pane_id> <rest…>` into its two
/// halves. `rest` is empty when the field it carries is unset — tmux prints the
/// separator either way — and may itself contain spaces, so only the first field
/// is consumed.
fn split_pane_row(line: &str) -> Option<(&str, &str)> {
    // A row with no separator at all is not a shape tmux produces for these
    // formats, but reading it as "id, no value" is strictly better than dropping
    // the pane from the listing — and it is the shape the *last* row takes, since
    // `run_checked_stdout` trims the trailing whitespace an unset field leaves.
    (!line.is_empty()).then(|| line.split_once(' ').unwrap_or((line, "")))
}

/// Every dispatch-created pane in `target`'s window, paired with the role it
/// was marked with.
///
/// **The primitive the other two lookups are built on.** All three ask tmux the
/// same question over the same `list-panes` shape and differ only in what they
/// keep, so the command construction, the `-F` format and the "an unmarked pane
/// is the agent's own" filter live here once. A caller that needs more than one
/// role at a time — the toggle, which retires the tree and takes the diff pane
/// with it — would otherwise pay a round-trip per role and, worse, read the
/// window at two different moments.
///
/// `target` may be a window name or a pane id. A pane id resolves to *its own*
/// window's panes, which is what lets a process inside a pane look up its
/// siblings knowing only `$TMUX_PANE`.
pub fn pane_roles(
    target: &str,
    option: &str,
    runner: &dyn ProcessRunner,
) -> Result<Vec<(String, String)>> {
    let resolved = window_target(target, runner)?;
    let format = format!("#{{pane_id}} {}", option_format(option));
    let out = run_checked_stdout(
        runner,
        &["list-panes", "-t", &resolved, "-F", &format],
        "list-panes",
    )?;
    Ok(out
        .lines()
        .filter_map(split_pane_row)
        .filter(|(_, role)| !role.is_empty())
        .map(|(id, role)| (id.to_string(), role.to_string()))
        .collect())
}

/// Pane ids in `target`'s window whose pane-scoped user option `option` is set to
/// a non-empty value — *any* value.
///
/// This is how dispatch asks "which panes in this window did I create?", over
/// [`PANE_ROLE_OPTION`]: the marker is written at creation ([`set_pane_option`])
/// and survives [`respawn_pane_running`], so a pane is identified by what it is
/// rather than by whether it happens to be the focused one — which is the
/// heuristic this replaced, true only for an untouched two-pane window. Use
/// [`pane_ids_with_option_value`] to ask for one specific role.
pub fn pane_ids_with_option(
    target: &str,
    option: &str,
    runner: &dyn ProcessRunner,
) -> Result<Vec<String>> {
    Ok(pane_roles(target, option, runner)?
        .into_iter()
        .map(|(id, _)| id)
        .collect())
}

/// Pane ids in `target`'s window whose pane-scoped user option `option` equals
/// `value` exactly.
///
/// The role-specific half of [`pane_ids_with_option`]: exact equality, never a
/// prefix or substring, because the roles of two panes in one window must not be
/// able to stand in for each other.
pub fn pane_ids_with_option_value(
    target: &str,
    option: &str,
    value: &str,
    runner: &dyn ProcessRunner,
) -> Result<Vec<String>> {
    Ok(pane_roles(target, option, runner)?
        .into_iter()
        .filter(|(_, role)| role == value)
        .map(|(id, _)| id)
        .collect())
}

/// The tmux format expression that expands to user option `option`'s value.
/// Read only by [`pane_roles`], which is the one place the lookups' command is
/// built.
fn option_format(option: &str) -> String {
    format!("#{{{option}}}")
}

/// List the ids of all tmux panes across all sessions.
///
/// The pane-level sibling of [`list_all_window_names`], with the same `-a`
/// rationale and the same "no server running" handling. Private: only
/// [`pane_exists`] needs it.
fn list_all_pane_ids(runner: &dyn ProcessRunner) -> Result<Vec<String>> {
    list_all(&["list-panes", "-a", "-F", "#{pane_id}"], runner)
}

/// Check whether a tmux pane with the given ID still exists.
///
/// Implemented as a membership test over [`list_all_pane_ids`], mirroring
/// [`has_window`], because the obvious-looking alternative does not work:
/// `display-message -t <pane> -p ''` **succeeds for a pane that has never
/// existed**. tmux resolves an unknown target by falling back to the current
/// pane rather than failing, and with an empty format string there is no output
/// to betray the substitution — so an exit-status check reports every pane as
/// alive, always. Verified against tmux 3.5a with `-t %999`.
///
/// That made this function's only caller — `exec_check_split_pane`, which polls
/// whether the user has closed the pinned split pane — permanently blind, so a
/// closed pane left the board in split mode with a dead pane. Found by the
/// real-tmux harness in tests/tmux_lifecycle.rs; the mock tests could not see it,
/// and in fact pinned the broken behaviour by asserting a non-zero exit that
/// real tmux never returns (the same trap as task #3781).
///
/// A query failure maps to "gone", which is the pre-existing behaviour and the
/// conservative choice for the polling caller: it exits split mode rather than
/// leaving a pane pinned that may no longer be there. Contrast
/// [`has_window_or_assume_present`], where the gated action is destructive and
/// the default therefore goes the other way.
pub fn pane_exists(pane_id: &str, runner: &dyn ProcessRunner) -> bool {
    list_all_pane_ids(runner)
        .map(|ids| ids.iter().any(|id| id == pane_id))
        .unwrap_or(false)
}

/// Capture the current on-screen content of `target`'s pane — exactly what a
/// human attached to it would see right now, not scrollback history.
///
/// Takes a target [`window_target`] has already resolved, not a window name:
/// its one caller, [`crate::notify::notify_tmux`], resolves the window once and
/// drives both this and [`send_keys_at`] from that single pane id. A
/// name-taking wrapper would have no callers — see [`send_keys`] for the shape
/// used where both kinds of caller exist.
///
/// Used by [`crate::notify`] to decide whether a pane is showing its normal
/// input surface before injecting keystrokes into it — see
/// `docs/superpowers/specs/2026-08-15-send-message-delivery-hardening-design.md`
/// for the reproduction this exists to guard against.
pub(crate) fn capture_pane(target: &str, runner: &dyn ProcessRunner) -> Result<String> {
    run_checked_stdout(
        runner,
        &["capture-pane", "-p", "-t", target],
        "capture-pane",
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
