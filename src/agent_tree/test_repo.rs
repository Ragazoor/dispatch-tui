//! A real git repository with one linked worktree, for the agent-tree and
//! diff-pane tests whose subject is what GIT reports — staged versus unstaged,
//! a commit against its parent, the commits since a fork point. A mock can
//! only prove which argv was sent; these prove what that argv answers.
//!
//! The fixture's own git runs with a sanitised environment (docs/testing.md:
//! "A test that builds a real git repo on disk"), so a developer's global
//! `commit.gpgsign` or hooks cannot change what gets built.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `<tmp>/repo` on `main` with one seed commit, and a linked worktree at
/// `<tmp>/repo/.worktrees/task` on branch `task`, branched from `main`.
pub(crate) struct TestRepo {
    _dir: tempfile::TempDir,
    pub repo: PathBuf,
    pub worktree: PathBuf,
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git must be on PATH");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

impl TestRepo {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        git_in(&repo, &["init", "-q", "--initial-branch=main", "."]);
        // Keep the linked worktree out of the main checkout's `git add -A`.
        std::fs::create_dir_all(repo.join(".git/info")).expect("mkdir");
        std::fs::write(repo.join(".git/info/exclude"), ".worktrees/\n").expect("exclude");
        std::fs::write(repo.join("seed.txt"), "seed\n").expect("write");
        git_in(&repo, &["add", "-A"]);
        git_in(&repo, &["commit", "-qm", "seed"]);
        let worktree = repo.join(".worktrees").join("task");
        git_in(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_str().expect("utf-8 path"),
                "-b",
                "task",
            ],
        );
        Self {
            _dir: dir,
            repo,
            worktree,
        }
    }

    /// The worktree, as the pane's root.
    pub(crate) fn root(&self) -> &Path {
        &self.worktree
    }

    /// The worktree path as the open-set helpers take it.
    pub(crate) fn root_str(&self) -> String {
        self.worktree.to_string_lossy().into_owned()
    }

    /// Run git in the worktree and return its stdout.
    pub(crate) fn git(&self, args: &[&str]) -> String {
        git_in(&self.worktree, args)
    }

    /// Run git in the main checkout (on `main`) and return its stdout.
    pub(crate) fn git_main(&self, args: &[&str]) -> String {
        git_in(&self.repo, args)
    }

    /// Write `contents` to `rel` inside the worktree, creating parents.
    pub(crate) fn write(&self, rel: &str, contents: &str) {
        let path = self.worktree.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write");
    }

    /// Write raw bytes to `rel` inside the worktree.
    pub(crate) fn write_bytes(&self, rel: &str, contents: &[u8]) {
        std::fs::write(self.worktree.join(rel), contents).expect("write");
    }

    /// Append `contents` to `rel` inside the worktree.
    pub(crate) fn append(&self, rel: &str, contents: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.worktree.join(rel))
            .expect("open");
        file.write_all(contents.as_bytes()).expect("append");
    }

    /// Stage everything and commit it in the worktree; returns the new id.
    pub(crate) fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
        self.head()
    }

    /// Commit a change on `main` in the main checkout; returns the new id.
    pub(crate) fn commit_on_main(&self, file: &str, message: &str) -> String {
        std::fs::write(self.repo.join(file), format!("{message}\n")).expect("write");
        self.git_main(&["add", "-A"]);
        self.git_main(&["commit", "-qm", message]);
        self.git_main(&["rev-parse", "HEAD"]).trim().to_string()
    }

    pub(crate) fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}
