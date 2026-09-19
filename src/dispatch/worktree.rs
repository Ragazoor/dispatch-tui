use anyhow::{Context, Result};
use std::fs;
use std::time::Duration;

use crate::models::{expand_tilde, slugify, Task, TmuxWindow};
use crate::process::ProcessRunner;
use crate::tmux;

use super::git_output::{WORKTREE_ALREADY_REMOVED, WORKTREE_LOCKED};
use crate::process::stderr_str;

/// Bounded retry budget for `git fetch origin <base>` during worktree
/// provisioning. Smooths over transient failures (e.g. ref-lock contention
/// when two dispatches fetch the same repo concurrently) without needing to
/// pattern-match git's stderr text.
/// `pub(super)` so `mock_sequence`'s scripts and the retry tests reference the
/// budget instead of mirroring the number — a mirrored `3` silently queues the
/// wrong response count the moment this changes.
pub(super) const FETCH_MAX_ATTEMPTS: u32 = 3;

/// Worst-case number of subprocess calls `provision_worktree` can issue
/// sequentially while provisioning a fresh, branch-based worktree, each
/// independently bounded by `SUBPROCESS_TIMEOUT`. Worst case is a fetch that
/// only succeeds on its last allowed attempt: `FETCH_MAX_ATTEMPTS` fetch
/// attempts, + 2 for `classify_fetch_failure`'s probes (fired once, after the
/// first failed attempt), + 1 for `select_start_point`'s ahead/behind
/// measurement, + 1 for the best-effort `git worktree prune` that clears a
/// stale admin record, + 1 for the final `git worktree add`. Exercised by
/// `provision_max_subprocess_calls_matches_the_worst_case_shape`, which reads
/// the add's index out of the scripted worst-case shape rather than mirroring
/// it here.
///
/// `pub(crate)` (via the re-export in `src/dispatch/mod.rs`) so
/// `DISPATCH_WATCHDOG_TIMEOUT` (`src/tui/mod.rs`) can derive its budget from
/// this instead of mirroring the number by hand — see `FETCH_MAX_ATTEMPTS`'s
/// own doc comment for why mirroring is the hazard this avoids (#4201).
///
/// Deliberately stops at `git worktree add`: the three tmux calls that follow
/// it in `provision_worktree`'s `post_add` step are unbounded today (they call
/// `runner.run(...)`, not `run_with_timeout`), so they are out of scope for
/// this count. If a future change (tracked separately) puts those calls behind
/// `run_with_timeout` too, they become sequential `SUBPROCESS_TIMEOUT`-bounded
/// calls *after* `WorktreeAdd`, and this constant must grow to cover them —
/// nothing here or in the test that pins this value would catch that drift on
/// its own.
pub(crate) const PROVISION_MAX_SUBPROCESS_CALLS: u32 = FETCH_MAX_ATTEMPTS + 5;

// Zero delay under `cfg(test)` so the retry tests below don't spend real
// wall-clock time sleeping — flagged by adversarial review of this plan,
// which pointed out a fixed 500ms delay would cost ~1s of real sleep per
// retry test. The retry *count* is still fully exercised either way.
#[cfg(not(test))]
const FETCH_RETRY_DELAY: Duration = Duration::from_millis(500);
#[cfg(test)]
const FETCH_RETRY_DELAY: Duration = Duration::from_millis(0);

/// Which ref a worktree branch was created from — and therefore what the agent
/// should rebase onto if it ever needs to.
///
/// Both arms carry the bare branch name because the `git fetch origin <base>`
/// line is identical either way; only the rebase target differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StartPoint {
    /// `origin/<base>`: origin is at least as new as local `<base>`.
    Remote { base: String },
    /// Bare local `<base>`: it carries commits `origin/<base>` does not, or
    /// origin has no such branch at all.
    Local { base: String },
}

impl StartPoint {
    /// The ref to hand `git worktree add`, and to rebase onto.
    pub(super) fn git_ref(&self) -> String {
        match self {
            StartPoint::Remote { base } => crate::git::origin_ref(base),
            StartPoint::Local { base } => base.clone(),
        }
    }

    /// The bare branch name, for the `git fetch origin <base>` line.
    pub(super) fn base(&self) -> &str {
        match self {
            StartPoint::Remote { base } | StartPoint::Local { base } => base,
        }
    }
}

#[derive(Debug)]
pub(super) struct ProvisionResult {
    pub(super) worktree_path: String,
    pub(super) tmux_window: TmuxWindow,
    /// `Some(...)` when `origin/<base>` could not be made current: either there
    /// is no such ref (and the local branch was used instead), or the worktree
    /// was being reused and origin turned out to be unreachable. Injected into
    /// the agent's prompt as a `Note:` line by `dispatch_with_prompt`.
    pub(super) fetch_warning: Option<String>,
    /// The ref the branch was created from. `None` when no base was given.
    /// The caller needs it to point the rebase preamble at the same ref.
    pub(super) start_point: Option<StartPoint>,
    /// True when the worktree directory already existed and `git worktree add`
    /// was skipped, so the branch may still hold a previous attempt's state.
    pub(super) reused_worktree: bool,
}

/// `git ls-remote --exit-code` returns this when no ref matched — as opposed to
/// 128, which means it could not reach the remote at all.
const LS_REMOTE_NO_MATCHING_REF: i32 = 2;

/// The outcome of making `origin/<base>` current before provisioning.
#[derive(Debug)]
enum FetchOutcome {
    /// `origin/<base>` is up to date locally.
    Fetched,
    /// There is no `origin/<base>` to fetch. Carries the message shown to the
    /// agent as a `Note:` line.
    NoOriginRef(String),
    /// Origin could not be reached, and provisioning does not need it to be.
    /// Only ever produced under [`FetchPolicy::BestEffort`] — under `Required`
    /// this same condition aborts. Carries the `Note:` line.
    Unreachable(String),
}

/// Whether provisioning still needs origin to be reachable.
///
/// The distinction is not a preference but a consequence of what the resolved
/// ref is about to be *used for*.
#[derive(Debug, Clone, Copy)]
enum FetchPolicy {
    /// A fresh worktree is about to be branched from the resolved ref, so an
    /// unreachable origin must abort: a worktree silently based on a stale
    /// local ref is worse than a dispatch that refuses to start.
    Required,
    /// The worktree directory already exists, so `git worktree add` is skipped
    /// and no ref is consumed to create anything — the resolved ref feeds only
    /// the rebase preamble. One attempt still runs, because the spec wants
    /// `origin/<base>` kept fresh for whatever rebases onto it later and the
    /// repo-sync drift indicator reads that ref. But failure is a warning, and
    /// neither the retry budget nor the classification probe is spent: both
    /// exist to serve the abort decision this policy does not make, and each
    /// costs a full `SUBPROCESS_TIMEOUT` against an unresponsive network.
    BestEffort,
}

