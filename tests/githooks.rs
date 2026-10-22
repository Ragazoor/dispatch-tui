//! Regression guard for the tracked pre-push hook.
//!
//! The hook used to be a tracked `.githooks/pre-push` but was silently deleted
//! in a "save" commit (8be4b3ea), leaving only an untracked `.git/hooks/pre-push`
//! that new clones never received. This test asserts the canonical, version-
//! controlled hook exists, is executable, and runs the full check sequence — so
//! the same drift cannot recur unnoticed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

fn pre_push_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".githooks")
        .join("pre-push")
}

#[test]
fn pre_push_hook_is_tracked() {
    let path = pre_push_path();
    assert!(
        path.exists(),
        "tracked pre-push hook missing at {}",
        path.display()
    );
}

#[test]
#[cfg(unix)]
fn pre_push_hook_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let path = pre_push_path();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert!(
        mode & 0o111 != 0,
        "pre-push hook is not executable (mode {mode:o})"
    );
}

#[test]
fn pre_push_hook_runs_full_check_sequence() {
    let body = std::fs::read_to_string(pre_push_path()).unwrap();
    for needle in [
        "scripts/check-toolchain.sh",
        "scripts/test-check-toolchain.sh",
        "cargo fmt",
        "cargo clippy --all-targets -- -D warnings",
        "scripts/check-doc-paths.sh",
        "scripts/test-check-doc-paths.sh",
        "scripts/check-doc-symbols.sh",
        "scripts/test-check-doc-symbols.sh",
        "scripts/check-doc-headings.sh",
        "scripts/test-check-doc-headings.sh",
        "scripts/check-no-test-sleep.sh",
        "scripts/test-check-no-test-sleep.sh",
        "scripts/test-check-commit-msg.sh",
        "scripts/test-fetch-reviews.sh",
        "scripts/test-bots-conf.sh",
        "scripts/build-managed-module.sh",
        "scripts/check-spacetime-module.sh",
    ] {
        assert!(
            body.contains(needle),
            "pre-push hook does not invoke `{needle}`; body:\n{body}"
        );
    }
}

#[test]
fn commit_msg_hook_runs_the_checker() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".githooks")
        .join("commit-msg");
    let body = std::fs::read_to_string(&path).expect("tracked commit-msg hook missing");
    assert!(
        body.contains("scripts/check-commit-msg.sh"),
        "commit-msg hook does not invoke the checker; body:\n{body}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "commit-msg hook is not executable");
    }
}
