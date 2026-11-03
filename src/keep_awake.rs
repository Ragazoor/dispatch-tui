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

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStdin, Command};

/// The line the inner command prints once the helper has taken the hold.
pub const READY: &str = "ready";

/// The inner command the helper runs while it holds the machine awake.
const INNER: &[&str] = &["sh", "-c", "echo ready; exec cat"];

/// Why the hold was asked for, as the helpers show it to the desktop.
const REASON: &str = "dispatch keep-awake";

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

/// Choose the helper. The GNOME session manager is preferred on GNOME,
/// because GNOME honours its inhibitors for screen lock and blanking; the
/// generic systemd inhibitor is the fallback everywhere else.
pub fn hold_command(desktop: Option<&str>, gnome_inhibit_on_path: bool) -> HoldCommand {
    let on_gnome = desktop.is_some_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("GNOME")));
    let why = format!("--why={REASON}");
    let (program, flags): (&str, Vec<&str>) = if on_gnome && gnome_inhibit_on_path {
        (
            "gnome-session-inhibit",
            vec!["--inhibit", "idle:suspend", "--reason", REASON],
        )
    } else {
        (
            "systemd-inhibit",
            vec!["--what=idle:sleep", "--who=dispatch", "--mode=block", &why],
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
pub fn hold_command_for_this_machine() -> HoldCommand {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    hold_command(
        desktop.as_deref(),
        crate::process::program_on_path("gnome-session-inhibit"),
    )
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
        let read_stdout = async {
            let mut granted = false;
            if let Some(out) = stdout {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !granted && line.trim() == READY {
                        granted = true;
                        on_event(HoldEvent::Granted);
                    }
                }
            }
            granted
        };
        let read_stderr = async {
            let mut text = String::new();
            if let Some(mut err) = stderr {
                // A read error only loses the reason text, never the event.
                let _ = err.read_to_string(&mut text).await;
            }
            text
        };
        let (granted, err_text) = tokio::join!(read_stdout, read_stderr);
        let status = child.wait().await;
        let reason = describe_exit(&err_text, status);
        on_event(if granted {
            HoldEvent::Ended(reason)
        } else {
            HoldEvent::Refused(reason)
        });
    });
    Hold { _stdin: stdin }
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

    #[test]
    fn gnome_uses_the_session_manager_for_idle_and_suspend() {
        let cmd = hold_command(Some("GNOME"), true);
        assert_eq!(cmd.program, "gnome-session-inhibit");
        assert!(cmd.args.contains(&"idle:suspend".to_string()), "{cmd:?}");
        assert!(cmd
            .args
            .ends_with(&INNER.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
    }

    #[test]
    fn gnome_variants_in_a_desktop_list_count_as_gnome() {
        assert_eq!(
            hold_command(Some("ubuntu:GNOME"), true).program,
            "gnome-session-inhibit"
        );
    }

    #[test]
    fn gnome_without_its_helper_falls_back_to_systemd() {
        assert_eq!(
            hold_command(Some("GNOME"), false).program,
            "systemd-inhibit"
        );
    }

    #[test]
    fn other_desktops_use_systemd_for_idle_and_sleep() {
        let cmd = hold_command(Some("KDE"), true);
        assert_eq!(cmd.program, "systemd-inhibit");
        assert!(
            cmd.args.contains(&"--what=idle:sleep".to_string()),
            "{cmd:?}"
        );
        assert!(cmd
            .args
            .ends_with(&INNER.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
        assert_eq!(hold_command(None, true).program, "systemd-inhibit");
    }
}
