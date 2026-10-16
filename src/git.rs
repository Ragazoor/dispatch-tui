//! Small git plumbing helpers shared across the crate.

use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};

use crate::process::{stderr_str, ProcessRunner, SUBPROCESS_TIMEOUT};

/// Run `git -C <dir> <args…>`, bounded by `timeout`.
///
/// The one place the crate spawns git against a repository, as
/// `crate::tmux` is for tmux. It only spawns: the exit status is the caller's
/// to read, because several callers branch on specific codes (`ls-remote
/// --exit-code`, `merge-base --is-ancestor`, `diff --no-index`). Callers that
/// treat any non-zero exit as failure use [`git_checked`] or [`run_git`].
pub(crate) fn git_in(
    runner: &dyn ProcessRunner,
    dir: &str,
    args: &[&str],
    timeout: Duration,
) -> anyhow::Result<Output> {
    runner.run_with_timeout("git", &with_dir(dir, args), timeout)
}

/// [`git_in`] with no timeout.
///
/// Only teardown's `git worktree remove --force` uses it: removing a large
/// worktree can legitimately outlast any bound, and a killed remove would
/// leave half a tree for the directory delete behind it to finish anyway.
/// Everything else is bounded.
pub(crate) fn git_in_unbounded(
    runner: &dyn ProcessRunner,
    dir: &str,
    args: &[&str],
) -> anyhow::Result<Output> {
    runner.run("git", &with_dir(dir, args))
}

