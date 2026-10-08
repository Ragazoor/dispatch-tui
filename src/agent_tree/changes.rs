//! The git queries behind the agent-tree pane: what changed in the worktree
//! ([`git_changes`]) and the commits on the agent's branch
//! ([`git_branch_commits`]).
//!
//! Git is the sole source of truth for what the tree shows — see the spec's
//! `AgentTreeIsGitDerived` guarantee. This module owns the running of git;
//! parsing its output and folding the result into a tree belong to
//! [`crate::agent_tree::model`]. It does not merge in a full worktree
//! filesystem scan and never will: git already answers the question a scan was
//! meant to approximate, which is what resolved the spec's old
//! `TreeScanExclusions` question.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use crate::agent_tree::model::{
    attach_line_counts, parse_name_status, parse_numstat, parse_untracked, GitFileChange,
};
use crate::agent_tree::render::commits::AgentCommit;
use crate::git::{git_error, git_in, run_git, COULD_NOT_RUN_GIT};
use crate::process::ProcessRunner;

/// How long ONE git command may run before it is killed and treated as a
/// failure — `config.agent_tree_git_timeout` in the spec.
///
/// Deliberately far below [`crate::process::SUBPROCESS_TIMEOUT`], which the
/// board's other git calls use. Those run on a worker while the TUI stays live;
/// these run inline in this loop, so the timeout bounds how long this pane can
/// ignore a keypress.
///
/// The bound is PER COMMAND, and a tree tick runs at most three of them
/// ([`git_changes`]): the two `diff`s and `ls-files` touch the index, and a lock
/// the agent's own git holds is by far the commonest cause of a slow query. The
/// commits section's fork-point resolution ([`git_branch_commits`], up to four
/// commands) runs on its own worker, never in this loop.
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// The most commits listed — the spec's `config.agent_tree_commits_max_listed`.
pub(crate) const MAX_LISTED: usize = 50;

/// The commit where this worktree forked from `git_ref`, or git's own error if
/// the ref does not resolve.
fn merge_base(root: &str, git_ref: &str, runner: &dyn ProcessRunner) -> Result<String> {
    let sha = run_git(runner, root, &["merge-base", "HEAD", git_ref], GIT_TIMEOUT)?
        .trim()
        .to_string();
    // Git prints a commit id whenever it exits zero, so this is defensive
    // rather than reachable — but an empty string would be handed to `git log`
    // as the range start, where it means something else entirely. Soft-fail into
    // "this ref is not a candidate" instead.
    if sha.is_empty() {
        return Err(anyhow!("git: no common ancestor of HEAD and {git_ref}"));
    }
    Ok(sha)
}

/// Whether `ancestor` is an ancestor of `descendant`.
///
/// `merge-base --is-ancestor` answers with an exit code and no output: 0 for
/// yes, 1 for no. Any other code — or a git that could not be spawned, or one
/// that overran [`GIT_TIMEOUT`] — is not an answer, and is reported as a
/// failure rather than folded into "no".
///
/// That distinction is load-bearing. "No" keeps the LOCAL fork point, so
/// reading an unanswered probe as "no" would silently reinstate exactly the
/// mis-attribution `AgentTreeBaselineIsTaskBaseBranch` exists to forbid — and
/// present it as a correct list, with no notice and no red border. A failure
/// instead reaches `AgentTreeCommitListFailureKeepsLastList`, which keeps the last
/// list and says so.
fn is_ancestor(
    root: &str,
    ancestor: &str,
    descendant: &str,
    runner: &dyn ProcessRunner,
) -> Result<bool> {
    let output = git_in(
        runner,
        root,
        &["merge-base", "--is-ancestor", ancestor, descendant],
        GIT_TIMEOUT,
    )
    .context(COULD_NOT_RUN_GIT)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(git_error(&output)),
    }
}

