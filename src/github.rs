use crate::models::{CiStatus, ReviewDecision, ReviewPr, Reviewer};
use crate::process::ProcessRunner;
use chrono::{DateTime, Utc};
use std::cmp::Reverse;

/// Determine the effective review decision for a PR node.
///
/// Uses the overall `reviewDecision` for APPROVED and CHANGES_REQUESTED.
/// For REVIEW_REQUIRED, checks whether the viewer has left comments (plain PR
/// comments or COMMENTED-state reviews) and whether the PR author has responded
/// since (via a new comment or a new commit).
fn classify_review_decision(node: &serde_json::Value, viewer_login: &str) -> ReviewDecision {
    let decision_str = node["reviewDecision"].as_str().unwrap_or("REVIEW_REQUIRED");
    match decision_str {
        "APPROVED" => return ReviewDecision::Approved,
        "CHANGES_REQUESTED" => return ReviewDecision::ChangesRequested,
        _ => {}
    }

    // Re-request is the strongest signal: if the viewer is in the current
    // reviewRequests list, the author explicitly asked for another look.
    let viewer_re_requested = node["reviewRequests"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|req| req["requestedReviewer"]["login"].as_str() == Some(viewer_login));

    if viewer_re_requested {
        return ReviewDecision::ReviewRequired;
    }

    let pr_author = node["author"]["login"].as_str().unwrap_or("");

    // Viewer's last plain comment
    let viewer_last_comment = node["comments"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["author"]["login"].as_str() == Some(viewer_login))
        .filter_map(|c| c["createdAt"].as_str()?.parse::<DateTime<Utc>>().ok())
        .max();

    // Viewer's last review (any state: APPROVED, CHANGES_REQUESTED, COMMENTED)
    let viewer_last_review = node["reviews"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["author"]["login"].as_str() == Some(viewer_login))
        .filter_map(|r| r["submittedAt"].as_str()?.parse::<DateTime<Utc>>().ok())
        .max();

    let viewer_last_interaction = viewer_last_comment.max(viewer_last_review);

    let Some(interaction_at) = viewer_last_interaction else {
        return ReviewDecision::ReviewRequired;
    };

    // Check if author has responded since the viewer's last interaction
    let author_last_comment = node["comments"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["author"]["login"].as_str() == Some(pr_author))
        .filter_map(|c| c["createdAt"].as_str()?.parse::<DateTime<Utc>>().ok())
        .max();

    let last_commit_date = node["commits"]["nodes"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|n| n["commit"]["committedDate"].as_str())
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());

    let author_responded = author_last_comment.is_some_and(|t| t > interaction_at)
        || last_commit_date.is_some_and(|t| t > interaction_at);

    if author_responded {
        ReviewDecision::ReviewRequired
    } else {
        ReviewDecision::WaitingForResponse
    }
}

/// Extract reviewers from a PR node by merging completed reviews and pending
/// review requests. A reviewer who has left an APPROVED or CHANGES_REQUESTED
/// review gets that decision; a pending request (not yet reviewed) gets `None`.
fn parse_reviewers(node: &serde_json::Value) -> Vec<Reviewer> {
    let mut by_login: std::collections::HashMap<String, Option<ReviewDecision>> =
        std::collections::HashMap::new();

    // Completed reviews — latest state per reviewer
    if let Some(reviews) = node["reviews"]["nodes"].as_array() {
        for review in reviews {
            if let Some(login) = review["author"]["login"].as_str() {
                let decision = match review["state"].as_str() {
                    Some("APPROVED") => Some(ReviewDecision::Approved),
                    Some("CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
                    _ => continue,
                };
                by_login.insert(login.to_string(), decision);
            }
        }
    }

    // Pending review requests — only add if not already reviewed
    if let Some(requests) = node["reviewRequests"]["nodes"].as_array() {
        for req in requests {
            if let Some(login) = req["requestedReviewer"]["login"].as_str() {
                by_login.entry(login.to_string()).or_insert(None);
            }
        }
    }

    by_login
        .into_iter()
        .map(|(login, decision)| Reviewer { login, decision })
        .collect()
}

/// The PR fields fragment used in both search aliases.
const PR_FIELDS: &str = r#"... on PullRequest {
        number
        title
        url
        isDraft
        createdAt
        updatedAt
        additions
        deletions
        reviewDecision
        body
        headRefName
        author { login }
        repository { nameWithOwner }
        labels(first: 10) { nodes { name } }
        comments(last: 50) { nodes { author { login } createdAt } }
        reviews(last: 20) { nodes { state author { login } submittedAt } }
        reviewRequests(first: 10) { nodes { requestedReviewer { ... on User { login } } } }
        commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
      }"#;

/// Build a GraphQL query body from a slice of GitHub search strings.
///
/// Each search string becomes an aliased search node (`q0`, `q1`, …) requesting
/// the standard `PR_FIELDS` fragment. The `viewer { login }` root is always
/// included for review-decision classification.
fn build_search_graphql(queries: &[String]) -> String {
    let mut body = String::from("{\n  viewer { login }\n");
    for (i, search) in queries.iter().enumerate() {
        use std::fmt::Write;
        write!(
            body,
            "  q{i}: search(query: \"{search}\", type: ISSUE, first: 100) {{\n    nodes {{\n      {PR_FIELDS}\n    }}\n  }}\n",
        )
        .unwrap();
    }
    body.push('}');
    body
}

/// Execute a set of GitHub search queries via `gh api graphql` and return merged PRs.
///
/// Builds a single GraphQL request with one aliased search per query string,
/// runs it via the ProcessRunner, and parses/deduplicates the results.
pub fn fetch_prs(runner: &dyn ProcessRunner, queries: &[String]) -> Result<Vec<ReviewPr>, String> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }

    let query = build_search_graphql(queries);

    let output = runner
        .run("gh", &["api", "graphql", "-f", &format!("query={query}")])
        .map_err(|e| format!("Failed to run gh: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api graphql failed: {stderr}"));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_prs_response(&json, queries.len())
}