fn with_dir<'a>(dir: &'a str, args: &[&'a str]) -> Vec<&'a str> {
    let mut argv = Vec::with_capacity(args.len() + 2);
    argv.extend_from_slice(&["-C", dir]);
    argv.extend_from_slice(args);
    argv
}

/// Why a [`git_checked`] call did not succeed.
///
/// Two variants because callers word the two differently: a git that never
/// ran (missing from `PATH`, killed at its timeout) names the spawn error,
/// while one that ran and refused names git's own stderr — and some callers
/// read the refused run's stdout or follow it with a status read.
#[derive(Debug)]
pub(crate) enum GitFailure {
    /// git could not be spawned, or overran its timeout.
    Spawn(anyhow::Error),
    /// git ran and exited non-zero.
    Exit(Output),
}

impl GitFailure {
    /// The spawn error's text, or git's own stderr — for callers that word
    /// both failures the same way.
    pub(crate) fn detail(&self) -> String {
        match self {
            GitFailure::Spawn(e) => e.to_string(),
            GitFailure::Exit(output) => stderr_str(output),
        }
    }
}

/// The context every agent-tree read attaches to a git that could not be
/// spawned or overran its timeout. One literal, so the pane's border cannot
/// word the same failure two ways.
pub(crate) const COULD_NOT_RUN_GIT: &str = "could not run git";

/// [`git_in`], with a non-zero exit turned into [`GitFailure::Exit`].
pub(crate) fn git_checked(
    runner: &dyn ProcessRunner,
    dir: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, GitFailure> {
    let output = git_in(runner, dir, args, timeout).map_err(GitFailure::Spawn)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(GitFailure::Exit(output))
    }
}

/// [`git_checked`] for callers that want stdout or a one-line error: the
/// failure is "could not run git" with the spawn error as its cause, or
/// [`git_error`] for a non-zero exit.
///
/// Stdout is returned untrimmed. [`crate::process::stdout_str`] trims the whole
/// buffer, which would eat a leading space off the first `-z` path; the
/// agent-tree pane's commands emit NUL-delimited records where every byte
/// between delimiters belongs to the filename.
pub(crate) fn run_git(
    runner: &dyn ProcessRunner,
    dir: &str,
    args: &[&str],
    timeout: Duration,
) -> anyhow::Result<String> {
    match git_checked(runner, dir, args, timeout) {
        Ok(output) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        Err(GitFailure::Spawn(e)) => Err(e).context(COULD_NOT_RUN_GIT),
        Err(GitFailure::Exit(output)) => Err(git_error(&output)),
    }
}

/// Git's own first line of stderr, as an error. That line is what reaches the
/// agent-tree pane's border, so every caller needs it to say the same kind of
/// thing.
pub(crate) fn git_error(output: &Output) -> anyhow::Error {
    let stderr = stderr_str(output);
    let detail = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("git failed");
    anyhow!("git: {detail}")
}

/// Best-effort `git -C <dir> <operation> --abort`, for a failed `rebase` or
/// `merge`. The outcome is ignored: the caller is already reporting the
/// failure that made the abort necessary, and that is the error worth
/// surfacing.
pub(crate) fn abort(runner: &dyn ProcessRunner, dir: &str, operation: &str, timeout: Duration) {
    let _ = git_in(runner, dir, &[operation, "--abort"], timeout);
}

/// Abort a `rebase` or `merge` that stopped on a conflict, returning the
/// conflicted paths.
///
/// The paths are read from `git status --porcelain` *before* the abort,
/// because the abort clears them (`ConflictFilesCapturedBeforeAbort`). The
/// order is the whole point of this helper: both conflict paths share it so
/// neither can reorder it. A status read that fails yields no paths; the
/// abort runs regardless.
pub(crate) fn abort_after_conflict(
    runner: &dyn ProcessRunner,
    dir: &str,
    operation: &str,
    timeout: Duration,
) -> Vec<String> {
    let conflicted = git_in(runner, dir, &["status", "--porcelain"], timeout)
        .map(|output| parse_unmerged_files(&output))
        .unwrap_or_default();
    abort(runner, dir, operation, timeout);
    conflicted
}

/// Best-effort `git -C <repo> worktree prune`. Prune drops only admin records
/// whose directory is gone, so it can never reach a live worktree; both
/// callers run it as a repair beside a step whose own error is the one worth
/// reporting, so its outcome is ignored.
pub(crate) fn prune_worktrees(runner: &dyn ProcessRunner, repo: &str, timeout: Duration) {
    let _ = git_in(runner, repo, &["worktree", "prune"], timeout);
}

/// Detect the default branch for a repo by inspecting `origin/HEAD`.
///
/// Falls back to `"main"` when the remote ref is missing or the command
/// fails (no remote, fresh clone without `git remote set-head`, etc.).
pub fn detect_default_branch(repo_path: &str, runner: &dyn ProcessRunner) -> String {
    if let Ok(output) = git_in(
        runner,
        repo_path,
        &["symbolic-ref", "refs/remotes/origin/HEAD"],
        SUBPROCESS_TIMEOUT,
    ) {
        if output.status.success() {
            let refname = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // e.g. "refs/remotes/origin/master" → "master"
            if let Some(branch) = refname.rsplit('/').next() {
                if !branch.is_empty() {
                    return branch.to_string();
                }
            }
        }
    }
    "main".to_string()
}

/// [`detect_default_branch`] off the async event loop.
///
/// It shells out synchronously, so every async caller has to hand it to the
/// blocking pool — and each then has to decide what a `JoinError` means. Three
/// callers made that decision three different ways before this existed. The
/// answer is the same one the sync helper already gives when it cannot read
/// the repo: fall back to the configured default. A join failure means the
/// question was never asked, which is not more informative than asking and
/// getting no answer.
pub async fn detect_default_branch_async(
    repo_path: String,
    runner: Arc<dyn ProcessRunner>,
) -> String {
    tokio::task::spawn_blocking(move || detect_default_branch(&repo_path, &*runner))
        .await
        .unwrap_or_else(|_| crate::models::DEFAULT_BASE_BRANCH.to_string())
}

/// The remote-tracking ref for a branch name: `origin/<base_branch>`.
///
/// The single place the crate decides that the remote is called `origin`.
/// Everything that reaches for a base branch's remote counterpart goes through
/// here — provisioning a worktree's start point
/// (`crate::dispatch::worktree`), measuring drift and merging
/// ([`crate::repo_sync`]), and resolving the agent tree's diff baseline
/// (`crate::agent_tree::changes`).
///
/// Being one definition is load-bearing, not tidiness. Two of those callers
/// must agree on which ref a worktree was branched from: dispatch picks the
/// start point, and the agent tree measures against it. Task #4539 was the bug
/// where they disagreed, and a second hardcoded `origin/` is how that
/// disagreement comes back. Making the remote name configurable is a separate,
/// whole-system decision — this helper is what would make it a one-line one.
pub fn origin_ref(base_branch: &str) -> String {
    format!("origin/{base_branch}")
}

/// Whether the repo has an `origin` remote configured.
///
/// Three outcomes, not two: `Ok(true)` and `Ok(false)` are the probe's own
/// answers, while `Err` means the probe could not be run at all and so answered
/// nothing. Collapsing the third into `Ok(false)` here would report "no origin
/// remote configured" as a positive finding on the strength of a failure to
/// look — which is exactly the wrong direction for callers that branch on
/// absence.
///
/// Callers decide what each outcome *means*, and all three answer differently:
///
/// - `crate::dispatch::finish::finish_task` skips its pull on `Ok(false)` but
///   fails outright on `Err`, because a git it cannot spawn is a real failure it
///   should name rather than rebase past.
/// - `classify_fetch_failure` (`src/dispatch/worktree.rs`) grants the
///   local-branch fallback only on `Ok(false)`; an `Err` is unreachable-origin,
///   since a failure to look identifies nothing.
/// - [`crate::repo_sync::sync_repo`] deliberately treats both as `NoRemote` —
///   for that operation "nothing to sync against" is the same fact either way,
///   a carve-out stated in `docs/specs/repo-sync.allium` under
///   `PreconditionsPrecedeEveryWrite`.
pub(crate) fn has_origin_remote(
    repo_path: &str,
    runner: &dyn ProcessRunner,
) -> std::result::Result<bool, String> {
    git_in(
        runner,
        repo_path,
        &["remote", "get-url", "origin"],
        SUBPROCESS_TIMEOUT,
    )
    .map(|o| o.status.success())
    .map_err(|e| format!("Failed to check for an origin remote: {e}"))
}

/// The repo's currently checked-out branch name.
///
/// One of the three preflight reads shared by the rebase path
/// (`crate::dispatch::finish::finish_task`) and the repo-sync path
/// ([`crate::repo_sync::sync_repo`]). Both need to know they are on the base
/// branch before writing, because rebase, merge and push all act on whatever is
/// checked out. Returns the branch rather than a yes/no so each caller can name
/// the actual branch in its own error variant.
pub(crate) fn current_branch(
    repo_path: &str,
    runner: &dyn ProcessRunner,
) -> std::result::Result<String, String> {
    git_in(
        runner,
        repo_path,
        &["rev-parse", "--abbrev-ref", "HEAD"],
        SUBPROCESS_TIMEOUT,
    )
    .map(|output| crate::process::stdout_str(&output))
    .map_err(|e| format!("Failed to check current branch: {e}"))
}

/// Every dirty or untracked path in the repo's working tree, empty when clean.
///
/// The second shared preflight read: both the rebase and the repo-sync path
/// refuse to touch a dirty checkout, because rebasing or merging into one is
/// how work gets lost. Returns the paths so each caller can list them in its
/// own error variant.
pub(crate) fn dirty_files(
    repo_path: &str,
    runner: &dyn ProcessRunner,
) -> std::result::Result<Vec<String>, String> {
    git_in(
        runner,
        repo_path,
        &["status", "--porcelain"],
        SUBPROCESS_TIMEOUT,
    )
    .map(|output| parse_porcelain_files(&output))
    .map_err(|e| format!("Failed to check working tree status: {e}"))
}

/// Splits every `git status --porcelain` line into its two-character status
/// code and the path that follows (after the status code and its separating
/// space). Operates on the raw `Output` rather than a pre-trimmed string:
/// the leading status-code column can itself be a space (e.g. `" M"`), which
/// a whole-buffer `.trim()` on the first line would incorrectly eat.
fn porcelain_entries(output: &Output) -> Vec<(String, String)> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.len() >= 3)
        .map(|line| (line[0..2].to_string(), line[3..].trim_end().to_string()))
        .collect()
}

/// Every dirty/untracked path from a `git status --porcelain` run.
///
/// Shared by the rebase path (`crate::dispatch::finish::finish_task`) and the
/// repo-sync path ([`crate::repo_sync::sync_repo`]) so that "is this checkout
/// dirty?" has exactly one answer.
pub(crate) fn parse_porcelain_files(output: &Output) -> Vec<String> {
    porcelain_entries(output)
        .into_iter()
        .map(|(_, path)| path)
        .collect()
}

/// Just the unmerged (conflicted) paths from a `git status --porcelain` run
/// — status codes `UU`, `AA`, `DD`, or any code containing `U` (added/deleted
/// by us/them). Structural and locale-independent, unlike parsing conflict
/// file names out of rebase's English stdout/stderr prose (which breaks on
/// rename/delete conflicts, whose message doesn't end in "... in <path>").
///
/// Shared by the rebase path and the repo-sync merge path, so conflict
/// detection is not duplicated.
pub(crate) fn parse_unmerged_files(output: &Output) -> Vec<String> {
    porcelain_entries(output)
        .into_iter()
        .filter(|(code, _)| code == "AA" || code == "DD" || code.contains('U'))
        .map(|(_, path)| path)
        .collect()
}

#[cfg(test)]
mod tests;
