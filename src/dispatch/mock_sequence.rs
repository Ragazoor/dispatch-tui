//! One declaration of the subprocess call sequence a dispatch issues, for
//! [`MockProcessRunner`]-based tests.
//!
//! # Why this exists
//!
//! `dispatch_with_prompt` → `provision_worktree` → tmux launch issues a fixed
//! sequence of `git` / `gh` / `tmux` calls, and `MockProcessRunner` answers from
//! a positional queue. Before this module, every test that drove a dispatch
//! hand-wrote that queue as a `vec![ok(), ok(), …]` with a trailing comment per
//! entry, then asserted against hard-coded `calls[N]` indices. Three costs:
//!
//! - Six of the twelve steps are conditional, so the index of every later step
//!   depends on choices the test made earlier — which each site re-derived.
//! - Adding one preflight call meant splicing a response into ~45 vectors at the
//!   right offset and renumbering by hand. #3810 did exactly that and still left
//!   a stale entry behind, caught only because a reviewer counted calls manually.
//! - A stale entry is invisible: a spare `ok_with_stdout` is silently consumed by
//!   whatever call comes next, because most callers ignore stdout.
//!
//! A [`DispatchScript`] declares the *shape* once — which optional steps are
//! present, how the fetch goes, where the sequence stops — and derives both the
//! response queue and the call indices from that one declaration, so the two
//! cannot disagree.
//!
//! # Assertions get stronger, not weaker
//!
//! [`DispatchScript::index_of`] replaces a literal index with a named step, and
//! [`DispatchScript::assert_matches`] turns "this queue is the right sequence"
//! from a trailing comment into a checked claim: an extra, missing, reordered, or
//! stale call fails. Tests that pin exact argv keep doing so — this module only
//! decides *which* call to look at, never what to assert about it.
//!
//! # Finish is the same problem, one operation over
//!
//! [`DispatchScript::finish`] declares `finish_task`'s sequence (three preflight
//! reads → pull? → rebase → mid-rebase status? → abort? → fast-forward?) the
//! same way, for the same reason: four of its eight calls are conditional and
//! each preflight read gates everything after it, and the dirty-worktree
//! preflight landing is exactly the splice-into-every-vector episode described
//! above.
//!
//! # Response index == recorded-call index
//!
//! `MockProcessRunner` answers `tmux::window_target`'s name lookup out of band
//! (see `WindowLookup::AnyName`): it is neither taken from the queue nor
//! recorded. This module relies on that, so a script's Nth step really is
//! `recorded_calls()[N]`. A test using `with_queued_window_lookup` cannot use a
//! script.

use std::process::Output;
use std::time::Duration;

use anyhow::{anyhow, Result};

use super::worktree::FETCH_MAX_ATTEMPTS;
use crate::process::MockProcessRunner;
use crate::tmux;

/// One call `MockProcessRunner` recorded: the program, then its argv.
pub(crate) type RecordedCall = (String, Vec<String>);

/// What driving a scripted `finish_task` produced — the calls it issued and the
/// outcome it returned. Named because the three test modules that drive a finish
/// shape all hand both back from their own helper.
pub(crate) type FinishRun = (
    Vec<RecordedCall>,
    std::result::Result<(), super::FinishError>,
);

/// tmux's reply to the companion pane's `split-window -P`: the new pane's id.
/// Arbitrary but non-positional on purpose — a pane id that does not look like
/// an index cannot be mistaken for one.
pub(crate) const COMPANION_PANE_ID: &[u8] = b"%9\n";

/// `git ls-remote --exit-code`'s "no matching ref" status, i.e. the only failure
/// `classify_fetch_failure` reads as a positive 404.
const LS_REMOTE_NO_MATCHING_REF: i32 = 2;
/// Any other `ls-remote` failure: could not reach the remote at all.
const LS_REMOTE_UNREACHABLE: i32 = 128;

/// The stderr a failing `git fetch origin <base>` stands in with.
const FETCH_FAILURE: &str = "fatal: unable to access 'origin': transient network error";

/// The stderr a `tmux` call stands in with when no server is running — shared
/// by [`Step::HasWindowQuery`], [`Step::NewWindowNameCheck`] and
/// [`Step::NewWindow`], since all three fail the same realistic way when there
/// is nothing to connect to.
const NO_TMUX_SERVER: &str = "no server running on /tmp/tmux-1000/default";

/// What the runner itself reports when a git call cannot be spawned at all —
/// the `Err` arm, as opposed to a git that ran and exited non-zero.
const GIT_NOT_ON_PATH: &str = "git: command not found";

/// `git remote get-url origin`'s reply for a repo that has one.
const ORIGIN_URL: &[u8] = b"git@github.com:org/repo.git\n";

/// The repo root, worktree and branch a finish shape runs against by default.
/// Only their argv matters — `finish_task` runs subprocesses and touches no
/// filesystem of its own, so nothing here has to exist on disk.
const FINISH_REPO: &str = "/repo";
const FINISH_WORKTREE: &str = "/repo/.worktrees/42-fix-bug";
const FINISH_BRANCH: &str = "42-fix-bug";

/// The bound every scripted finish runs under. Short enough that a timing-out
/// shape resolves instantly: `MockProcessRunner::run_with_timeout` bails
/// *without* sleeping once a scripted delay reaches the timeout.
pub(crate) const FINISH_TIMEOUT: Duration = Duration::from_millis(50);

/// `git symbolic-ref refs/remotes/origin/HEAD`'s reply for a repo whose default
/// branch is `branch` — the form `crate::git::detect_default_branch` parses.
fn default_branch_ref(branch: &str) -> Vec<u8> {
    format!("refs/remotes/origin/{branch}\n").into_bytes()
}

/// `gh pr view --json headRefName,isCrossRepository`'s reply: the head branch,
/// then whether the PR comes from a fork, one per line. Stated here once so no
/// call site has to re-derive that the second line drives the fork fallback.
pub(crate) fn pr_view_reply(head: &str, cross_repository: bool) -> Vec<u8> {
    format!("{head}\n{cross_repository}\n").into_bytes()
}

/// `git rev-list --count --left-right <base>...origin/<base>`'s reply: commits
/// local-only, then commits origin-only, tab separated. `level()` is the common
/// case and states once that a zero pair means "no drift, so prefer origin".
fn rev_list_counts(left: u32, right: u32) -> Vec<u8> {
    format!("{left}\t{right}\n").into_bytes()
}

