//! The `dispatch statusline` decorator (see docs/specs/dispatch.allium:
//! TokenBudgetIndicator, ActiveModelIndicator).
//!
//! Wired as the `statusLine` command of every dispatch-spawned Claude session.
//! Records the payload's `rate_limits` and active model to a snapshot file,
//! then runs the user's previous statusLine command and prints its output
//! verbatim.
//!
//! Two hard constraints:
//!
//! 1. **Never fail.** This runs on Claude Code's 300 ms statusLine debounce. Any
//!    error must still exit 0, or the user's status line breaks.
//! 2. **Never open the database.** At several invocations per second per session,
//!    across every agent, database work here would be pure waste. This module has
//!    no `Store` import and must keep it that way.

use crate::models::budget::BudgetSnapshot;
use crate::process::run_bounded;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Wall-clock budget for the chained command, covering **both** phases: it
/// closing its stdout (i.e. finishing producing output) and it exiting. On
/// expiry it is killed and the chain yields an empty string — consistent with
/// this module's "any failure -> blank status line" philosophy. The real chained
/// command runs several `git -C` invocations, which can block on a lock, NFS,
/// or a network remote, so this bound is load-bearing, not decorative. See
/// docs/specs/observability.allium: StatusLineDecorator
/// (`@guarantee ChainedCommandIsBounded`).
const CHAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Parse the payload and atomically publish a snapshot. Returns whether a
/// snapshot was written. Never panics; every failure is a silent `false`.
pub fn record_snapshot(stdin: &str, snapshot_path: &Path, now: i64) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(stdin) else {
        return false;
    };
    let Some(snapshot) = BudgetSnapshot::from_status_payload(&payload, now) else {
        return false;
    };
    let Ok(text) = serde_json::to_string(&snapshot) else {
        return false;
    };
    write_atomically(snapshot_path, &text)
}

/// Publish `text` at `path` via a **uniquely named** temp file in the same
/// directory, then rename.
///
/// The unique name is load-bearing: every Claude session writes this same path
/// concurrently. With a fixed temp name, writer A could rename bytes that writer
/// B had truncated and partially written, publishing a torn value attributed to
/// the wrong writer. With a unique temp file, each writer only ever renames its
/// own complete bytes, so "last rename wins" is true — and since all writers
/// report the same account-global value, that is the correct outcome.
fn write_atomically(path: &Path, text: &str) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let Ok(mut file) = tempfile::NamedTempFile::new_in(dir) else {
        return false;
    };
    if file.write_all(text.as_bytes()).is_err() {
        return false;
    }
    if file.flush().is_err() {
        return false;
    }
    file.persist(path).is_ok()
}

/// Run the chained command with `stdin` on its stdin, returning its stdout.
///
/// The subprocess mechanics — the payload written concurrently with the drain,
/// and the whole of [`CHAIN_TIMEOUT`]'s bound — are
/// [`crate::process::run_bounded`]'s, and documented there. All this adds is the
/// translation of every failure into an empty string: a blank status line rather
/// than a broken one.
fn run_chain(chain: &str, stdin: &str, timeout: Duration) -> String {
    match run_bounded("sh", &["-c", chain], Some(stdin), timeout) {
        // Deliberately not conditioned on the exit status: a chain that exits
        // non-zero having printed something still had a status line to show.
        Ok(output) => String::from_utf8(output.stdout)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        Err(_) => String::new(),
    }
}

/// Decorator core: record, then chain. Always returns exit code 0.
pub fn run(stdin: &str, snapshot_path: &Path, chain: Option<&str>, now: i64) -> i32 {
    let out = run_capturing(stdin, snapshot_path, chain, now);
    print!("{out}");
    0
}

/// `run` without the side effect of printing, so tests can assert the output.
pub fn run_capturing(stdin: &str, snapshot_path: &Path, chain: Option<&str>, now: i64) -> String {
    record_snapshot(stdin, snapshot_path, now);
    match chain {
        Some(cmd) if !cmd.trim().is_empty() => run_chain(cmd, stdin, CHAIN_TIMEOUT),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;
