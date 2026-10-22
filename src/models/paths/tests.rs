use super::*;

// --- repo_name_from_url tests ---

#[test]
fn repo_name_from_url_extracts_last_segment() {
    assert_eq!(
        repo_name_from_url("https://github.com/org/my-repo/pull/42"),
        "my-repo"
    );
    assert_eq!(
        repo_name_from_url("https://github.com/org/my-repo"),
        "my-repo"
    );
    assert_eq!(
        repo_name_from_url("https://github.com/org/another-repo/issues/1"),
        "another-repo"
    );
}

#[test]
fn repo_name_from_url_returns_other_for_non_github() {
    assert_eq!(repo_name_from_url(""), "other");
    assert_eq!(
        repo_name_from_url("https://example.com/not-github"),
        "other"
    );
    assert_eq!(repo_name_from_url("https://gitlab.com/org/repo"), "other");
}

/// The store decides feed routing now (store-split M3); these are the
/// module's twins of the two URL helpers, and must not drift.
#[test]
fn url_helpers_agree_with_the_modules_twins() {
    for url in [
        "https://github.com/org/my-repo/pull/42",
        "https://github.com/org/my-repo",
        "https://github.com/org/my-repo/",
        "https://github.com/org",
        "https://github.com/",
        "https://github.com//x",
        "https://gitlab.com/org/repo",
        "http://github.com/org/repo",
        "",
        "not a url",
    ] {
        assert_eq!(
            extract_github_repo(url),
            dispatch_spacetime_module::extract_github_repo(url),
            "{url:?}"
        );
        assert_eq!(
            repo_name_from_url(url),
            dispatch_spacetime_module::repo_name_from_url(url),
            "{url:?}"
        );
    }
}

/// The route and exclusion rules moved into the module (feeds.allium:
/// RouteSignals, ExcludeFromReviews); the board's `Signal` names are what
/// the feed script emits and the module reads, so each must serialise to
/// the spelling the module matches on.
#[test]
fn signal_names_are_the_ones_the_module_reads() {
    use crate::models::Signal::*;
    let names: Vec<String> = [
        DirectRequest,
        TeamRequest,
        Reviewed,
        Commented,
        AuthorBot,
        AuthorMe,
        OrgReview,
        Approved,
    ]
    .iter()
    .map(|s| {
        serde_json::to_value(s)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    })
    .collect();
    assert_eq!(
        names,
        [
            "direct-request",
            "team-request",
            "reviewed",
            "commented",
            "author-bot",
            "author-me",
            "org-review",
            "approved"
        ]
    );
    assert!(dispatch_spacetime_module::excluded_from_reviews(&[names
        [5]
    .clone()]));
    assert_eq!(
        dispatch_spacetime_module::route_signals(&[names[1].clone()]),
        "team-reviews"
    );
}

// --- extract_github_repo tests ---

#[test]
fn extract_github_repo_pr_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/pull/42"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_issue_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/issues/5"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_root_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_root_url_with_trailing_slash() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_tree_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/tree/main"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_non_github_url() {
    assert_eq!(
        extract_github_repo("https://jira.company.com/browse/PROJ-123"),
        None
    );
}

#[test]
fn extract_github_repo_empty_string() {
    assert_eq!(extract_github_repo(""), None);
}

#[test]
fn extract_github_repo_only_one_segment() {
    assert_eq!(extract_github_repo("https://github.com/org"), None);
}

#[test]
fn extract_github_repo_malformed_url() {
    assert_eq!(extract_github_repo("not-a-url"), None);
}

#[test]
fn repo_name_from_path_uses_basename() {
    assert_eq!(repo_name_from_path("/home/u/dispatch"), "dispatch");
    assert_eq!(repo_name_from_path("/home/u/dispatch/"), "dispatch");
    assert_eq!(repo_name_from_path(""), UNKNOWN_REPO_GROUP);
    assert_eq!(repo_name_from_path("/"), UNKNOWN_REPO_GROUP);
}

#[test]
fn expand_tilde_with_path() {
    let home = std::env::var("HOME").unwrap();
    assert_eq!(
        expand_tilde("~/projects/foo"),
        format!("{home}/projects/foo")
    );
}

#[test]
fn expand_tilde_bare() {
    let home = std::env::var("HOME").unwrap();
    assert_eq!(expand_tilde("~"), home);
}

/// docs/specs/observability.allium: StatusLineDecorator,
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
///
/// `~` has no meaning without a home directory, so it must stay literal
/// rather than expand to the filesystem root. `/repo` is a well-formed
/// path that nothing downstream can tell from a real one, and this
/// expansion is what `dispatch::trust::project_key` keys the trust store
/// on and what the repo-path migrations write back to the database.
///
/// The home directory is a parameter, so both spellings of "unavailable"
/// are reachable without mutating the process environment.
#[test]
fn expand_tilde_leaves_the_path_alone_when_there_is_no_home_directory() {
    for absent in [None, Some("")] {
        assert_eq!(expand_tilde_with_home("~/repo", absent), "~/repo");
        assert_eq!(expand_tilde_with_home("~", absent), "~");
    }
    assert_eq!(expand_tilde_with_home("~/repo", Some("/h")), "/h/repo");
    assert_eq!(expand_tilde_with_home("~", Some("/h")), "/h");
}

#[test]
fn expand_tilde_absolute_unchanged() {
    assert_eq!(expand_tilde("/home/user/foo"), "/home/user/foo");
}

// --- validate_repo_path tests ---

#[test]
fn validate_repo_path_existing_dir() {
    assert!(validate_repo_path("/tmp").is_ok());
}

#[test]
fn validate_repo_path_nonexistent() {
    let result = validate_repo_path("/nonexistent/path");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("does not exist"));
}

#[test]
fn validate_repo_path_not_a_dir() {
    let result = validate_repo_path("/etc/hostname");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Not a directory"));
}
