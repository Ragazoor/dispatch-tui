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
    pub fn wrap(&self, command: &str) -> String {
        format!(
            "systemd-run --user --scope --quiet --collect --slice={} \
             -p OOMPolicy=continue -- {command}",
            self.slice
        )
    }
}

/// Make sure the agents' slice carries a memory cap, and hand back the scope a
/// launch should run in — or `None` to launch unwrapped, when the user service
/// manager cannot be reached or the cap cannot be read or written.
///
/// A cap that is already set, by an earlier launch or by the operator, is kept.
pub fn prepare_agent_scope<R: ProcessRunner + ?Sized>(runner: &R) -> Option<AgentScope> {
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
