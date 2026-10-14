//! `ReusedBranchWithNoOwnWorkIsFastForwarded` (docs/specs/dispatch.allium).

use super::*;

fn dispatch_reusing(script: &DispatchScript) -> (MockProcessRunner, String) {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = script.runner();
    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, &mock, None, &LearningInjections::default()).unwrap();
    let worktree = worktree_dir.to_str().unwrap().to_string();
    (mock, worktree)
}

fn assert_no_fast_forward(calls: &[(String, Vec<String>)], why: &str) {
    assert!(
        calls.iter().all(|c| !has_arg(c, "--ff-only")),
        "{why}: {calls:?}"
    );
}

fn has_arg(call: &(String, Vec<String>), needle: &str) -> bool {
    call.1.iter().any(|a| a == needle)
}

#[test]
fn a_reused_branch_behind_the_base_with_no_own_commits_is_fast_forwarded() {
    let script = DispatchScript::dispatch().reused_branch_behind(60);
    let (mock, worktree) = dispatch_reusing(&script);
    let calls = mock.recorded_calls();

    script.assert_matches(&calls);
    let drift = &calls[script.index_of(Step::ReusedBranchDrift)];
    assert!(has_arg(drift, "origin/main...HEAD"), "got: {drift:?}");
    assert!(
        has_arg(drift, &worktree),
        "measured in the worktree: {drift:?}"
    );
    let ff = &calls[script.index_of(Step::ReusedBranchFastForward)];
    assert!(has_arg(ff, "origin/main"), "got: {ff:?}");
    assert!(has_arg(ff, &worktree), "run in the worktree: {ff:?}");
}

#[test]
fn a_reused_branch_is_fast_forwarded_to_local_base_when_local_is_ahead() {
    let script = DispatchScript::dispatch()
        .local_ahead(3)
        .reused_branch_behind(5);
    let (mock, _) = dispatch_reusing(&script);
    let calls = mock.recorded_calls();

    script.assert_matches(&calls);
    let ff = &calls[script.index_of(Step::ReusedBranchFastForward)];
    assert!(has_arg(ff, "main"), "got: {ff:?}");
    assert!(!has_arg(ff, "origin/main"), "got: {ff:?}");
}

#[test]
fn a_reused_branch_level_with_the_base_is_left_alone() {
    let script = DispatchScript::dispatch();
    let (mock, _) = dispatch_reusing(&script);
    let calls = mock.recorded_calls();

    script.assert_matches(&calls);
    assert_no_fast_forward(&calls, "nothing to move");
}

#[test]
fn a_reused_branch_with_own_commits_is_not_moved() {
    let script = DispatchScript::dispatch().reused_branch_with_own_commits(2);
    let (mock, _) = dispatch_reusing(&script);
    let calls = mock.recorded_calls();

    script.assert_matches(&calls);
    assert_no_fast_forward(&calls, "a branch with work on it must not be moved");
}

#[test]
fn a_refused_fast_forward_does_not_abort_the_dispatch() {
    let script = DispatchScript::dispatch().reused_branch_behind(4);
    let ff_index = script.index_of(Step::ReusedBranchFastForward);
    let mut responses = script.responses();
    responses[ff_index].1 =
        MockProcessRunner::fail("error: Your local changes would be overwritten");
    let mock = MockProcessRunner::new_with_delays(responses);

    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, &mock, None, &LearningInjections::default());

    assert!(result.is_ok(), "best-effort, got: {:?}", result.err());
    script.assert_matches(&mock.recorded_calls());
}

#[test]
fn an_unreadable_drift_count_does_not_abort_the_dispatch() {
    let script = DispatchScript::dispatch().reused_branch_behind(4);
    let drift_index = script.index_of(Step::ReusedBranchDrift);
    let mut responses = script.responses();
    responses[drift_index].1 = MockProcessRunner::fail("fatal: bad revision");
    // The merge never runs, so its response is not consumed.
    let _ = responses.remove(script.index_of(Step::ReusedBranchFastForward));
    let mock = MockProcessRunner::new_with_delays(responses);

    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, &mock, None, &LearningInjections::default());

    assert!(result.is_ok(), "best-effort, got: {:?}", result.err());
    assert!(
        mock.recorded_calls()
            .iter()
            .all(|c| !has_arg(c, "--ff-only")),
        "no count, no move"
    );
}
