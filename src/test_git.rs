//! Running real git in a test fixture.
//!
//! A test that builds a real repo on disk must run git with a sanitised
//! environment (docs/testing.md: "A test that builds a real git repo on
//! disk"). Without it the fixture is whatever the developer's global config
//! makes it — `commit.gpgsign`, `core.hooksPath` and `init.templateDir` all
//! change what gets built — and on a machine with no configured `user.email`,
//! a fresh CI container, the seed commit fails outright. This is the one copy;
//! `tests/` targets reach it through the `test-support` feature.

#![allow(clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Output};

/// Run `git <args>` in `dir` with the sanitised environment and return its
/// stdout. Panics with git's stderr when git fails.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = try_git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// [`git`] without the success assertion, for a caller that expects git to
/// FAIL and wants to read why.
pub fn try_git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git must be on PATH")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_commit_carries_the_fixture_identity() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "--initial-branch=main", "."]);
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "seed"]);
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%an <%ae>"]).trim(),
            "test <test@example.com>"
        );
    }

    #[test]
    fn try_git_hands_back_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let out = try_git(dir.path(), &["rev-parse", "HEAD"]);
        assert!(!out.status.success());
        assert!(!out.stderr.is_empty());
    }
}