/// One recorded subprocess call in a dispatch's sequence.
///
/// Ordered as issued. [`Step::Fetch`] is the only one that can repeat (see
/// `fetch_origin`); [`DispatchScript::index_of`] reports its first attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    /// `tmux list-windows -a -F #{window_name}` — `resume_agent`'s live
    /// `has_window` check for a stray already-running window under the
    /// deterministic name, issued before anything else. Resume-only: a fresh
    /// [`DispatchScript::dispatch`] never issues this.
    HasWindowQuery,
    /// `git symbolic-ref refs/remotes/origin/HEAD` — only when the caller passed
    /// no base branch, i.e. quick dispatch.
    DetectDefaultBranch,
    /// `gh pr view` — only for a review-tagged task carrying a PR url.
    PrHeadLookup,
    /// `git fetch origin <base>`, retried under `FetchPolicy::Required`.
    Fetch,
    /// `git remote get-url origin` — the first half of `classify_fetch_failure`,
    /// issued once after the first failed fetch under `FetchPolicy::Required`.
    OriginProbe,
    /// `git ls-remote --exit-code origin refs/heads/<base>` — the second half,
    /// which tells a 404 (fall back to local) from an unreachable remote.
    LsRemote,
    /// `git rev-parse --verify --quiet <base>^{commit}` — the local-base probe
    /// that licenses the 404-class fallback (dispatch.allium:
    /// `UnresolvableBaseIsRefusedByName`). Only for `BaseRef::Branch`, only on
    /// a FRESH worktree, and only after `ls-remote` classified the failure as a
    /// missing origin ref: that branch picks local `<base>` on no evidence it
    /// exists, and every other way a start point is chosen is already proven.
    LocalBaseProbe,
    /// `git rev-list --count --left-right <base>...origin/<base>` — `select_start_point`'s
    /// measurement. Only for `BaseRef::Branch`, and only after a fetch succeeded:
    /// a PR head branch is never compared against a local ref.
    AheadBehind,
    /// `git rev-list --count --left-right <start>...HEAD` in the worktree — how
    /// far a REUSED branch is behind the start point and how many commits it
    /// holds of its own (dispatch.allium: `ReusedBranchWithNoOwnWorkIsFastForwarded`).
    /// Reuse path only, for a branch base.
    ReusedBranchDrift,
    /// `git merge --ff-only <start>` in the worktree — only when the reused
    /// branch is behind and holds nothing of its own.
    ReusedBranchFastForward,
    /// `git worktree prune` — clears a stale `.git/worktrees/<name>` record
    /// left by any cause, so the add below cannot fail with "'<branch>' is
    /// already used by worktree at ...". Fresh path only, immediately before
    /// the add (dispatch.allium:
    /// `StaleAdminRecordIsPrunedBeforeWorktreeAdd`).
    WorktreePrune,
    /// `git worktree add` — only when the worktree directory does not exist yet.
    WorktreeAdd,
    /// `tmux list-windows -a -F #{window_name}` — `tmux::new_window`'s own
    /// duplicate-name refusal (dispatch.allium: `TmuxWindowNamesAreUnique`),
    /// issued immediately before the create. Its own variant despite sharing
    /// [`Step::HasWindowQuery`]'s argv, because the two are separately
    /// addressable: that one is `resume_agent`'s caller-level liveness check
    /// and can end the sequence, this one precedes *every* creation. A resume
    /// that creates a window issues both.
    NewWindowNameCheck,
    /// `tmux new-window`
    NewWindow,
    /// `tmux set-option -w … @dispatch_dir`
    SetDispatchDir,
    /// `tmux set-hook after-split-window`
    SetSplitHook,
    /// `tmux send-keys -l <the claude command>`
    SendKeysLiteral,
    /// `tmux send-keys … Enter`
    SendKeysEnter,
    /// `tmux split-window` for the `dispatch agent-tree` companion pane.
    CompanionSplit,
    /// `tmux set-option -p … @dispatch_pane_role agent_tree` on the pane that
    /// split just returned — how dispatch will recognise that pane again.
    CompanionRoleMark,

    // --- finish (`DispatchScript::finish`) ---
    //
    // A finish is a different operation with a disjoint sequence; only
    // [`Step::OriginProbe`] is shared, because it is literally the same
    // `git remote get-url origin` call reached from the other side.
    /// `git rev-parse --abbrev-ref HEAD` in the repo root — `git::current_branch`,
    /// the first thing a finish reads.
    CurrentBranch,
    /// `git status --porcelain` in the repo root — `git::dirty_files`, the
    /// preflight that refuses to rebase into an uncommitted mess.
    DirtyCheck,
    /// `git pull --no-rebase origin <base>` in the repo root — only when the
    /// origin probe found a remote.
    Pull,
    /// `git rebase <base>` in the worktree.
    Rebase,
    /// `git status --porcelain` in the *worktree*, read mid-rebase to name the
    /// conflicted files before the abort below clears that state. Only for a
    /// rebase failure that looks like a conflict.
    ConflictStatus,
    /// `git rebase --abort` in the worktree — after any rebase that ran and
    /// failed.
    RebaseAbort,
    /// `git merge --ff-only <branch>` in the repo root, the last step of a
    /// successful finish.
    FastForward,
}

impl Step {
    /// Whether a recorded call is this step, by program plus the argv token that
    /// distinguishes it from its siblings.
    fn matches(self, program: &str, args: &[String]) -> bool {
        let has = |needle: &str| args.iter().any(|a| a == needle);
        let command_is = |name: &str| args.first().is_some_and(|a| a == name);
        match self {
            Step::HasWindowQuery => program == "tmux" && command_is("list-windows"),
            Step::DetectDefaultBranch => program == "git" && has("symbolic-ref"),
            Step::PrHeadLookup => program == "gh" && has("view"),
            Step::Fetch => program == "git" && has("fetch"),
            Step::OriginProbe => program == "git" && has("remote") && has("get-url"),
            Step::LsRemote => program == "git" && has("ls-remote"),
            // `--verify` is what separates this from `CurrentBranch`, which
            // also runs `rev-parse` (with `--abbrev-ref`).
            Step::LocalBaseProbe => program == "git" && has("rev-parse") && has("--verify"),
            Step::AheadBehind | Step::ReusedBranchDrift => program == "git" && has("rev-list"),
            Step::ReusedBranchFastForward => program == "git" && has("merge") && has("--ff-only"),
            // Both are `git worktree <verb>`, so the verb is the whole
            // difference — `has("worktree")` alone would let a prune pass as
            // an add, which is exactly the ordering this pair exists to pin.
            Step::WorktreePrune => program == "git" && has("worktree") && has("prune"),
            Step::WorktreeAdd => program == "git" && has("worktree") && has("add"),
            Step::NewWindowNameCheck => program == "tmux" && command_is("list-windows"),
            Step::NewWindow => program == "tmux" && command_is("new-window"),
            // The two `set-option` calls differ in scope and in which option they
            // name: the window's dispatch dir, and the new pane's role.
            Step::SetDispatchDir => {
                program == "tmux" && command_is("set-option") && has("@dispatch_dir")
            }
            Step::CompanionRoleMark => {
                program == "tmux" && command_is("set-option") && has(tmux::PANE_ROLE_OPTION)
            }
            Step::SetSplitHook => program == "tmux" && command_is("set-hook"),
            // The two send-keys calls differ only by `-l`, which is exactly what
            // separates the payload from the Enter that submits it.
            Step::SendKeysLiteral => program == "tmux" && command_is("send-keys") && has("-l"),
            Step::SendKeysEnter => program == "tmux" && command_is("send-keys") && !has("-l"),
            Step::CompanionSplit => program == "tmux" && command_is("split-window"),

            Step::CurrentBranch => program == "git" && has("rev-parse"),
            // The two porcelain reads differ only in their `-C` path — the repo
            // root for the preflight, the worktree for the mid-rebase read —
            // which this matcher, being argv-token based, cannot see. Sharing a
            // predicate costs nothing: the rebase sits between them, so
            // `assert_matches` still rejects a reordering, and the two tests
            // that care about the scope assert it through the error's own
            // `path` field.
            Step::DirtyCheck | Step::ConflictStatus => {
                program == "git" && has("status") && has("--porcelain")
            }
            Step::Pull => program == "git" && has("pull"),
            // The abort is a `rebase` too, and `--abort` is the whole difference.
            Step::Rebase => program == "git" && has("rebase") && !has("--abort"),
            Step::RebaseAbort => program == "git" && has("rebase") && has("--abort"),
            Step::FastForward => program == "git" && has("merge") && has("--ff-only"),
        }
    }
}

