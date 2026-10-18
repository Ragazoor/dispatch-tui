//! Real-git tests for behaviour that is git's own: what `git status` reports,
//! and what `git worktree add` does to an existing branch.

use crate::dispatch::worktree::{add_worktree, ensure_worktrees_excluded};
use crate::process::{RealProcessRunner, SUBPROCESS_TIMEOUT};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
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
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repo whose `.gitignore` does not mention `.worktrees/`, as in a target repo.
fn repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "--initial-branch=main", "."]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let path = repo.to_string_lossy().into_owned();
    (dir, path)
}

#[test]
fn a_worktree_in_an_unignored_repo_makes_the_primary_checkout_look_dirty() {
    let (_dir, repo) = repo();
    add_worktree(
        &RealProcessRunner::default(),
        &repo,
        &format!("{repo}/.worktrees/1-x"),
        "1-x",
        None,
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let dirty = crate::git::dirty_files(&repo, &RealProcessRunner::default()).unwrap();
    assert!(dirty.is_empty(), "wrap_up would be refused: {dirty:?}");
}

#[test]
fn ensure_worktrees_excluded_is_idempotent() {
    let (_dir, repo) = repo();
    ensure_worktrees_excluded(&repo);
    ensure_worktrees_excluded(&repo);
    let exclude = std::fs::read_to_string(format!("{repo}/.git/info/exclude")).unwrap();
    assert_eq!(exclude.matches(".worktrees/").count(), 1, "{exclude}");
}

#[test]
fn add_worktree_refuses_to_reset_a_branch_holding_unpushed_commits() {
    let (_dir, repo) = repo();
    git(Path::new(&repo), &["branch", "1-x"]);
    git(Path::new(&repo), &["checkout", "-q", "1-x"]);
    std::fs::write(format!("{repo}/own.txt"), "own\n").unwrap();
    git(Path::new(&repo), &["add", "-A"]);
    git(Path::new(&repo), &["commit", "-qm", "unpushed work"]);
    git(Path::new(&repo), &["checkout", "-q", "main"]);
    let before = git(Path::new(&repo), &["rev-parse", "1-x"]);

    let err = add_worktree(
        &RealProcessRunner::default(),
        &repo,
        &format!("{repo}/.worktrees/1-x"),
        "1-x",
        Some("main"),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("unpushed"), "{err:#}");
    assert_eq!(git(Path::new(&repo), &["rev-parse", "1-x"]), before);
}

#[test]
fn add_worktree_reuses_an_existing_branch_that_is_behind_the_start_point() {
    let (_dir, repo) = repo();
    git(Path::new(&repo), &["branch", "1-x"]);
    std::fs::write(format!("{repo}/b.txt"), "b\n").unwrap();
    git(Path::new(&repo), &["add", "-A"]);
    git(Path::new(&repo), &["commit", "-qm", "main moves"]);

    add_worktree(
        &RealProcessRunner::default(),
        &repo,
        &format!("{repo}/.worktrees/1-x"),
        "1-x",
        Some("main"),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    assert_eq!(
        git(Path::new(&repo), &["rev-parse", "1-x"]),
        git(Path::new(&repo), &["rev-parse", "main"])
    );
}

#[test]
fn ensure_worktrees_excluded_finds_the_git_dir_of_a_repo_with_a_git_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let git_dir = dir.path().join("elsewhere.git");
    std::fs::create_dir_all(&repo).unwrap();
    git(
        &repo,
        &[
            "init",
            "-q",
            "--initial-branch=main",
            "--separate-git-dir",
            git_dir.to_str().unwrap(),
            ".",
        ],
    );
    assert!(repo.join(".git").is_file());

    ensure_worktrees_excluded(repo.to_str().unwrap());

    let exclude = std::fs::read_to_string(git_dir.join("info/exclude")).unwrap();
    assert!(exclude.lines().any(|l| l == ".worktrees/"), "{exclude}");
}

#[test]
fn ensure_worktrees_excluded_keeps_existing_rules_and_leaves_no_temp_file() {
    let (_dir, repo) = repo();
    let info = format!("{repo}/.git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(format!("{info}/exclude"), "*.log").unwrap();

    ensure_worktrees_excluded(&repo);

    let exclude = std::fs::read_to_string(format!("{info}/exclude")).unwrap();
    assert_eq!(exclude, "*.log\n.worktrees/\n");
    let leftovers: Vec<_> = std::fs::read_dir(&info)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().starts_with(".atomic-write."))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
