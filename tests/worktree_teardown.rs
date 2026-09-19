//! `WorktreeDirectoryMustNotSurviveTeardown` (docs/specs/tasks.allium) against
//! REAL git, in real repositories on disk.
//!
//! The unit tests in `src/dispatch/tests.rs` mock the `git` calls, so they
//! prove what teardown does with a given answer from git — not that git gives
//! those answers. This file pins the premise the fix rests on: that
//! `git worktree remove --force` has failure modes in which it removes
//! *nothing* and leaves a multi-gigabyte `target/` on disk, and that one of
//! them reports `WORKTREE_ALREADY_REMOVED`, the string dispatch reads as
//! success (#4882).
//!
//! Without these, a future git whose behaviour or wording changed would leave
//! the leak back in place with a green suite.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use dispatch_tui::dispatch::teardown_task;
use dispatch_tui::process::RealProcessRunner;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run git with a sanitised environment, as `tests/tmux_lifecycle.rs` does and
/// for the same reason: a developer's global `commit.gpgsign`, `core.hooksPath`
/// or `init.templateDir` would otherwise change what the fixture builds, and a
/// machine with no configured `user.email` — a fresh CI container — cannot
/// commit at all without the identity vars.
fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    let out = try_git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// The sanitised-env half of [`git`], for a caller that expects git to FAIL —
/// `git`'s own success assertion would panic before the caller ever sees that
/// failure's stderr.
fn try_git(dir: &Path, args: &[&str]) -> std::process::Output {
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

/// A repo with one worktree at `<repo>/.worktrees/<slug>`, holding a tracked
/// file and the gitignored build output that made the leak expensive.
fn repo_with_worktree(slug: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "--initial-branch=main", "."]);
    std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(repo.join("CLAUDE.md"), "tracked\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);

    let worktree = repo.join(".worktrees").join(slug);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().unwrap(),
            "-b",
            slug,
        ],
    );
    std::fs::create_dir_all(worktree.join("target/debug")).unwrap();
    std::fs::write(worktree.join("target/debug/huge.rlib"), b"build output").unwrap();
    (dir, repo, worktree)
}

fn tear_down(repo: &Path, worktree: &Path) -> Result<(), dispatch_tui::dispatch::TeardownFailure> {
    teardown_task(
        repo.to_str().unwrap(),
        Some(worktree.to_str().unwrap()),
        None,
        &RealProcessRunner::default(),
    )
}

#[test]
fn a_healthy_worktree_with_gitignored_build_output_is_fully_gone() {
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");

    tear_down(&repo, &worktree).unwrap();

    assert!(!worktree.exists(), "the worktree path must not exist");
}

#[test]
fn a_worktree_whose_git_pointer_file_is_missing_is_still_fully_removed() {
    // Real git fails this with "validation failed, cannot remove working tree"
    // and removes NOTHING — tracked files included. Teardown's own delete is
    // what finishes the job.
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    std::fs::remove_file(worktree.join(".git")).unwrap();

    // Git's exit code does not decide step 2: what is left on disk does.
    tear_down(&repo, &worktree).unwrap();

    assert!(
        !worktree.exists(),
        "a worktree git refused to validate must still be removed — this is \
         one of the two husk shapes measured in #4877"
    );
}