/// What `gh pr view` reports for a review task's PR.
#[derive(Clone, Copy)]
pub(crate) enum PrHead {
    /// A same-repo PR. `provision_worktree` receives `BaseRef::PrHead`, so no
    /// [`Step::AheadBehind`] is issued.
    Branch(&'static str),
    /// A fork PR: resolvable, but soft-falls back to the base branch — which
    /// means the dispatch proceeds as `BaseRef::Branch` and *does* measure.
    Fork(&'static str),
    /// `gh` itself failed, so nothing is resolved and the base branch is used —
    /// likewise a measuring path.
    Unresolvable,
}

impl PrHead {
    /// Whether this outcome leaves the dispatch on the PR head branch, i.e. the
    /// one case that skips the ahead/behind measurement.
    fn resolves_to_pr_head(self) -> bool {
        matches!(self, PrHead::Branch(_))
    }
}

/// How `git fetch origin <base>` goes. Which of these are even reachable depends
/// on the fetch policy `provision_worktree` picks, which is itself decided by
/// whether the worktree directory already exists — see [`DispatchScript::attempts`].
#[derive(Clone, Copy)]
enum FetchOutcome {
    /// No base ref was resolved, so no fetch is attempted at all.
    Absent,
    /// Attempts `1..n` fail, attempt `n` succeeds. `1` is the plain happy path;
    /// anything higher needs `FetchPolicy::Required`, i.e. a fresh worktree.
    SucceedsOnAttempt(u32),
    /// Every attempt fails and the classification probe positively identifies a
    /// missing ref. Not retried — retrying a branch that does not exist cannot
    /// succeed — so exactly one attempt plus the two probes.
    NoOriginRef,
    /// Every attempt fails and the remote is unreachable. Under `Required` this
    /// costs the probes plus the full retry budget and then aborts the dispatch;
    /// under `BestEffort` (reuse path) it is one attempt, no probes, a warning.
    Unreachable,
    /// Every attempt answers *successfully* but takes longer than the caller's
    /// timeout, so `run_with_timeout` kills it and reports an error instead.
    ///
    /// Deliberately distinct from [`Self::Unreachable`]: a plain failure would
    /// pass even if `provision_worktree` regressed to the unbounded `run`, so the
    /// watchdog tests need the response itself to be a success.
    TimesOut(Duration),
}

/// How one of the finish path's fallible git calls answers. Only [`Self::Ok`]
/// lets the sequence continue; the other three end it where they sit.
#[derive(Clone, Copy)]
enum CallOutcome {
    /// Exits zero.
    Ok,
    /// Exits non-zero, with the step's own realistic stderr (see
    /// [`failure_stderr`]).
    Fails,
    /// The process could not be spawned at all, i.e. the runner itself returns
    /// `Err`. Distinct from [`Self::Fails`] because `finish_task` reaches it
    /// through a different arm — `map_err` rather than a `status.success()`
    /// check — and names it differently.
    CannotRun,
    /// Answers *successfully*, but only after `delay`, so a caller bounding the
    /// call kills it first. Deliberately not a plain failure: a success held
    /// back past the deadline still fails if the call ever regresses to the
    /// unbounded `run`, which is exactly what the bounding tests defend.
    TimesOut(Duration),
}

impl CallOutcome {
    fn is_ok(self) -> bool {
        matches!(self, CallOutcome::Ok)
    }

    /// This outcome's response, with `stderr` used only by [`Self::Fails`].
    fn response(self, stderr: &str) -> (Option<Duration>, Result<Output>) {
        match self {
            CallOutcome::Ok => (None, MockProcessRunner::ok()),
            CallOutcome::Fails => (None, MockProcessRunner::fail(stderr)),
            CallOutcome::CannotRun => (None, Err(anyhow!("{GIT_NOT_ON_PATH}"))),
            CallOutcome::TimesOut(delay) => (Some(delay), MockProcessRunner::ok()),
        }
    }
}

/// How `git rebase <base>` goes. A rebase has every [`CallOutcome`] the other
/// calls do, plus one that is its alone: a *conflict*, which is what decides
/// whether the mid-rebase porcelain read happens at all. Only that arm is
/// modelled here; the rest delegate, so a new outcome shape is added once.
#[derive(Clone, Copy)]
enum RebaseOutcome {
    /// Goes the way any other call can go. [`CallOutcome::Fails`] is the
    /// non-conflict failure — an invalid upstream, a dirty worktree — which is
    /// aborted and reported as `FinishError::Other` with no mid-rebase read,
    /// because there is no conflict to name.
    Plain(CallOutcome),
    /// Exits non-zero carrying git's `CONFLICT (content): …` marker. Which
    /// stream carries it is a real axis, not a detail: `is_rebase_conflict`
    /// reads both, and a regression to reading only one would pass the other's
    /// tests. `files` appear both in the marker and in the mid-rebase porcelain
    /// read that follows.
    Conflicts {
        files: &'static [&'static str],
        on_stderr: bool,
    },
}

/// What `git rev-parse --abbrev-ref HEAD` reports for the repo root. One enum
/// rather than a branch plus a can-be-read flag, so "HEAD is on `x`" and "HEAD
/// could not be read" cannot both be declared at once.
#[derive(Clone, Copy)]
enum HeadOutcome {
    /// HEAD is on the base branch, so the finish proceeds.
    OnBase,
    /// HEAD is on some other branch, so the finish refuses before touching
    /// anything.
    On(&'static str),
    /// The read's subprocess could not be spawned.
    CannotRun,
}

/// Which checkout a finish's call runs in, i.e. what its `-C` names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    /// The repo root, where the base branch is checked out.
    Repo,
    /// The task's worktree, where the task branch is.
    Worktree,
}

/// What `git remote get-url origin` reports, i.e. whether the pull happens.
#[derive(Clone, Copy)]
enum RemoteOutcome {
    /// A remote is configured, so the base branch is pulled first.
    Present,
    /// No remote: the probe exits non-zero and the pull is skipped.
    Absent,
    /// The probe could not be *run*, which is not the same as finding no
    /// remote — `finish_task` stops rather than rebasing onto a base it never
    /// refreshed.
    CannotRun,
}

/// The shape of one `finish_task` call sequence: the preflight reads, the
/// optional pull, the rebase and whatever that rebase's outcome pulls in.
///
/// Its own struct behind one `Option` on [`DispatchScript`] rather than more
/// flat fields, because a finish's axes and a dispatch's are disjoint — folding
/// them together would put `fails_at(Step::NewWindow)` in reach of a shape that
/// launches no tmux window at all.
#[derive(Clone, Copy)]
struct Finish {
    /// The repo root and the task worktree the finish is driven against. Held
    /// here so [`DispatchScript::drive_finish`] can build the `FinishContext`
    /// from the same declaration the scope assertion reads.
    repo_path: &'static str,
    worktree: &'static str,
    /// The task branch, i.e. the fast-forward target.
    branch: &'static str,
    /// The branch the repo root must be on, and the rebase/pull target.
    base_branch: &'static str,
    head: HeadOutcome,
    /// Paths the repo root's working tree reports as dirty. Empty is clean.
    dirty: &'static [&'static str],
    remote: RemoteOutcome,
    pull: CallOutcome,
    rebase: RebaseOutcome,
    fast_forward: CallOutcome,
}

