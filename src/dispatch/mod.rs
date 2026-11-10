use anyhow::Context;

use crate::models::extract_github_repo;
use crate::process::{stderr_str, stdout_str, ProcessRunner, SUBPROCESS_TIMEOUT};

/// The tmux window environment every agent launch sets — task or resumed
/// alike, including `spacetime/module`'s separate wasm32 workspace, which picks
/// up a plain environment variable with no config of its own.
///
/// - `DISPATCH_TASK_ID`, the task this window was launched for. The hook
///   scripts act on it rather than on the cwd's branch, so an agent that
///   checks out another task's branch cannot move that task (`HookDelivery`
///   in docs/specs/agent-health.allium).
/// - `RUSTC_WRAPPER=sccache`, so every `cargo build`/`cargo test` goes through
///   sccache's shared compiler-object cache instead of paying full
///   per-worktree compile cost (`DispatchedAgentsShareASccacheNotATargetDir`
///   in docs/specs/dispatch.allium). Left out when `sccache` is not on this
///   machine's `PATH`: it is a build-time speedup, not a correctness
///   requirement, so its absence degrades the launch silently rather than
///   blocking it. When it is set, the sccache server is started here first,
///   from the board's own process, so it does not land inside the first
///   agent's memory-capped scope (`SccacheServerStartsOutsideTheCap`).
/// - `CARGO_BUILD_JOBS`: the board's own value when it has one; otherwise
///   the default, unless a cargo config in reach of `worktree_path` sets
///   `build.jobs`, which the variable would override
///   (`AgentBuildsRunFewerParallelJobs`).
fn agent_launch_env(
    runner: &dyn ProcessRunner,
    task_id: crate::models::TaskId,
    worktree_path: &str,
) -> Vec<(&'static str, String)> {
    let mut env = vec![("DISPATCH_TASK_ID", task_id.0.to_string())];
    if runner.sccache_available() {
        runner.start_sccache_server();
        env.push(("RUSTC_WRAPPER", "sccache".to_string()));
    }
    match runner.board_cargo_build_jobs() {
        Some(jobs) => env.push(("CARGO_BUILD_JOBS", jobs)),
        None if runner.cargo_config_sets_jobs(worktree_path) => {}
        None => env.push((
            "CARGO_BUILD_JOBS",
            build_jobs::default_build_jobs(runner.available_cores()).to_string(),
        )),
    }
    env
}

/// [`agent_launch_env`] in the borrowed form [`crate::tmux::new_window`] takes.
fn borrowed_env<'a>(env: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    env.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// `command` wrapped in the agents' shared memory-capped scope when this
/// machine offers one, unchanged otherwise (`AgentsShareOneMemoryCap` in
/// docs/specs/dispatch.allium).
fn in_agent_scope(command: String, runner: &dyn ProcessRunner) -> String {
    match runner.agent_scope() {
        Some(scope) => scope.wrap(&command),
        None => command,
    }
}

mod agents;
mod allium_specs;
mod build_jobs;
mod build_output;
mod bump;
mod caller_identity;
mod finish;
pub(crate) mod git_output;
/// Shared `MockProcessRunner` scripts for the dispatch call sequence. Lives here
/// rather than in a test module because the sequence it declares is this
/// module's own, and three other test suites drive it.
#[cfg(test)]
pub(crate) mod mock_sequence;
mod prompts;
mod request_host;
mod split_panes;
mod trust;
mod worktree;

pub use agents::{
    agent_tree_pane_id, companion_pane_ids, current_agent_name, dispatch_agent,
    fetch_verify_command, prepare_inputs, quick_dispatch_agent, research_agent, resume_agent,
    resume_agent_with_message, resync_agent_tree_pane, run_agent_for_mode, toggle_agent_tree_pane,
    DispatchInputs, REVIEW_COMMAND_KEY,
};
pub(crate) use build_jobs::cargo_config_sets_jobs;
pub use build_output::{clear_build_output, BuildOutputCleared, CACHEDIR_TAG_SIGNATURE};
pub use finish::{finish_task, finish_task_with_abort_runner, FinishContext, FinishError};
// Test-only re-export: `prompts` is private, and the routing tests in
// `src/runtime/tests/` and `src/service/tasks/tests/` assert on this marker to
// prove `DispatchMode::Research` reached `build_research_prompt`.
#[cfg(test)]
pub(crate) use prompts::RESEARCH_AGENT_INTRO;
pub use prompts::{
    build_and_record_injections, effective_review_command, EpicContext, LearningInjections,
};
pub use request_host::{
    DeadlineRunner, DispatchRequestObserver, SessionRequestObserver, SessionWork, DEADLINE_PASSED,
    EXIT_SESSION_TEARDOWN_TIMEOUT, WRAP_UP_REBASE_TIMEOUT,
};
pub use split_panes::{
    join_task_window_into_pane, kill_task_window, restore_pinned_pane, swap_task_window_into_pane,
    SwapFailure,
};
pub(crate) use trust::{is_trusted_at, trust_at};
pub(crate) use worktree::rollback_failed_provisioning;
pub use worktree::{branch_from_worktree, teardown_task, TeardownFailure};