#[test]
fn the_same_task_can_be_dispatched_again_after_the_bad_teardown() {
    // The leftover is only recoverable if git can reuse the name. When git
    // removed nothing it also kept `.git/worktrees/<name>`, and that record
    // makes `git worktree add` fail with "is already used by worktree at"
    // however thoroughly the directory was deleted. Deleting the husk without
    // pruning trades a disk leak for a task that can never run again.
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    std::fs::remove_file(worktree.join(".git")).unwrap();

    tear_down(&repo, &worktree).unwrap();

    let out = Command::new("git")
        .current_dir(&repo)
        .args([
            "worktree",
            "add",
            worktree.to_str().unwrap(),
            "-B",
            "42-fix-bug",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "re-dispatch must succeed after teardown: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_locked_worktree_is_left_exactly_as_it_was() {
    // OneRefusalIsObeyed, against the real lock. Git refuses under a single
    // `--force` and names `remove -f -f`; dispatch takes neither that override
    // nor a delete around it.
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    git(&repo, &["worktree", "lock", worktree.to_str().unwrap()]);

    let failure = tear_down(&repo, &worktree).unwrap_err();

    assert_eq!(
        failure.worktree_left.as_deref(),
        Some(worktree.to_str().unwrap())
    );
    assert!(
        worktree.join("target/debug/huge.rlib").exists(),
        "a locked worktree must keep its contents"
    );
    assert!(
        format!("{failure:#}").contains("locked working tree"),
        "git's lock wording changed; WORKTREE_LOCKED needs updating: {failure:#}"
    );
}

#[test]
fn a_worktree_whose_admin_record_is_gone_is_deleted_rather_than_silently_kept() {
    // The leak, end to end. Git fails with WORKTREE_ALREADY_REMOVED ("is not a
    // working tree") and removes nothing; dispatch reads that as a released
    // REGISTRATION and returns Ok. Before #4882 the directory stayed forever.
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    std::fs::remove_dir_all(repo.join(".git/worktrees/42-fix-bug")).unwrap();

    tear_down(&repo, &worktree).unwrap();

    assert!(
        !worktree.exists(),
        "an unregistered worktree's directory must still be deleted"
    );
}

#[test]
fn the_already_removed_marker_still_matches_this_git_version() {
    // Pins the string the arm above turns on, independently of the outcome, so
    // a git that rewords it fails HERE rather than by quietly reverting the
    // Ok-path to a hard error.
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    std::fs::remove_dir_all(repo.join(".git/worktrees/42-fix-bug")).unwrap();

    let out = Command::new("git")
        .current_dir(&repo)
        .args(["worktree", "remove", "--force", worktree.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(!out.status.success());
    assert!(
        stderr.contains("is not a working tree"),
        "git's wording changed; dispatch's WORKTREE_ALREADY_REMOVED needs \
         updating. stderr was: {stderr}"
    );
}

#[test]
fn a_symlink_out_of_the_worktree_loses_the_link_and_keeps_the_target() {
    let (dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    let precious = dir.path().join("precious");
    std::fs::create_dir_all(&precious).unwrap();
    std::fs::write(precious.join("keep.txt"), b"keep me").unwrap();
    std::os::unix::fs::symlink(&precious, worktree.join("link")).unwrap();
    // Break the registration so git removes nothing and teardown's own delete
    // is what walks the tree — the path where the link actually matters.
    std::fs::remove_dir_all(repo.join(".git/worktrees/42-fix-bug")).unwrap();

    tear_down(&repo, &worktree).unwrap();

    assert!(!worktree.exists());
    assert!(
        precious.join("keep.txt").exists(),
        "a symlink target outside the worktree must survive"
    );
}

/// The premise `StaleAdminRecordIsPrunedBeforeWorktreeAdd` (dispatch.allium)
/// rests on, for a husk NO teardown ever saw.
///
/// `the_same_task_can_be_dispatched_again_after_the_bad_teardown` above pins
/// the same repair on the path teardown witnessed. This one removes the
/// directory the way an operator or a crashed dispatch does — straight off
/// disk, with dispatch never told — which is the case teardown's prune cannot
/// reach, and the one the 104 husks in #4877 are in. Two assertions, because
/// both halves are load-bearing: that the record really does block the add,
/// and that a repo-wide prune really does clear it.
#[test]
fn an_admin_record_left_by_no_teardown_blocks_the_add_until_pruned() {
    let (_dir, repo, worktree) = repo_with_worktree("42-fix-bug");
    std::fs::remove_dir_all(&worktree).unwrap();

    let add = || {
        try_git(
            &repo,
            &[
                "worktree",
                "add",
                worktree.to_str().unwrap(),
                "-B",
                "42-fix-bug",
            ],
        )
    };

    let blocked = add();
    let stderr = String::from_utf8_lossy(&blocked.stderr).to_string();
    assert!(
        !blocked.status.success() && stderr.contains("is already used by worktree at"),
        "a record left behind must still block re-dispatch on this git; \
         stderr was: {stderr}"
    );

    git(&repo, &["worktree", "prune"]);

    assert!(
        add().status.success(),
        "a repo-wide prune must clear the record and let the add through"
    );
}

/// The other half of the prune's safety claim: it is repo-wide, so it has to
/// be unable to reach a worktree that is still there. Git drops only records
/// whose directory is missing — a sibling task's live worktree in the same
/// repo survives.
#[test]
fn a_repo_wide_prune_leaves_a_live_sibling_worktree_alone() {
    let (_dir, repo, dead) = repo_with_worktree("42-fix-bug");
    let alive = repo.join(".worktrees").join("43-other");
    git(
        &repo,
        &["worktree", "add", alive.to_str().unwrap(), "-b", "43-other"],
    );
    std::fs::remove_dir_all(&dead).unwrap();

    git(&repo, &["worktree", "prune"]);

    assert!(alive.exists(), "the live sibling's directory must survive");
    assert!(
        repo.join(".git/worktrees/43-other").exists(),
        "the live sibling's admin record must survive"
    );
    assert!(
        !repo.join(".git/worktrees/42-fix-bug").exists(),
        "the husk's record must be gone"
    );
}