impl Finish {
    /// The branch HEAD reports, which is the base branch unless a test moved it.
    fn head_branch(&self) -> &'static str {
        match self.head {
            HeadOutcome::On(branch) => branch,
            HeadOutcome::OnBase | HeadOutcome::CannotRun => self.base_branch,
        }
    }

    /// Which checkout `step` runs in — the repo root or the task worktree.
    ///
    /// `finish_task` splits its calls across both, and getting that wrong is not
    /// cosmetic: a mid-rebase porcelain read against the repo root names the
    /// wrong files in `RebaseConflict`, and a preflight dirty check against the
    /// worktree clears the wrong tree. Argv alone cannot tell the two porcelain
    /// reads apart, so [`DispatchScript::assert_matches`] checks this separately.
    fn scope_of(step: Step) -> Scope {
        match step {
            Step::CurrentBranch | Step::DirtyCheck | Step::OriginProbe | Step::Pull => Scope::Repo,
            Step::Rebase | Step::ConflictStatus | Step::RebaseAbort => Scope::Worktree,
            // The fast-forward is the repo root's own merge, not the worktree's.
            Step::FastForward => Scope::Repo,
            other => unreachable!("{other:?} is not part of a finish sequence"),
        }
    }

    /// Assert call `i`'s `-C` path is the checkout [`Self::scope_of`] expects.
    fn assert_scope(&self, i: usize, step: Step, args: &[String]) {
        let (scope, expected) = match Self::scope_of(step) {
            Scope::Repo => (Scope::Repo, self.repo_path),
            Scope::Worktree => (Scope::Worktree, self.worktree),
        };
        // `-C <path>` is how every one of these calls names its checkout, so the
        // token after `-C` is the whole claim.
        let got = args
            .iter()
            .position(|a| a == "-C")
            .and_then(|at| args.get(at + 1));
        assert_eq!(
            got.map(String::as_str),
            Some(expected),
            "call {i} ({step:?}) must run in the {scope:?} ({expected}), got argv {args:?}",
        );
    }

    /// The steps a finish of this shape issues, one entry per recorded call.
    /// Mirrors `finish_task`'s own control flow, which is what the self-tests in
    /// this module hold it to.
    fn steps(&self) -> Vec<Step> {
        let mut steps = vec![Step::CurrentBranch];
        // A HEAD that could not be read, or that is not on the base branch, is
        // refused before anything is touched.
        if !matches!(self.head, HeadOutcome::OnBase) {
            return steps;
        }
        steps.push(Step::DirtyCheck);
        if !self.dirty.is_empty() {
            return steps;
        }
        steps.push(Step::OriginProbe);
        match self.remote {
            RemoteOutcome::CannotRun => return steps,
            RemoteOutcome::Present => {
                steps.push(Step::Pull);
                if !self.pull.is_ok() {
                    return steps;
                }
            }
            RemoteOutcome::Absent => {}
        }
        steps.push(Step::Rebase);
        match self.rebase {
            RebaseOutcome::Plain(CallOutcome::Ok) => steps.push(Step::FastForward),
            // The conflicted files are read out of the worktree's own status
            // while the rebase is still mid-flight, because the abort clears it.
            RebaseOutcome::Conflicts { .. } => {
                steps.push(Step::ConflictStatus);
                steps.push(Step::RebaseAbort);
            }
            // A rebase that ran and exited non-zero is aborted, and so is one
            // that could not finish — a spawn failure or a kill at the bound
            // may have left the worktree mid-rebase.
            RebaseOutcome::Plain(
                CallOutcome::Fails | CallOutcome::CannotRun | CallOutcome::TimesOut(_),
            ) => steps.push(Step::RebaseAbort),
        }
        steps
    }

    /// How `step` answers under this shape.
    fn response(&self, step: Step) -> (Option<Duration>, Result<Output>) {
        match step {
            Step::CurrentBranch => match self.head {
                HeadOutcome::CannotRun => (None, Err(anyhow!("{GIT_NOT_ON_PATH}"))),
                HeadOutcome::OnBase | HeadOutcome::On(_) => {
                    let head = format!("{}\n", self.head_branch());
                    (None, MockProcessRunner::ok_with_stdout(head.as_bytes()))
                }
            },
            Step::DirtyCheck => (
                None,
                MockProcessRunner::ok_with_stdout(&porcelain_lines(" M", self.dirty)),
            ),
            Step::OriginProbe => match self.remote {
                RemoteOutcome::Present => (None, MockProcessRunner::ok_with_stdout(ORIGIN_URL)),
                // "No remote" is a non-zero exit, not an error: `has_origin_remote`
                // reads the status, and only an `Err` means it could not look.
                RemoteOutcome::Absent => (None, MockProcessRunner::fail("")),
                RemoteOutcome::CannotRun => (None, Err(anyhow!("{GIT_NOT_ON_PATH}"))),
            },
            Step::Pull => self.pull.response(failure_stderr(Step::Pull)),
            Step::Rebase => rebase_response(self.rebase),
            Step::ConflictStatus => match self.rebase {
                RebaseOutcome::Conflicts { files, .. } => (
                    None,
                    MockProcessRunner::ok_with_stdout(&porcelain_lines("UU", files)),
                ),
                _ => unreachable!("ConflictStatus is only emitted for a conflicting rebase"),
            },
            // Best-effort cleanup whose result `finish_task` discards.
            Step::RebaseAbort => (None, MockProcessRunner::ok()),
            Step::FastForward => self
                .fast_forward
                .response(failure_stderr(Step::FastForward)),
            other => unreachable!("{other:?} is not part of a finish sequence"),
        }
    }
}

/// `git rebase`'s reply for each [`RebaseOutcome`]. Only the conflict arm is
/// built here — the rest are ordinary [`CallOutcome`]s — and it is built by hand
/// because it is a *failure carrying stdout*, which no `MockProcessRunner`
/// helper produces.
fn rebase_response(outcome: RebaseOutcome) -> (Option<Duration>, Result<Output>) {
    match outcome {
        RebaseOutcome::Plain(plain) => plain.response(failure_stderr(Step::Rebase)),
        RebaseOutcome::Conflicts { files, on_stderr } => {
            let marker = conflict_markers(files);
            let response = if on_stderr {
                MockProcessRunner::fail(&marker)
            } else {
                Ok(Output {
                    status: crate::process::exit_fail(),
                    stdout: marker.into_bytes(),
                    stderr: Vec::new(),
                })
            };
            (None, response)
        }
    }
}

/// `git status --porcelain`'s reply listing `paths` under status `code`.
///
/// The code is what each reader selects on: `parse_porcelain_files` keeps every
/// entry regardless (so ` M` stands in for any dirty path), while
/// `parse_unmerged_files` keeps only the conflict codes (so `UU`). That both
/// parse the column correctly — including the leading-space and `??` forms — is
/// covered directly in `src/git.rs`, so this need only pick one form of each.
fn porcelain_lines(code: &str, paths: &[&str]) -> Vec<u8> {
    paths
        .iter()
        .map(|path| format!("{code} {path}\n"))
        .collect::<String>()
        .into_bytes()
}

/// `git rebase`'s conflict report for `files`, one marker line each — the form
/// `is_rebase_conflict` matches on.
fn conflict_markers(files: &[&str]) -> String {
    files
        .iter()
        .map(|file| format!("CONFLICT (content): Merge conflict in {file}\n"))
        .collect()
}

/// Where the sequence ends.
#[derive(Clone, Copy)]
enum Ending {
    /// Runs through [`Step::CompanionRoleMark`], i.e. the whole companion-pane
    /// tail.
    Complete,
    /// This step succeeds and nothing beyond it is queued — for a dispatch that
    /// stops for a reason other than a subprocess failure (e.g. the
    /// `.claude-prompt` write failing because the mock never created the
    /// worktree directory).
    StopsAfter(Step),
    /// This step returns a failure and nothing beyond it is queued, so a call
    /// the code should not have made panics the mock instead of passing.
    FailsAt(Step),
}

/// Where a REUSED worktree's branch stands against the start point.
#[derive(Clone, Copy)]
enum ReusedBranch {
    /// Level with the start point: nothing to do.
    InSync,
    /// `n` commits behind and none of its own: fast-forwarded.
    Behind(u32),
    /// Holds `n` commits of its own: left alone.
    OwnCommits(u32),
}

/// The shape of one dispatch's call sequence.
///
/// Holds configuration only — no responses — so it stays usable for
/// [`Self::index_of`] and [`Self::assert_matches`] after [`Self::runner`] has
/// built the queue.
#[derive(Clone, Copy)]
pub(crate) struct DispatchScript {
    default_branch: Option<&'static str>,
    pr_head: Option<PrHead>,
    fetch: FetchOutcome,
    /// Commits local `<base>` holds that origin lacks, as reported by
    /// [`Step::AheadBehind`]. `0` means "no drift", so origin wins.
    local_ahead: u32,
    fresh_worktree: bool,
    /// Only read on the reuse path with a branch base.
    reused_branch: ReusedBranch,
    /// Whether this shape is a resume (as opposed to a fresh dispatch) — the
    /// only shape that issues [`Step::HasWindowQuery`], since the live
    /// `has_window` check lives in `resume_agent`, not `dispatch_agent`.
    is_resume: bool,
    /// Only meaningful when `is_resume`: the window `has_window` looks for is
    /// already alive, so `resume_agent` short-circuits after the check —
    /// see [`Self::window_already_alive`].
    window_already_alive: bool,
    ending: Ending,
    /// Present only for a [`Self::finish`] shape. `Some` makes every field
    /// above irrelevant: a finish issues none of a dispatch's calls.
    finish: Option<Finish>,
}

