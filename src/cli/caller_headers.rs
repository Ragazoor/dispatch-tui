//! Pure path-parsing helper for the `dispatch caller-headers` subcommand.
//!
//! Used as a Claude Code `headersHelper` — runs on every MCP session
//! start/reconnect, reads only its own CWD, and prints the identity
//! header JSON to stdout. No database, no network, no async.

use serde_json::json;
use std::path::Path;

const SESSION_JSON: &str = r#"{"X-Caller-Kind":"session"}"#;

/// Inspect the path and emit the JSON headers payload for it.
///
/// Returns `(stdout, exit_code)`. Always exits 0: "no match" is a
/// successful `session` response, not an error.
pub fn resolve_headers_for_path(cwd: &Path) -> (String, i32) {
    let canonical = match cwd.canonicalize() {
        Ok(p) => p,
        Err(_) => return (SESSION_JSON.to_string(), 0),
    };

    let task_id = canonical
        .components()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|w| {
            let parent = w[0].as_os_str().to_str()?;
            let leaf = w[1].as_os_str().to_str()?;
            if parent != ".worktrees" {
                return None;
            }
            let head = leaf.split_once('-').map(|(h, _)| h).unwrap_or(leaf);
            head.parse::<i64>().ok()
        });

    match task_id {
        Some(id) => (json!({ "X-Caller-Task-Id": id.to_string() }).to_string(), 0),
        None => (SESSION_JSON.to_string(), 0),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn make_worktree_dir(base: &Path, leaf: &str) -> PathBuf {
        let dir = base.join(".worktrees").join(leaf);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn worktree_basename_with_task_id_emits_task_header() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = make_worktree_dir(tmp.path(), "732-validate-create-task");
        let (out, code) = resolve_headers_for_path(&wt);
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["X-Caller-Task-Id"], "732");
    }

    #[test]
    fn worktree_basename_with_only_id_emits_task_header() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = make_worktree_dir(tmp.path(), "9");
        let (out, _) = resolve_headers_for_path(&wt);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["X-Caller-Task-Id"], "9");
    }

    #[test]
    fn non_worktree_path_emits_session() {
        let tmp = tempfile::tempdir().unwrap();
        let (out, _) = resolve_headers_for_path(tmp.path());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["X-Caller-Kind"], "session");
    }

    #[test]
    fn worktree_with_non_numeric_prefix_emits_session() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = make_worktree_dir(tmp.path(), "feature-foo");
        let (out, _) = resolve_headers_for_path(&wt);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["X-Caller-Kind"], "session");
    }

    #[test]
    fn nonexistent_path_falls_back_to_session() {
        let (out, code) = resolve_headers_for_path(Path::new("/no/such/path"));
        assert_eq!(code, 0);
        assert!(out.contains("session"));
    }

    #[test]
    fn nested_subdir_under_worktree_still_resolves() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = make_worktree_dir(tmp.path(), "42-sub");
        let nested = wt.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        let (out, _) = resolve_headers_for_path(&nested);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["X-Caller-Task-Id"], "42");
    }
}