/// Why a `git fetch origin <base>` failed.
enum FetchFailure {
    /// Nothing to fetch: no `origin` remote, or origin has no such branch.
    NoOriginRef(String),
    /// Origin has the branch, or we could not even determine that. Either way
    /// this is infrastructure, not a missing ref.
    Unreachable,
}

/// Classify a fetch failure without pattern-matching git's stderr text.
///
/// `git fetch` exits 128 for a missing ref, an unresolvable host and an
/// unreadable remote alike, so its own status cannot classify.
/// `git ls-remote --exit-code` can: 2 means "no matching ref", 128 means "could
/// not reach the remote". Anything we cannot positively identify as a missing
/// ref is treated as unreachable — the safe polarity, since only a recognised
/// 404 earns the local-branch fallback.
///
/// Private to this module on purpose: [`crate::repo_sync`] runs a very similar
/// fetch but has no need to tell a deleted upstream branch from a network blip.
/// If it ever grows that need, lift this and `LS_REMOTE_NO_MATCHING_REF` into
/// `src/git.rs` beside the other shared preflight helpers rather than writing a
/// second answer to the same question.
fn classify_fetch_failure(
    runner: &dyn ProcessRunner,
    repo_path: &str,
    base: &str,
    timeout: Duration,
) -> FetchFailure {
    match crate::git::has_origin_remote(repo_path, runner) {
        // The probe ran and found no origin: a positive identification, so the
        // local-branch fallback is earned.
        Ok(false) => {
            return FetchFailure::NoOriginRef("no origin remote is configured".to_string())
        }
        Ok(true) => {}
        // The probe could not be run at all, so it identified nothing. Reading
        // that as "no origin remote" would hand the local-branch fallback to a
        // failure to look — the exact inversion of this function's rule.
        Err(_) => return FetchFailure::Unreachable,
    }
    let refspec = format!("refs/heads/{base}");
    let probe = runner.run_with_timeout(
        "git",
        &[
            "-C",
            repo_path,
            "ls-remote",
            "--exit-code",
            "origin",
            &refspec,
        ],
        timeout,
    );
    match probe {
        Ok(output) if output.status.code() == Some(LS_REMOTE_NO_MATCHING_REF) => {
            FetchFailure::NoOriginRef(format!("origin has no branch {base}"))
        }
        _ => FetchFailure::Unreachable,
    }
}

/// Make `origin/<base>` current, or establish that it cannot be made current.
///
/// Under [`FetchPolicy::Required`] an infrastructure failure is retried up to
/// `FETCH_MAX_ATTEMPTS` and then aborts the dispatch: a worktree silently
/// branched off a stale local ref is worse than a dispatch that refuses to
/// start. A missing ref is not retried — retrying a branch that does not exist
/// cannot succeed — and is not an error, because local `<base>` is then the
/// only ref there is.
///
/// Under [`FetchPolicy::BestEffort`] a single attempt runs and any failure
/// yields [`FetchOutcome::Unreachable`] without classifying or retrying. See
/// that variant's doc comment for why both are dead weight there.
fn fetch_origin(
    runner: &dyn ProcessRunner,
    repo_path: &str,
    base: &str,
    timeout: Duration,
    policy: FetchPolicy,
) -> Result<FetchOutcome> {
    let max_attempts = match policy {
        FetchPolicy::Required => FETCH_MAX_ATTEMPTS,
        FetchPolicy::BestEffort => 1,
    };
    let mut last_err = String::new();
    for attempt in 1..=max_attempts {
        match runner.run_with_timeout("git", &["-C", repo_path, "fetch", "origin", base], timeout) {
            Ok(output) if output.status.success() => return Ok(FetchOutcome::Fetched),
            Ok(output) => last_err = stderr_str(&output),
            Err(e) => last_err = e.to_string(),
        }
        // Classify once, on the first failure: the answer cannot change between
        // attempts, and a 404 must not burn the retry budget.
        if attempt == 1 && matches!(policy, FetchPolicy::Required) {
            if let FetchFailure::NoOriginRef(reason) =
                classify_fetch_failure(runner, repo_path, base, timeout)
            {
                tracing::info!(base, %reason, "no origin ref to fetch; using the local branch");
                return Ok(FetchOutcome::NoOriginRef(format!(
                    "Could not fetch origin/{base} ({reason}); this worktree is based on \
                     the local {base} branch."
                )));
            }
        }
        if attempt < max_attempts {
            std::thread::sleep(FETCH_RETRY_DELAY);
        }
    }
    // Every attempt failed without classifying as a 404. The policy alone
    // decides what that means.
    match policy {
        FetchPolicy::BestEffort => {
            tracing::warn!(
                base,
                error = %last_err,
                "could not reach origin; reusing the existing worktree anyway"
            );
            Ok(FetchOutcome::Unreachable(format!(
                "Could not reach origin to fetch {base} ({last_err}); this worktree was reused \
                 from a previous attempt, so nothing needed the network. `origin/{base}` may be \
                 stale — re-run the fetch yourself once you are back online."
            )))
        }
        FetchPolicy::Required => {
            tracing::warn!(base, error = %last_err, "could not reach origin; aborting dispatch");
            anyhow::bail!(
                "Could not reach origin to fetch {base} after {FETCH_MAX_ATTEMPTS} attempts: \
                 {last_err}. Check network connectivity and that origin is reachable \
                 (`git -C <repo> fetch origin {base}`), then dispatch again."
            )
        }
    }
}

/// What a worktree is being based on, and whether local history may be
/// preferred over origin's.
#[derive(Debug, Clone, Copy)]
pub(super) enum BaseRef<'a> {
    /// The repo's base branch. Local `<base>` may legitimately hold commits
    /// origin lacks, so the two are compared and the one with unique commits
    /// wins.
    Branch(&'a str),
    /// A PR's head branch. Always `origin/<branch>`, never compared: a review
    /// must see exactly the PR's code, and a stale local branch of the same
    /// name would silently poison it.
    ///
    /// Constructed by `dispatch::agents::dispatch_with_prompt` whenever a
    /// review task resolves a PR head branch. Exercised directly by
    /// `provision_worktree_never_measures_a_pr_head_branch`.
    PrHead(&'a str),
}

impl BaseRef<'_> {
    fn name(&self) -> &str {
        match self {
            BaseRef::Branch(b) | BaseRef::PrHead(b) => b,
        }
    }
}

