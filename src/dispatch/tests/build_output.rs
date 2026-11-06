//! `tasks.allium: ClearBuildOutputOfDoneTask` — what counts as build output,
//! and what clearing it removes.

use std::fs;
use std::path::Path;

use super::super::build_output::{build_output_dirs, clear_build_output, CACHEDIR_TAG_SIGNATURE};

/// Make `wt` look like a linked worktree: `.git` is a `gitdir:` file.
fn linked(wt: &Path) {
    fs::write(wt.join(".git"), b"gitdir: /somewhere\n").unwrap();
}

fn tagged_dir(worktree: &Path, name: &str) {
    let dir = worktree.join(name);
    fs::create_dir_all(dir.join("debug/deps")).unwrap();
    fs::write(
        dir.join("CACHEDIR.TAG"),
        format!("{CACHEDIR_TAG_SIGNATURE}\n# This file is a cache directory tag.\n"),
    )
    .unwrap();
    fs::write(dir.join("debug/deps/libfoo.rlib"), b"artifact").unwrap();
}

#[test]
fn a_top_level_dir_with_a_signed_cachedir_tag_is_build_output() {
    let wt = tempfile::tempdir().unwrap();
    tagged_dir(wt.path(), "target");
    tagged_dir(wt.path(), ".venv");
    fs::create_dir(wt.path().join("src")).unwrap();

    let mut dirs = build_output_dirs(wt.path());
    dirs.sort();
    assert_eq!(
        dirs,
        vec![wt.path().join(".venv"), wt.path().join("target")]
    );
}

#[test]
fn a_cachedir_tag_without_the_signature_does_not_count() {
    let wt = tempfile::tempdir().unwrap();
    let decoy = wt.path().join("notes");
    fs::create_dir(&decoy).unwrap();
    fs::write(decoy.join("CACHEDIR.TAG"), b"just a file with that name\n").unwrap();

    assert!(build_output_dirs(wt.path()).is_empty());
}

#[test]
fn a_nested_tagged_dir_is_build_output() {
    // spacetime/module-tests/target is the case that filled the disk.
    let wt = tempfile::tempdir().unwrap();
    fs::create_dir_all(wt.path().join("spacetime/module-tests")).unwrap();
    tagged_dir(&wt.path().join("spacetime/module-tests"), "target");
    tagged_dir(&wt.path().join("spacetime"), "module/target");
    tagged_dir(wt.path(), "target");

    let mut dirs = build_output_dirs(wt.path());
    dirs.sort();
    assert_eq!(
        dirs,
        vec![
            wt.path().join("spacetime/module/target"),
            wt.path().join("spacetime/module-tests/target"),
            wt.path().join("target"),
        ]
    );
}

#[test]
fn a_tag_inside_a_tagged_dir_is_not_listed_separately() {
    let wt = tempfile::tempdir().unwrap();
    tagged_dir(wt.path(), "target");
    tagged_dir(&wt.path().join("target"), "nested");

    assert_eq!(build_output_dirs(wt.path()), vec![wt.path().join("target")]);
}