impl DispatchScript {
    /// One successful dispatch: an existing worktree directory, a fetch that
    /// succeeds first try, a level ahead/behind reading, and a launch that runs
    /// through the companion pane.
    ///
    /// The default because it is what most dispatch tests want: they pre-create
    /// the worktree directory so the `.claude-prompt` write succeeds, which is
    /// itself what puts provisioning on the reuse branch.
    pub(crate) fn dispatch() -> Self {
        Self {
            default_branch: None,
            pr_head: None,
            fetch: FetchOutcome::SucceedsOnAttempt(1),
            local_ahead: 0,
            fresh_worktree: false,
            reused_branch: ReusedBranch::InSync,
            is_resume: false,
            window_already_alive: false,
            ending: Ending::Complete,
            finish: None,
        }
    }

    /// `finish_task`'s sequence: the three preflight reads, a pull, the rebase
    /// and the fast-forward. The default is the fullest successful path — a
    /// remote is configured, so the pull happens, and every call succeeds.
    ///
    /// Shares nothing with a dispatch but [`Step::OriginProbe`], so the
    /// dispatch modifiers do not apply to it and panic if used — as the finish
    /// modifiers do on a dispatch shape.
    pub(crate) fn finish() -> Self {
        let mut script = Self::dispatch();
        script.finish = Some(Finish {
            repo_path: FINISH_REPO,
            worktree: FINISH_WORKTREE,
            branch: FINISH_BRANCH,
            base_branch: "main",
            head: HeadOutcome::OnBase,
            dirty: &[],
            remote: RemoteOutcome::Present,
            pull: CallOutcome::Ok,
            rebase: RebaseOutcome::Plain(CallOutcome::Ok),
            fast_forward: CallOutcome::Ok,
        });
        script
    }

    /// The finish configuration, for the modifiers below.
    ///
    /// # Panics
    ///
    /// If this is not a [`Self::finish`] shape — a finish modifier on a
    /// dispatch shape would otherwise be a silent no-op.
    fn finish_mut(&mut self) -> &mut Finish {
        match self.finish.as_mut() {
            Some(finish) => finish,
            None => panic!("this modifier only applies to a finish() shape"),
        }
    }

    /// Guards a *dispatch* modifier, the mirror of [`Self::finish_mut`].
    ///
    /// Without it, `finish().fetch_times_out(d)` would set a field `steps()`
    /// early-returns past: it compiles, passes, and asserts nothing — the exact
    /// silent no-op the finish-side guard exists to prevent.
    ///
    /// # Panics
    ///
    /// If this is a [`Self::finish`] shape.
    fn assert_is_dispatch(&self) {
        assert!(
            self.finish.is_none(),
            "this modifier only applies to a dispatch/resume/provision shape"
        );
    }

    /// The branch the repo root is on and the rebase targets. Defaults to
    /// `main`.
    pub(crate) fn base_branch(mut self, branch: &'static str) -> Self {
        self.finish_mut().base_branch = branch;
        self
    }

    /// The repo root is on `branch` rather than the base branch, so the finish
    /// refuses before touching anything — its sequence is the HEAD read alone.
    pub(crate) fn head_branch(mut self, branch: &'static str) -> Self {
        self.finish_mut().head = HeadOutcome::On(branch);
        self
    }

    /// `git::current_branch`'s subprocess cannot be spawned, so the finish stops
    /// at its very first call.
    pub(crate) fn current_branch_cannot_run(mut self) -> Self {
        self.finish_mut().head = HeadOutcome::CannotRun;
        self
    }

