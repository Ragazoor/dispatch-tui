use anyhow::{Context, Result};

use crate::models::{expand_tilde, ReviewDecision};
use crate::process::ProcessRunner;

mod agents;
mod finish;
mod prompts;
mod worktree;

pub use agents::{
    brainstorm_agent, build_fix_prompt, dispatch_agent, dispatch_fix_agent, dispatch_review_agent,
    epic_planning_agent, is_wrappable, plan_agent, quick_dispatch_agent, resume_agent,
};
pub use finish::{finish_task, FinishError};
pub use prompts::EpicContext;
pub use worktree::{branch_from_worktree, cleanup_task, validate_repo_path};

/// Extract stderr from a process `Output` as a trimmed `String`.
pub(super) fn stderr_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

/// Extract stdout from a process `Output` as a trimmed `String`.
pub(super) fn stdout_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// ---------------------------------------------------------------------------
// PR types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PrResult {
    pub pr_url: String,
}

#[derive(Debug)]
pub enum PrError {
    PushFailed(String),
    CreateFailed(String),
    Other(String),
}

impl std::fmt::Display for PrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrError::PushFailed(msg) => write!(f, "Push failed: {msg}"),
            PrError::CreateFailed(msg) => write!(f, "PR creation failed: {msg}"),
            PrError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug)]
pub struct PrStatus {
    pub state: PrState,
    pub review_decision: Option<ReviewDecision>,
}

// ---------------------------------------------------------------------------
// PR functions
// ---------------------------------------------------------------------------

/// Extract "owner/repo" from a git remote URL.
/// Handles both SSH (git@github.com:owner/repo.git) and HTTPS (https://github.com/owner/repo.git).
fn parse_repo_slug(remote_url: &str) -> Option<String> {
    let url = remote_url.trim().trim_end_matches(".git");
    // SSH: git@github.com:owner/repo
    if let Some(path) = url.strip_prefix("git@github.com:") {
        return Some(path.to_string());
    }
    // HTTPS: https://github.com/owner/repo
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return Some(rest.to_string());
    }
    None
}

/// Push the branch to origin and create a GitHub PR using `gh`.
///
/// `push_dir` is the directory used for `git push`. Pass the worktree path so
/// the pre-push hook runs in the worktree's working directory, where dirty files
/// from other worktrees or the main repo are invisible.
pub fn create_pr(
    push_dir: &str,
    branch: &str,
    title: &str,
    description: &str,
    base_branch: &str,
    runner: &dyn ProcessRunner,
) -> std::result::Result<PrResult, PrError> {
    let push_dir = &expand_tilde(push_dir);

    // 1. Push the branch
    let output = runner
        .run("git", &["-C", push_dir, "push", "-u", "origin", branch])
        .map_err(|e| PrError::PushFailed(format!("Failed to run git push: {e}")))?;
    if !output.status.success() {
        return Err(PrError::PushFailed(stderr_str(&output)));
    }

    // 2. Get the repo slug from git remote
    let remote_output = runner
        .run("git", &["-C", push_dir, "remote", "get-url", "origin"])
        .map_err(|e| PrError::Other(format!("Failed to get remote URL: {e}")))?;
    let remote_url = stdout_str(&remote_output);
    let repo_slug = parse_repo_slug(&remote_url)
        .ok_or_else(|| PrError::Other(format!("Could not parse repo slug from: {remote_url}")))?;

    // 3. Create the PR.
    // Use owner:branch format for --head so gh resolves the branch in the same repo as --repo.
    // Without the owner prefix, gh defaults to the authenticated user's namespace, causing
    // "Head sha can't be blank" errors when the user isn't the repo owner.
    let owner = repo_slug
        .split('/')
        .next()
        .ok_or_else(|| PrError::Other(format!("Invalid repo slug: {repo_slug}")))?;
    let head_ref = format!("{owner}:{branch}");
    let output = runner
        .run(
            "gh",
            &[
                "pr",
                "create",
                "--draft",
                "--title",
                title,
                "--body",
                description,
                "--head",
                &head_ref,
                "--base",
                base_branch,
                "--repo",
                &repo_slug,
            ],
        )
        .map_err(|e| PrError::Other(format!("Failed to run gh: {e}")))?;
    if !output.status.success() {
        let stderr = stderr_str(&output);
        // gh emits "…already exists:\nhttps://github.com/…/pull/N" — treat as success
        // so that wrap_up pr is idempotent (calling it again returns the existing URL).
        if stderr.contains("already exists") {
            if let Some(url) = stderr.lines().find(|l| l.starts_with("https://")) {
                return Ok(PrResult {
                    pr_url: url.to_string(),
                });
            }
        }
        return Err(PrError::CreateFailed(stderr));
    }

    // 4. Parse the PR URL from stdout
    let pr_url = stdout_str(&output);
    Ok(PrResult { pr_url })
}

