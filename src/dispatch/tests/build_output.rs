//! `tasks.allium: ClearBuildOutputOfDoneTask` — what counts as build output,
//! and what clearing it removes.

use std::fs;
use std::path::Path;

use super::super::build_output::{build_output_dirs, clear_build_output, CACHEDIR_TAG_SIGNATURE};

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
fn a_nested_tagged_dir_is_not_examined() {
    let wt = tempfile::tempdir().unwrap();
    fs::create_dir(wt.path().join("crates")).unwrap();
    tagged_dir(&wt.path().join("crates"), "target");

    assert!(build_output_dirs(wt.path()).is_empty());
}

#[test]
fn a_symlinked_tagged_dir_is_neither_followed_nor_removed() {
    let outside = tempfile::tempdir().unwrap();
    tagged_dir(outside.path(), "target");
    let wt = tempfile::tempdir().unwrap();
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
