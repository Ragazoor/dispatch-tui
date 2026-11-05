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

fn gates_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("gates.sh")
}

fn script(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The hook and the verify command must run one shared gate, so a rebase
/// wrap-up (which never pushes) is held to what the hook holds a push to.
#[test]
fn hook_and_verify_share_one_gate_script() {
    let hook = std::fs::read_to_string(pre_push_path()).unwrap();
    assert!(
        hook.contains("scripts/gates.sh"),
        "pre-push hook does not call scripts/gates.sh; body:\n{hook}"
    );
    let verify = script("verify.sh");
    assert!(
        verify.contains("scripts/gates.sh"),
        "verify.sh does not call scripts/gates.sh; body:\n{verify}"
    );
    assert!(
        verify.contains("cargo test --no-fail-fast"),
        "verify.sh does not run the suite; body:\n{verify}"
    );
}

/// Verify checks; the hook may reformat. `cargo fmt` without `--check` in the
/// verify path would rewrite the tree during a wrap-up instead of failing.
#[test]
fn gates_script_checks_formatting_unless_asked_to_fix() {
    let gates = script("gates.sh");
    assert!(
        gates.contains("cargo fmt --check"),
        "gates.sh has no check-mode `cargo fmt --check`; body:\n{gates}"
    );
    let hook = std::fs::read_to_string(pre_push_path()).unwrap();
    assert!(
        hook.contains("--fix"),
        "pre-push hook must ask gates.sh to fix formatting; body:\n{hook}"
    );
    assert!(
        !script("verify.sh").contains("--fix"),
        "verify.sh must never reformat the tree"
    );
}

/// Every argument-less `scripts/check-*.sh` must be in the shared gate.
/// (`check-commit-msg.sh` needs a file, so it is exercised by its self-test.)
#[test]
fn gates_script_runs_every_argumentless_check() {
    let gates = script("gates.sh");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.starts_with("check-") && name.ends_with(".sh") && name != "check-commit-msg.sh" {
            assert!(
                gates.contains(&format!("scripts/{name}")),
                "scripts/{name} is not run by scripts/gates.sh"
            );
        }
    }
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
    // The sequence lives in `scripts/gates.sh`, shared with `scripts/verify.sh`.
    let body = std::fs::read_to_string(gates_path()).unwrap();
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
        "scripts/check-single-write-service.sh",
        "scripts/test-check-single-write-service.sh",
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