/// Refuse a fresh dispatch whose base branch resolves to nothing at all.
///
/// Implements `UnresolvableBaseIsRefusedByName` (docs/specs/dispatch.allium).
/// Only one of the three ways a start point is chosen leaves it unproven, and
/// this is it: the 404-class branch in [`resolve_start_point`] falls back to
/// local `<base>` purely because origin has no such branch — which says
/// nothing whatever about whether the local repo has one. A successful fetch
/// proves `origin/<base>`, and `select_start_point` takes local only on a
/// positive `ahead > 0` reading, which no unresolvable ref can produce. So the
/// probe belongs here and nowhere else.
///
/// The case is ordinary rather than exotic: a task row carries `base_branch`
/// as a plain string, and "main" against a repo whose default is "master" has
/// neither ref. Before this check it reached `git worktree add` and came back
/// as git's own `fatal: invalid reference: main`, naming neither the refs that
/// were tried nor the field holding the bad value — and when the dispatch was
/// the epic auto-dispatch chain, which cannot fail its caller, that line was
/// the entire evidence a human ever saw.
///
/// Returning `Err` here aborts before `.worktrees/` is created, so nothing is
/// left on disk, exactly as for the infra-class fetch abort.
fn ensure_local_base_resolves(
    runner: &dyn ProcessRunner,
    repo_path: &str,
    base: &str,
    timeout: Duration,
) -> Result<()> {
    // `--verify --quiet` with a `<name>^{commit}` peel: the question is whether
    // a commit can be reached under this name, which is exactly what
    // `git worktree add` is about to ask.
    let peeled = format!("{base}^{{commit}}");
    let resolves = runner
        .run_with_timeout(
            "git",
            &["-C", repo_path, "rev-parse", "--verify", "--quiet", &peeled],
            timeout,
        )
        .map(|output| output.status.success())
        // A probe that could not be spawned has answered nothing. Treating
        // that as "absent" would abort a dispatch over a process-spawn hiccup,
        // so it defers to `git worktree add`, which fails clearly on a real
        // absence anyway.
        .unwrap_or(true);

    anyhow::ensure!(
        resolves,
        "Cannot base a worktree on {base}: this repository has neither \
         origin/{base} nor a local {base}. Set the task's base branch to one \
         the repository actually has (`git -C {repo_path} branch -a`), then \
         dispatch again."
    );
    Ok(())
}

/// Choose the ref a new worktree branch starts from, given a fetch that just
/// succeeded so both refs are current.
///
/// Local `<base>` wins only on a positive `ahead > 0` reading. That polarity is
/// load-bearing: `ahead_behind` yields `None` whenever local `<base>` does not
/// resolve, which is the normal case for a base branch the human never checked
/// out, and preferring local there would fail `git worktree add`.
fn select_start_point(runner: &dyn ProcessRunner, repo_path: &str, base: &str) -> StartPoint {
    let base = base.to_string();
    match crate::repo_sync::ahead_behind(repo_path, &base, runner) {
        Some(counts) if counts.ahead > 0 => StartPoint::Local { base },
        _ => StartPoint::Remote { base },
    }
}

/// The paths and names a worktree is provisioned under, derived once from the
/// task so every later phase of `provision_worktree` shares one source of
/// truth for them.
struct WorktreePaths {
    repo_path: String,
    worktree_name: String,
    worktree_path: String,
    tmux_window: TmuxWindow,
}

/// Derive a task's worktree and tmux-window names from its repo path and
/// title. No branching: a rejected `repo_path` is the only failure mode.
fn worktree_paths(task: &Task) -> Result<WorktreePaths> {
    let repo_path = validate_repo_path(&task.repo_path).map_err(|e| anyhow::anyhow!(e))?;
    let slug = slugify(&task.title);
    let worktree_name = format!("{}-{slug}", task.id);
    let worktree_path = worktrees_root(&repo_path)
        .join(&worktree_name)
        .to_string_lossy()
        .into_owned();
    let tmux_window = TmuxWindow::for_task(task.id);
    Ok(WorktreePaths {
        repo_path,
        worktree_name,
        worktree_path,
        tmux_window,
    })
}

/// Make `origin/<base>` current and choose the ref a new worktree branch
/// should start from — or establish that neither is possible and say why.
///
/// The fetch runs unconditionally — even when reusing an existing worktree
/// directory — so `origin/<base>` stays fresh for whatever rebases onto it
/// later. On a fresh worktree an unreachable origin aborts here rather than
/// quietly producing a worktree based on a stale local ref; on the reuse path
/// it is downgraded to a warning. See `FetchPolicy`.
///
/// `select_start_point` (below) runs on the reuse path too, and that is
/// deliberate rather than wasted work: `reused_rebase_preamble` targets
/// whatever `start_point` reports, so skipping the measurement here would
/// leave that preamble pointing at the wrong ref — the same
/// `git rebase origin/main`-onto-a-local-based-branch history-duplication
/// hazard this branch exists to remove.
fn resolve_start_point(
    runner: &dyn ProcessRunner,
    repo_path: &str,
    base: Option<BaseRef<'_>>,
    reused_worktree: bool,
    timeout: Duration,
) -> Result<(Option<StartPoint>, Option<String>)> {
    let policy = if reused_worktree {
        FetchPolicy::BestEffort
    } else {
        FetchPolicy::Required
    };
    let base_ref = match base {
        Some(base_ref) => base_ref,
        None => return Ok((None, None)),
    };
    match fetch_origin(runner, repo_path, base_ref.name(), timeout, policy)? {
        FetchOutcome::Fetched => {
            let sp = match base_ref {
                BaseRef::PrHead(b) => StartPoint::Remote {
                    base: b.to_string(),
                },
                BaseRef::Branch(b) => select_start_point(runner, repo_path, b),
            };
            Ok((Some(sp), None))
        }
        FetchOutcome::NoOriginRef(warning) => match base_ref {
            // There is no other candidate ref: local `<base>` is the only
            // thing that *could* exist, so falling back to it (with a `Note:`
            // the agent can see) is the right call — once we have established
            // that it does. See `ensure_local_base_resolves`.
            BaseRef::Branch(b) => {
                if !reused_worktree {
                    ensure_local_base_resolves(runner, repo_path, b, timeout)?;
                }
                Ok((
                    Some(StartPoint::Local {
                        base: b.to_string(),
                    }),
                    Some(warning),
                ))
            }
            // A PR head branch must never fall back to a local branch of
            // the same name — see `BaseRef::PrHead`'s doc comment. If
            // origin doesn't have it, there is nothing safe to base the
            // review on, so abort rather than silently reviewing the
            // wrong code.
            BaseRef::PrHead(b) => anyhow::bail!(
                "origin has no branch {b}; refusing to base a PR review on a local branch \
                 of the same name"
            ),
        },
        // Reuse path only. Nothing is being created from this ref, so the
        // choice only sets the preamble's rebase target.
        FetchOutcome::Unreachable(warning) => match base_ref {
            // `origin/<base>` could not be refreshed, so pointing the
            // preamble at it risks replaying local <base>'s unpushed
            // commits under new SHAs. Local <base> is the ref we can vouch
            // for, and the one wrap-up rebases onto.
            BaseRef::Branch(b) => Ok((
                Some(StartPoint::Local {
                    base: b.to_string(),
                }),
                Some(warning),
            )),
            // A review must never be handed a local branch of the same
            // name — see `BaseRef::PrHead`. The reused worktree already
            // holds the PR's code from the previous attempt, so staying
            // pinned to the origin ref is both safe and honest: if the
            // preamble's rebase cannot reach it, the agent sees that
            // directly, and the `Note:` explains why.
            BaseRef::PrHead(b) => Ok((
                Some(StartPoint::Remote {
                    base: b.to_string(),
                }),
                Some(warning),
            )),
        },
    }
}

