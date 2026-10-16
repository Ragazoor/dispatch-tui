//! Path and repo-identity utilities shared across the domain model.
//!
//! The repo-grouping family lives here as one unit — both halves must agree on
//! [`UNKNOWN_REPO_GROUP`], so splitting them across a layer boundary would put
//! the shared fallback one module away from one of its users. Turning a
//! resolved repo name into a *local filesystem path* is a different job and
//! stays in the dispatch adapter (`resolve_repo_path`), which is the only
//! thing here that needs to know what is on disk.

/// Expand a leading `~` or `~/` against a home directory *value*.
///
/// Returns the path unchanged if it doesn't start with `~`, or if the home
/// directory is unavailable. An empty value counts as unavailable rather than
/// as the filesystem root — otherwise `~/repo` becomes `/repo`, a well-formed
/// path that nothing downstream can tell from a real one, and this expansion
/// feeds durable state (repo paths recorded in the database, and the v18/v31
/// migrations that rewrite them). Same rule as `crate::setup::home_dir`; see
/// docs/specs/dispatch.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
///
/// Takes the value as a parameter so the unavailable cases are testable:
/// `std::env::set_var` is `unsafe` in edition 2024 and races the test
/// harness's threads regardless (see `crate::editor::resolve_editor`).
pub fn expand_tilde_with_home(path: &str, home: Option<&str>) -> String {
    let home = home.filter(|home| !home.is_empty());
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home {
            return format!("{home}/{rest}");
        }
    } else if path == "~" {
        if let Some(home) = home {
            return home.to_string();
        }
    }
    path.to_string()
}

/// [`expand_tilde_with_home`] against the real process environment.
pub fn expand_tilde(path: &str) -> String {
    let home = std::env::var("HOME").ok();
    expand_tilde_with_home(path, home.as_deref())
}

/// Grouping key for anything whose repo cannot be determined.
pub const UNKNOWN_REPO_GROUP: &str = "other";

/// Derive a repo-grouping key from a filesystem repo path: the final path
/// component (basename). Empty / root paths fall back to
/// [`UNKNOWN_REPO_GROUP`], matching the URL-based grouping used by feeds.
pub fn repo_name_from_path(repo_path: &str) -> String {
    std::path::Path::new(repo_path.trim_end_matches('/'))
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(UNKNOWN_REPO_GROUP)
        .to_string()
}

/// Extract the short repo name (e.g. `"my-repo"`) from a GitHub URL — the
/// URL-side twin of [`repo_name_from_path`], sharing its fallback.
///
/// Returns [`UNKNOWN_REPO_GROUP`] for non-GitHub URLs, empty strings, and malformed input.
pub fn repo_name_from_url(url: &str) -> String {
    extract_github_repo(url)
        .and_then(|s| s.split('/').next_back())
        .unwrap_or(UNKNOWN_REPO_GROUP)
        .to_string()
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

#[cfg(test)]
mod tests;
