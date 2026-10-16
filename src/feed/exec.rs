use std::collections::HashMap;
use std::time::Duration;

use crate::git::detect_default_branch;
use crate::process::ProcessRunner;

/// Resolve a base branch for each `repo_paths[i]`, caching by unique path so
/// `git symbolic-ref` is invoked at most once per distinct repo. Empty paths
/// (unresolved repos) get `"main"` without shelling out.
pub(crate) fn resolve_base_branches(
    repo_paths: &[String],
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let mut cache: HashMap<&str, String> = HashMap::new();
    repo_paths
        .iter()
        .map(|path| {
            cache
                .entry(path.as_str())
                .or_insert_with(|| {
                    if path.is_empty() {
                        "main".to_string()
                    } else {
                        detect_default_branch(path, runner)
                    }
                })
                .clone()
        })
        .collect()
}

/// A `ProcessRunner` that always fails — used in tests that only need the
/// `"main"` fallback and don't want to set up git subprocess stubs.
#[cfg(test)]
pub(super) struct AlwaysFailRunner;

#[cfg(test)]
impl ProcessRunner for AlwaysFailRunner {
    fn run(&self, _: &str, _: &[&str]) -> anyhow::Result<std::process::Output> {
        crate::process::MockProcessRunner::fail("not a git repo")
    }
}

/// Upper bound on how long [`exec_feed_command`]'s child process may run
/// before it is killed and reaped. See core.allium config `feed_command_timeout`
/// and feeds.allium `FeedCommandFailure` bucket 7 / `SerialisedFeedCycle`'s
/// bounded-cost note.
pub(crate) const FEED_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Maximum number of characters of a feed command's stderr that are logged and
/// carried on [`FeedOutput`]. A verbose script must not be able to write
/// unbounded output into `app.log` on every poll.
pub(crate) const MAX_FEED_STDERR_CHARS: usize = 2000;

/// A successful feed command's output.
#[derive(Debug)]
pub(crate) struct FeedOutput {
    /// Raw stdout bytes, to be parsed as a FeedItem JSON array.
    pub(crate) stdout: Vec<u8>,
    /// Anything the command wrote to stderr while still exiting 0 — trimmed
    /// and truncated to [`MAX_FEED_STDERR_CHARS`]. Empty when it wrote
    /// nothing. A non-empty value is the signature of a script that swallowed
    /// an internal error and emitted a degraded array anyway, so it is logged
    /// here and surfaced by the manual-refresh path (feeds.allium:
    /// FeedCommandStderrOnSuccess).
    pub(crate) stderr: String,
}

/// Execute the feed shell command.
///
/// On success returns stdout plus any stderr the command wrote while exiting 0,
/// logging a warning for that stderr — it is a diagnostic, not a failure, and
/// the caller syncs the emission as normal.
///
/// On spawn failure or non-zero exit, logs a warning (as before) and returns
/// `Err` with the failure text, so a caller that must surface the error to the
/// user — `exec_trigger_epic_feed`'s status bar — has it.
pub(crate) async fn exec_feed_command(
    cmd: &str,
    epic_id: i64,
    epic_title: &str,
    timeout: Duration,
) -> Result<FeedOutput, String> {
    // Bounded so a command that never exits cannot hold its epic's
    // feed-cycle claim (feeds.allium: SerialisedFeedCycle) for the life of
    // the process. `run_bounded` (src/process.rs), the repo's one
    // kill-on-timeout primitive, only covers SYNCHRONOUS `ProcessRunner`
    // children spawned via `std::process::Command` and so cannot be reused
    // here directly — `tokio::time::timeout` + `kill_on_drop(true)` is the
    // async counterpart: dropping the timed-out future drops the `Child`,
    // and `kill_on_drop` is what turns that drop into a kill (reaped by
    // tokio's orphan-reaper task) rather than an orphaned process.
    let result = tokio::time::timeout(
        timeout,
        tokio::process::Command::new("sh")
            .args(["-c", cmd])
            .kill_on_drop(true)
            .output(),
    )
    .await;

    let output = match result {
        Ok(Ok(o)) => o,
        Ok(Err(err)) => {
            let msg = format!("{err:#}");
            tracing::warn!(epic_id, epic_title, "feed: failed to spawn command: {msg}");
            return Err(msg);
        }
        Err(_elapsed) => {
            let msg = format!("command timed out after {timeout:?}");
            tracing::warn!(epic_id, epic_title, "feed: {msg}");
            return Err(msg);
        }
    };

    let stderr = truncate_stderr(&output.stderr);

    if !output.status.success() {
        tracing::warn!(
            epic_id,
            epic_title,
            "feed: command exited non-zero: {stderr}"
        );
        return Err(stderr);
    }

    if !stderr.is_empty() {
        tracing::warn!(
            epic_id,
            epic_title,
            "feed: command wrote to stderr but exited 0: {stderr}"
        );
    }

    Ok(FeedOutput {
        stdout: output.stdout,
        stderr,
    })
}

/// Classify a zero-exit emission that produced no items while writing to
/// stderr. That combination is the signature of a script which failed
/// internally and soft-failed to an empty array — syncing it would delete
/// every feed task in the epic's subtree.
///
/// Returns `Some(reason)` to suppress the sync, `None` to proceed.
///
/// A genuinely-empty clean run (`stderr` empty) returns `None` and reconciles
/// normally, so merged and closed PRs are still removed. A non-empty emission
/// returns `None` here regardless of stderr — it is never suppressed, only
/// downgraded to an additive sync by [`degraded_partial_emission`].
///
/// See feeds.allium: DegradedEmptyEmission.
pub(crate) fn degraded_empty_emission(item_count: usize, stderr: &str) -> Option<String> {
    if item_count == 0 && !stderr.is_empty() {
        Some(format!(
            "command emitted no items but wrote to stderr: {stderr}"
        ))
    } else {
        None
    }
}

/// Classify a zero-exit emission that produced items while ALSO writing to
/// stderr. That combination is a PARTIALLY degraded emission: part of what the
/// command was asked to fetch came back and part of it soft-failed, so the
/// emission is evidence of what exists but not of what does not.
///
/// Returns `Some(reason)` to downgrade the sync to [`SyncMode::Additive`],
/// `None` to reconcile as normal.
///
/// Deliberately the same shape as [`degraded_empty_emission`], and mutually
/// exclusive with it by the `item_count` arm — the empty case suppresses the
/// sync entirely, this one only withholds its removals.
///
/// See feeds.allium: DegradedNonEmptyEmission.
///
/// [`SyncMode::Additive`]: super::SyncMode::Additive
pub(crate) fn degraded_partial_emission(item_count: usize, stderr: &str) -> Option<String> {
    if item_count > 0 && !stderr.is_empty() {
        Some(format!(
            "command wrote to stderr, so its omissions are not trusted: {stderr}"
        ))
    } else {
        None
    }
}

/// Decode stderr bytes lossily, trim surrounding whitespace, and cap the
/// length at [`MAX_FEED_STDERR_CHARS`]. Truncates on a character boundary, so
/// multi-byte output cannot panic here.
fn truncate_stderr(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    trimmed.chars().take(MAX_FEED_STDERR_CHARS).collect()
}

#[cfg(test)]
mod tests;