/// Check the current status of a PR using `gh pr view`.
pub fn check_pr_status(pr_url: &str, runner: &dyn ProcessRunner) -> Result<PrStatus> {
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
        .context("Failed to run gh pr view")?;
    if !output.status.success() {
        anyhow::bail!("gh pr view failed: {}", stderr_str(&output));
    }

    let stdout = stdout_str(&output);
    let mut lines = stdout.lines();
    let state_str = lines.next().unwrap_or("").to_uppercase();
    let review_str = lines.next().unwrap_or("").to_uppercase();

    let state = match state_str.as_str() {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    };

    let review_decision = ReviewDecision::parse(&review_str);

    Ok(PrStatus {
        state,
        review_decision,
    })
}

/// Merge a GitHub PR using `gh pr merge --squash`.
pub fn merge_pr(pr_url: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner
        .run("gh", &["pr", "merge", "--squash", pr_url])
        .context("Failed to run gh pr merge")?;
    if !output.status.success() {
        anyhow::bail!("{}", stderr_str(&output));
    }
    Ok(())
}

/// Extract `"org/repo"` from a GitHub URL.
///
/// Handles `https://github.com/org/repo`, `.../pull/N`, `.../issues/N`,
/// `.../tree/...`, and similar paths — any URL whose host is `github.com`.
/// Returns `None` for non-GitHub URLs, empty strings, and single-segment paths.
pub fn extract_github_repo(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://github.com/")?;
    let rest = rest.trim_end_matches('/');
    // Need at least two path segments: "org/repo[/...]"
    let slash = rest.find('/')?;
    let after_org = &rest[slash + 1..];
    if after_org.is_empty() {
        return None;
    }
    let end = after_org.find('/').unwrap_or(after_org.len());
    let repo = &after_org[..end];
    if repo.is_empty() {
        return None;
    }
    Some(&rest[..slash + 1 + end])
}

/// Resolve the local repo path for each feed item from its URL.
///
/// For each item, attempts `extract_github_repo(url)` → `resolve_repo_path(...)`.
/// Items whose URL cannot be resolved get an empty-string sentinel (`""`).
pub fn resolve_feed_item_repo_paths(
    items: &[crate::models::FeedItem],
    known_paths: &[String],
) -> Vec<String> {
    items
        .iter()
        .map(|item| {
            extract_github_repo(&item.url)
                .and_then(|r| resolve_repo_path(r, known_paths))
                .unwrap_or_default()
        })
        .collect()
}

/// Resolve a GitHub repo name (e.g. `"org/repo"`) to a local filesystem path
/// by matching against known repo paths.  Returns the first path whose
/// directory name equals the short repo name.
pub fn resolve_repo_path(github_repo: &str, known_paths: &[String]) -> Option<String> {
    let repo_short = github_repo.split('/').next_back().unwrap_or(github_repo);
    known_paths
        .iter()
        .find(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|dir| dir == repo_short)
        })
        .cloned()
}

#[cfg(test)]
mod tests;