    /// The repo root's working tree reports `paths` as dirty, so the finish
    /// stops after the preflight read — before any pull, rebase or merge.
    pub(crate) fn dirty_primary(mut self, paths: &'static [&'static str]) -> Self {
        self.finish_mut().dirty = paths;
        self
    }

    /// No origin remote is configured, so the pull is skipped entirely.
    pub(crate) fn no_remote(mut self) -> Self {
        self.finish_mut().remote = RemoteOutcome::Absent;
        self
    }

    /// The origin probe could not be *run*. Not the same as [`Self::no_remote`]:
    /// a probe that identified nothing is a failure worth naming, not a licence
    /// to rebase onto a base that was never refreshed.
    pub(crate) fn remote_probe_cannot_run(mut self) -> Self {
        self.finish_mut().remote = RemoteOutcome::CannotRun;
        self
    }

    /// The pull exits non-zero.
    pub(crate) fn pull_fails(mut self) -> Self {
        self.finish_mut().pull = CallOutcome::Fails;
        self
    }

    /// The pull's subprocess cannot be spawned.
    pub(crate) fn pull_cannot_run(mut self) -> Self {
        self.finish_mut().pull = CallOutcome::CannotRun;
        self
    }

    /// The pull succeeds but only after `delay`, so a bounded caller kills it.
    /// See [`CallOutcome::TimesOut`] for why this is not just a failure.
    pub(crate) fn pull_times_out(mut self, delay: Duration) -> Self {
        self.finish_mut().pull = CallOutcome::TimesOut(delay);
        self
    }

    /// The rebase conflicts, with git's marker on **stdout** — one of the two
    /// streams `is_rebase_conflict` reads. `files` come back both in the marker
    /// and in the mid-rebase porcelain read.
    pub(crate) fn rebase_conflicts_in_stdout(mut self, files: &'static [&'static str]) -> Self {
        self.finish_mut().rebase = RebaseOutcome::Conflicts {
            files,
            on_stderr: false,
        };
        self
    }

    /// The rebase conflicts with the marker on **stderr** — the other stream.
    pub(crate) fn rebase_conflicts_in_stderr(mut self, files: &'static [&'static str]) -> Self {
        self.finish_mut().rebase = RebaseOutcome::Conflicts {
            files,
            on_stderr: true,
        };
        self
    }

    /// The rebase exits non-zero with no conflict marker, so it is aborted and
    /// reported as `Other` — with no mid-rebase porcelain read.
    pub(crate) fn rebase_fails(mut self) -> Self {
        self.finish_mut().rebase = RebaseOutcome::Plain(CallOutcome::Fails);
        self
    }

    /// The rebase's subprocess cannot be spawned; the abort is still attempted.
    pub(crate) fn rebase_cannot_run(mut self) -> Self {
        self.finish_mut().rebase = RebaseOutcome::Plain(CallOutcome::CannotRun);
        self
    }

    /// The rebase answers past the caller's bound and is killed.
    pub(crate) fn rebase_times_out(mut self, delay: Duration) -> Self {
        self.finish_mut().rebase = RebaseOutcome::Plain(CallOutcome::TimesOut(delay));
        self
    }

    /// Drive a real `finish_task` under this shape, against the paths and base
    /// branch the shape itself declares, and return the calls it recorded with
    /// its outcome — after asserting the calls are exactly the declared steps.
    ///
    /// Every test module that drives a finish goes through here, so the context
    /// can never contradict the script: before this existed, three separate
    /// helpers each built their own `FinishContext` and two of them hardcoded a
    /// base branch the script was free to change underneath them.
    ///
    /// # Panics
    ///
    /// If this is not a [`Self::finish`] shape, or if the recorded calls are not
    /// the declared sequence.
    pub(crate) fn drive_finish(&self) -> FinishRun {
        let mock = self.runner();
        let result = super::finish_task(&self.finish_context(), &mock);
        let calls = mock.recorded_calls();
        self.assert_matches(&calls);
        (calls, result)
    }

    /// The `FinishContext` this shape declares — the paths, the branch, the base
    /// branch and the bound, all from one place.
    ///
    /// [`Self::drive_finish`] is the usual way in; reach for this directly only
    /// when a test needs the `MockProcessRunner` itself afterwards (to read
    /// `recorded_timeouts`, say), which `drive_finish` does not hand back.
    ///
    /// # Panics
    ///
    /// If this is not a [`Self::finish`] shape.
    pub(crate) fn finish_context(&self) -> super::FinishContext<'static> {
        let finish = match self.finish.as_ref() {
            Some(finish) => finish,
            None => panic!("finish_context only applies to a finish() shape"),
        };
        super::FinishContext {
            repo_path: finish.repo_path,
            worktree: finish.worktree,
            branch: finish.branch,
            base_branch: finish.base_branch,
            timeout: FINISH_TIMEOUT,
        }
    }

    /// The fast-forward exits non-zero.
    pub(crate) fn fast_forward_fails(mut self) -> Self {
        self.finish_mut().fast_forward = CallOutcome::Fails;
        self
    }

    /// The fast-forward's subprocess cannot be spawned.
    pub(crate) fn fast_forward_cannot_run(mut self) -> Self {
        self.finish_mut().fast_forward = CallOutcome::CannotRun;
        self
    }

    /// The fast-forward answers past the caller's bound and is killed.
    pub(crate) fn fast_forward_times_out(mut self, delay: Duration) -> Self {
        self.finish_mut().fast_forward = CallOutcome::TimesOut(delay);
        self
    }

    /// `resume_agent`'s sequence: the tmux tail only, since resume reuses the
    /// worktree that already exists and touches git not at all.
    pub(crate) fn resume() -> Self {
        let mut script = Self::dispatch().no_fetch();
        script.is_resume = true;
        script
    }

    /// The window `resume_agent`'s `has_window` check looks for (the
    /// deterministic `task-<id>`, matching every resume test's `TaskId(42)` —
    /// see `resume_script_matches_a_real_resume`) is already alive. Resume
    /// then reports success without creating a window, sending keys, or
    /// spawning a companion pane — so this shape's sequence ends right after
    /// [`Step::HasWindowQuery`].
    ///
    /// # Panics
    ///
    /// If called on a non-resume shape — the check this models only exists in
    /// `resume_agent`.
    pub(crate) fn window_already_alive(mut self) -> Self {
        assert!(
            self.is_resume,
            "window_already_alive only applies to a resume() shape"
        );
        self.window_already_alive = true;
        self
    }

    /// `provision_worktree` called directly: everything up to and including the
    /// split hook, with no prompt write and no agent launch.
    pub(crate) fn provision() -> Self {
        Self::dispatch().stops_after(Step::SetSplitHook)
    }

    /// The worktree directory does not exist yet, so `git worktree add` runs —
    /// and the fetch becomes `FetchPolicy::Required`, which is what makes the
    /// retry budget and the classification probes reachable.
    pub(crate) fn fresh_worktree(mut self) -> Self {
        self.assert_is_dispatch();
        self.fresh_worktree = true;
        self
    }

    /// No base ref was resolved, so no fetch is attempted.
    pub(crate) fn no_fetch(mut self) -> Self {
        self.assert_is_dispatch();
        self.fetch = FetchOutcome::Absent;
        self
    }

    /// The fetch fails until attempt `n`, which succeeds. Needs a fresh worktree
    /// for `n > 1`: the reuse path only ever makes one attempt.
    pub(crate) fn fetch_succeeds_on_attempt(mut self, n: u32) -> Self {
        self.assert_is_dispatch();
        assert!(n >= 1, "fetch attempts are 1-based");
        self.fetch = FetchOutcome::SucceedsOnAttempt(n);
        self
    }

    /// The fetch fails and the probes identify a missing origin ref, so the local
    /// branch is used with a `Note:` and no retry happens.
    pub(crate) fn fetch_finds_no_origin_ref(mut self) -> Self {
        self.assert_is_dispatch();
        self.fetch = FetchOutcome::NoOriginRef;
        self
    }

    /// The fetch fails with the remote unreachable. On a fresh worktree that
    /// aborts the dispatch after the probes and the full budget; on the reuse
    /// path it is one attempt, no probes, and a warning.
    pub(crate) fn fetch_is_unreachable(mut self) -> Self {
        self.assert_is_dispatch();
        self.fetch = FetchOutcome::Unreachable;
        self
    }

    /// Every fetch attempt succeeds but takes `delay` to answer, so a caller
    /// passing a shorter timeout kills it instead. See
    /// [`FetchOutcome::TimesOut`] for why this is not the same as
    /// [`Self::fetch_is_unreachable`].
    pub(crate) fn fetch_times_out(mut self, delay: Duration) -> Self {
        self.assert_is_dispatch();
        self.fetch = FetchOutcome::TimesOut(delay);
        self
    }

    /// Local `<base>` holds `n` commits origin lacks, so `select_start_point`
    /// prefers the local ref. The default is `0` — no drift, origin wins.
    pub(crate) fn local_ahead(mut self, n: u32) -> Self {
        self.assert_is_dispatch();
        self.local_ahead = n;
        self
    }

    /// The reused worktree's branch is `n` commits behind the start point with
    /// none of its own, so provisioning fast-forwards it.
    pub(crate) fn reused_branch_behind(mut self, n: u32) -> Self {
        self.assert_is_dispatch();
        self.reused_branch = ReusedBranch::Behind(n);
        self
    }

    /// The reused worktree's branch holds `n` commits of its own, so
    /// provisioning leaves it where it is.
    pub(crate) fn reused_branch_with_own_commits(mut self, n: u32) -> Self {
        self.assert_is_dispatch();
        self.reused_branch = ReusedBranch::OwnCommits(n);
        self
    }

    /// The caller passed no base branch, so the dispatch detects the repo's
    /// default branch first and then fetches `branch`.
    pub(crate) fn detecting_default_branch(mut self, branch: &'static str) -> Self {
        self.assert_is_dispatch();
        self.default_branch = Some(branch);
        self
    }

    /// This is a review task carrying a PR url, so `gh pr view` runs first.
    pub(crate) fn pr_head(mut self, head: PrHead) -> Self {
        self.assert_is_dispatch();
        self.pr_head = Some(head);
        self
    }

    /// `step` succeeds and nothing after it is queued. See [`Ending::StopsAfter`].
    ///
    /// Private because [`Self::provision`] is the only shape that needs it; widen
    /// it if a test ever needs a different stopping point.
    fn stops_after(mut self, step: Step) -> Self {
        self.assert_is_dispatch();
        self.ending = Ending::StopsAfter(step);
        self
    }

    /// `step` fails and nothing after it is queued. See [`Ending::FailsAt`].
    pub(crate) fn fails_at(mut self, step: Step) -> Self {
        // Not the generic guard: a finish shape *can* state a failure, just not
        // this way, so the message points at the modifier that does it.
        assert!(
            self.finish.is_none(),
            "a finish() shape states its failures directly — use one of the \
             *_fails / *_cannot_run modifiers instead"
        );
        assert!(
            !matches!(step, Step::Fetch | Step::OriginProbe | Step::LsRemote),
            "a failing fetch is retried and classified, not terminal — use one of \
             the fetch_* modifiers instead"
        );
        self.ending = Ending::FailsAt(step);
        self
    }

    /// Whether the fetch runs under `FetchPolicy::Required`, which is what makes
    /// retries and the classification probes reachable. Mirrors
    /// `provision_worktree`: the policy follows the worktree directory's
    /// existence, nothing else.
    fn fetch_is_required(&self) -> bool {
        self.fresh_worktree
    }

    /// How many `git fetch` calls this shape issues.
    fn attempts(&self) -> u32 {
        match self.fetch {
            FetchOutcome::Absent => 0,
            FetchOutcome::SucceedsOnAttempt(n) => n,
            // A positively-identified 404 is never retried.
            FetchOutcome::NoOriginRef => 1,
            FetchOutcome::Unreachable | FetchOutcome::TimesOut(_) => {
                if self.fetch_is_required() {
                    FETCH_MAX_ATTEMPTS
                } else {
                    1
                }
            }
        }
    }

    /// Whether `classify_fetch_failure`'s two probes run: only when a fetch
    /// actually failed *and* the policy is `Required`, since `BestEffort` never
    /// needs to tell a 404 from an outage.
    fn classifies(&self) -> bool {
        if !self.fetch_is_required() {
            return false;
        }
        match self.fetch {
            FetchOutcome::Absent => false,
            FetchOutcome::SucceedsOnAttempt(n) => n > 1,
            FetchOutcome::NoOriginRef | FetchOutcome::Unreachable | FetchOutcome::TimesOut(_) => {
                true
            }
        }
    }

    /// Whether `select_start_point` measures local against origin. Requires a
    /// fetch that ultimately succeeded, and a base that is not a PR head.
    fn measures(&self) -> bool {
        let fetched = matches!(self.fetch, FetchOutcome::SucceedsOnAttempt(_));
        let on_pr_head = self.pr_head.is_some_and(PrHead::resolves_to_pr_head);
        fetched && !on_pr_head
    }

    /// Whether provisioning reconciles a reused branch with the start point:
    /// the reuse path, a base to reconcile with, and not a PR head.
    fn reconciles_reused_branch(&self) -> bool {
        let on_pr_head = self.pr_head.is_some_and(PrHead::resolves_to_pr_head);
        !self.fresh_worktree
            && !self.is_resume
            && !matches!(self.fetch, FetchOutcome::Absent)
            && !on_pr_head
    }

    /// Whether the local-base probe runs (dispatch.allium:
    /// `UnresolvableBaseIsRefusedByName`). The 404-class fallback to local
    /// `<base>` is the one start-point choice nothing else proves, so it is the
    /// one that is probed — and only where the ref is about to be consumed,
    /// which is the fresh path. A PR head never falls back to a local ref at
    /// all, so it never reaches the probe.
    fn probes_local_base(&self) -> bool {
        let on_pr_head = self.pr_head.is_some_and(PrHead::resolves_to_pr_head);
        self.fresh_worktree
            && self.fetch_is_required()
            && matches!(self.fetch, FetchOutcome::NoOriginRef)
            && !on_pr_head
    }

    /// Whether the fetch itself ends the dispatch. Under `FetchPolicy::Required`
    /// an unreachable origin is a hard error — a worktree silently branched off a
    /// stale local ref is worse than a dispatch that refuses to start — so
    /// `provision_worktree` propagates it and nothing after the last attempt runs.
    ///
    /// A 404 is *not* an abort for a branch base: local `<base>` is then the only
    /// ref there is, so provisioning continues on it.
    fn fetch_aborts(&self) -> bool {
        self.fetch_is_required()
            && matches!(
                self.fetch,
                FetchOutcome::Unreachable | FetchOutcome::TimesOut(_)
            )
    }

    /// The steps this shape issues, one entry per recorded call, in order.
    fn steps(&self) -> Vec<Step> {
        if let Some(finish) = self.finish.as_ref() {
            return finish.steps();
        }
        let mut steps = Vec::new();
        if self.default_branch.is_some() {
            steps.push(Step::DetectDefaultBranch);
        }
        if self.pr_head.is_some() {
            steps.push(Step::PrHeadLookup);
        }
        // The classification probes sit *between* attempt 1 and the rest:
        // `fetch_origin` classifies once, on the first failure.
        let attempts = self.attempts();
        for attempt in 1..=attempts {
            steps.push(Step::Fetch);
            if attempt == 1 && self.classifies() {
                steps.push(Step::OriginProbe);
                steps.push(Step::LsRemote);
            }
        }
        // An aborting fetch is itself the end of the sequence: `provision_worktree`
        // propagates the error, so no start point is measured and no worktree or
        // tmux window is created.
        if self.fetch_aborts() {
            return steps;
        }
        if self.probes_local_base() {
            steps.push(Step::LocalBaseProbe);
        }
        if self.measures() {
            steps.push(Step::AheadBehind);
        }
        if self.reconciles_reused_branch() {
            steps.push(Step::ReusedBranchDrift);
            if matches!(self.reused_branch, ReusedBranch::Behind(_)) {
                steps.push(Step::ReusedBranchFastForward);
            }
        }
        if self.fresh_worktree {
            steps.push(Step::WorktreePrune);
            steps.push(Step::WorktreeAdd);
        }
        if self.is_resume {
            steps.push(Step::HasWindowQuery);
            if self.window_already_alive {
                // Found alive: resume_agent returns immediately, without
                // creating a window, sending keys, or spawning a companion
                // pane.
                return steps;
            }
        }
        steps.extend([
            Step::NewWindowNameCheck,
            Step::NewWindow,
            Step::SetDispatchDir,
            Step::SetSplitHook,
            Step::SendKeysLiteral,
            Step::SendKeysEnter,
            Step::CompanionSplit,
            Step::CompanionRoleMark,
        ]);

        let last = match self.ending {
            Ending::Complete => return steps,
            Ending::StopsAfter(step) | Ending::FailsAt(step) => step,
        };
        let cut = steps
            .iter()
            .position(|s| *s == last)
            .unwrap_or_else(|| panic!("{last:?} is not part of this script's sequence"));
        steps.truncate(cut + 1);
        steps
    }

    /// The recorded-call index of `step` — its *first* call, for the repeatable
    /// [`Step::Fetch`].
    ///
    /// # Panics
    ///
    /// If `step` is not part of this shape. Asserting against a call the script
    /// never queued is a test bug, and a loud one beats an index that silently
    /// points at a neighbouring call.
    pub(crate) fn index_of(&self, step: Step) -> usize {
        self.steps()
            .iter()
            .position(|s| *s == step)
            .unwrap_or_else(|| panic!("{step:?} is not part of this script's sequence"))
    }

    /// The response queue for this shape, ready to inject as a `ProcessRunner`.
    pub(crate) fn runner(&self) -> MockProcessRunner {
        MockProcessRunner::new_with_delays(self.responses())
    }

    /// [`Self::runner`] behind an `Arc`, for the async fixtures that hold their
    /// runner as one.
    pub(crate) fn shared_runner(&self) -> std::sync::Arc<MockProcessRunner> {
        std::sync::Arc::new(self.runner())
    }

    /// One `(delay, response)` pair per step, in order. Only `Fetch` ever carries
    /// a delay (see [`Self::fetch_times_out`]).
    /// `pub(crate)` so a test can append its own trailing responses after an
    /// `Ending::FailsAt` shape — e.g. the rollback calls a provisioning
    /// failure now triggers, which this script has no vocabulary to model
    /// itself. See `dispatch_agent_propagates_tmux_new_window_failure` for
    /// the pattern.
    pub(crate) fn responses(&self) -> Vec<(Option<Duration>, Result<Output>)> {
        let steps = self.steps();
        if let Some(finish) = self.finish.as_ref() {
            return steps
                .into_iter()
                .map(|step| finish.response(step))
                .collect();
        }
        let failing_last = matches!(self.ending, Ending::FailsAt(_));
        let mut fetch_attempt = 0;
        let mut out = Vec::with_capacity(steps.len());
        for (i, step) in steps.iter().copied().enumerate() {
            // The terminal step of a `fails_at` shape reports failure; every
            // other step answers as its own kind dictates.
            if failing_last && i + 1 == steps.len() {
                out.push((None, MockProcessRunner::fail(failure_stderr(step))));
                continue;
            }
            let (delay, response) = match step {
                Step::Fetch => {
                    fetch_attempt += 1;
                    self.fetch_response(fetch_attempt)
                }
                // The probe answers "yes, there is an origin"; it is `ls-remote`
                // that then distinguishes the two failure classes.
                Step::OriginProbe => (None, MockProcessRunner::ok()),
                Step::LsRemote => (None, self.ls_remote_response()),
                // The branch exists locally, so the fallback stands. A script
                // wanting the refusal drives `fails_at(Step::LocalBaseProbe)`.
                Step::LocalBaseProbe => (None, MockProcessRunner::ok()),
                Step::AheadBehind => (
                    None,
                    MockProcessRunner::ok_with_stdout(&rev_list_counts(self.local_ahead, 0)),
                ),
                Step::ReusedBranchDrift => {
                    let (behind, own) = match self.reused_branch {
                        ReusedBranch::InSync => (0, 0),
                        ReusedBranch::Behind(n) => (n, 0),
                        ReusedBranch::OwnCommits(n) => (0, n),
                    };
                    (
                        None,
                        MockProcessRunner::ok_with_stdout(&rev_list_counts(behind, own)),
                    )
                }
                // Both lookups answer from the very option that put them in the
                // sequence, so `steps()` has already ruled out the `None` arm and
                // no fallback value has to be invented for it.
                Step::DetectDefaultBranch => match self.default_branch {
                    Some(branch) => (
                        None,
                        MockProcessRunner::ok_with_stdout(&default_branch_ref(branch)),
                    ),
                    None => unreachable!("DetectDefaultBranch is only emitted for Some(branch)"),
                },
                Step::PrHeadLookup => match self.pr_head {
                    Some(head) => (None, pr_head_response(head)),
                    None => unreachable!("PrHeadLookup is only emitted for Some(head)"),
                },
                Step::CompanionSplit => {
                    (None, MockProcessRunner::ok_with_stdout(COMPANION_PANE_ID))
                }
                Step::HasWindowQuery => (
                    None,
                    if self.window_already_alive {
                        // Matches TaskId(42) -> "task-42", the id every resume
                        // test drives (see resume_script_matches_a_real_resume).
                        MockProcessRunner::ok_with_stdout(b"task-42\n")
                    } else {
                        MockProcessRunner::ok()
                    },
                ),
                _ => (None, MockProcessRunner::ok()),
            };
            out.push((delay, response));
        }
        out
    }

    /// How fetch attempt `n` (1-based) answers under this shape's outcome, with
    /// the delay that makes a timeout shape time out.
    fn fetch_response(&self, n: u32) -> (Option<Duration>, Result<Output>) {
        match self.fetch {
            // A success the caller's timeout never waits long enough to see.
            FetchOutcome::TimesOut(delay) => (Some(delay), MockProcessRunner::ok()),
            FetchOutcome::SucceedsOnAttempt(target) if n >= target => {
                (None, MockProcessRunner::ok())
            }
            _ => (None, MockProcessRunner::fail(FETCH_FAILURE)),
        }
    }

    /// `ls-remote --exit-code`'s status is the whole classification: 2 is a
    /// positively-identified missing ref, anything else is "could not reach".
    fn ls_remote_response(&self) -> Result<Output> {
        let code = match self.fetch {
            FetchOutcome::NoOriginRef => LS_REMOTE_NO_MATCHING_REF,
            _ => LS_REMOTE_UNREACHABLE,
        };
        MockProcessRunner::fail_with_code(code, "")
    }

    /// Assert `calls` is exactly the sequence this shape declares.
    ///
    /// The point of the whole module: a queue that has drifted from the code no
    /// longer sits there as a stale comment, it fails here.
    pub(crate) fn assert_matches(&self, calls: &[RecordedCall]) {
        let steps = self.steps();
        for (i, step) in steps.iter().enumerate() {
            let Some((program, args)) = calls.get(i) else {
                panic!(
                    "expected {} calls, got {}; missing {step:?} at index {i}. Recorded: {calls:#?}",
                    steps.len(),
                    calls.len(),
                );
            };
            assert!(
                step.matches(program, args),
                "call {i} should be {step:?}, got {program} {args:?}. \
                 Full sequence expected: {steps:?}",
            );
            // A finish splits its calls across two checkouts, and argv alone
            // cannot tell its two `git status --porcelain` reads apart. Checking
            // the `-C` path here is what stops a swapped scope — a mid-rebase
            // read against the repo root, which would name the wrong files in
            // `RebaseConflict` — from passing as the right sequence.
            if let Some(finish) = self.finish.as_ref() {
                finish.assert_scope(i, *step, args);
            }
        }
        // A short `calls` already panicked in the loop above, so anything left is
        // a call the code should not have made.
        assert_eq!(
            calls.len(),
            steps.len(),
            "unexpected extra call(s) past {:?}: {:#?}",
            steps.last(),
            &calls[steps.len()..],
        );
    }
}