// ---------------------------------------------------------------------------
// PR types
// ---------------------------------------------------------------------------
//
// The agent creates its own PR through the /wrap-up skill — see
// plugin/skills/wrap-up/SKILL.md and the WrapUpPr rule in
// docs/specs/pr-workflow.allium. Dispatch only reads PR status here.

/// What `gh pr view` printed, untouched: the state word and the review
/// decision (empty for a repo without branch protection). The board does not
/// interpret either; the module parses them and writes the task
/// (`pr-workflow.allium: ReportPrStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPrStatus {
    pub state: String,
    pub review_decision: String,
}

// ---------------------------------------------------------------------------
// PR functions
// ---------------------------------------------------------------------------

/// Why one `gh pr view` attempt failed, split by the only distinction the
/// caller acts on: whether repeating the same call could ever answer
/// differently.
///
/// Retrying a permanent failure on a timer is what turned five unreadable PRs
/// into 63,000 identical log lines, so the split is load-bearing rather than
/// cosmetic. See `PrCheckOutcome` in `docs/specs/core.allium` and
/// `PollPrStatus` in `docs/specs/pr-workflow.allium`.
#[derive(Debug)]
pub enum PrCheckFailure {
    /// Cannot succeed until something outside dispatch changes: the repository
    /// does not resolve, credentials are bad or missing, there is no PR behind
    /// the url, or gh reported a state dispatch cannot map.
    Permanent(anyhow::Error),
    /// May succeed on a later attempt: connection failure, timeout, 5xx, a
    /// spent rate limit, a reset stream.
    Transient(anyhow::Error),
}

impl PrCheckFailure {
    pub fn is_permanent(&self) -> bool {
        matches!(self, PrCheckFailure::Permanent(_))
    }

    fn inner(&self) -> &anyhow::Error {
        match self {
            PrCheckFailure::Permanent(e) | PrCheckFailure::Transient(e) => e,
        }
    }
}

impl std::fmt::Display for PrCheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.inner())
    }
}

/// Substrings of `gh`'s stderr that mean retrying cannot help.
///
/// String matching is unsatisfying, but gh exposes no machine-readable failure
/// kind: the exit status is 1 for a spent rate limit and for a repository that
/// does not exist alike. Every entry is a verbatim fragment observed in a real
/// app.log, kept short enough to survive gh rewording the sentence around it.
const PERMANENT_GH_FAILURE_MARKERS: &[&str] = &[
    // The gh account cannot see the repository — renamed, deleted, or (much
    // more often) never granted access.
    "Could not resolve to a Repository",
    "Bad credentials",
    "Requires authentication",
    // The url is not a PR at all. Real cases in the log include issue urls, a
    // dependabot alert url, and a static-reports link.
    "no pull requests found for branch",
];

/// Whether any `marker` occurs in `text`. The shared primitive behind every
/// gh-stderr classifier in this module — `gh` exposes no machine-readable
/// failure kind, so each classifier keeps its own marker list (the meaning of
/// "expected" differs per call), but the matching itself is one function.
pub(crate) fn matches_any_marker(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

/// Classify a `gh` failure, defaulting to [`PrCheckFailure::Transient`].
///
/// The default direction matters: a transient failure misread as permanent
/// strands a task under `pr_unreachable`, while a permanent one misread as
/// transient costs only a slowing trickle of doomed calls. So anything
/// uncatalogued retries.
fn classify_gh_failure(stderr: &str, error: anyhow::Error) -> PrCheckFailure {
    if matches_any_marker(stderr, PERMANENT_GH_FAILURE_MARKERS) {
        PrCheckFailure::Permanent(error)
    } else {
        PrCheckFailure::Transient(error)
    }
}

/// Read the current status of a PR with `gh pr view`, raw.
pub fn check_pr_status(
    pr_url: &str,
    runner: &dyn ProcessRunner,
) -> std::result::Result<RawPrStatus, PrCheckFailure> {
    let output = runner
        .run(
            "gh",
            &[
                "pr",
                "view",
                pr_url,
                "--json",
                "state,reviewDecision",
                "-q",
                r#"[.state, .reviewDecision] | join("\n")"#,
            ],
        )
        .context("Failed to run gh pr view")
        // A spawn failure is about this machine, not about this PR, so it must
        // not count towards giving up on the task.
        .map_err(PrCheckFailure::Transient)?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        let error = anyhow::anyhow!("gh pr view failed: {stderr}");
        return Err(classify_gh_failure(&stderr, error));
    }

    let stdout = stdout_str(&output);
    let mut lines = stdout.lines();
    // No output where gh reported success is unmappable, and stays unmappable
    // however often it is retried.
    let state = lines
        .next()
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .ok_or_else(|| PrCheckFailure::Permanent(anyhow::anyhow!("gh pr view: no output")))?
        .to_string();
    // review_decision is optional — repos without branch-protection rules omit it.
    let review_decision = lines.next().unwrap_or("").trim().to_string();

    Ok(RawPrStatus {
        state,
        review_decision,
    })
}