/// Does anything exist at `path`? — the ONE answer both provisioning and
/// teardown work from (`WorktreeExistenceIsOneQuestion` in
/// docs/specs/dispatch.allium).
///
/// `Ok(None)` is absence, `Ok(Some(_))` is presence, and `Err` is the third
/// answer a stat has: a path that cannot be inspected at all.
///
/// The two callers used to ask this separately and got different answers.
/// [`provision_worktree`] asked `Path::exists()`, which FOLLOWS symlinks and
/// collapses every error to `false`; [`delete_leftover_worktree_dir`] asks
/// `symlink_metadata`, where only `NotFound` is absence and a link is itself
/// the subject. A dangling or unreadable worktree path therefore read "fresh"
/// to one and "present" to the other — and a "fresh" path that `git worktree
/// add` then fails on is one [`rollback_failed_provisioning`] is allowed to
/// delete, which is exactly what the "never removes a REUSED worktree" rule
/// exists to prevent.
///
/// The metadata is returned rather than a bool because teardown needs the file
/// type to tell a link from a directory — see
/// `SymlinksAreUnlinkedNeverFollowed` in docs/specs/tasks.allium. The `Err` is
/// left bare for the same reason: the two callers are refusing for different
/// reasons and each names its own, so the shared answer states only what the
/// filesystem said.
fn worktree_path_metadata(path: &std::path::Path) -> std::io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Is there a worktree at `path` this dispatch can reuse? (`PresenceIsNotYetReuse`
/// in docs/specs/dispatch.allium.)
///
/// `false` means the path is absent and the caller must create it. `true` means
/// a usable directory is already there. An `Err` refuses the dispatch outright.
///
/// Two questions, in order, because they are answered about different things:
///
/// 1. PRESENCE, via [`worktree_path_metadata`] — the link itself, never its
///    target. This is the one that settles what may be DELETED, so it must not
///    follow a link: a dangling link read as absent is one
///    [`rollback_failed_provisioning`] would be free to remove.
/// 2. USABILITY, via [`fs::metadata`] — the target, since that is what an agent
///    would actually work in. A symlink to a real directory qualifies.
///
/// A path that is present but not a usable directory refuses rather than
/// falling back to either answer. Reuse is nothing but skipping
/// `git worktree add`, so it produces no downstream error of its own: tmux
/// takes an unresolvable `-c` without complaint and starts the pane in the
/// operator's HOME (probed against tmux 3.7c, #4889), which would launch an
/// agent outside any worktree with nothing looking wrong. Calling it FRESH
/// instead would abort at the add — but only after declaring a path this
/// attempt does not own to be its own, which is the hazard the split exists to
/// avoid.
fn worktree_is_reusable(path: &std::path::Path) -> Result<bool> {
    let present = worktree_path_metadata(path)
        .with_context(|| format!("failed to inspect worktree path {}", path.display()))?
        .is_some();
    if !present {
        return Ok(false);
    }
    // Follows the link deliberately, and only here: this asks about the
    // directory an agent would work in, not about what may be deleted.
    //
    // The two ways this can fail are kept apart, because they tell the
    // operator different things and the clause promises the underlying error
    // where there is one. A stat that SUCCEEDS on a non-directory means
    // something else is in the way — a file, a socket, a device — and there is
    // no io error to name. A stat that FAILS means the target could not be
    // reached at all (a dangling link, a loop, an unreadable chain), and that
    // error is the whole of what the operator needs.
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => anyhow::bail!(
            "refusing to dispatch into {}: something is already there, and it \
             is not a directory",
            path.display()
        ),
        Err(error) => Err(anyhow::Error::new(error).context(format!(
            "refusing to dispatch into {}: it exists but cannot be resolved to \
             a usable worktree directory",
            path.display()
        ))),
    }
}

