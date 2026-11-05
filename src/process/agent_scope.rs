//! The shared memory cap every dispatched agent runs under — see
//! `AgentsShareOneMemoryCap` in docs/specs/dispatch.allium.
//!
//! Every agent's `claude` command runs in its own transient systemd user scope
//! inside one slice, so all agents' builds count against one cap. When the cap
//! is reached the kernel kills the largest process in the slice — in practice a
//! linker or rustc — and `OOMPolicy=continue` keeps the rest of that agent's
//! scope, its session included, alive.

use std::time::Duration;

use super::ProcessRunner;

/// How long one `systemctl --user` call may take before the launch gives up on
/// the cap and proceeds unwrapped. Short: it runs on every agent launch, and a
/// local service manager answers in milliseconds.
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest `prepare_agent_scope` can run: the probe, the cap read and the
/// cap write.
pub const PREPARE_SCOPE_WORST_CASE: Duration = Duration::from_secs(30);

/// `dispatch.allium: config.agent_slice`.
pub const AGENT_SLICE: &str = "dispatch-agents.slice";

/// The cap the slice gets when none is set, in `systemctl set-property` form:
/// `config.agent_slice_memory_high`, `config.agent_slice_memory_max` and
/// `config.agent_slice_swap_max`. Percentages are of physical memory.
const DEFAULT_CAP: [&str; 3] = ["MemoryHigh=80%", "MemoryMax=85%", "MemorySwapMax=2G"];

/// What `systemctl show -p MemoryMax --value` prints for a slice with no cap.
const NO_CAP: &str = "infinity";

/// A systemd user scope inside the agents' shared slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentScope {
    slice: String,
}

impl AgentScope {
    pub fn new(slice: impl Into<String>) -> Self {
        Self {
            slice: slice.into(),
        }
    }

    /// `command` (one shell command line, as sent to the agent's pane) run in a
    /// fresh scope inside this slice. `--collect` unloads the scope even when
    /// its command fails, so a crashed agent leaves no failed unit behind.
    ///
    /// The pane's shell can see another user bus than the board did (a tmux
    /// server started over ssh or from a TTY), and `systemd-run` may be absent
    /// from its `PATH`. So the pane probes first and runs the command bare when
    /// the probe fails: a window must never be left with no agent in it. An
    /// `if`, not `A || B`, so a failing agent is never run a second time. The
    /// command travels as `$1` of a POSIX `sh`, which also keeps the line valid
    /// in whatever shell the pane runs.
    pub fn wrap(&self, command: &str) -> String {
        let script = format!(
            "if systemd-run --user --scope --quiet -- true >/dev/null 2>&1; then \
             exec systemd-run --user --scope --quiet --collect --slice={} \
             -p OOMPolicy=continue -- sh -c \"$1\"; \
             else exec sh -c \"$1\"; fi",
            self.slice
        );
        format!("sh -c '{script}' sh {}", super::shell_quote(command))
    }
}

/// Make sure the agents' slice carries a memory cap, and hand back the scope a
/// launch should run in — or `None` to launch unwrapped, when the user service
/// manager cannot be reached or the cap cannot be read or written.
///
/// A cap that is already set, by an earlier launch or by the operator, is kept.
pub fn prepare_agent_scope<R: ProcessRunner + ?Sized>(runner: &R) -> Option<AgentScope> {
    probe_scope(runner)?;
    let shown = systemctl(
        runner,
        &["--user", "show", AGENT_SLICE, "-p", "MemoryMax", "--value"],
    )?;
    if shown.trim() == NO_CAP {
        let mut args = vec!["--user", "set-property", "--runtime", AGENT_SLICE];
        args.extend(DEFAULT_CAP);
        systemctl(runner, &args)?;
    }
    Some(AgentScope::new(AGENT_SLICE))
}

/// Start a throwaway scope, the way a launch would. `systemctl` and
/// `systemd-run` can disagree about the bus, and `systemd-run` may not be
/// installed, so reading the slice is no proof that a scope can start.
fn probe_scope<R: ProcessRunner + ?Sized>(runner: &R) -> Option<()> {
    let args = ["--user", "--scope", "--quiet", "--", "true"];
    match runner.run_with_timeout("systemd-run", &args, SYSTEMCTL_TIMEOUT) {
        Ok(out) if out.status.success() => Some(()),
        other => {
            let why = match other {
                Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
                Err(e) => e.to_string(),
            };
            tracing::warn!(%why, "systemd-run cannot start a scope; launching the agent without the shared memory cap");
            None
        }
    }
}

/// Run `systemctl` and return its stdout, or log why not and return `None`.
fn systemctl<R: ProcessRunner + ?Sized>(runner: &R, args: &[&str]) -> Option<String> {
    match runner.run_with_timeout("systemctl", args, SYSTEMCTL_TIMEOUT) {
        Ok(out) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            tracing::warn!(
                ?args,
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "systemctl failed; launching the agent without the shared memory cap"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                ?args,
                error = %e,
                "systemctl could not run; launching the agent without the shared memory cap"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
