//! Local-first repo sync: keeping a repository's primary checkout in step with
//! origin on its own default branch.
//!
//! Spec: `docs/specs/repo-sync.allium` (the `RepoSyncEngine` contract and the
//! `SyncRepo` rule). Structured like [`crate::dispatch::finish`] — synchronous,
//! [`ProcessRunner`]-driven, with no TUI or database coupling, so the same three
//! operations back the board action and the CLI.

use std::collections::HashMap;

use crate::git::{abort_after_conflict, git_checked, GitFailure};
use crate::models::expand_tilde;
use crate::process::{stderr_str, stdout_str, ProcessRunner, SUBPROCESS_TIMEOUT};

/// A two-sided commit count between a repository's local base branch and its
/// origin counterpart: `ahead` commits are reachable only from the local
/// branch, `behind` commits only from `origin/<base>`.
///
/// The pair is always read together, from the same observation, so it is one
/// value rather than two fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AheadBehind {
    pub ahead: u32,
    pub behind: u32,
}

impl AheadBehind {
    /// Any non-zero side is drift worth surfacing.
    pub fn has_drift(&self) -> bool {
        self.ahead > 0 || self.behind > 0
    }

    /// Both sides non-zero: local and origin have each moved independently.
    /// This is the case resolved by merging rather than rebasing — the one
    /// `ReportRepoSyncFailure`'s `push_rejected` exception names, where the
    /// merge commit is already local when the push fails. `SyncRepo` handles
    /// it through the unconditional `git merge` (fast-forwards when
    /// `ahead = 0`, merge commit otherwise) rather than a branch on this
    /// value, so it stays a `derived` value on `AheadBehind`
    /// (`docs/specs/repo-sync.allium`) with no direct caller.
    #[cfg(test)]
    pub fn is_diverged(&self) -> bool {
        self.ahead > 0 && self.behind > 0
    }
}

/// The two ways a sync can succeed. Distinguishable to the caller on purpose:
/// `AlreadyInSync` means the fetch ran and found nothing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Both counts were zero (or unmeasurable) after the fetch. No merge and no
    /// push were performed.
    AlreadyInSync,
    /// Commits actually pulled into local base and pushed to origin.
    Synced { pulled: u32, pushed: u32 },
}

/// Every way a sync can fail, one variant per cause. The split is the point:
/// each cause has a different remedy and a different message, so a dirty tree
/// can never masquerade as a merge conflict.
#[derive(Debug)]
pub enum SyncError {
    /// The repository has no `origin` remote configured.
    NoRemote,
    /// The primary checkout is on some other branch. Both the merge and the
    /// push act on whatever is checked out, so this stops the operation.
    NotOnBaseBranch { current: String, expected: String },
    /// The primary checkout has uncommitted changes; merging into a dirty tree
    /// is how work is lost.
    DirtyPrimaryWorktree { path: String, files: Vec<String> },
    /// `origin/<base>` did not merge cleanly. The merge was aborted and the
    /// conflicted paths reported.
    MergeConflict { files: Vec<String> },
    /// Origin moved between the fetch and the push. Retryable as-is.
    PushRejected { stderr: String },
    /// A git invocation failed for some other reason.
    Other(String),
}