/// Create a git worktree and open a tmux window.
/// Shared by `dispatch_agent`, `research_agent`, and `quick_dispatch_agent`,
/// all of which reach it via `dispatch_with_prompt`.
///
/// `timeout` is passed to `run_with_timeout` for long-running git subprocesses
/// (`git fetch`, `git worktree add`). Use [`crate::process::SUBPROCESS_TIMEOUT`]
/// in production; pass a short duration in tests.
pub(super) fn provision_worktree(
    task: &Task,
    runner: &dyn ProcessRunner,
    base: Option<BaseRef<'_>>,
    timeout: Duration,
) -> Result<ProvisionResult> {
    let WorktreePaths {
        repo_path,
        worktree_name,
        worktree_path,
        tmux_window,
    } = worktree_paths(task)?;

    tracing::info!(task_id = task.id.0, %worktree_path, ?base, "provisioning worktree");

    // Measured before the fetch, because it decides the fetch policy: on the
    // reuse path `git worktree add` is skipped, so no ref is consumed to create
    // anything and an unreachable origin has nothing to corrupt.
    //
    // A stat that answers neither yes nor no aborts here, before anything on
    // disk or in tmux is touched — see `worktree_path_metadata` for which
    // later choice that protects.
    let reused_worktree = worktree_is_reusable(std::path::Path::new(&worktree_path))?;

    let (start_point, fetch_warning) =
        resolve_start_point(runner, &repo_path, base, reused_worktree, timeout)?;

    let start_ref = start_point.as_ref().map(StartPoint::git_ref);

    if reused_worktree {
        tracing::info!(task_id = task.id.0, %worktree_path, "worktree already exists, reusing");
    } else {
        // Deliberately below the fetch and inside this branch: the directory
        // exists only to hold the worktree `git worktree add` is about to
        // create, so a provisioning attempt that gives up before that point
        // leaves nothing behind. Hoisting it back above the fetch reintroduces
        // an empty `.worktrees/` on every aborted dispatch.
        fs::create_dir_all(worktrees_root(&repo_path))
            .context("failed to create .worktrees directory")?;

        // A `.git/worktrees/<name>` record whose directory is gone still
        // claims the branch, and makes the add below fail with "'<branch>' is
        // already used by worktree at ...". Teardown prunes the records it
        // witnessed, but a record left by an operator's `rm -rf`, a crashed
        // dispatch, or a husk deleted by hand has no teardown behind it — so
        // the repair belongs here, on the path that actually suffers. Repo-wide
        // is safe: prune drops only records whose directory is missing, so it
        // cannot reach a live worktree, a sibling task's included.
        //
        // Best-effort: the add is the step whose success decides the dispatch,
        // and its error is the one worth reporting. See
        // `StaleAdminRecordIsPrunedBeforeWorktreeAdd` in docs/specs/dispatch.allium.
        let _ = runner.run_with_timeout("git", &["-C", &repo_path, "worktree", "prune"], timeout);

        let mut args = vec![
            "-C",
            &repo_path,
            "worktree",
            "add",
            &worktree_path,
            "-B",
            &worktree_name,
        ];
        if let Some(sp) = start_ref.as_deref() {
            args.push(sp);
        }
        let output = runner
            .run_with_timeout("git", &args, timeout)
            .context("failed to run git worktree add")?;
        anyhow::ensure!(
            output.status.success(),
            "git worktree add failed: {}",
            stderr_str(&output)
        );
    }

    // Outside the steps below so each rollback passes a constant rather than a
    // flag kept in sync: a create that never happened leaves no window for this
    // attempt to roll back. See "Provisioning-failure rollback" in
    // docs/specs/dispatch.allium for why that distinction matters.
    if let Err(e) = tmux::new_window(&tmux_window, &worktree_path, runner)
        .context("failed to create tmux window")
    {
        rollback_failed_provisioning(&repo_path, &worktree_path, None, reused_worktree, runner);
        return Err(e);
    }
    let post_add: Result<()> = (|| {
        tmux::set_window_dispatch_dir(&tmux_window, &worktree_path, runner)
            .context("failed to set tmux window dispatch dir")?;
        tmux::ensure_split_hook(runner).context("failed to ensure tmux split hook")?;
        Ok(())
    })();
    if let Err(e) = post_add {
        rollback_failed_provisioning(
            &repo_path,
            &worktree_path,
            Some(&tmux_window),
            reused_worktree,
            runner,
        );
        return Err(e);
    }

    tracing::info!(
        task_id = task.id.0,
        base = start_point.as_ref().map(StartPoint::base),
        start_ref = start_ref.as_deref(),
        reused_worktree,
        "worktree provisioned"
    );

    Ok(ProvisionResult {
        worktree_path,
        tmux_window,
        fetch_warning,
        start_point,
        reused_worktree,
    })
}

/// `TaskTeardown` from docs/specs/tasks.allium: kill the tmux window if there is
/// one, remove the git worktree if there is one, delete its branch best-effort.
///
/// The two resources are **independent optionals**, and that is the whole point
/// of this signature: a task owning a window but no worktree still owes step 1
/// (`TeardownIsOwedWheneverThereIsSomethingToRelease`). This is the one
/// implementation of that clause — its callers
/// (`crate::runtime::TuiRuntime::exec_cleanup`, `cleanup_removed_feed_tasks`)
/// differ only in what they do with an `Err`, and must not re-branch on the
/// worktree themselves. Two wrappers that each decided this for themselves is
/// exactly how the archive path came to leak windows (#4096).
///
/// Branch deletion is reachable only through the worktree arm, deliberately: a
/// task that never had a worktree never had a branch either.
pub fn teardown_task(
    repo_path: &str,
    worktree_path: Option<&str>,
    tmux_window: Option<&TmuxWindow>,
    runner: &dyn ProcessRunner,
) -> Result<(), TeardownFailure> {
    tracing::info!(?worktree_path, ?tmux_window, "tearing down task");

    if let Some(window) = tmux_window {
        if let Err(error) = tmux::kill_window_if_present(window, runner)
            .context("failed to kill tmux window during cleanup")
        {
            // The kill aborts before step 2 runs, so a worktree this task owns is
            // still on disk — which is the caller's gate, not this failure's kind.
            return Err(TeardownFailure {
                worktree_left: worktree_path.map(str::to_string),
                error,
            });
        }
    }

    let Some(worktree_path) = worktree_path else {
        return Ok(());
    };

    remove_worktree_and_branch(repo_path, worktree_path, runner).map_err(|error| TeardownFailure {
        worktree_left: Some(worktree_path.to_string()),
        error,
    })
}

/// A failed [`teardown_task`], and — the part its callers' policies turn on —
/// whether it left a worktree on disk.
///
/// Reporting step 2's outcome is the point. `WorktreeReleaseIsGated` in
/// docs/specs/tasks.allium keys the requesting operation's follow-up on the
/// worktree's *release*, so the wrapper must not have to infer that from the
/// arguments it passed plus the order this function happens to run its steps in.
/// Whoever changes that order changes `worktree_left` here, in one place, instead
/// of silently inverting a gate two modules away.
#[derive(Debug)]
pub struct TeardownFailure {
    /// `Some(path)` when the task owned a worktree that is still on disk.
    pub worktree_left: Option<String>,
    pub error: anyhow::Error,
}

impl std::fmt::Display for TeardownFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `{:#}` renders anyhow's full context chain, which is what every caller
        // logs — so a bare `{failure}` already carries it.
        write!(f, "{:#}", self.error)
    }
}