/// What `gh pr view` answers for each [`PrHead`]. `Unresolvable` is a *failure*
/// response — the lookup soft-fails and the dispatch continues on the base
/// branch — which is why this is not folded in with the successes.
fn pr_head_response(head: PrHead) -> Result<Output> {
    match head {
        PrHead::Branch(branch) => MockProcessRunner::ok_with_stdout(&pr_view_reply(branch, false)),
        PrHead::Fork(branch) => MockProcessRunner::ok_with_stdout(&pr_view_reply(branch, true)),
        PrHead::Unresolvable => MockProcessRunner::fail("gh: not authenticated"),
    }
}

/// The stderr a deliberately-failing `step` reports. Wording matches what the
/// real tool says, so an error message a test asserts on stays realistic.
fn failure_stderr(step: Step) -> &'static str {
    match step {
        Step::HasWindowQuery => NO_TMUX_SERVER,
        Step::DetectDefaultBranch => "fatal: ref refs/remotes/origin/HEAD is not a symbolic ref",
        Step::PrHeadLookup => "gh: not authenticated",
        Step::Fetch | Step::OriginProbe | Step::LsRemote => FETCH_FAILURE,
        // `--quiet` means a missing ref prints nothing and exits non-zero.
        Step::LocalBaseProbe => "",
        Step::AheadBehind => "fatal: ambiguous argument: unknown revision",
        Step::WorktreeAdd => "fatal: not a git repository",
        // The prune is best-effort: provisioning ignores its result and
        // carries on to the add, so no scripted shape ever ENDS here. A test
        // that wants a failing prune queues one by hand, because `FailsAt`
        // also truncates the sequence — which would be the wrong shape.
        Step::WorktreePrune | Step::ReusedBranchDrift | Step::ReusedBranchFastForward => {
            unreachable!("{step:?} is best-effort and never ends a sequence")
        }
        Step::NewWindowNameCheck | Step::NewWindow => NO_TMUX_SERVER,
        Step::SetDispatchDir => "can't find window",
        Step::SetSplitHook => "unknown hook",
        Step::SendKeysLiteral | Step::SendKeysEnter => "can't find pane",
        Step::CompanionSplit => "no target pane",
        Step::CompanionRoleMark => "unknown option",

        // The finish steps with no non-zero-exit mode of their own: the HEAD
        // read and the porcelain reads only ever succeed or fail to spawn, and
        // the abort's result `finish_task` discards outright. Left unreachable
        // rather than given invented prose that would read as configuration.
        Step::CurrentBranch | Step::DirtyCheck | Step::ConflictStatus | Step::RebaseAbort => {
            unreachable!("{step:?} has no failing-exit shape")
        }
        Step::Pull => "fatal: unable to access remote",
        // Deliberately free of every marker `is_rebase_conflict` reads —
        // "CONFLICT", "could not apply", "Merge conflict" — since this is the
        // stderr that must classify as a *non*-conflict failure.
        Step::Rebase => "fatal: refusing to rebase onto unrelated histories",
        Step::FastForward => "fatal: Not possible to fast-forward, aborting.",
    }
}

#[cfg(test)]
mod tests;
