//! Holding the operator's machine awake for the board (docs/specs/keep-awake.allium).
//!
//! The hold is a child process: an inhibit helper (`gnome-session-inhibit`
//! or `systemd-inhibit`) that takes the hold and then runs a small inner
//! command. The inner command prints [`READY`] and then waits on its stdin,
//! which is a pipe from the board. So:
//!
//! - "ready" on stdout means the helper has taken the hold (both helpers
//!   fail before running the inner command when they cannot take it), which
//!   is [`HoldEvent::Granted`];
//! - an exit before "ready" is [`HoldEvent::Refused`];
//! - an exit after "ready" is [`HoldEvent::Ended`].
//!
//! Dropping the [`Hold`] closes the pipe, the inner command sees end of
//! input and exits, and the helper drops the hold. The kernel closes the
//! pipe the same way when the board process dies for any reason, so no
//! cleanup step can be skipped (keep-awake.allium: HoldDiesWithTheBoard).

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};

/// The line the inner command prints once the helper has taken the hold.
const READY: &str = "ready";

/// The inner command the helper runs while it holds the machine awake.
const INNER: &[&str] = &["sh", "-c", "echo ready; exec cat"];

/// Why the hold was asked for, as the helpers show it to the desktop.
const REASON: &str = "dispatch keep-awake";

/// `systemd-inhibit`'s spelling of [`REASON`].
const WHY_FLAG: &str = "--why=dispatch keep-awake";

/// What the operating system answered, or what became of a granted hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldEvent {
    /// The helper took the hold.
    Granted,
    /// The helper exited before taking the hold. Carries why.
    Refused(String),
    /// A granted hold ended. Carries why.
    Ended(String),
}

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// The variables that name the desktop. `XDG_CURRENT_DESKTOP` alone is not
/// enough: a tmux server started from ssh, a TTY or a user unit lacks it,
/// while the session variables often survive.
const DESKTOP_VARS: &[&str] = &[
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
];

/// The values of [`DESKTOP_VARS`] that `read` finds, in that order.
fn desktop_vars(read: impl Fn(&str) -> Option<String>) -> Vec<String> {
    DESKTOP_VARS.iter().filter_map(|name| read(name)).collect()
}

/// Whether a desktop name such as `ubuntu:GNOME`, `gnome` or `gnome-xorg`
/// is GNOME.
fn names_gnome(desktop: &str) -> bool {
    desktop
        .split([':', '-'])
        .any(|p| p.eq_ignore_ascii_case("GNOME"))
}

/// Choose the helper. The GNOME session manager is preferred on GNOME,
/// because GNOME honours its inhibitors for screen lock and blanking; the
/// generic systemd inhibitor is the fallback everywhere else. `desktops`
/// holds the values of [`DESKTOP_VARS`] that are set; none set counts as
/// GNOME, since an environment that names no desktop is no evidence against it.
pub fn hold_command<S: AsRef<str>>(desktops: &[S], gnome_inhibit_on_path: bool) -> HoldCommand {
    // No variable at all means the board's environment says nothing (a tmux
    // server started over ssh), not that the desktop is not GNOME, so the
    // helper is tried when installed.
    let on_gnome = desktops.is_empty() || desktops.iter().any(|d| names_gnome(d.as_ref()));
    let (program, flags): (&str, Vec<&str>) = if on_gnome && gnome_inhibit_on_path {
        (
            "gnome-session-inhibit",
            vec!["--inhibit", "idle:suspend", "--reason", REASON],
        )
    } else {
        (
            "systemd-inhibit",
            vec![
                "--what=idle:sleep",
                "--who=dispatch",
                "--mode=block",
                WHY_FLAG,
            ],
        )
    };
    HoldCommand {
        program: program.to_string(),
        args: flags
            .into_iter()
            .chain(INNER.iter().copied())
            .map(str::to_string)
            .collect(),
    }
}

/// The helper for this machine, read from its environment.
///
/// Memoized: neither the desktop nor the installed helpers change over the
/// life of the board, and the PATH scan is filesystem work that must not
/// repeat on the async command path (docs/conventions.md).
pub fn hold_command_for_this_machine() -> &'static HoldCommand {
    static CACHED: std::sync::OnceLock<HoldCommand> = std::sync::OnceLock::new();
    CACHED.get_or_init(|| {
        let desktops = desktop_vars(|name| std::env::var(name).ok());
        hold_command(
            &desktops,
            crate::process::program_on_path("gnome-session-inhibit"),
        )
    })
}

/// A requested hold. Dropping it releases the hold.
#[derive(Debug)]
pub struct Hold {
    _stdin: Option<ChildStdin>,
}