/// Steps 2 and 3 of `TaskTeardown`, reached only for a task that owns a worktree.
fn remove_worktree_and_branch(
    repo_path: &str,
    worktree_path: &str,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let repo = expand_tilde(repo_path);
    // A ProcessRunner error (git missing from PATH, spawn failure) is an
    // outcome of 2a like any other, and the worktree is no less on disk for it
    // — so it falls through to 2b rather than returning early. Only the lock
    // arm below skips the delete.
    let outcome = runner.run(
        "git",
        &["-C", &repo, "worktree", "remove", "--force", worktree_path],
    );
    let failure = match &outcome {
        Err(error) => Some(format!("failed to run git worktree remove: {error:#}")),
        Ok(output) if !output.status.success() => Some(stderr_str(output)),
        Ok(_) => None,
    };
    match failure {
        // The one refusal step 2b must not delete its way past — see
        // `WORKTREE_LOCKED`.
        Some(stderr) if stderr.contains(WORKTREE_LOCKED) => anyhow::bail!(
            "git worktree remove failed for path {worktree_path}: {}",
            stderr
        ),
        // Every other failure falls through to 2b, because git removing
        // NOTHING is exactly the case that leaked. Whether the step succeeded
        // is then decided by what is left on disk, not by git's exit code.
        Some(stderr) => {
            if stderr.contains(WORKTREE_ALREADY_REMOVED) {
                tracing::info!(worktree_path, "worktree already unregistered");
            } else {
                tracing::warn!(
                    worktree_path,
                    stderr = %stderr,
                    "git worktree remove failed; deleting the directory directly"
                );
            }
            delete_leftover_worktree_dir(&repo, worktree_path).with_context(|| {
                format!("git worktree remove failed for path {worktree_path}: {stderr}")
            })?;

            // Git that removed nothing also KEPT its admin record under
            // `.git/worktrees/<name>`, and that record makes the next
            // `git worktree add` for this task fail with "is already used by
            // worktree at ..." — and the branch delete below fail too, while
            // the record still claims the branch. So it runs BEFORE step 3.
            // Best-effort: prune only drops records whose directory is gone,
            // which the delete above has just made true for this one.
            let _ = runner.run("git", &["-C", &repo, "worktree", "prune"]);
        }
        // Git removed its own record here, so there is nothing to prune.
        None => delete_leftover_worktree_dir(&repo, worktree_path)?,
    }

    if let Some(branch) = branch_from_worktree(worktree_path) {
        // Best-effort: ignore errors (branch may not exist).
        let _ = runner.run("git", &["-C", &repo, "branch", "-D", &branch]);
    }

    Ok(())
}

/// Step 2b of `TaskTeardown`: make the worktree path not exist.
///
/// `git worktree remove` is only the first half of releasing a worktree: it has
/// two failure modes in which it removes **nothing**, one of them silently. The
/// three outcomes, why they leaked 756 GB, and what each half owes are in
/// `WorktreeDirectoryMustNotSurviveTeardown` in docs/specs/tasks.allium; they
/// are pinned to a real git version by tests/worktree_teardown.rs.
///
/// Absence is the success condition, so a path that is already gone is already
/// satisfied and this does nothing. That is also what keeps every mock-only
/// teardown test passing: they name paths that never existed on disk.
///
/// # Bound
///
/// This deletes recursively, so where it may point is part of the behaviour
/// (`DeletionIsBoundedToTheWorktreesDirectory` in docs/specs/tasks.allium). The
/// path must lie strictly inside [`worktrees_root`]. Anything else — the repo,
/// `.worktrees` itself, a `..` traversal out of it, a pointer to somewhere else
/// entirely — is refused and reported, never acted on.
///
/// The test is LEXICAL, and deliberately so: `~` expanded and `.`/`..` resolved
/// as text, with no `canonicalize` on either side. Resolving symlinks would
/// decide the question about a worktree's TARGET rather than about the
/// worktree, and the target is the one path that must never be deleted here.
///
/// # Symlinks
///
/// Links are unlinked, never followed (`SymlinksAreUnlinkedNeverFollowed`). A
/// worktree may hold links into the operator's wider filesystem and an agent
/// may leave more; none of those targets belong to the task.
/// [`fs::remove_dir_all`] already unlinks a link it meets *inside* the tree
/// rather than descending it. The worktree path being ITSELF a link is the case
/// this handles explicitly: it is unlinked with [`fs::remove_file`], because
/// `remove_dir_all` on a symlink has not behaved the same way across Rust
/// versions — it has both errored with `ENOTDIR` and silently unlinked the
/// link, and neither is a behaviour to inherit for a recursive delete.
fn delete_leftover_worktree_dir(repo: &str, worktree_path: &str) -> Result<()> {
    let target = lexical_path(&expand_tilde(worktree_path));

    // The shared existence question — `symlink_metadata`, not `exists`, so a
    // dangling link is presence and only NotFound is absence. Any other error
    // (EACCES on a parent, EIO, an unmounted mount point) is a directory that
    // may well be there and cannot be seen; reporting THAT as released is how
    // the gate comes to clear a pointer to something still on disk, which is
    // the orphan WorktreeReleaseIsGated (c) exists to prevent. Provisioning
    // asks the same question the same way — see `worktree_path_metadata`.
    let metadata = match worktree_path_metadata(&target)
        .with_context(|| format!("failed to inspect leftover worktree {}", target.display()))?
    {
        Some(metadata) => metadata,
        None => return Ok(()),
    };

    let root = lexical_path(&worktrees_root(repo).to_string_lossy());
    // Both sides come from the same recorded repo_path today, so they agree on
    // spelling. Making them absolute keeps a relative one from failing the
    // prefix test — and refusing forever — if that ever stops being true.
    // `std::path::absolute` is lexical: it prepends the working directory and
    // resolves no symlinks (nor, on Unix, `..` — which is what `lexical_path`
    // is for). With no working directory to resolve against, comparing the
    // paths as given is the narrow answer, so the bound never widens.
    let target = std::path::absolute(&target).unwrap_or(target);
    let root = std::path::absolute(&root).unwrap_or(root);
    if !target.starts_with(&root) || target == root {
        anyhow::bail!(
            "refusing to delete leftover worktree {}: it is not inside {}",
            target.display(),
            root.display()
        );
    }

    tracing::warn!(
        worktree_path,
        "git left the worktree directory on disk; deleting it"
    );

    if metadata.file_type().is_symlink() {
        fs::remove_file(&target)
    } else {
        fs::remove_dir_all(&target)
    }
    .with_context(|| format!("failed to delete leftover worktree {}", target.display()))
}