/// Resolve a PR's head (source) branch via `gh pr view`, so a review worktree
/// can be based on the PR's code.
///
/// Returns `None` — so callers fall back to the task's base branch — on any
/// command failure, empty output, or a cross-repository (fork) PR. A fork PR's
/// head branch lives on the fork, not `origin`, so it cannot be fetched by name;
/// basing on it would fail `git worktree add`, hence the fork fall-back.
pub fn pr_head_branch(pr_url: &str, runner: &dyn ProcessRunner) -> Option<String> {
    let output = runner
        .run_with_timeout(
            "gh",
            &[
                "pr",
                "view",
                pr_url,
                "--json",
                "headRefName,isCrossRepository",
                "-q",
                r#"[.headRefName, (.isCrossRepository|tostring)] | join("\n")"#,
            ],
            SUBPROCESS_TIMEOUT,
        )
        .ok()?;
    if !output.status.success() {
        tracing::warn!(
            pr_url,
            "gh pr view headRefName failed; falling back to base branch"
        );
        return None;
    }

    let stdout = stdout_str(&output);
    let mut lines = stdout.lines();
    let head = lines.next().unwrap_or("").trim().to_string();
    let is_fork = lines.next().unwrap_or("").trim() == "true";
    if head.is_empty() || is_fork {
        return None;
    }
    Some(head)
}

/// Resolve the local repo path for each feed item from its URL.
///
/// For each item, attempts `extract_github_repo(url)` → `resolve_repo_path(...)`.
/// Items whose URL cannot be resolved get an empty-string sentinel (`""`).
///
/// `known_paths` must be THIS host's clones (`list_repo_paths_saved_by_this_host`),
/// since the result is stamped on a task this host dispatches from. A clone's
/// `origin` remote is read at most once per path per call, and only for a path
/// whose directory name matches a repo some item names.
pub fn resolve_feed_item_repo_paths(
    items: &[crate::models::FeedItem],
    known_paths: &[String],
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let mut remotes: std::collections::HashMap<String, Option<String>> = Default::default();
    items
        .iter()
        .map(|item| {
            extract_github_repo(&item.url)
                .and_then(|r| {
                    resolve_repo_path_with(r, known_paths, |path| {
                        remotes
                            .entry(path.to_string())
                            .or_insert_with(|| crate::git::origin_github_repo(path, runner))
                            .clone()
                    })
                })
                .unwrap_or_default()
        })
        .collect()
}

/// Resolve a GitHub repo name (e.g. `"org/repo"`) to a local filesystem path
/// by matching against known repo paths.  Returns the first path whose
/// directory name equals the short repo name. Name only: see
/// [`resolve_repo_path_with`] for matching on the owner too.
pub fn resolve_repo_path(github_repo: &str, known_paths: &[String]) -> Option<String> {
    resolve_repo_path_with(github_repo, known_paths, |_| None)
}

/// [`resolve_repo_path`], then narrowed by each candidate's `origin` remote
/// (`remote_of`, as `owner/repo`): a candidate whose remote is positively a
/// different repository is rejected, so `orgA/api` and `orgB/api` do not share
/// a clone; a candidate whose remote is unknown still matches by name; and a
/// candidate whose remote equals `github_repo` (case-insensitively) wins over
/// one that is merely unknown.
fn resolve_repo_path_with(
    github_repo: &str,
    known_paths: &[String],
    mut remote_of: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    let repo_short = github_repo.split('/').next_back().unwrap_or(github_repo);
    let mut unknown: Option<&String> = None;
    for path in known_paths.iter().filter(|p| {
        std::path::Path::new(p)
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|dir| dir == repo_short)
    }) {
        match remote_of(path) {
            Some(remote) if remote.eq_ignore_ascii_case(github_repo) => return Some(path.clone()),
            Some(_) => {}
            None => {
                unknown.get_or_insert(path);
            }
        }
    }
    unknown.cloned()
}

/// Close a task's attached PR via `gh pr close`.
///
/// Best-effort: the caller (`TaskService::spawn_close_attached_pr`,
/// `ClosePrOnDone` in `docs/specs/pr-workflow.allium`) logs the outcome
/// rather than treating it as a service failure. `run_with_timeout` bounds
/// it the same way `pr_head_branch` bounds its own `gh pr view` call — this
/// runs after the task's Done write has already persisted, so nothing may
/// wedge on it.
pub fn close_pr(pr_url: &str, runner: &dyn ProcessRunner) -> anyhow::Result<()> {
    let output = runner
        .run_with_timeout("gh", &["pr", "close", pr_url], SUBPROCESS_TIMEOUT)
        .context("Failed to run gh pr close")?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        anyhow::bail!("gh pr close failed: {stderr}");
    }
    Ok(())
}

#[cfg(test)]
mod prompts_snapshots;
/// `pub(crate)` only so the shared fixture builders in here
/// (`make_test_repo_with_worktree`) can be reached from other modules' tests
/// instead of being copied into them.
#[cfg(test)]
pub(crate) mod tests;