impl SyncError {
    /// A rejected push means origin moved between the fetch and the push, so
    /// repeating the same action is the fix. Every other cause needs the
    /// operator to change something first.
    pub fn retryable(&self) -> bool {
        matches!(self, SyncError::PushRejected { .. })
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::NoRemote => write!(f, "No origin remote configured — nothing to sync with"),
            SyncError::NotOnBaseBranch { current, expected } => write!(
                f,
                "Repo root is not on {expected} (currently on {current}) — checkout {expected} first"
            ),
            SyncError::DirtyPrimaryWorktree { path, files } => write!(
                f,
                "Primary worktree at {path} has uncommitted changes ({}) — commit or stash them before syncing",
                files.join(", ")
            ),
            SyncError::MergeConflict { files } => {
                let location = if files.is_empty() {
                    String::new()
                } else {
                    format!(" in {}", files.join(", "))
                };
                write!(
                    f,
                    "Merge conflict{location} — the merge was aborted; resolve it by hand and try again"
                )
            }
            SyncError::PushRejected { stderr } => write!(
                f,
                "Push rejected — origin moved since the fetch; try again: {stderr}"
            ),
            SyncError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// The `<base>...origin/<base>` range whose two-sided count is the drift.
fn count_range(base_branch: &str) -> String {
    format!("{base_branch}...{}", crate::git::origin_ref(base_branch))
}

/// Count commits on each side of `<base>...origin/<base>`.
///
/// Yields `None` — never `AheadBehind { 0, 0 }` — when `origin/<base_branch>`
/// does not resolve (no remote, never fetched) or when the output cannot be
/// parsed. A repository that cannot be measured must not be reported as clean
/// (`UnmeasurableIsNotInSync`).
///
/// Bounded by [`SUBPROCESS_TIMEOUT`] like every other subprocess here: walking
/// history can block on a lock, and this runs on the dispatch path and the TUI's
/// drift poll, neither of which may hang on it. A timed-out walk is simply an
/// unmeasurable one.
pub fn ahead_behind(
    repo_path: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Option<AheadBehind> {
    let repo_path = expand_tilde(repo_path);
    let output = git_checked(
        runner,
        &repo_path,
        &[
            "rev-list",
            "--count",
            "--left-right",
            &count_range(base_branch),
        ],
        SUBPROCESS_TIMEOUT,
    )
    .ok()?;
    let stdout = stdout_str(&output);
    let mut fields = stdout.split_whitespace();
    let ahead = fields.next()?.parse().ok()?;
    let behind = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        // More than two counts is not output this command produces; refusing to
        // guess is the whole point of the invariant.
        return None;
    }
    Some(AheadBehind { ahead, behind })
}

/// Fetch `origin/<base_branch>` so the counts that follow are trustworthy.
///
/// Yields `Ok(())` on success and the failure message otherwise; a failed fetch
/// is non-fatal everywhere it is used.
pub fn fetch_base(
    repo_path: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<(), String> {
    let repo_path = expand_tilde(repo_path);
    let detail = match git_checked(
        runner,
        &repo_path,
        &["fetch", "origin", base_branch],
        SUBPROCESS_TIMEOUT,
    ) {
        Ok(_) => return Ok(()),
        Err(failure) => failure.detail(),
    };
    Err(format!("Failed to fetch origin {base_branch}: {detail}"))
}

/// Bring the repository's primary checkout into step with `origin/<base_branch>`
/// and publish whatever it is ahead by.
///
/// Every precondition is checked before any write, each as its own error
/// variant (`PreconditionsPrecedeEveryWrite`). The fetch then runs
/// unconditionally, before the counts that decide whether to merge or push
/// (`FetchPrecedesCounting`). Divergence is closed by merging, never by
/// rebasing or resetting local base (`LocalBaseHistoryIsNeverRewritten`), so
/// worktrees already branched off it stay valid.
pub fn sync_repo(
    repo_path: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<SyncOutcome, SyncError> {
    let repo = expand_tilde(repo_path);

    check_preconditions(&repo, base_branch, runner)?;

    // --- Fetch, unconditionally, then count against the refreshed refs ---

    fetch_base(&repo, base_branch, runner).map_err(SyncError::Other)?;

    let Some(counts) = ahead_behind(&repo, base_branch, runner) else {
        // Nothing measurable to act on even against freshly fetched refs.
        return Ok(SyncOutcome::AlreadyInSync);
    };
    if !counts.has_drift() {
        return Ok(SyncOutcome::AlreadyInSync);
    }

    // --- Merge (fast-forwards when ahead = 0, merge commit when diverged) ---

    let mut ahead = counts.ahead;
    if counts.behind > 0 {
        merge_origin_base(&repo, base_branch, runner)?;
        // A merge commit is itself something to publish, so the ahead count is
        // re-read rather than reused. Unmeasurable after the merge means no
        // push: refusing to guess beats pushing a count we cannot justify.
        ahead = ahead_behind(&repo, base_branch, runner)
            .map(|c| c.ahead)
            .unwrap_or(0);
    }

    // --- Push whatever local base is ahead by ---

    if ahead > 0 {
        push_base(&repo, base_branch, runner)?;
    }

    Ok(SyncOutcome::Synced {
        pulled: counts.behind,
        pushed: ahead,
    })
}

/// The preconditions of `sync_repo`, all checked before any write.
fn check_preconditions(
    repo: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<(), SyncError> {
    // 1. An origin remote must exist. Both a probe that cannot be run and one
    //    that reports no origin mean the same thing here — nothing to sync
    //    against — so both report NoRemote rather than splitting the first into
    //    Other. Spec: PreconditionsPrecedeEveryWrite's stated carve-out.
    if !crate::git::has_origin_remote(repo, runner).unwrap_or(false) {
        return Err(SyncError::NoRemote);
    }

    // 2. The checkout must be on the base branch — the merge and the push both
    //    act on whatever is checked out.
    let current = crate::git::current_branch(repo, runner).map_err(SyncError::Other)?;
    if current != base_branch {
        return Err(SyncError::NotOnBaseBranch {
            current,
            expected: base_branch.to_string(),
        });
    }

    // 3. The checkout must be clean — merging into a dirty tree loses work.
    let dirty = crate::git::dirty_files(repo, runner).map_err(SyncError::Other)?;
    if !dirty.is_empty() {
        return Err(SyncError::DirtyPrimaryWorktree {
            path: repo.to_string(),
            files: dirty,
        });
    }
    Ok(())
}

/// Merge `origin/<base_branch>` into the checked-out base. On failure the merge
/// is aborted, so the checkout is left as it was found.
fn merge_origin_base(
    repo: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<(), SyncError> {
    let output = match git_checked(
        runner,
        repo,
        &["merge", "--no-edit", &crate::git::origin_ref(base_branch)],
        SUBPROCESS_TIMEOUT,
    ) {
        Ok(_) => return Ok(()),
        Err(GitFailure::Spawn(e)) => {
            return Err(SyncError::Other(format!("Failed to run git merge: {e}")))
        }
        Err(GitFailure::Exit(output)) => output,
    };
    // The conflicted paths are read before the abort clears them
    // (`ConflictFilesCapturedBeforeAbort`).
    let conflicted = abort_after_conflict(runner, repo, "merge", SUBPROCESS_TIMEOUT);
    if !conflicted.is_empty() {
        return Err(SyncError::MergeConflict { files: conflicted });
    }
    Err(SyncError::Other(format!(
        "Merge of origin/{base_branch} failed: {}",
        stderr_str(&output)
    )))
}

/// Push the checked-out base to `origin`.
fn push_base(repo: &str, base_branch: &str, runner: &dyn ProcessRunner) -> Result<(), SyncError> {
    match git_checked(
        runner,
        repo,
        &["push", "origin", base_branch],
        SUBPROCESS_TIMEOUT,
    ) {
        Ok(_) => Ok(()),
        Err(GitFailure::Spawn(e)) => Err(SyncError::Other(format!("Failed to run git push: {e}"))),
        Err(GitFailure::Exit(output)) => Err(SyncError::PushRejected {
            stderr: stderr_str(&output),
        }),
    }
}

// ---------------------------------------------------------------------------
// Measurement state — the `RepoSyncState` entity and its per-repo cache
// ---------------------------------------------------------------------------

/// The current drift measurement for one repository (spec: entity
/// `RepoSyncState`).
///
/// A measurement, not a record: every consumer establishes it for itself — the
/// board refreshes it on the trigger events, the CLI computes it when it runs —
/// so it crosses no process boundary and is never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSyncState {
    /// Absolute path to the repository's primary checkout; identifies the
    /// measurement.
    pub repo_path: String,
    /// The repository's own default branch, as measured.
    pub base_branch: String,
    /// `None` when `origin/<base_branch>` could not be measured at all.
    pub counts: Option<AheadBehind>,
    /// Message from the most recent failed fetch; cleared once a fetch succeeds.
    pub last_fetch_error: Option<String>,
}

impl RepoSyncState {
    /// Whether the repository could be measured. An unmeasured repository is
    /// distinct from a clean one and must never be presented as clean
    /// (`UnmeasuredIsNeverPresentedAsClean`). `dispatch repo status` honours
    /// it by matching on `counts` directly, so only the tests call this.
    #[cfg(test)]
    pub fn is_measured(&self) -> bool {
        self.counts.is_some()
    }

    /// Drift the user can act on. False both when clean and when unmeasured.
    pub fn has_drift(&self) -> bool {
        self.counts.is_some_and(|c| c.has_drift())
    }
}

/// One refresh observation, before it is folded into the cached state.
///
/// Kept apart from [`RepoSyncState`] because a failed fetch must record only its
/// error and leave the previously known counts alone — a merge the observation
/// itself cannot perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSyncMeasurement {
    pub repo_path: String,
    pub base_branch: String,
    pub counts: Option<AheadBehind>,
    pub fetch_error: Option<String>,
}

/// The per-repository measurement cache, keyed by `repo_path`.
///
/// Keying by path is what enforces `UniqueMeasurementPerRepo`: a repeated
/// refresh replaces the repository's measurement rather than adding a second.
#[derive(Debug, Default, Clone)]
pub struct RepoSyncCache(HashMap<String, RepoSyncState>);

impl RepoSyncCache {
    /// The measurement for `repo_path`, or `None` when it has never been
    /// refreshed.
    pub fn get(&self, repo_path: &str) -> Option<&RepoSyncState> {
        self.0.get(repo_path)
    }

    /// Number of repositories measured.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no repository has been measured yet.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Fold one observation in, per the `RefreshRepoSyncState` rule: create the
    /// state on a first observation; on a later one, a failed fetch records only
    /// the error (keeping the previously known counts, so a slow or offline
    /// network leaves the drift indicator undisturbed) while a successful fetch
    /// replaces the branch, the counts and clears the error.
    pub fn apply(&mut self, m: RepoSyncMeasurement) {
        match self.0.get_mut(&m.repo_path) {
            None => {
                self.0.insert(
                    m.repo_path.clone(),
                    RepoSyncState {
                        repo_path: m.repo_path,
                        base_branch: m.base_branch,
                        counts: m.counts,
                        last_fetch_error: m.fetch_error,
                    },
                );
            }
            Some(state) => {
                if m.fetch_error.is_some() {
                    state.last_fetch_error = m.fetch_error;
                } else {
                    state.base_branch = m.base_branch;
                    state.counts = m.counts;
                    state.last_fetch_error = None;
                }
            }
        }
    }
}

/// Measure one repository: resolve its own default branch, optionally fetch, and
/// read the two-sided count from the refreshed refs.
///
/// This is the measurement half of `RefreshRepoSyncState`. `fetch_first` is true
/// only for the TUI's startup refresh and for `dispatch repo status` without
/// `--no-fetch`; every other caller rides refs some other operation just
/// refreshed, making this a pure local ref read.
///
/// A failed fetch yields the error and *no* counts: counting against refs this
/// call failed to refresh is exactly what `FetchPrecedesCounting` forbids.
pub fn measure_repo(
    repo_path: &str,
    fetch_first: bool,
    runner: &dyn ProcessRunner,
) -> RepoSyncMeasurement {
    let base_branch = crate::git::detect_default_branch(repo_path, runner);
    let fetch_error = if fetch_first {
        fetch_base(repo_path, &base_branch, runner).err()
    } else {
        None
    };
    let counts = if fetch_error.is_none() {
        ahead_behind(repo_path, &base_branch, runner)
    } else {
        None
    };
    RepoSyncMeasurement {
        repo_path: repo_path.to_string(),
        base_branch,
        counts,
        fetch_error,
    }
}

#[cfg(test)]
mod tests;