/// Resolve `.` and `..` as TEXT, touching no filesystem.
///
/// [`std::fs::canonicalize`] is the wrong tool for the bound check above: it
/// resolves symlinks, which would answer about a worktree's target instead of
/// the worktree, and it fails outright on a path that does not exist. This
/// keeps `..` from being a hole in a `starts_with` check without either.
fn lexical_path(path: &str) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::CurDir => {}
            // A leading `..` has nothing to pop, so it is kept verbatim rather
            // than dropped — dropping it would turn `../x` into `x` and widen
            // the bound instead of narrowing it.
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Best-effort teardown of what this dispatch attempt has created, once a
/// later step in the same attempt has failed: the tmux window it opened (a
/// window is created fresh on every attempt, so always a candidate), and —
/// only when the worktree itself was freshly created rather than reused —
/// the worktree and its branch.
///
/// Delegates to [`teardown_task`], the one implementation of
/// kill-window-then-remove-worktree ordering (see its own doc comment and
/// #4096), rather than re-deciding that order here.
///
/// Never removes a REUSED worktree — see the "Provisioning-failure rollback"
/// guidance in docs/specs/dispatch.allium: it predates this attempt and this
/// flow did not create it, so it is never a candidate for removal here.
///
/// `tmux_window` is `None` for the symmetrical case on the window side: an
/// attempt that never opened a window of its own. Same guidance, same reason.
/// Teardown failure is logged, never propagated: the caller already has the
/// real provisioning error to report, and a cleanup failure on top of that
/// would only obscure it.
pub(super) fn rollback_failed_provisioning(
    repo_path: &str,
    worktree_path: &str,
    tmux_window: Option<&TmuxWindow>,
    reused_worktree: bool,
    runner: &dyn ProcessRunner,
) {
    let worktree_arg = (!reused_worktree).then_some(worktree_path);
    if let Err(e) = teardown_task(repo_path, worktree_arg, tmux_window, runner) {
        tracing::warn!(
            worktree_path,
            tmux_window = tmux_window.map(|w| w.as_str()),
            error = %e,
            "failed to roll back a failed dispatch"
        );
    }
}

/// The directory every task worktree lives in, `<repo>/.worktrees`.
///
/// Three callers need it and one of them is a safety bound: the recursive
/// delete in [`delete_leftover_worktree_dir`] refuses anything outside this
/// directory. A bound that restates the layout independently of the code that
/// creates the paths does not fail loudly when the two drift — it makes every
/// teardown refuse forever. `repo` is expected already tilde-expanded, as
/// `validate_repo_path` returns it.
fn worktrees_root(repo: &str) -> std::path::PathBuf {
    std::path::Path::new(repo).join(".worktrees")
}