/// Start the helper and report what happens to it through `on_event`:
/// at most one [`HoldEvent::Granted`], then exactly one
/// [`HoldEvent::Refused`] or [`HoldEvent::Ended`].
///
/// Must be called inside a tokio runtime.
pub fn start(command: &HoldCommand, on_event: impl Fn(HoldEvent) + Send + Sync + 'static) -> Hold {
    let spawned = Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            on_event(HoldEvent::Refused(format!(
                "could not start {}: {e}",
                command.program
            )));
            return Hold { _stdin: None };
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        let mut out_lines = stdout.map(|out| BufReader::new(out).lines());
        let mut err = stderr;
        let mut err_text = Vec::new();
        let mut granted = false;
        let mut buf = [0u8; 1024];
        // Read until the helper exits, not until its pipes close: a process
        // the helper started (the inner command, or a grandchild after the
        // helper dies alone) can keep the pipes open, and the end of the
        // hold must not wait for it (keep-awake.allium: BadgeMeansHeld).
        let status = loop {
            tokio::select! {
                line = next_line(&mut out_lines), if out_lines.is_some() => match line {
                    Some(line) => note_line(&line, &mut granted, &on_event),
                    None => out_lines = None,
                },
                n = read_some(&mut err, &mut buf), if err.is_some() => match n {
                    Some(n) => err_text.extend_from_slice(&buf[..n]),
                    None => err = None,
                },
                status = child.wait() => break status,
            }
        };
        // Whatever the helper wrote before it exited is already in the
        // pipes; take it without waiting for an end of file that may never
        // come.
        while let Some(line) = now_or_never(next_line(&mut out_lines)).await.flatten() {
            note_line(&line, &mut granted, &on_event);
        }
        while let Some(n) = now_or_never(read_some(&mut err, &mut buf)).await.flatten() {
            err_text.extend_from_slice(&buf[..n]);
        }
        let reason = describe_exit(&String::from_utf8_lossy(&err_text), status);
        on_event(if granted {
            HoldEvent::Ended(reason)
        } else {
            HoldEvent::Refused(reason)
        });
    });
    Hold { _stdin: stdin }
}

/// The future's value if it is ready on its first poll, else `None`.
async fn now_or_never<T>(fut: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        v = fut => Some(v),
        _ = std::future::ready(()) => None,
    }
}

/// The next stdout line, or `None` at the end of the stream, on a read error
/// or when there is no stream.
async fn next_line(lines: &mut Option<Lines<BufReader<ChildStdout>>>) -> Option<String> {
    lines.as_mut()?.next_line().await.ok().flatten()
}

/// Some stderr bytes, or `None` at the end of the stream, on a read error
/// or when there is no stream. A read error only loses the reason text,
/// never the event.
async fn read_some(err: &mut Option<ChildStderr>, buf: &mut [u8]) -> Option<usize> {
    match err.as_mut()?.read(buf).await {
        Ok(0) | Err(_) => None,
        Ok(n) => Some(n),
    }
}

/// Report [`HoldEvent::Granted`] on the first ready line.
fn note_line(line: &str, granted: &mut bool, on_event: &impl Fn(HoldEvent)) {
    if !*granted && line.trim() == READY {
        *granted = true;
        on_event(HoldEvent::Granted);
    }
}

