use super::*;

// --- finish_task tests ---

/// Drive `finish_task` under `script`, which declares the base branch, the
/// paths and the whole subprocess sequence — and asserts the recorded calls are
/// exactly that sequence. See `docs/conventions.md`, "Driving a dispatch:
/// `DispatchScript`, never a hand-written queue"; the same rule holds for a
/// finish.
pub(in crate::dispatch) fn run_finish(script: &DispatchScript) -> FinishRun {
    script.drive_finish()
}

#[test]
fn finish_task_happy_path() {
    let (calls, result) = run_finish(&DispatchScript::finish());

    result.unwrap();
    assert!(calls.iter().any(|c| c.1.contains(&"rebase".to_string())));
    assert!(calls.iter().any(|c| c.1.contains(&"--ff-only".to_string())));
    // No worktree removal
    assert!(!calls.iter().any(|c| c.1.contains(&"remove".to_string())));
}

#[test]
fn finish_task_with_master_default_branch() {
    let script = DispatchScript::finish().base_branch("master");
    let (calls, result) = run_finish(&script);

    result.unwrap();
    // pull and rebase should both reference "master", not "main"
    for step in [Step::Pull, Step::Rebase] {
        assert!(
            calls[script.index_of(step)]
                .1
                .contains(&"master".to_string()),
            "{step:?} should target master, got: {calls:?}"
        );
    }
}

#[test]
fn finish_task_not_on_default_branch() {
    let (_calls, result) = run_finish(&DispatchScript::finish().head_branch("feature-branch"));

    let err = result.unwrap_err();
    assert!(matches!(err, FinishError::NotOnDefaultBranch { .. }));
    assert!(err.to_string().contains("feature-branch"));
}

#[test]
fn finish_task_rebase_conflict() {
    let script = DispatchScript::finish()
        .no_remote()
        .rebase_conflicts_in_stderr(&["src/main.rs"]);
    let (calls, result) = run_finish(&script);

    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            FinishError::RebaseConflict { ref files, .. } if files == &["src/main.rs".to_string()]
        ),
        "expected RebaseConflict naming src/main.rs, got: {err}"
    );
    assert!(calls.last().unwrap().1.contains(&"--abort".to_string()));
}

#[test]
fn finish_task_pull_fails() {
    let (_calls, result) = run_finish(&DispatchScript::finish().pull_fails());
    assert!(matches!(result.unwrap_err(), FinishError::Other(_)));
}

#[test]
fn finish_task_dirty_primary_worktree_returns_error_before_pull() {
    let (calls, result) =
        run_finish(&DispatchScript::finish().dirty_primary(&["src/unrelated.rs"]));

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::DirtyPrimaryWorktree { ref path, ref files }
            if path == "/repo" && files == &["src/unrelated.rs".to_string()]),
        "expected DirtyPrimaryWorktree naming /repo and src/unrelated.rs, got: {err}"
    );

    assert!(
        !calls.iter().any(|c| c.1.contains(&"pull".to_string())
            || c.1.contains(&"rebase".to_string())
            || c.1.contains(&"--ff-only".to_string())),
        "a dirty primary worktree must be reported before any pull/rebase/merge is attempted, got: {calls:?}"
    );
}

// --- dispatch guard tests ---

#[test]
fn dispatch_agent_fails_fast_with_empty_repo_path() {
    let mock = MockProcessRunner::new(vec![]);
    let mut task = make_task("/some/repo");
    task.repo_path = "".to_string();
    let result = dispatch_agent(&task, &mock, &mock, None, &LearningInjections::default());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Repository path"),
        "error should mention 'Repository path', got: {msg}"
    );
}

// --- check_pr_status tests ---

fn raw(state: &str, review_decision: &str) -> RawPrStatus {
    RawPrStatus {
        state: state.into(),
        review_decision: review_decision.into(),
    }
}

#[test]
fn check_pr_status_returns_gh_output_raw() {
    for (stdout, expected) in [
        (
            &b"OPEN\nREVIEW_REQUIRED\n"[..],
            raw("OPEN", "REVIEW_REQUIRED"),
        ),
        (b"MERGED\n", raw("MERGED", "")),
        (b"CLOSED\n", raw("CLOSED", "")),
        (b"OPEN\nAPPROVED\n", raw("OPEN", "APPROVED")),
        (
            b"OPEN\nCHANGES_REQUESTED\n",
            raw("OPEN", "CHANGES_REQUESTED"),
        ),
        // Not interpreted here: the module decides what a state means.
        (b"draft\n", raw("draft", "")),
    ] {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(stdout)]);
        let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
        assert_eq!(result, expected);
    }
}

#[test]
fn finish_task_no_remote_skips_pull() {
    let (calls, result) = run_finish(&DispatchScript::finish().no_remote());

    result.unwrap();
    // Should not have a "pull" call
    assert!(!calls.iter().any(|c| c.1.contains(&"pull".to_string())));
}

// --- new TDD tests for explicit base_branch ---

#[test]
fn finish_task_uses_explicit_base_branch_not_auto_detected() {
    // "develop" is passed explicitly; no symbolic-ref (detect_default_branch) call
    let script = DispatchScript::finish().base_branch("develop").no_remote();
    let (calls, result) = run_finish(&script);

    result.unwrap();
    // No symbolic-ref call — branch was provided explicitly
    assert!(
        !calls
            .iter()
            .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
        "symbolic-ref must not be called when base_branch is explicit"
    );
    // Rebase should target "develop"
    assert!(calls[script.index_of(Step::Rebase)]
        .1
        .contains(&"develop".to_string()));
}

#[test]
fn dispatch_agent_uses_task_base_branch_in_prompt() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    // No detect_default_branch call expected — task.base_branch is used directly
    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let mut task = make_task(&repo_path);
    task.base_branch = "develop".into();
    dispatch_agent(&task, &mock, &mock, None, &LearningInjections::default()).unwrap();

    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("git rebase origin/develop"),
        "prompt should reference task.base_branch (develop), got: {prompt}"
    );
    // No symbolic-ref call
    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
        "dispatch_agent must not call symbolic-ref when task.base_branch is set"
    );
}