/// Extract the branch name from a worktree path (its last path component).
pub fn branch_from_worktree(worktree: &str) -> Option<String> {
    std::path::Path::new(worktree)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Validate that a repo path points to an existing directory.
///
/// Returns the expanded path on success, or an error message on failure.
pub fn validate_repo_path(path: &str) -> Result<String, String> {
    let expanded = expand_tilde(path);
    let p = std::path::Path::new(&expanded);
    if !p.exists() {
        return Err(format!("Directory does not exist: {expanded}"));
    }
    if !p.is_dir() {
        return Err(format!("Not a directory: {expanded}"));
    }
    Ok(expanded)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod fetch_tests {
    use super::*;
    use crate::process::MockProcessRunner;
    use std::time::Duration;

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn successful_fetch_reports_fetched() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        assert!(matches!(
            fetch_origin(&mock, "/repo", "main", T, FetchPolicy::Required).unwrap(),
            FetchOutcome::Fetched
        ));
        assert_eq!(mock.recorded_calls().len(), 1, "no classification needed");
    }

    #[test]
    fn missing_origin_remote_is_not_retried() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("no such remote"), // git fetch
            MockProcessRunner::fail("no origin"),      // git remote get-url origin
        ]);
        let outcome = fetch_origin(&mock, "/repo", "main", T, FetchPolicy::Required).unwrap();
        let FetchOutcome::NoOriginRef(warning) = outcome else {
            panic!("expected NoOriginRef");
        };
        assert!(warning.contains("origin remote"), "got: {warning}");
        assert_eq!(
            mock.recorded_calls().len(),
            2,
            "one fetch, one classification — no retries: {:?}",
            mock.recorded_calls()
        );
    }

    // A remote probe that cannot be *run* identifies nothing, so it must not
    // earn the local-branch fallback the way a probe that ran and found no
    // origin does. Treating a spawn failure as "no origin remote" would invert
    // classify_fetch_failure's own rule — only a positive identification of a
    // missing ref may fall back to local.
    #[test]
    fn an_unrunnable_remote_probe_is_treated_as_unreachable_not_as_a_missing_ref() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("Could not resolve host"), // fetch 1
            Err(anyhow::anyhow!("git: command not found")),    // remote get-url origin
            MockProcessRunner::fail("Could not resolve host"), // fetch 2
            MockProcessRunner::fail("Could not resolve host"), // fetch 3
        ]);
        let err = fetch_origin(&mock, "/repo", "main", T, FetchPolicy::Required)
            .expect_err("an unidentified failure must abort, not fall back to local");
        assert!(
            err.to_string().contains("Could not reach origin"),
            "got: {err}"
        );
        assert_eq!(
            mock.recorded_calls().len(),
            4,
            "the probe short-circuits ls-remote, then the fetch is retried: {:?}",
            mock.recorded_calls()
        );
    }

    #[test]
    fn branch_absent_from_origin_is_not_retried() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("couldn't find remote ref"), // git fetch
            MockProcessRunner::ok(),                             // git remote get-url origin
            MockProcessRunner::fail_with_code(2, ""),            // git ls-remote --exit-code
        ]);
        let outcome = fetch_origin(&mock, "/repo", "nosuch", T, FetchPolicy::Required).unwrap();
        let FetchOutcome::NoOriginRef(warning) = outcome else {
            panic!("expected NoOriginRef");
        };
        assert!(warning.contains("nosuch"), "got: {warning}");
        assert_eq!(mock.recorded_calls().len(), 3, "no retries after a 404");
    }

    #[test]
    fn unreachable_origin_is_retried_then_aborts() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("Could not resolve host"), // fetch 1
            MockProcessRunner::ok(),                           // remote get-url origin
            MockProcessRunner::fail_with_code(128, ""),        // ls-remote: unreachable
            MockProcessRunner::fail("Could not resolve host"), // fetch 2
            MockProcessRunner::fail("Could not resolve host"), // fetch 3
        ]);
        let err = fetch_origin(&mock, "/repo", "main", T, FetchPolicy::Required).unwrap_err();
        assert!(
            err.to_string().contains("Could not reach origin"),
            "got: {err}"
        );
        assert_eq!(
            mock.recorded_calls().len(),
            5,
            "classify once, then retry the fetch: {:?}",
            mock.recorded_calls()
        );
    }

    #[test]
    fn existing_ref_that_fails_to_fetch_aborts_rather_than_using_local() {
        // ls-remote finds the ref, so origin is reachable and the branch is
        // there — a fetch that still fails is infrastructure, never a 404.
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("early EOF"),
            MockProcessRunner::ok(), // remote get-url origin
            MockProcessRunner::ok(), // ls-remote: exit 0, ref exists
            MockProcessRunner::fail("early EOF"),
            MockProcessRunner::fail("early EOF"),
        ]);
        assert!(fetch_origin(&mock, "/repo", "main", T, FetchPolicy::Required).is_err());
    }

    #[test]
    fn fetch_succeeding_on_retry_reports_fetched() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("early EOF"),
            MockProcessRunner::ok(), // remote get-url origin
            MockProcessRunner::fail_with_code(128, ""), // ls-remote: unreachable
            MockProcessRunner::ok(), // fetch 2 succeeds
        ]);
        assert!(matches!(
            fetch_origin(&mock, "/repo", "main", T, FetchPolicy::Required).unwrap(),
            FetchOutcome::Fetched
        ));
    }

    // ------------------------------------------------------------------
    // FetchPolicy::BestEffort — the reuse path (#3843)
    // ------------------------------------------------------------------

    #[test]
    fn best_effort_reports_unreachable_instead_of_aborting() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("Could not resolve host")]);
        let outcome = fetch_origin(&mock, "/repo", "main", T, FetchPolicy::BestEffort)
            .expect("a best-effort fetch never aborts the dispatch");
        let FetchOutcome::Unreachable(warning) = outcome else {
            panic!("expected Unreachable, got: {outcome:?}");
        };
        assert!(warning.contains("main"), "got: {warning}");
    }

    // The budget, asserted at the unit level: one subprocess, not five. Against
    // a blackholing network each avoided call is a full SUBPROCESS_TIMEOUT.
    #[test]
    fn best_effort_spends_exactly_one_subprocess_on_a_failure() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("Could not resolve host")]);
        let _ = fetch_origin(&mock, "/repo", "main", T, FetchPolicy::BestEffort);
        assert_eq!(
            mock.recorded_calls().len(),
            1,
            "no classification probe and no retries: {:?}",
            mock.recorded_calls()
        );
    }

    // A 404 and an unreachable host both land on Unreachable here. That is the
    // point: the two only diverge because Required has to decide whether to
    // abort, and BestEffort never does.
    #[test]
    fn best_effort_does_not_classify_a_missing_branch() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
            "couldn't find remote ref nosuch",
        )]);
        let outcome = fetch_origin(&mock, "/repo", "nosuch", T, FetchPolicy::BestEffort).unwrap();
        assert!(
            matches!(outcome, FetchOutcome::Unreachable(_)),
            "got: {outcome:?}"
        );
        assert_eq!(mock.recorded_calls().len(), 1, "no ls-remote probe");
    }

    // The happy path is unchanged: origin still gets refreshed on reuse, which
    // is what keeps the repo-sync drift indicator honest.
    #[test]
    fn best_effort_still_fetches_successfully() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        assert!(matches!(
            fetch_origin(&mock, "/repo", "main", T, FetchPolicy::BestEffort).unwrap(),
            FetchOutcome::Fetched
        ));
        assert_eq!(mock.recorded_calls().len(), 1);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod selection_tests {
    use super::*;
    use crate::process::MockProcessRunner;

    // `git rev-list --count --left-right <base>...origin/<base>` prints
    // "<ahead>\t<behind>".
    fn counts(ahead: u32, behind: u32) -> anyhow::Result<std::process::Output> {
        MockProcessRunner::ok_with_stdout(format!("{ahead}\t{behind}\n").as_bytes())
    }

    #[test]
    fn local_wins_when_it_holds_commits_origin_lacks() {
        let mock = MockProcessRunner::new(vec![counts(3, 0)]);
        assert_eq!(
            select_start_point(&mock, "/repo", "main"),
            StartPoint::Local {
                base: "main".to_string()
            }
        );
    }

    #[test]
    fn origin_wins_when_local_is_behind() {
        let mock = MockProcessRunner::new(vec![counts(0, 2)]);
        assert_eq!(
            select_start_point(&mock, "/repo", "main"),
            StartPoint::Remote {
                base: "main".to_string()
            }
        );
    }

    #[test]
    fn origin_wins_when_the_two_are_level() {
        let mock = MockProcessRunner::new(vec![counts(0, 0)]);
        assert_eq!(
            select_start_point(&mock, "/repo", "main"),
            StartPoint::Remote {
                base: "main".to_string()
            }
        );
    }

    #[test]
    fn diverged_takes_local_silently() {
        let mock = MockProcessRunner::new(vec![counts(3, 2)]);
        assert_eq!(
            select_start_point(&mock, "/repo", "main"),
            StartPoint::Local {
                base: "main".to_string()
            }
        );
    }

    #[test]
    fn unmeasurable_falls_to_origin_not_local() {
        // This is what "local <base> does not exist" looks like — a base branch
        // the human never checked out. Preferring local here would hand
        // `git worktree add` a ref that is not there.
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("unknown revision")]);
        assert_eq!(
            select_start_point(&mock, "/repo", "develop"),
            StartPoint::Remote {
                base: "develop".to_string()
            }
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod start_point_tests {
    use super::*;

    #[test]
    fn remote_start_point_refs_origin_and_keeps_the_bare_base() {
        let sp = StartPoint::Remote {
            base: "develop".to_string(),
        };
        assert_eq!(sp.git_ref(), "origin/develop");
        assert_eq!(sp.base(), "develop");
    }

    #[test]
    fn local_start_point_refs_the_bare_branch() {
        let sp = StartPoint::Local {
            base: "develop".to_string(),
        };
        assert_eq!(sp.git_ref(), "develop");
        assert_eq!(sp.base(), "develop");
    }
}