/// The helper's last stderr line if it said anything, else its exit status.
fn describe_exit(stderr: &str, status: std::io::Result<std::process::ExitStatus>) -> String {
    if let Some(line) = stderr.lines().rev().find(|l| !l.trim().is_empty()) {
        return line.trim().to_string();
    }
    match status {
        Ok(s) => format!("helper exited ({s})"),
        Err(e) => format!("helper could not be waited on: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn inner() -> Vec<String> {
        INNER.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_inner_command_prints_the_ready_line() {
        assert!(INNER[2].starts_with(&format!("echo {READY};")), "{INNER:?}");
    }

    #[test]
    fn the_why_flag_carries_the_reason() {
        assert_eq!(WHY_FLAG, format!("--why={REASON}"));
    }

    fn sh(script: &str) -> HoldCommand {
        HoldCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
        }
    }

    fn start_with_channel(cmd: &HoldCommand) -> (Hold, mpsc::UnboundedReceiver<HoldEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let hold = start(cmd, move |e| {
            let _ = tx.send(e);
        });
        (hold, rx)
    }

    #[tokio::test]
    async fn ready_is_granted_and_dropping_the_hold_ends_it() {
        let (hold, mut rx) = start_with_channel(&sh("echo ready; exec cat"));
        assert_eq!(rx.recv().await, Some(HoldEvent::Granted));
        drop(hold);
        assert!(matches!(rx.recv().await, Some(HoldEvent::Ended(_))));
        assert_eq!(rx.recv().await, None, "no event after the end");
    }

    #[tokio::test]
    async fn exit_before_ready_is_refused_with_the_helpers_reason() {
        let (_hold, mut rx) = start_with_channel(&sh("echo 'no session bus' >&2; exit 1"));
        assert_eq!(
            rx.recv().await,
            Some(HoldEvent::Refused("no session bus".to_string()))
        );
    }

    #[tokio::test]
    async fn exit_after_ready_is_an_end() {
        let (_hold, mut rx) = start_with_channel(&sh("echo ready; echo 'went away' >&2; exit 0"));
        assert_eq!(rx.recv().await, Some(HoldEvent::Granted));
        assert_eq!(
            rx.recv().await,
            Some(HoldEvent::Ended("went away".to_string()))
        );
    }

    #[tokio::test]
    async fn a_silent_exit_names_the_exit_status() {
        let (_hold, mut rx) = start_with_channel(&sh("exit 3"));
        match rx.recv().await {
            Some(HoldEvent::Refused(reason)) => assert!(reason.contains('3'), "got {reason:?}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_helper_is_refused() {
        let cmd = HoldCommand {
            program: "dispatch-no-such-inhibit-helper".to_string(),
            args: vec![],
        };
        let (_hold, mut rx) = start_with_channel(&cmd);
        match rx.recv().await {
            Some(HoldEvent::Refused(reason)) => {
                assert!(reason.contains("could not start"), "got {reason:?}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    // T1: a helper that dies while a grandchild keeps its pipes open.
    #[tokio::test]
    async fn the_helper_exiting_ends_the_hold_even_if_a_grandchild_holds_the_pipes() {
        let (hold, mut rx) =
            start_with_channel(&sh("echo ready; echo 'helper died' >&2; cat <&0 & exit 0"));
        let guard = std::time::Duration::from_secs(20);
        assert_eq!(
            tokio::time::timeout(guard, rx.recv()).await.ok().flatten(),
            Some(HoldEvent::Granted)
        );
        assert_eq!(
            tokio::time::timeout(guard, rx.recv()).await.ok().flatten(),
            Some(HoldEvent::Ended("helper died".to_string())),
            "the end must not wait for the grandchild to close the pipes"
        );
        drop(hold); // closes the grandchild's stdin, so it exits too
    }

    #[tokio::test]
    async fn the_helper_exiting_before_ready_is_refused_even_if_a_grandchild_holds_the_pipes() {
        let (hold, mut rx) = start_with_channel(&sh("cat <&0 & exit 1"));
        let event = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
            .await
            .ok()
            .flatten();
        assert!(
            matches!(event, Some(HoldEvent::Refused(_))),
            "got {event:?}"
        );
        drop(hold);
    }

    // T2: the desktop is read from every variable that names it.
    #[test]
    fn gnome_is_found_in_any_desktop_variable() {
        assert_eq!(
            hold_command(&["gnome-xorg"], true).program,
            "gnome-session-inhibit"
        );
        assert_eq!(
            hold_command(&["KDE", "gnome"], true).program,
            "gnome-session-inhibit"
        );
    }

    #[test]
    fn the_desktop_variables_are_read_in_order_of_presence() {
        let vars = desktop_vars(|name| match name {
            "XDG_SESSION_DESKTOP" => Some("gnome".to_string()),
            "DESKTOP_SESSION" => Some("gnome-xorg".to_string()),
            _ => None,
        });
        assert_eq!(vars, vec!["gnome".to_string(), "gnome-xorg".to_string()]);
    }

    #[test]
    fn gnome_uses_the_session_manager_for_idle_and_suspend() {
        let cmd = hold_command(&["GNOME"], true);
        assert_eq!(cmd.program, "gnome-session-inhibit");
        assert!(cmd.args.contains(&"idle:suspend".to_string()), "{cmd:?}");
        assert!(cmd.args.ends_with(&inner()));
    }

    #[test]
    fn gnome_variants_in_a_desktop_list_count_as_gnome() {
        assert_eq!(
            hold_command(&["ubuntu:GNOME"], true).program,
            "gnome-session-inhibit"
        );
    }

    #[test]
    fn gnome_without_its_helper_falls_back_to_systemd() {
        assert_eq!(hold_command(&["GNOME"], false).program, "systemd-inhibit");
    }

    #[test]
    fn other_desktops_use_systemd_for_idle_and_sleep() {
        let cmd = hold_command(&["KDE"], true);
        assert_eq!(cmd.program, "systemd-inhibit");
        assert!(
            cmd.args.contains(&"--what=idle:sleep".to_string()),
            "{cmd:?}"
        );
        assert!(cmd.args.ends_with(&inner()));
    }

    /// A tmux server started over ssh names no desktop at all. The GNOME
    /// helper is then tried when installed, rather than the fallback.
    #[test]
    fn an_unknown_desktop_tries_the_gnome_helper_when_installed() {
        assert_eq!(
            hold_command::<&str>(&[], true).program,
            "gnome-session-inhibit"
        );
        assert_eq!(hold_command::<&str>(&[], false).program, "systemd-inhibit");
    }
}
