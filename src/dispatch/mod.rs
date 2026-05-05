use anyhow::{Context, Result};

use crate::models::ReviewDecision;
use crate::process::ProcessRunner;

mod agents;
mod finish;
mod prompts;
mod worktree;

pub use agents::{
    build_fix_prompt, dispatch_agent, dispatch_fix_agent, dispatch_review_agent,
    epic_planning_agent, fix_task_agent, is_wrappable, pr_review_agent, quick_dispatch_agent,
    research_agent, resume_agent,
};
pub use finish::{finish_task, FinishError};
pub use prompts::{EpicContext, ProjectContext};
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
//
// PR creation moved to the agent /wrap-up skill — see
// plugin/skills/wrap-up/SKILL.md and the WrapUpPr rule in
// docs/specs/tasks.allium. The dispatch-side `create_pr` helper has
// been removed; only status-check and merge helpers remain here.

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