/// Parse a GraphQL response with `q0..qN` aliased search results into ReviewPrs.
///
/// Extracts nodes from each `data.qN.nodes` array, deduplicates by URL, filters
/// drafts, and sorts by `updated_at` descending.
fn parse_prs_response(json: &str, alias_count: usize) -> Result<Vec<ReviewPr>, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let viewer_login = root
        .pointer("/data/viewer/login")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut seen_urls = std::collections::HashSet::new();
    let mut all_nodes: Vec<serde_json::Value> = Vec::new();
    for i in 0..alias_count {
        let path = format!("/data/q{i}/nodes");
        if let Some(nodes) = root.pointer(&path).and_then(|v| v.as_array()) {
            for node in nodes {
                let url = node["url"].as_str().unwrap_or("").to_string();
                if !url.is_empty() && seen_urls.insert(url) {
                    all_nodes.push(node.clone());
                }
            }
        }
    }

    let mut prs = Vec::with_capacity(all_nodes.len());
    for node in &all_nodes {
        if node["isDraft"].as_bool() == Some(true) {
            continue;
        }

        let review_decision = classify_review_decision(node, viewer_login);

        let labels = node["labels"]["nodes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let created_at = node["createdAt"]
            .as_str()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or_else(Utc::now);
        let updated_at = node["updatedAt"]
            .as_str()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or_else(Utc::now);

        let body = node["body"].as_str().unwrap_or("").to_string();
        let head_ref = node["headRefName"].as_str().unwrap_or("").to_string();

        let ci_state = node["commits"]["nodes"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|n| n["commit"]["statusCheckRollup"]["state"].as_str());
        let ci_status = CiStatus::from_github(ci_state);

        let reviewers = parse_reviewers(node);

        prs.push(ReviewPr {
            number: node["number"].as_i64().unwrap_or(0),
            title: node["title"].as_str().unwrap_or("").to_string(),
            author: node["author"]["login"].as_str().unwrap_or("").to_string(),
            repo: node["repository"]["nameWithOwner"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            url: node["url"].as_str().unwrap_or("").to_string(),
            is_draft: node["isDraft"].as_bool().unwrap_or(false),
            created_at,
            updated_at,
            additions: node["additions"].as_i64().unwrap_or(0),
            deletions: node["deletions"].as_i64().unwrap_or(0),
            review_decision,
            labels,
            body,
            head_ref,
            ci_status,
            reviewers,
        });
    }

    prs.sort_by_key(|pr| Reverse(pr.updated_at));
    Ok(prs)
}

// ---------------------------------------------------------------------------
// Security alerts
// ---------------------------------------------------------------------------

use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

/// Map a single vulnerability alert node to a `SecurityAlert`.
fn parse_repo_alerts(repo: &str, alert_nodes: &[serde_json::Value]) -> Vec<SecurityAlert> {
    alert_nodes
        .iter()
        .map(|node| {
            let number = node["number"].as_i64().unwrap_or(0);
            let severity_str = node
                .pointer("/securityVulnerability/severity")
                .and_then(|v| v.as_str())
                .unwrap_or("MODERATE");
            let severity = AlertSeverity::parse(severity_str).unwrap_or(AlertSeverity::Medium);
            let title = node
                .pointer("/securityAdvisory/summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let package = node
                .pointer("/securityVulnerability/package/name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let vulnerable_range = node
                .pointer("/securityVulnerability/vulnerableVersionRange")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let fixed_version = node
                .pointer("/securityVulnerability/firstPatchedVersion/identifier")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let cvss_score = node
                .pointer("/securityAdvisory/cvss/score")
                .and_then(|v| v.as_f64());
            let url = format!("https://github.com/{repo}/security/dependabot/{number}");
            let created_at = node["createdAt"]
                .as_str()
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .unwrap_or_else(Utc::now);
            let description = node
                .pointer("/securityAdvisory/description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            SecurityAlert {
                number,
                repo: repo.to_string(),
                severity,
                kind: AlertKind::Dependabot,
                title,
                package,
                vulnerable_range,
                fixed_version,
                cvss_score,
                url,
                created_at,
                state: "open".to_string(),
                description,
            }
        })
        .collect()
}

/// Parse the GraphQL vulnerability alerts response (viewer.repositories) into `SecurityAlert`s.
#[cfg(test)]
fn parse_graphql_security_alerts(json: &str) -> Result<Vec<SecurityAlert>, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let repos = root
        .pointer("/data/viewer/repositories/nodes")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .clone();

    let mut alerts = Vec::new();
    for repo_node in &repos {
        let repo = repo_node["nameWithOwner"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let alert_nodes = match repo_node
            .pointer("/vulnerabilityAlerts/nodes")
            .and_then(|v| v.as_array())
        {
            Some(nodes) => nodes,
            None => continue,
        };
        alerts.extend(parse_repo_alerts(&repo, alert_nodes));
    }
    Ok(alerts)
}

/// Parse a per-repo aliased GraphQL response (`r0`, `r1`, …) into `SecurityAlert`s.
///
/// `count` is the length of the original `repos` slice. Aliases are named after
/// their original index in that slice, so some `rN` keys may be absent when a
/// slug at position N was invalid and skipped. Absent keys are ignored via `continue`.
fn parse_graphql_security_alerts_per_repo(
    json: &str,
    count: usize,
) -> Result<Vec<SecurityAlert>, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let mut alerts = Vec::new();
    for i in 0..count {
        let key = format!("r{i}");
        let repo_node = match root.pointer(&format!("/data/{key}")) {
            Some(n) => n,
            None => continue,
        };
        let repo = repo_node["nameWithOwner"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let nodes = repo_node
            .pointer("/vulnerabilityAlerts/nodes")
            .and_then(|v| v.as_array())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        alerts.extend(parse_repo_alerts(&repo, nodes));
    }
    Ok(alerts)
}

/// Fetch security alerts for the given repos using per-repo GraphQL aliases.
///
/// Each entry in `repos` must be an "owner/repo" slug. Returns an empty vec
/// immediately if `repos` is empty (caller is responsible for showing the
/// unconfigured prompt). Invalid slugs (no '/') are silently skipped.
/// Results are sorted by severity (critical first), then CVSS score descending.
pub fn fetch_security_alerts(
    runner: &dyn ProcessRunner,
    repos: &[String],
) -> Result<Vec<SecurityAlert>, String> {
    if repos.is_empty() {
        return Ok(vec![]);
    }

    let repo_fields: Vec<String> = repos
        .iter()
        .enumerate()
        .filter_map(|(i, slug)| {
            let (owner, name) = slug.split_once('/')?;
            Some(format!(
                r#"r{i}: repository(owner: "{owner}", name: "{name}") {{
    nameWithOwner
    vulnerabilityAlerts(first: 25, states: OPEN) {{
      nodes {{
        number
        createdAt
        securityVulnerability {{
          severity
          package {{ name }}
          vulnerableVersionRange
          firstPatchedVersion {{ identifier }}
        }}
        securityAdvisory {{
          summary
          description
          cvss {{ score }}
        }}
      }}
    }}
  }}"#
            ))
        })
        .collect();

    if repo_fields.is_empty() {
        return Ok(vec![]);
    }

    let query = format!("{{\n  {}\n}}", repo_fields.join("\n  "));

    let output = runner
        .run("gh", &["api", "graphql", "-f", &format!("query={query}")])
        .map_err(|e| format!("Failed to run gh: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api graphql failed: {stderr}"));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    let mut alerts = parse_graphql_security_alerts_per_repo(&json, repos.len())?;

    alerts.sort_by(|a, b| {
        a.severity
            .column_index()
            .cmp(&b.severity.column_index())
            .then_with(|| {
                b.cvss_score
                    .unwrap_or(0.0)
                    .partial_cmp(&a.cvss_score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    Ok(alerts)
}

// ---------------------------------------------------------------------------
// Dependabot structured config
// ---------------------------------------------------------------------------

pub const DEFAULT_DEPENDABOT_BASE_QUERY: &str = "is:pr is:open author:app/dependabot -is:draft";

/// Parsed representation of the `dependabot_config` settings blob.
#[derive(Debug, Clone, PartialEq)]
pub struct DependabotConfig {
    /// Base GitHub search string applied to every repo query.
    pub base_query: String,
    /// Ordered list of `owner/repo` slugs to query.
    pub repos: Vec<String>,
}

impl Default for DependabotConfig {
    fn default() -> Self {
        Self {
            base_query: DEFAULT_DEPENDABOT_BASE_QUERY.to_string(),
            repos: Vec::new(),
        }
    }
}

/// Parse the two-section structured config text into a `DependabotConfig`.
///
/// Format:
/// ```text
/// # Base query
/// is:pr is:open author:app/dependabot -is:draft
///
/// # Repositories
/// owner/repo-a
/// owner/repo-b
/// ```
///
/// Lines whose trimmed form starts with `#` are treated as comments and
/// ignored within each section (except the `# Repositories` section marker
/// which is the split point between sections).
///
/// If the base-query section is empty, `DEFAULT_DEPENDABOT_BASE_QUERY` is used.
pub fn parse_dependabot_config(input: &str) -> DependabotConfig {
    let mut base_lines: Vec<&str> = Vec::new();
    let mut repo_lines: Vec<String> = Vec::new();
    let mut in_repos_section = false;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("# repositories") {
            in_repos_section = true;
            continue;
        }
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if in_repos_section {
            repo_lines.push(trimmed.to_string());
        } else {
            base_lines.push(trimmed);
        }
    }

    let base_query = if base_lines.is_empty() {
        DEFAULT_DEPENDABOT_BASE_QUERY.to_string()
    } else {
        base_lines.join(" ")
    };

    DependabotConfig {
        base_query,
        repos: repo_lines,
    }
}

/// Render a `DependabotConfig` back to its structured text representation.
pub fn format_dependabot_config(config: &DependabotConfig) -> String {
    let repos = config.repos.join("\n");
    format!(
        "# Base query\n{}\n\n# Repositories\n{}\n",
        config.base_query, repos
    )
}

/// Assemble GitHub search query strings from a `DependabotConfig`.
///
/// Returns `(queries, warnings)`. Slugs that do not contain `/` are skipped
/// and a warning string is added for each.
pub fn assemble_dependabot_queries(config: &DependabotConfig) -> (Vec<String>, Vec<String>) {
    let mut queries = Vec::new();
    let mut warnings = Vec::new();
    for repo in &config.repos {
        if repo.contains('/') {
            queries.push(format!("{} repo:{repo}", config.base_query));
        } else {
            warnings.push(format!("Invalid repo slug (missing '/'): '{repo}'"));
        }
    }
    (queries, warnings)
}

/// Migrate the old `github_queries_bot` setting to `dependabot_config`.
///
/// Extracts `repo:owner/name` tokens from the old query lines and constructs
/// a new structured config with the default base query. Deletes the old key
/// on success. No-ops if `dependabot_config` is already set or `github_queries_bot`
/// is absent.
pub fn migrate_bot_queries_to_dependabot_config(
    old_value: Option<&str>,
) -> Option<DependabotConfig> {
    let old = old_value?;
    let mut repos: Vec<String> = Vec::new();
    for line in old.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        for token in trimmed.split_whitespace() {
            if let Some(slug) = token.strip_prefix("repo:") {
                if slug.contains('/') && !repos.contains(&slug.to_string()) {
                    repos.push(slug.to_string());
                }
            }
        }
    }
    Some(DependabotConfig {
        base_query: DEFAULT_DEPENDABOT_BASE_QUERY.to_string(),
        repos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::MockProcessRunner;

    // PR #42 is in q0 (pending review), PR #99 is a draft (filtered),
    // PR #50 is in q1 (already reviewed, approved),
    // PR #60 is in q2 (only left comments, no formal review).
    const SAMPLE_RESPONSE: &str = r#"{
        "data": {
            "viewer": {"login": "me"},
            "q0": {
                "nodes": [
                    {
                        "number": 42,
                        "title": "Fix login flow",
                        "url": "https://github.com/acme/app/pull/42",
                        "isDraft": false,
                        "createdAt": "2026-03-28T10:00:00Z",
                        "updatedAt": "2026-03-29T14:00:00Z",
                        "additions": 15,
                        "deletions": 3,
                        "reviewDecision": "REVIEW_REQUIRED",
                        "author": {"login": "alice"},
                        "repository": {"nameWithOwner": "acme/app"},
                        "labels": {"nodes": [{"name": "bug"}, {"name": "urgent"}]},
                        "comments": {"nodes": []},
                        "reviews": {"nodes": []},
                        "commits": {"nodes": [{"commit": {"committedDate": "2026-03-28T10:00:00Z"}}]}
                    },
                    {
                        "number": 99,
                        "title": "Update sbt to 1.12",
                        "url": "https://github.com/acme/app/pull/99",
                        "isDraft": true,
                        "createdAt": "2026-03-27T08:00:00Z",
                        "updatedAt": "2026-03-27T08:00:00Z",
                        "additions": 1,
                        "deletions": 1,
                        "reviewDecision": "REVIEW_REQUIRED",
                        "author": {"login": "scala-steward"},
                        "repository": {"nameWithOwner": "acme/app"},
                        "labels": {"nodes": []},
                        "comments": {"nodes": []},
                        "reviews": {"nodes": []},
                        "commits": {"nodes": [{"commit": {"committedDate": "2026-03-27T08:00:00Z"}}]}
                    }
                ]
            },
            "q1": {
                "nodes": [
                    {
                        "number": 50,
                        "title": "Refactor auth module",
                        "url": "https://github.com/acme/backend/pull/50",
                        "isDraft": false,
                        "createdAt": "2026-03-25T12:00:00Z",
                        "updatedAt": "2026-03-29T09:00:00Z",
                        "additions": 200,
                        "deletions": 80,
                        "reviewDecision": "APPROVED",
                        "author": {"login": "bob"},
                        "repository": {"nameWithOwner": "acme/backend"},
                        "labels": {"nodes": [{"name": "refactor"}]},
                        "comments": {"nodes": []},
                        "reviews": {"nodes": []},
                        "commits": {"nodes": [{"commit": {"committedDate": "2026-03-25T12:00:00Z"}}]}
                    }
                ]
            },
            "q2": {
                "nodes": [
                    {
                        "number": 60,
                        "title": "Add logging to auth",
                        "url": "https://github.com/acme/backend/pull/60",
                        "isDraft": false,
                        "createdAt": "2026-03-26T08:00:00Z",
                        "updatedAt": "2026-03-29T10:00:00Z",
                        "additions": 30,
                        "deletions": 5,
                        "reviewDecision": "REVIEW_REQUIRED",
                        "author": {"login": "carol"},
                        "repository": {"nameWithOwner": "acme/backend"},
                        "labels": {"nodes": []},
                        "comments": {"nodes": [
                            {"author": {"login": "me"}, "createdAt": "2026-03-27T10:00:00Z"}
                        ]},
                        "reviews": {"nodes": []},
                        "commits": {"nodes": [{"commit": {"committedDate": "2026-03-26T08:00:00Z"}}]}
                    }
                ]
            }
        }
    }"#;

    #[test]
    fn parse_review_prs_extracts_all_fields() {
        let prs = parse_prs_response(SAMPLE_RESPONSE, 3).unwrap();
        // Draft PR #99 is filtered out, leaving 3
        assert_eq!(prs.len(), 3);

        let pr = &prs[0];
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Fix login flow");
        assert_eq!(pr.author, "alice");
        assert_eq!(pr.repo, "acme/app");
        assert_eq!(pr.url, "https://github.com/acme/app/pull/42");
        assert!(!pr.is_draft);
        assert_eq!(pr.additions, 15);
        assert_eq!(pr.deletions, 3);
        assert_eq!(pr.review_decision, ReviewDecision::ReviewRequired);
        assert_eq!(pr.labels, vec!["bug", "urgent"]);
    }

    #[test]
    fn parse_prs_response_filters_drafts_and_handles_approved() {
        let prs = parse_prs_response(SAMPLE_RESPONSE, 3).unwrap();
        // Draft PR #99 excluded, leaving 3 PRs sorted by updated_at desc:
        // #42 (14:00), #60 (10:00), #50 (09:00)
        assert_eq!(prs.len(), 3);
        assert_eq!(prs[2].review_decision, ReviewDecision::Approved);
        assert_eq!(prs[2].number, 50);
    }

    #[test]
    fn parse_review_prs_empty_nodes() {
        let json = r#"{"data":{"viewer":{"login":"me"},"q0":{"nodes":[]},"q1":{"nodes":[]},"q2":{"nodes":[]}}}"#;
        let prs = parse_prs_response(json, 3).unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn parse_prs_response_invalid_json() {
        let result = parse_prs_response("not json", 1);
        assert!(result.is_err());
    }

    #[test]
    fn parse_prs_response_null_review_decision_defaults_to_review_required() {
        let json = r#"{"data":{"viewer":{"login":"me"},"q0":{"nodes":[{
            "number": 1, "title": "T", "url": "https://github.com/o/r/pull/1", "isDraft": false,
            "createdAt": "2026-03-28T10:00:00Z", "updatedAt": "2026-03-28T10:00:00Z",
            "additions": 0, "deletions": 0,
            "reviewDecision": null,
            "author": {"login": "a"}, "repository": {"nameWithOwner": "o/r"},
            "labels": {"nodes": []},
            "comments": {"nodes": []},
            "reviews": {"nodes": []},
            "commits": {"nodes": []}
        }]}}}"#;
        let prs = parse_prs_response(json, 1).unwrap();
        assert_eq!(prs[0].review_decision, ReviewDecision::ReviewRequired);
    }

    fn review_queries() -> Vec<String> {
        vec![
            "is:pr is:open review-requested:@me -is:draft -author:app/dependabot -author:app/renovate archived:false".into(),
            "is:pr is:open reviewed-by:@me -author:@me -is:draft -author:app/dependabot -author:app/renovate archived:false".into(),
            "is:pr is:open commenter:@me -author:@me -is:draft -author:app/dependabot -author:app/renovate archived:false".into(),
        ]
    }

    #[test]
    fn fetch_prs_calls_gh_and_parses() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            SAMPLE_RESPONSE.as_bytes(),
        )]);
        let prs = fetch_prs(&runner, &review_queries()).unwrap();
        assert_eq!(prs.len(), 3); // draft filtered out

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "gh");
        assert!(calls[0].1.contains(&"graphql".to_string()));
    }

    #[test]
    fn fetch_prs_gh_failure() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("gh: not authenticated")]);
        let result = fetch_prs(&runner, &review_queries());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not authenticated"));
    }

    #[test]
    fn fetch_prs_query_includes_all_searches() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            SAMPLE_RESPONSE.as_bytes(),
        )]);
        let _ = fetch_prs(&runner, &review_queries());
        let calls = runner.recorded_calls();
        let query_arg = calls[0].1.iter().find(|a| a.contains("query=")).unwrap();
        assert!(
            query_arg.contains("review-requested:@me"),
            "missing review-requested qualifier"
        );
        assert!(
            query_arg.contains("reviewed-by:@me"),
            "missing reviewed-by qualifier"
        );
        assert!(
            query_arg.contains("commenter:@me"),
            "missing commenter qualifier"
        );
        assert!(query_arg.contains("-is:draft"));
        assert!(query_arg.contains("-author:app/dependabot"));
        assert!(query_arg.contains("-author:app/renovate"));
        assert!(query_arg.contains("-author:@me"));
    }

    #[test]
    fn parse_prs_response_deduplicates_across_aliases() {
        // PR #42 appears in all three aliases — should only be counted once.
        let json = r#"{
            "data": {
                "viewer": {"login": "me"},
                "q0": {"nodes": [{
                    "number": 42, "title": "Fix login flow",
                    "url": "https://github.com/acme/app/pull/42",
                    "isDraft": false,
                    "createdAt": "2026-03-28T10:00:00Z", "updatedAt": "2026-03-28T10:00:00Z",
                    "additions": 15, "deletions": 3,
                    "reviewDecision": "REVIEW_REQUIRED",
                    "author": {"login": "alice"}, "repository": {"nameWithOwner": "acme/app"},
                    "labels": {"nodes": []}, "comments": {"nodes": []},
                    "reviews": {"nodes": []}, "commits": {"nodes": []}
                }]},
                "q1": {"nodes": [{
                    "number": 42, "title": "Fix login flow",
                    "url": "https://github.com/acme/app/pull/42",
                    "isDraft": false,
                    "createdAt": "2026-03-28T10:00:00Z", "updatedAt": "2026-03-28T10:00:00Z",
                    "additions": 15, "deletions": 3,
                    "reviewDecision": "REVIEW_REQUIRED",
                    "author": {"login": "alice"}, "repository": {"nameWithOwner": "acme/app"},
                    "labels": {"nodes": []}, "comments": {"nodes": []},
                    "reviews": {"nodes": []}, "commits": {"nodes": []}
                }]},
                "q2": {"nodes": [{
                    "number": 42, "title": "Fix login flow",
                    "url": "https://github.com/acme/app/pull/42",
                    "isDraft": false,
                    "createdAt": "2026-03-28T10:00:00Z", "updatedAt": "2026-03-28T10:00:00Z",
                    "additions": 15, "deletions": 3,
                    "reviewDecision": "REVIEW_REQUIRED",
                    "author": {"login": "alice"}, "repository": {"nameWithOwner": "acme/app"},
                    "labels": {"nodes": []}, "comments": {"nodes": []},
                    "reviews": {"nodes": []}, "commits": {"nodes": []}
                }]}
            }
        }"#;
        let prs = parse_prs_response(json, 3).unwrap();
        assert_eq!(prs.len(), 1, "duplicate should be deduplicated");
        assert_eq!(prs[0].number, 42);
    }

    // -----------------------------------------------------------------------
    // classify_review_decision tests
    // -----------------------------------------------------------------------

    fn make_pr_node(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn classify_approved_takes_priority() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "APPROVED",
            "author": {"login": "alice"},
            "comments": {"nodes": [
                {"author": {"login": "me"}, "createdAt": "2026-03-28T12:00:00Z"}
            ]},
            "reviews": {"nodes": []},
            "commits": {"nodes": []}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::Approved
        );
    }

    #[test]
    fn classify_changes_requested_takes_priority() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "CHANGES_REQUESTED",
            "author": {"login": "alice"},
            "comments": {"nodes": [
                {"author": {"login": "me"}, "createdAt": "2026-03-28T12:00:00Z"}
            ]},
            "reviews": {"nodes": []},
            "commits": {"nodes": []}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::ChangesRequested,
        );
    }

    #[test]
    fn classify_no_viewer_interaction() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": []},
            "reviews": {"nodes": []},
            "commits": {"nodes": []}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::ReviewRequired,
        );
    }

    #[test]
    fn classify_viewer_comment_no_author_response() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": [
                {"author": {"login": "me"}, "createdAt": "2026-03-28T12:00:00Z"}
            ]},
            "reviews": {"nodes": []},
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-27T10:00:00Z"}}]}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::WaitingForResponse,
        );
    }

    #[test]
    fn classify_viewer_commented_review_no_response() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": []},
            "reviews": {"nodes": [
                {"state": "COMMENTED", "author": {"login": "me"}, "submittedAt": "2026-03-28T12:00:00Z"}
            ]},
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-27T10:00:00Z"}}]}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::WaitingForResponse,
        );
    }

    #[test]
    fn classify_viewer_approved_review_no_response() {
        // Viewer approved but overall PR still needs other reviews.
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": []},
            "reviews": {"nodes": [
                {"state": "APPROVED", "author": {"login": "me"}, "submittedAt": "2026-03-28T12:00:00Z"}
            ]},
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-27T10:00:00Z"}}]}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::WaitingForResponse,
        );
    }

    #[test]
    fn classify_author_comment_after_viewer() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": [
                {"author": {"login": "me"}, "createdAt": "2026-03-28T12:00:00Z"},
                {"author": {"login": "alice"}, "createdAt": "2026-03-28T13:00:00Z"}
            ]},
            "reviews": {"nodes": []},
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-27T10:00:00Z"}}]}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::ReviewRequired,
        );
    }

    #[test]
    fn classify_new_commit_after_viewer_comment() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": [
                {"author": {"login": "me"}, "createdAt": "2026-03-28T12:00:00Z"}
            ]},
            "reviews": {"nodes": []},
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-28T14:00:00Z"}}]}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::ReviewRequired,
        );
    }

    #[test]
    fn classify_author_comment_before_viewer() {
        let node = make_pr_node(
            r#"{
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "comments": {"nodes": [
                {"author": {"login": "alice"}, "createdAt": "2026-03-28T10:00:00Z"},
                {"author": {"login": "me"}, "createdAt": "2026-03-28T12:00:00Z"}
            ]},
            "reviews": {"nodes": []},
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-27T10:00:00Z"}}]}
        }"#,
        );
        assert_eq!(
            classify_review_decision(&node, "me"),
            ReviewDecision::WaitingForResponse,
        );
    }

    #[test]
    fn parse_prs_response_extracts_ci_status_and_body() {
        let json = r#"{"data":{"viewer":{"login":"me"},"q0":{"nodes":[{
            "number": 77,
            "title": "Fix auth bug",
            "url": "https://github.com/acme/app/pull/77",
            "isDraft": false,
            "createdAt": "2026-03-28T10:00:00Z",
            "updatedAt": "2026-03-29T14:00:00Z",
            "additions": 10,
            "deletions": 2,
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "alice"},
            "repository": {"nameWithOwner": "acme/app"},
            "labels": {"nodes": []},
            "comments": {"nodes": []},
            "reviews": {"nodes": []},
            "body": "This fixes the auth bug",
            "headRefName": "fix-auth-bug",
            "commits": {"nodes": [{"commit": {"committedDate": "2026-03-28T10:00:00Z", "statusCheckRollup": {"state": "SUCCESS"}}}]},
            "reviewRequests": {"nodes": []}
        }]}}}"#;
        let prs = parse_prs_response(json, 1).unwrap();
        assert_eq!(prs.len(), 1);
        let pr = &prs[0];
        assert_eq!(pr.ci_status, CiStatus::Success);
        assert_eq!(pr.body, "This fixes the auth bug");
        assert_eq!(pr.head_ref, "fix-auth-bug");
    }

    #[test]
    fn parse_reviewers_from_reviews_and_requests() {
        let node = make_pr_node(
            r#"{
            "reviews": {"nodes": [
                {"state": "APPROVED", "author": {"login": "bob"}, "submittedAt": "2026-03-28T12:00:00Z"}
            ]},
            "reviewRequests": {"nodes": [
                {"requestedReviewer": {"login": "carol"}}
            ]}
        }"#,
        );
        let mut reviewers = parse_reviewers(&node);
        reviewers.sort_by(|a, b| a.login.cmp(&b.login));
        assert_eq!(reviewers.len(), 2);
        assert_eq!(reviewers[0].login, "bob");
        assert_eq!(reviewers[0].decision, Some(ReviewDecision::Approved));
        assert_eq!(reviewers[1].login, "carol");
        assert_eq!(reviewers[1].decision, None);
    }

    #[test]
    fn classify_rerequest_moves_to_review_required() {
        let node = serde_json::json!({
            "reviewDecision": "REVIEW_REQUIRED",
            "author": { "login": "alice" },
            "comments": { "nodes": [
                { "author": { "login": "viewer" }, "createdAt": "2026-01-01T01:00:00Z" }
            ] },
            "reviews": { "nodes": [] },
            "commits": { "nodes": [{ "commit": { "committedDate": "2026-01-01T00:00:00Z" } }] },
            "reviewRequests": { "nodes": [
                { "requestedReviewer": { "login": "viewer" } }
            ] }
        });
        let decision = classify_review_decision(&node, "viewer");
        assert_eq!(decision, ReviewDecision::ReviewRequired);
    }

    #[test]
    fn classify_no_rerequest_stays_waiting() {
        let node = serde_json::json!({
            "reviewDecision": "REVIEW_REQUIRED",
            "author": { "login": "alice" },
            "comments": { "nodes": [
                { "author": { "login": "viewer" }, "createdAt": "2026-01-01T01:00:00Z" }
            ] },
            "reviews": { "nodes": [] },
            "commits": { "nodes": [{ "commit": { "committedDate": "2026-01-01T00:00:00Z" } }] },
            "reviewRequests": { "nodes": [] }
        });
        let decision = classify_review_decision(&node, "viewer");
        assert_eq!(decision, ReviewDecision::WaitingForResponse);
    }

    #[test]
    fn classify_draft_filtered_in_parse() {
        let json = r#"{"data":{"viewer":{"login":"me"},"q0":{"nodes":[{
            "number": 1, "title": "T", "url": "https://github.com/o/r/pull/1", "isDraft": true,
            "createdAt": "2026-03-28T10:00:00Z", "updatedAt": "2026-03-28T10:00:00Z",
            "additions": 0, "deletions": 0,
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "a"}, "repository": {"nameWithOwner": "o/r"},
            "labels": {"nodes": []},
            "comments": {"nodes": []},
            "reviews": {"nodes": []},
            "commits": {"nodes": []}
        }]}}}"#;
        let prs = parse_prs_response(json, 1).unwrap();
        assert!(prs.is_empty());
    }

    /// Integration test: calls the real `gh` CLI to verify fetch works end-to-end.
    /// Run with: cargo test fetch_prs_real -- --ignored
    #[test]
    #[ignore]
    fn fetch_prs_real() {
        let runner = crate::process::RealProcessRunner;
        let result = fetch_prs(&runner, &review_queries());
        eprintln!("result: {result:?}");
        assert!(result.is_ok(), "fetch failed: {}", result.unwrap_err());
        let prs = result.unwrap();
        eprintln!("fetched {} PRs", prs.len());
        for pr in &prs {
            eprintln!("  #{} {} [{:?}]", pr.number, pr.title, pr.review_decision);
        }
    }

    // -----------------------------------------------------------------------
    // my PRs / fetch_prs with single-query tests
    // -----------------------------------------------------------------------

    const MY_PRS_RESPONSE: &str = r#"{
    "data": {
        "viewer": {"login": "me"},
        "q0": {
            "nodes": [
                {
                    "number": 101,
                    "title": "My feature PR",
                    "url": "https://github.com/acme/app/pull/101",
                    "isDraft": false,
                    "createdAt": "2026-03-28T10:00:00Z",
                    "updatedAt": "2026-03-29T14:00:00Z",
                    "additions": 50,
                    "deletions": 10,
                    "reviewDecision": "REVIEW_REQUIRED",
                    "author": {"login": "me"},
                    "repository": {"nameWithOwner": "acme/app"},
                    "labels": {"nodes": [{"name": "feature"}]},
                    "comments": {"nodes": []},
                    "reviews": {"nodes": []},
                    "body": "Adds a new feature",
                    "headRefName": "my-feature",
                    "commits": {"nodes": [{"commit": {"committedDate": "2026-03-28T10:00:00Z", "statusCheckRollup": {"state": "SUCCESS"}}}]},
                    "reviewRequests": {"nodes": [{"requestedReviewer": {"login": "alice"}}]}
                }
            ]
        }
    }
}"#;

    #[test]
    fn parse_my_prs_extracts_fields() {
        let prs = parse_prs_response(MY_PRS_RESPONSE, 1).unwrap();
        assert_eq!(prs.len(), 1);
        let pr = &prs[0];
        assert_eq!(pr.number, 101);
        assert_eq!(pr.title, "My feature PR");
        assert_eq!(pr.author, "me");
        assert_eq!(pr.review_decision, ReviewDecision::ReviewRequired);
        assert_eq!(pr.ci_status, CiStatus::Success);
        assert_eq!(pr.reviewers.len(), 1);
        assert_eq!(pr.reviewers[0].login, "alice");
    }

    #[test]
    fn parse_my_prs_filters_drafts() {
        let json = r#"{"data":{"viewer":{"login":"me"},"q0":{"nodes":[{
            "number": 1, "title": "T", "url": "https://github.com/o/r/pull/1", "isDraft": true,
            "createdAt": "2026-03-28T10:00:00Z", "updatedAt": "2026-03-28T10:00:00Z",
            "additions": 0, "deletions": 0,
            "reviewDecision": "REVIEW_REQUIRED",
            "author": {"login": "me"}, "repository": {"nameWithOwner": "o/r"},
            "labels": {"nodes": []}, "comments": {"nodes": []},
            "reviews": {"nodes": []}, "body": "", "headRefName": "x",
            "commits": {"nodes": []}, "reviewRequests": {"nodes": []}
        }]}}}"#;
        let prs = parse_prs_response(json, 1).unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn fetch_prs_with_single_query() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            MY_PRS_RESPONSE.as_bytes(),
        )]);
        let queries = vec!["is:pr is:open author:@me -is:draft archived:false".into()];
        let prs = fetch_prs(&runner, &queries).unwrap();
        assert_eq!(prs.len(), 1);

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        let query_arg = calls[0].1.iter().find(|a| a.contains("query=")).unwrap();
        assert!(
            query_arg.contains("author:@me"),
            "missing author:@me qualifier"
        );
        assert!(query_arg.contains("-is:draft"));
    }

    // --- Security alert parsing tests ---

    const GRAPHQL_ALERTS_RESPONSE: &str = r#"{
        "data": {
            "viewer": {
                "repositories": {
                    "pageInfo": {"hasNextPage": false, "endCursor": null},
                    "nodes": [
                        {
                            "nameWithOwner": "acme/app",
                            "vulnerabilityAlerts": {
                                "nodes": [
                                    {
                                        "number": 1,
                                        "createdAt": "2026-03-01T10:00:00Z",
                                        "securityVulnerability": {
                                            "severity": "CRITICAL",
                                            "package": {"name": "lodash"},
                                            "vulnerableVersionRange": "< 4.17.21",
                                            "firstPatchedVersion": {"identifier": "4.17.21"}
                                        },
                                        "securityAdvisory": {
                                            "summary": "Prototype Pollution in lodash",
                                            "cvss": {"score": 9.8},
                                            "description": "A prototype pollution vuln."
                                        }
                                    },
                                    {
                                        "number": 5,
                                        "createdAt": "2026-03-05T10:00:00Z",
                                        "securityVulnerability": {
                                            "severity": "MODERATE",
                                            "package": {"name": "express"},
                                            "vulnerableVersionRange": "< 5.0.0",
                                            "firstPatchedVersion": null
                                        },
                                        "securityAdvisory": {
                                            "summary": "Open redirect in express",
                                            "cvss": {"score": 5.3},
                                            "description": "An open redirect."
                                        }
                                    }
                                ]
                            }
                        },
                        {
                            "nameWithOwner": "acme/lib",
                            "vulnerabilityAlerts": {
                                "nodes": []
                            }
                        }
                    ]
                }
            }
        }
    }"#;

    #[test]
    fn parse_graphql_security_alerts_basic() {
        let alerts = parse_graphql_security_alerts(GRAPHQL_ALERTS_RESPONSE).unwrap();
        assert_eq!(alerts.len(), 2);

        assert_eq!(alerts[0].number, 1);
        assert_eq!(alerts[0].repo, "acme/app");
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert_eq!(alerts[0].kind, AlertKind::Dependabot);
        assert_eq!(alerts[0].package.as_deref(), Some("lodash"));
        assert_eq!(alerts[0].vulnerable_range.as_deref(), Some("< 4.17.21"));
        assert_eq!(alerts[0].fixed_version.as_deref(), Some("4.17.21"));
        assert_eq!(alerts[0].cvss_score, Some(9.8));
        assert_eq!(
            alerts[0].url,
            "https://github.com/acme/app/security/dependabot/1"
        );
        assert_eq!(alerts[0].title, "Prototype Pollution in lodash");

        assert_eq!(alerts[1].number, 5);
        assert_eq!(alerts[1].repo, "acme/app");
        assert_eq!(alerts[1].severity, AlertSeverity::Medium);
        assert_eq!(alerts[1].fixed_version, None);
    }

    #[test]
    fn parse_graphql_security_alerts_empty_repos() {
        let json = r#"{"data":{"viewer":{"repositories":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}"#;
        let alerts = parse_graphql_security_alerts(json).unwrap();
        assert!(alerts.is_empty());
    }

    #[test]
    fn parse_graphql_security_alerts_invalid_json() {
        let result = parse_graphql_security_alerts("not json");
        assert!(result.is_err());
    }

    const PER_REPO_RESPONSE: &str = r#"{
        "data": {
            "r0": {
                "nameWithOwner": "acme/app",
                "vulnerabilityAlerts": {
                    "nodes": [
                        {
                            "number": 1,
                            "createdAt": "2026-03-01T10:00:00Z",
                            "securityVulnerability": {
                                "severity": "CRITICAL",
                                "package": {"name": "lodash"},
                                "vulnerableVersionRange": "< 4.17.21",
                                "firstPatchedVersion": {"identifier": "4.17.21"}
                            },
                            "securityAdvisory": {
                                "summary": "Prototype Pollution in lodash",
                                "cvss": {"score": 9.8},
                                "description": "A prototype pollution vuln."
                            }
                        }
                    ]
                }
            },
            "r1": {
                "nameWithOwner": "acme/lib",
                "vulnerabilityAlerts": {
                    "nodes": []
                }
            }
        }
    }"#;

    #[test]
    fn fetch_security_alerts_empty_repos_returns_empty_without_shell_call() {
        let runner = MockProcessRunner::new(vec![]);
        let alerts = fetch_security_alerts(&runner, &[]).unwrap();
        assert!(alerts.is_empty());
        assert_eq!(
            runner.recorded_calls().len(),
            0,
            "should not call gh when repos is empty"
        );
    }

    #[test]
    fn fetch_security_alerts_builds_per_repo_graphql() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            PER_REPO_RESPONSE.as_bytes(),
        )]);
        let alerts =
            fetch_security_alerts(&runner, &["acme/app".to_string(), "acme/lib".to_string()])
                .unwrap();

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].repo, "acme/app");
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert_eq!(alerts[0].package.as_deref(), Some("lodash"));

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        let query_arg = calls[0].1.iter().find(|a| a.contains("query=")).unwrap();
        assert!(
            query_arg.contains("r0: repository"),
            "query should use r0 alias"
        );
        assert!(
            query_arg.contains(r#"owner: "acme""#),
            "query should include owner"
        );
        assert!(
            query_arg.contains(r#"name: "app""#),
            "query should include repo name"
        );
        assert!(
            query_arg.contains("r1: repository"),
            "query should have r1 alias for second repo"
        );
    }

    #[test]
    fn fetch_security_alerts_per_repo_sorted_by_severity() {
        let response = r#"{
            "data": {
                "r0": {
                    "nameWithOwner": "acme/app",
                    "vulnerabilityAlerts": {"nodes": [
                        {"number": 2, "createdAt": "2026-03-01T10:00:00Z",
                         "securityVulnerability": {"severity": "HIGH", "package": {"name": "x"}, "vulnerableVersionRange": "< 1.0", "firstPatchedVersion": null},
                         "securityAdvisory": {"summary": "High vuln", "cvss": {"score": 7.5}, "description": "d"}}
                    ]}
                },
                "r1": {
                    "nameWithOwner": "acme/lib",
                    "vulnerabilityAlerts": {"nodes": [
                        {"number": 3, "createdAt": "2026-03-01T10:00:00Z",
                         "securityVulnerability": {"severity": "CRITICAL", "package": {"name": "y"}, "vulnerableVersionRange": "< 2.0", "firstPatchedVersion": {"identifier": "2.0"}},
                         "securityAdvisory": {"summary": "Critical vuln", "cvss": {"score": 9.8}, "description": "d"}}
                    ]}
                }
            }
        }"#;
        let runner =
            MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(response.as_bytes())]);
        let alerts =
            fetch_security_alerts(&runner, &["acme/app".to_string(), "acme/lib".to_string()])
                .unwrap();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert_eq!(alerts[1].severity, AlertSeverity::High);
    }

    #[test]
    fn parse_graphql_security_alerts_per_repo_basic() {
        let alerts = parse_graphql_security_alerts_per_repo(PER_REPO_RESPONSE, 2).unwrap();
        assert_eq!(alerts.len(), 1, "acme/lib has no alerts");
        assert_eq!(alerts[0].number, 1);
        assert_eq!(alerts[0].repo, "acme/app");
        assert_eq!(alerts[0].severity, AlertSeverity::Critical);
        assert_eq!(alerts[0].kind, AlertKind::Dependabot);
        assert_eq!(alerts[0].package.as_deref(), Some("lodash"));
        assert_eq!(alerts[0].fixed_version.as_deref(), Some("4.17.21"));
        assert_eq!(alerts[0].cvss_score, Some(9.8));
        assert_eq!(
            alerts[0].url,
            "https://github.com/acme/app/security/dependabot/1"
        );
    }

    #[test]
    fn parse_graphql_security_alerts_per_repo_invalid_json() {
        let result = parse_graphql_security_alerts_per_repo("not json", 1);
        assert!(result.is_err());
    }

    #[test]
    fn fetch_security_alerts_skips_invalid_slugs() {
        // "noslash" (index 0) has no '/' and is skipped; "acme/app" (index 1) becomes r1
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            r#"{"data":{"r1":{"nameWithOwner":"acme/app","vulnerabilityAlerts":{"nodes":[]}}}}"#
                .as_bytes(),
        )]);
        let result =
            fetch_security_alerts(&runner, &["noslash".to_string(), "acme/app".to_string()]);
        assert!(result.is_ok());
        assert_eq!(runner.recorded_calls().len(), 1);
        let calls = runner.recorded_calls();
        let query_arg = calls[0].1.iter().find(|a| a.contains("query=")).unwrap();
        assert!(
            query_arg.contains("r1: repository"),
            "valid slug at index 1 should become r1"
        );
        assert!(
            !query_arg.contains("r0: repository"),
            "invalid slug at index 0 should be absent"
        );
    }

    #[test]
    fn fetch_prs_empty_queries_returns_empty() {
        let runner = MockProcessRunner::new(vec![]);
        let prs = fetch_prs(&runner, &[]).unwrap();
        assert!(prs.is_empty());
        assert!(runner.recorded_calls().is_empty());
    }

    #[test]
    fn build_search_graphql_single_query() {
        let queries = vec!["is:pr is:open author:@me".into()];
        let gql = build_search_graphql(&queries);
        assert!(gql.contains("viewer { login }"));
        assert!(gql.contains("q0: search(query: \"is:pr is:open author:@me\""));
        assert!(!gql.contains("q1:"));
    }

    #[test]
    fn build_search_graphql_multiple_queries() {
        let queries = vec!["query-a".into(), "query-b".into(), "query-c".into()];
        let gql = build_search_graphql(&queries);
        assert!(gql.contains("q0: search(query: \"query-a\""));
        assert!(gql.contains("q1: search(query: \"query-b\""));
        assert!(gql.contains("q2: search(query: \"query-c\""));
    }

    // -----------------------------------------------------------------------
    // DependabotConfig parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_dependabot_config_splits_base_and_repos() {
        let input = "# Base query\nis:pr is:open author:app/dependabot -is:draft\n\n# Repositories\nacme/frontend\nacme/backend\n";
        let cfg = parse_dependabot_config(input);
        assert_eq!(
            cfg.base_query,
            "is:pr is:open author:app/dependabot -is:draft"
        );
        assert_eq!(cfg.repos, vec!["acme/frontend", "acme/backend"]);
    }

    #[test]
    fn parse_dependabot_config_ignores_comments_and_blanks() {
        let input = "# Base query\nis:pr is:open -is:draft\n\n# Repositories\n# skip this\nacme/frontend\n\nacme/backend\n# also skip\n";
        let cfg = parse_dependabot_config(input);
        assert_eq!(cfg.repos, vec!["acme/frontend", "acme/backend"]);
    }

    #[test]
    fn parse_dependabot_config_empty_repos_returns_empty_vec() {
        let input = "# Base query\nis:pr is:open\n\n# Repositories\n";
        let cfg = parse_dependabot_config(input);
        assert!(cfg.repos.is_empty());
    }

    #[test]
    fn parse_dependabot_config_missing_repos_section_uses_empty() {
        let input = "# Base query\nis:pr is:open author:app/dependabot\n";
        let cfg = parse_dependabot_config(input);
        assert!(cfg.repos.is_empty());
        assert_eq!(cfg.base_query, "is:pr is:open author:app/dependabot");
    }

    #[test]
    fn parse_dependabot_config_missing_base_uses_default() {
        let input = "# Base query\n\n# Repositories\nacme/app\n";
        let cfg = parse_dependabot_config(input);
        assert_eq!(cfg.base_query, DEFAULT_DEPENDABOT_BASE_QUERY);
        assert_eq!(cfg.repos, vec!["acme/app"]);
    }

    #[test]
    fn assemble_queries_produces_base_plus_repo() {
        let cfg = DependabotConfig {
            base_query: "is:pr is:open author:app/dependabot".to_string(),
            repos: vec!["acme/frontend".to_string(), "acme/backend".to_string()],
        };
        let (queries, warnings) = assemble_dependabot_queries(&cfg);
        assert!(warnings.is_empty());
        assert_eq!(
            queries,
            vec![
                "is:pr is:open author:app/dependabot repo:acme/frontend",
                "is:pr is:open author:app/dependabot repo:acme/backend",
            ]
        );
    }

    #[test]
    fn assemble_queries_skips_invalid_slug() {
        let cfg = DependabotConfig {
            base_query: "is:pr is:open".to_string(),
            repos: vec!["notaslug".to_string(), "acme/valid".to_string()],
        };
        let (queries, warnings) = assemble_dependabot_queries(&cfg);
        assert_eq!(queries, vec!["is:pr is:open repo:acme/valid"]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("notaslug"));
    }

    #[test]
    fn migrate_bot_queries_extracts_repos() {
        let old = "is:pr is:open author:app/dependabot repo:acme/frontend repo:acme/backend\n";
        let cfg = migrate_bot_queries_to_dependabot_config(Some(old)).unwrap();
        assert_eq!(cfg.repos, vec!["acme/frontend", "acme/backend"]);
        assert_eq!(cfg.base_query, DEFAULT_DEPENDABOT_BASE_QUERY);
    }

    #[test]
    fn migrate_bot_queries_deduplicates_repos() {
        let old = "repo:acme/app\nrepo:acme/app repo:acme/backend\n";
        let cfg = migrate_bot_queries_to_dependabot_config(Some(old)).unwrap();
        assert_eq!(cfg.repos, vec!["acme/app", "acme/backend"]);
    }

    #[test]
    fn migrate_bot_queries_noop_when_none() {
        assert!(migrate_bot_queries_to_dependabot_config(None).is_none());
    }

    #[test]
    fn format_dependabot_config_roundtrips() {
        let cfg = DependabotConfig {
            base_query: "is:pr is:open author:app/dependabot -is:draft".to_string(),
            repos: vec!["acme/frontend".to_string(), "acme/backend".to_string()],
        };
        let formatted = format_dependabot_config(&cfg);
        let parsed = parse_dependabot_config(&formatted);
        assert_eq!(parsed, cfg);
    }
}