/// Resolve this worktree's fork point from `base_branch`, which bounds the
/// commits section's list ([`git_branch_commits`]). The tree itself no longer
/// resolves any branch (`AgentTreeShowsUnstagedWorkOnly`).
///
/// A base branch is a NAME, and a repo can hold two refs under it: the local
/// branch and its remote-tracking counterpart. Dispatch branches a worktree
/// from whichever of the two is ahead at provision time
/// (`crate::dispatch::worktree`'s `select_start_point`), and the two drift in
/// BOTH directions during normal operation — a base branch the human has not
/// pulled leaves local behind, while a wrap-up that fast-forwards local without
/// pushing leaves it ahead. So each ref is probed and the fork point nearer
/// HEAD wins; see the spec's `AgentTreeBaselineIsTaskBaseBranch` for why
/// picking either ref unconditionally mis-attributes other people's commits to
/// the agent.
///
/// The remote ref comes from [`crate::git::origin_ref`], the crate's one
/// definition of it — deliberately the same one `select_start_point` reaches
/// through, so the two cannot disagree about which ref a worktree branched
/// from. In a repo whose remote is named anything else that ref never
/// resolves, and the fork point falls back to the local branch alone, with the
/// stale-branch mis-attribution that implies.
///
/// A ref that does not resolve is simply not a candidate: a base branch never
/// checked out locally is ordinary and must leave the pane working. Only when
/// NEITHER resolves is there no fork point, and then the LOCAL branch's error is
/// the one returned — that is the name the user put on the task, so it is the
/// one they can act on. A ranking probe that could not answer is a different
/// thing and fails the whole query; see [`is_ancestor`].
fn fork_point(root: &str, base_branch: &str, runner: &dyn ProcessRunner) -> Result<String> {
    let local = merge_base(root, base_branch, runner);
    let remote = merge_base(root, &crate::git::origin_ref(base_branch), runner);

    match (local, remote) {
        // Nearness is ancestry, not commit count. Equal fork points need no
        // ranking, so the probe is skipped — that is the ordinary case, where
        // the two refs agree. Where neither is an ancestor of the other the
        // refs have truly diverged, no choice is right, and the spec settles it
        // by fixed rule: the local ref wins.
        (Ok(local), Ok(remote)) => {
            if local != remote && is_ancestor(root, &local, &remote, runner)? {
                Ok(remote)
            } else {
                Ok(local)
            }
        }
        (Ok(local), Err(_)) => Ok(local),
        (Err(_), Ok(remote)) => Ok(remote),
        (Err(local_err), Err(_)) => Err(local_err),
    }
}

/// Run the git queries behind the tree for one source and return everything
/// they reported, with paths relative to `root` — the spec's
/// `AgentTreeGitQuery`.
///
/// `commit = None` is UNSTAGED WORK (the default): the working tree against
/// the index — `git diff --name-status --no-renames -z` and
/// `git diff --numstat --no-renames -z`, naming no revision — plus
/// `git ls-files --others --exclude-standard -z` for untracked files, which
/// come back Added with no counts (`AgentTreeShowsUnstagedWorkOnly`,
/// `UntrackedFilesHaveNoLineCounts`).
///
/// `commit = Some(id)` is ONE COMMIT against its first parent (the empty tree
/// for a root commit): two NUL-delimited, rename-free diffs and no untracked
/// listing. Neither form resolves a branch.
pub fn git_changes(
    root: &Path,
    commit: Option<&str>,
    runner: &dyn ProcessRunner,
) -> Result<Vec<GitFileChange>> {
    let root = root.to_string_lossy().into_owned();

    let (diff, numstat) = match commit {
        None => (
            run_git(
                runner,
                &root,
                &["diff", "--name-status", "--no-renames", "-z"],
                GIT_TIMEOUT,
            )?,
            // The SAME comparison and the same rename setting as the diff
            // above: one question asked twice, so a row's badge and its
            // numbers cannot answer different ones.
            run_git(
                runner,
                &root,
                &["diff", "--numstat", "--no-renames", "-z"],
                GIT_TIMEOUT,
            )?,
        ),
        // `git show --first-parent --format=` is the commit against its first
        // parent, and a root commit against the empty tree.
        Some(commit) => (
            run_git(
                runner,
                &root,
                &[
                    "show",
                    "--first-parent",
                    "--format=",
                    "--name-status",
                    "--no-renames",
                    "-z",
                    commit,
                ],
                GIT_TIMEOUT,
            )?,
            run_git(
                runner,
                &root,
                &[
                    "show",
                    "--first-parent",
                    "--format=",
                    "--numstat",
                    "--no-renames",
                    "-z",
                    commit,
                ],
                GIT_TIMEOUT,
            )?,
        ),
    };

    let mut changes = parse_name_status(&diff);
    attach_line_counts(&mut changes, &parse_numstat(&numstat));
    if commit.is_none() {
        // Appended after the counts are attached, and deliberately: an
        // untracked path is not in the index, so the numstat query never saw
        // it and there is nothing to attach (UntrackedFilesHaveNoLineCounts).
        let untracked = run_git(
            runner,
            &root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            GIT_TIMEOUT,
        )?;
        changes.extend(parse_untracked(&untracked));
    }
    Ok(changes)
}

/// The agent's own commits for the commits section: the newest
/// [`MAX_LISTED`] commits reachable from HEAD
/// and not from this worktree's [`fork_point`] from `base_branch`, newest
/// first — the spec's `git_branch_commits` (`RefreshAgentTreeCommitList`,
/// `AgentTreeBaselineIsTaskBaseBranch`).
pub fn git_branch_commits(
    root: &Path,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> Result<Vec<crate::agent_tree::render::commits::AgentCommit>> {
    let root = root.to_string_lossy().into_owned();
    let fork = fork_point(&root, base_branch, runner)?;
    let range = format!("{fork}..HEAD");
    let max = format!("--max-count={}", MAX_LISTED);
    let listing = run_git(
        runner,
        &root,
        &["log", &max, "--format=%H %s", &range],
        GIT_TIMEOUT,
    )?;
    Ok(listing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (id, subject) = line.split_once(' ').unwrap_or((line, ""));
            AgentCommit {
                id: id.to_string(),
                subject: subject.to_string(),
            }
        })
        .collect())
}