#[test]
fn the_walk_does_not_enter_git_or_another_repository() {
    let wt = tempfile::tempdir().unwrap();
    linked(wt.path());
    // Something tagged under a `.git` directory.
    fs::create_dir(wt.path().join("sub")).unwrap();
    fs::create_dir(wt.path().join("sub/.git")).unwrap();
    tagged_dir(&wt.path().join("sub/.git"), "cache");
    // A nested repository (own `.git` directory) and a nested worktree (own
    // `.git` file): their build output is theirs, not this worktree's.
    let repo = wt.path().join("vendor/other-repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    tagged_dir(&repo, "target");
    let nested_wt = wt.path().join(".worktrees/1-x");
    fs::create_dir_all(&nested_wt).unwrap();
    linked(&nested_wt);
    tagged_dir(&nested_wt, "target");

    assert!(build_output_dirs(wt.path()).is_empty());
    assert!(clear_build_output(wt.path()).removed.is_empty());
    assert!(repo.join("target").exists());
    assert!(nested_wt.join("target").exists());
}

#[test]
fn a_nested_symlink_is_not_followed() {
    let outside = tempfile::tempdir().unwrap();
    tagged_dir(outside.path(), "target");
    fs::create_dir(outside.path().join("deeper")).unwrap();
    tagged_dir(&outside.path().join("deeper"), "target");
    let wt = tempfile::tempdir().unwrap();
    linked(wt.path());
    fs::create_dir(wt.path().join("crates")).unwrap();
    std::os::unix::fs::symlink(outside.path(), wt.path().join("crates/link")).unwrap();

    assert!(build_output_dirs(wt.path()).is_empty());
    assert!(clear_build_output(wt.path()).removed.is_empty());
    assert!(outside.path().join("deeper/target").exists());
}

#[test]
fn clearing_removes_nested_build_output_and_keeps_the_source() {
    let wt = tempfile::tempdir().unwrap();
    linked(wt.path());
    let krate = wt.path().join("spacetime/module-tests");
    fs::create_dir_all(krate.join("src")).unwrap();
    fs::write(krate.join("src/lib.rs"), b"").unwrap();
    tagged_dir(&krate, "target");

    let cleared = clear_build_output(wt.path());

    assert_eq!(cleared.removed, vec![krate.join("target")]);
    assert!(!krate.join("target").exists());
    assert!(krate.join("src/lib.rs").exists());
}

#[test]
fn a_symlinked_tagged_dir_is_neither_followed_nor_removed() {
    let outside = tempfile::tempdir().unwrap();
    tagged_dir(outside.path(), "target");
    let wt = tempfile::tempdir().unwrap();
    linked(wt.path());
    std::os::unix::fs::symlink(outside.path().join("target"), wt.path().join("target")).unwrap();

    assert!(build_output_dirs(wt.path()).is_empty());
    let cleared = clear_build_output(wt.path());
    assert!(cleared.removed.is_empty());
    assert!(outside
        .path()
        .join("target/debug/deps/libfoo.rlib")
        .exists());
    assert!(wt.path().join("target").symlink_metadata().is_ok());
}

#[test]
fn clearing_removes_only_the_build_output() {
    let wt = tempfile::tempdir().unwrap();
    linked(wt.path());
    tagged_dir(wt.path(), "target");
    fs::create_dir(wt.path().join("src")).unwrap();
    fs::write(wt.path().join("src/main.rs"), b"fn main() {}").unwrap();
    fs::write(wt.path().join(".git"), b"gitdir: /somewhere\n").unwrap();

    let cleared = clear_build_output(wt.path());

    assert_eq!(cleared.removed, vec![wt.path().join("target")]);
    assert!(cleared.failed.is_empty());
    assert!(!wt.path().join("target").exists());
    assert!(wt.path().join("src/main.rs").exists());
    assert!(wt.path().join(".git").exists());
}

#[test]
fn a_missing_worktree_is_nothing_to_clear() {
    let wt = tempfile::tempdir().unwrap();
    let gone = wt.path().join("no-such-worktree");

    let cleared = clear_build_output(&gone);

    assert!(cleared.removed.is_empty());
    assert!(cleared.failed.is_empty());
}

#[test]
fn a_dir_that_cannot_be_removed_is_reported_not_fatal() {
    use std::os::unix::fs::PermissionsExt;
    let wt = tempfile::tempdir().unwrap();
    linked(wt.path());
    tagged_dir(wt.path(), "target");
    tagged_dir(wt.path(), ".venv");
    // A read-only parent makes the files under it unlinkable for a non-root
    // user. Root ignores the bits, so the test only means something otherwise.
    let locked = wt.path().join("target/debug/deps");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let is_root = fs::write(locked.join("probe"), b"").is_ok();

    let cleared = clear_build_output(wt.path());

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if is_root {
        return;
    }
    assert_eq!(cleared.removed, vec![wt.path().join(".venv")]);
    assert_eq!(cleared.failed.len(), 1);
    assert_eq!(cleared.failed[0].0, wt.path().join("target"));
}

#[test]
fn a_dir_that_is_not_a_linked_worktree_is_left_alone() {
    let plain = tempfile::tempdir().unwrap();
    tagged_dir(plain.path(), ".cache");
    assert!(clear_build_output(plain.path()).removed.is_empty());
    assert!(plain.path().join(".cache").exists());

    // A repository's own checkout has a `.git` directory.
    let checkout = tempfile::tempdir().unwrap();
    fs::create_dir(checkout.path().join(".git")).unwrap();
    tagged_dir(checkout.path(), "target");
    assert!(clear_build_output(checkout.path()).removed.is_empty());
    assert!(checkout.path().join("target").exists());
}
