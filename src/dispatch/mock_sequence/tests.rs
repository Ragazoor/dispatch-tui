
use super::*;
use crate::dispatch::tests::{make_task, make_test_repo_with_worktree, pr_review_task};
use crate::dispatch::worktree::BaseRef;
use crate::dispatch::{dispatch_agent, resume_agent, FinishError};
use crate::models::TaskId;
use crate::process::SUBPROCESS_TIMEOUT;

/// The worktree directory name `make_task`'s id and title slugify to. Every
/// reused-worktree shape needs it pre-created, which is also what puts
/// provisioning on the reuse branch.
const WORKTREE_SLUG: &str = "42-fix-bug";

/// A repo whose task worktree already exists, so provisioning reuses it and
/// the `.claude-prompt` write succeeds.
fn repo_with_worktree() -> (tempfile::TempDir, String) {
    let (dir, repo_path, _) = make_test_repo_with_worktree(WORKTREE_SLUG);
    (dir, repo_path)
}

/// A repo with no task worktree, so provisioning takes the fresh path.
fn bare_repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    (dir, path)
}

/// Drive `provision_worktree` for `base` and return what it recorded.
fn provision(
    repo_path: &str,
    base: Option<BaseRef<'_>>,
    script: &DispatchScript,
) -> Vec<(String, Vec<String>)> {
    let mock = script.runner();
    let _ = crate::dispatch::worktree::provision_worktree(
        &make_task(repo_path),
        &mock,
        &mock,
        base,
        SUBPROCESS_TIMEOUT,
    );
    mock.recorded_calls()
}

// --- the scripts describe what the code really does ---

/// The load-bearing self-test: the happy-path shape drives a real dispatch
/// end to end and the calls it recorded are exactly the declared steps. If a
/// preflight call is ever added to `provision_worktree`, this fails until the
/// step list here is updated — one place, not ~45.
#[test]
fn dispatch_script_matches_a_real_dispatch() {
    let (_dir, repo_path) = repo_with_worktree();
    let script = DispatchScript::dispatch();
    let mock = script.runner();

    dispatch_agent(
        &make_task(&repo_path),
        &mock,
        &mock,
        None,
        &Default::default(),
    )
    .unwrap();

    script.assert_matches(&mock.recorded_calls());
}

#[test]
fn resume_script_matches_a_real_resume() {
    let (dir, _repo_path) = bare_repo();
    let script = DispatchScript::resume();
    let mock = script.runner();

    resume_agent(TaskId(42), dir.path().to_str().unwrap(), &mock).unwrap();

    script.assert_matches(&mock.recorded_calls());
}

/// The reattach path: a live tmux window already answers to the
/// deterministic name, so resume_agent must not create a duplicate — it
/// should stop right after the has_window check with no new-window,
/// send-keys, or companion-pane calls at all.
#[test]
fn resume_script_matches_a_real_resume_when_window_already_alive() {
    let (dir, _repo_path) = bare_repo();
    let script = DispatchScript::resume().window_already_alive();
    let mock = script.runner();

    resume_agent(TaskId(42), dir.path().to_str().unwrap(), &mock).unwrap();

    script.assert_matches(&mock.recorded_calls());
}

#[test]
fn provision_script_matches_provision_worktree_alone() {
    let (_dir, repo_path) = repo_with_worktree();
    let script = DispatchScript::provision();
    script.assert_matches(&provision(
        &repo_path,
        Some(BaseRef::Branch("main")),
        &script,
    ));
}

#[test]
fn fresh_worktree_script_matches_a_real_provision() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision().fresh_worktree();
    script.assert_matches(&provision(
        &repo_path,
        Some(BaseRef::Branch("main")),
        &script,
    ));
}

#[test]
fn no_fetch_script_matches_a_provision_with_no_base_ref() {
    let (_dir, repo_path) = repo_with_worktree();
    let script = DispatchScript::provision().no_fetch();
    script.assert_matches(&provision(&repo_path, None, &script));
}

/// The reuse path is best-effort: one attempt, and crucially *no*
/// classification probes. That budget is what
/// `provision_worktree_reuse_does_not_retry_or_probe_an_unreachable_origin`
/// exists to defend, so the script must model it exactly.
#[test]
fn unreachable_on_the_reuse_path_is_one_attempt_and_no_probes() {
    let (_dir, repo_path) = repo_with_worktree();
    let script = DispatchScript::provision().fetch_is_unreachable();
    assert_eq!(script.attempts(), 1);
    script.assert_matches(&provision(
        &repo_path,
        Some(BaseRef::Branch("main")),
        &script,
    ));
}

/// A fresh worktree fetches under `Required`, so the probes run and the full
/// retry budget is spent before the dispatch aborts.
#[test]
fn unreachable_on_a_fresh_worktree_probes_then_spends_the_whole_budget() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_is_unreachable();
    assert_eq!(script.attempts(), FETCH_MAX_ATTEMPTS);
    let calls = provision(&repo_path, Some(BaseRef::Branch("main")), &script);
    // The script declares the abort, so this is what pins "fetch ×3 plus the
    // two probes and nothing whatsoever after".
    script.assert_matches(&calls);
    assert!(
        !calls.iter().any(|(prog, _)| prog == "tmux"),
        "an aborted dispatch must issue no tmux call at all, got: {calls:?}"
    );
}

/// A positively-identified 404 is not retried, so it costs one attempt plus
/// the probes — and then provisioning continues on the local branch.
#[test]
fn no_origin_ref_is_classified_once_and_never_retried() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_finds_no_origin_ref();
    assert_eq!(script.attempts(), 1);
    script.assert_matches(&provision(
        &repo_path,
        Some(BaseRef::Branch("main")),
        &script,
    ));
}

#[test]
fn a_retried_fetch_classifies_once_then_retries_to_success() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_succeeds_on_attempt(3);
    script.assert_matches(&provision(
        &repo_path,
        Some(BaseRef::Branch("main")),
        &script,
    ));
}

/// `BaseRef::PrHead` must never be measured against a local ref — the
/// guarantee `dispatch_pr_review_task_never_measures_the_pr_head_branch`
/// asserts. Declared as the *absence* of a step, so `assert_matches` rejects
/// a `rev-list` appearing.
#[test]
fn a_pr_head_base_declares_no_ahead_behind_step() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision()
        .fresh_worktree()
        .pr_head(PrHead::Branch("feature-x"));
    let calls = provision(&repo_path, Some(BaseRef::PrHead("feature-x")), &script);
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.contains(&"rev-list".to_string())),
        "a PR head branch must never be compared against a local ref: {calls:?}"
    );
}

/// End-to-end: a review task carrying a PR url resolves the head branch and
/// then skips the measurement, so the `gh` step is present and the `rev-list`
/// one is not. Drives the real `dispatch_agent` so the `BaseRef::PrHead`
/// construction is exercised, not just `provision_worktree`'s half of it.
#[test]
fn pr_head_script_matches_a_real_review_dispatch() {
    let (_dir, repo_path) = repo_with_worktree();
    let script = DispatchScript::dispatch().pr_head(PrHead::Branch("feature-x"));
    let mock = script.runner();

    dispatch_agent(
        &pr_review_task(&repo_path),
        &mock,
        &mock,
        None,
        &Default::default(),
    )
    .unwrap();

    script.assert_matches(&mock.recorded_calls());
}

/// A branch base *is* measured, and the reading decides the start point.
#[test]
fn a_branch_base_measures_and_prefers_local_when_ahead() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision().fresh_worktree().local_ahead(3);
    let calls = provision(&repo_path, Some(BaseRef::Branch("main")), &script);
    script.assert_matches(&calls);
    assert_eq!(
        calls[script.index_of(Step::WorktreeAdd)].1.last().unwrap(),
        "main",
        "a local branch that is ahead wins the start point: {calls:?}"
    );
}

#[test]
fn a_level_branch_base_prefers_origin() {
    let (_dir, repo_path) = bare_repo();
    let script = DispatchScript::provision().fresh_worktree();
    let calls = provision(&repo_path, Some(BaseRef::Branch("main")), &script);
    assert_eq!(
        calls[script.index_of(Step::WorktreeAdd)].1.last().unwrap(),
        "origin/main",
        "no drift means origin wins: {calls:?}"
    );
}

// --- the finish shape describes what finish_task really does ---

/// Drive `script` and hand back only the outcome — `drive_finish` has
/// already asserted the calls are exactly its declared steps.
fn assert_finish_matches(script: &DispatchScript) -> std::result::Result<(), FinishError> {
    script.drive_finish().1
}

/// The finish counterpart of `dispatch_script_matches_a_real_dispatch`, and
/// load-bearing in the same way: a preflight call added to `finish_task`
/// fails here until `Finish::steps` names it, rather than shifting an
/// unrelated test's queue by one.
#[test]
fn finish_script_matches_a_real_finish() {
    assert_finish_matches(&DispatchScript::finish()).expect("the happy path succeeds");
}

#[test]
fn finish_script_matches_a_finish_with_no_remote() {
    let (calls, result) = DispatchScript::finish().no_remote().drive_finish();
    result.expect("no remote is not an error, it just skips the pull");
    // The absence is the claim: assert_matches already rejects an extra
    // call, and this names why that one in particular must not appear.
    assert!(
        !calls.iter().any(|(_, args)| args.contains(&"pull".into())),
        "no remote means no pull: {calls:?}"
    );
}

// That a finish issues no tmux call at all is asserted where the behaviour
// lives, by `finish_task_issues_no_tmux_command` in `src/dispatch/finish.rs`.

#[test]
fn finish_script_stops_when_head_is_not_on_the_base_branch() {
    let err =
        assert_finish_matches(&DispatchScript::finish().head_branch("feature-x")).unwrap_err();
    assert!(matches!(err, FinishError::NotOnDefaultBranch { .. }));
}

#[test]
fn finish_script_stops_on_a_dirty_primary_worktree() {
    let script = DispatchScript::finish().dirty_primary(&["src/unrelated.rs"]);
    let err = assert_finish_matches(&script).unwrap_err();
    assert!(
        matches!(err, FinishError::DirtyPrimaryWorktree { ref files, .. }
                if files == &["src/unrelated.rs".to_string()]),
        "the declared dirty paths must reach the error, got: {err}"
    );
}

/// Both conflict shapes must classify as conflicts and name the same files,
/// because `is_rebase_conflict` reads both streams — a regression to reading
/// only one would still pass the other's test.
#[test]
fn finish_script_matches_a_conflicting_rebase_on_either_stream() {
    for script in [
        DispatchScript::finish().rebase_conflicts_in_stdout(&["lib.rs"]),
        DispatchScript::finish().rebase_conflicts_in_stderr(&["lib.rs"]),
    ] {
        let err = assert_finish_matches(&script).unwrap_err();
        assert!(
            matches!(err, FinishError::RebaseConflict { ref files, .. }
                    if files == &["lib.rs".to_string()]),
            "a conflict shape must name its files, got: {err}"
        );
    }
}

/// A non-conflict failure is aborted too, but reads no porcelain: there is
/// no conflict to name. Declared as the absence of `ConflictStatus`, so
/// `assert_matches` rejects one appearing.
#[test]
fn finish_script_matches_a_non_conflict_rebase_failure() {
    let script = DispatchScript::finish().rebase_fails();
    let err = assert_finish_matches(&script).unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Rebase failed")),
        "a non-conflict rebase failure is Other, got: {err}"
    );
    assert_eq!(
        script.index_of(Step::RebaseAbort),
        script.index_of(Step::Rebase) + 1,
        "the abort follows the rebase directly, with no porcelain read between"
    );
}

/// A rebase that could not be spawned is still aborted, in case it left the
/// worktree mid-rebase.
#[test]
fn finish_script_matches_a_rebase_that_could_not_be_spawned() {
    let script = DispatchScript::finish().rebase_cannot_run();
    let err = assert_finish_matches(&script).unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to run git rebase")),
        "a rebase that could not run must say so, got: {err}"
    );
    assert_eq!(*script.steps().last().unwrap(), Step::RebaseAbort);
}

#[test]
fn finish_script_matches_a_remote_probe_that_could_not_be_run() {
    let script = DispatchScript::finish().remote_probe_cannot_run();
    let err = assert_finish_matches(&script).unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains(GIT_NOT_ON_PATH)),
        "a probe that cannot run must carry why, got: {err}"
    );
    assert_eq!(*script.steps().last().unwrap(), Step::OriginProbe);
}

#[test]
fn finish_script_matches_a_failing_pull() {
    let err = assert_finish_matches(&DispatchScript::finish().pull_fails()).unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to pull")),
        "a failing pull is Other, got: {err}"
    );
}

#[test]
fn finish_script_matches_a_failing_fast_forward() {
    let err = assert_finish_matches(&DispatchScript::finish().fast_forward_fails()).unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Fast-forward failed")),
        "a failing fast-forward is Other, got: {err}"
    );
}

#[test]
fn finish_script_matches_a_head_read_that_could_not_be_run() {
    let script = DispatchScript::finish().current_branch_cannot_run();
    let err = assert_finish_matches(&script).unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to check current branch")),
        "a HEAD read that cannot run must say so, got: {err}"
    );
    assert_eq!(script.steps(), vec![Step::CurrentBranch]);
}

/// Every timing-out shape must surface as a *timeout*, which only holds
/// because the queued response is a success the bound never waits for.
#[test]
fn finish_script_times_out_each_bounded_call() {
    for (script, expected) in [
        (
            DispatchScript::finish().pull_times_out(FINISH_TIMEOUT),
            "Failed to pull",
        ),
        (
            DispatchScript::finish().rebase_times_out(FINISH_TIMEOUT),
            "Failed to run git rebase",
        ),
        (
            DispatchScript::finish().fast_forward_times_out(FINISH_TIMEOUT),
            "Failed to fast-forward",
        ),
    ] {
        let err = assert_finish_matches(&script).unwrap_err();
        assert!(
            matches!(err, FinishError::Other(ref m) if m.contains(expected) && m.contains("timed out")),
            "expected a timed-out {expected}, got: {err}"
        );
    }
}

/// The base branch reaches both the HEAD reply and the calls' argv, so a
/// non-default one needs no hand-built response.
#[test]
fn finish_script_drives_a_non_default_base_branch() {
    let script = DispatchScript::finish().base_branch("develop").no_remote();
    let (calls, result) = script.drive_finish();
    result.expect("develop is as good a base as main");
    assert!(
        calls[script.index_of(Step::Rebase)]
            .1
            .contains(&"develop".to_string()),
        "the rebase must target the declared base branch: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.iter().any(|a| a == "symbolic-ref")),
        "an explicit base branch must not be detected: {calls:?}"
    );
}

/// The finish counterpart of `index_of_shifts_with_the_optional_leading_steps`:
/// the pull is conditional, so everything after it moves by one and no call
/// site has to know that.
#[test]
fn finish_index_of_shifts_with_the_optional_pull() {
    let with_pull = DispatchScript::finish();
    assert_eq!(with_pull.index_of(Step::CurrentBranch), 0);
    assert_eq!(with_pull.index_of(Step::DirtyCheck), 1);
    assert_eq!(with_pull.index_of(Step::OriginProbe), 2);
    assert_eq!(with_pull.index_of(Step::Pull), 3);
    assert_eq!(with_pull.index_of(Step::Rebase), 4);
    assert_eq!(with_pull.index_of(Step::FastForward), 5);

    let without = DispatchScript::finish().no_remote();
    assert_eq!(without.index_of(Step::Rebase), 3);
    assert_eq!(without.index_of(Step::FastForward), 4);
}

#[test]
#[should_panic(expected = "Pull is not part of this script's sequence")]
fn finish_index_of_panics_for_a_step_this_shape_never_issues() {
    DispatchScript::finish().no_remote().index_of(Step::Pull);
}

#[test]
#[should_panic(expected = "only applies to a finish() shape")]
fn a_finish_modifier_on_a_dispatch_shape_panics() {
    let _ = DispatchScript::dispatch().no_remote();
}

/// The mirror guard. Without it a dispatch modifier on a finish shape sets a
/// field `steps()` skips past — it compiles, passes, and asserts nothing.
#[test]
#[should_panic(expected = "only applies to a dispatch/resume/provision shape")]
fn a_dispatch_modifier_on_a_finish_shape_panics() {
    let _ = DispatchScript::finish().fetch_times_out(Duration::from_millis(1));
}

/// The scope check earns its keep on `ConflictStatus` above all: a
/// mid-rebase porcelain read issued against the repo root instead of the
/// worktree returns the *repo's* status, so `RebaseConflict` would name the
/// wrong files — and argv alone cannot tell the two reads apart.
#[test]
#[should_panic(expected = "must run in the Worktree")]
fn assert_matches_rejects_a_mid_rebase_read_against_the_repo_root() {
    let script = DispatchScript::finish()
        .no_remote()
        .rebase_conflicts_in_stdout(&["lib.rs"]);
    let (mut calls, _) = script.drive_finish();
    // Re-point the mid-rebase read at the repo root, leaving argv otherwise
    // identical — exactly the drift a program+argv matcher cannot see.
    let at = script.index_of(Step::ConflictStatus);
    let dash_c = calls[at].1.iter().position(|a| a == "-C").unwrap();
    calls[at].1[dash_c + 1] = FINISH_REPO.to_string();
    script.assert_matches(&calls);
}

/// And the other direction: the preflight dirty check belongs to the repo
/// root, not the worktree.
#[test]
#[should_panic(expected = "must run in the Repo")]
fn assert_matches_rejects_a_dirty_check_against_the_worktree() {
    let script = DispatchScript::finish().no_remote();
    let (mut calls, _) = script.drive_finish();
    let at = script.index_of(Step::DirtyCheck);
    let dash_c = calls[at].1.iter().position(|a| a == "-C").unwrap();
    calls[at].1[dash_c + 1] = FINISH_WORKTREE.to_string();
    script.assert_matches(&calls);
}

#[test]
#[should_panic(expected = "a finish() shape states its failures directly")]
fn fails_at_rejects_a_finish_shape() {
    let _ = DispatchScript::finish().fails_at(Step::Rebase);
}

// --- step bookkeeping ---

#[test]
fn index_of_reports_the_recorded_call_position() {
    let script = DispatchScript::dispatch();
    assert_eq!(script.index_of(Step::Fetch), 0);
    assert_eq!(script.index_of(Step::AheadBehind), 1);
    assert_eq!(script.index_of(Step::ReusedBranchDrift), 2);
    assert_eq!(script.index_of(Step::NewWindowNameCheck), 3);
    assert_eq!(script.index_of(Step::NewWindow), 4);
    assert_eq!(script.index_of(Step::SendKeysLiteral), 7);
    assert_eq!(script.index_of(Step::CompanionSplit), 9);
}

/// The whole point of deriving indices: optional steps ahead of the one being
/// asserted on shift it, and no call site has to know by how much.
#[test]
fn index_of_shifts_with_the_optional_leading_steps() {
    let script = DispatchScript::dispatch()
        .detecting_default_branch("main")
        .fresh_worktree();
    assert_eq!(script.index_of(Step::DetectDefaultBranch), 0);
    assert_eq!(script.index_of(Step::Fetch), 1);
    assert_eq!(script.index_of(Step::AheadBehind), 2);
    assert_eq!(script.index_of(Step::WorktreePrune), 3);
    assert_eq!(script.index_of(Step::WorktreeAdd), 4);
    assert_eq!(script.index_of(Step::SendKeysLiteral), 9);
}

/// The retried-fetch case is exactly what a hand-written queue got wrong: the
/// probes land between attempt 1 and attempt 2, so everything after moves by
/// four rather than by the two the retries alone suggest.
#[test]
fn index_of_accounts_for_every_attempt_and_both_probes() {
    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_succeeds_on_attempt(3);
    assert_eq!(script.index_of(Step::Fetch), 0, "the first attempt");
    assert_eq!(script.index_of(Step::OriginProbe), 1);
    assert_eq!(script.index_of(Step::LsRemote), 2);
    // attempts 2 and 3 occupy 3 and 4
    assert_eq!(script.index_of(Step::AheadBehind), 5);
    assert_eq!(script.index_of(Step::WorktreePrune), 6);
    assert_eq!(script.index_of(Step::WorktreeAdd), 7);
}

#[test]
#[should_panic(expected = "WorktreeAdd is not part of this script's sequence")]
fn index_of_panics_for_a_step_this_shape_never_issues() {
    DispatchScript::dispatch().index_of(Step::WorktreeAdd);
}

#[test]
fn stops_after_queues_nothing_beyond_the_named_step() {
    let script = DispatchScript::dispatch().stops_after(Step::SetSplitHook);
    assert_eq!(
        script.responses().len(),
        script.index_of(Step::SetSplitHook) + 1
    );
}

#[test]
fn fails_at_queues_a_failure_as_the_last_response() {
    let script = DispatchScript::dispatch().fails_at(Step::CompanionSplit);
    let responses = script.responses();
    assert_eq!(responses.len(), script.index_of(Step::CompanionSplit) + 1);
    let (_delay, last) = responses.last().unwrap();
    assert!(
        !last.as_ref().unwrap().status.success(),
        "the named step must fail"
    );
}

#[test]
#[should_panic(expected = "a failing fetch is retried and classified")]
fn fails_at_rejects_the_retried_fetch() {
    let _ = DispatchScript::dispatch().fails_at(Step::Fetch);
}

/// The timeout shape is a *success* held back past the deadline, not a
/// failure — the distinction the watchdog test depends on.
#[test]
fn fetch_times_out_queues_delayed_successes() {
    let delay = Duration::from_millis(100);
    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_times_out(delay);
    let (got_delay, response) = script.responses().into_iter().next().unwrap();
    assert_eq!(got_delay, Some(delay));
    assert!(
        response.unwrap().status.success(),
        "a timeout shape must queue successes; only the delay defeats them"
    );
}

// --- assert_matches really discriminates ---

#[test]
#[should_panic(expected = "unexpected extra call")]
fn assert_matches_rejects_an_extra_call() {
    let mut calls = recorded_resume_calls();
    calls.push(("git".to_string(), vec!["rev-list".to_string()]));
    DispatchScript::resume().assert_matches(&calls);
}

#[test]
#[should_panic(expected = "missing CompanionRoleMark")]
fn assert_matches_rejects_a_missing_call() {
    let mut calls = recorded_resume_calls();
    calls.pop();
    DispatchScript::resume().assert_matches(&calls);
}

#[test]
#[should_panic(expected = "should be HasWindowQuery")]
fn assert_matches_rejects_a_reordered_call() {
    let mut calls = recorded_resume_calls();
    // Against call 2 (`new-window`), not call 1: since `tmux::new_window`
    // gained its own duplicate-name guard, calls 0 and 1 are both
    // `list-windows` and swapping them is not a reordering at all.
    calls.swap(0, 2);
    DispatchScript::resume().assert_matches(&calls);
}

/// A real resume's calls, as raw material for the rejection tests above.
fn recorded_resume_calls() -> Vec<(String, Vec<String>)> {
    let (dir, _repo_path) = bare_repo();
    let mock = DispatchScript::resume().runner();
    resume_agent(TaskId(42), dir.path().to_str().unwrap(), &mock).unwrap();
    mock.recorded_calls()
}

// The response formatters need no tests of their own: a malformed
// `default_branch_ref` makes `exec_quick_dispatch_sets_base_branch_to_repo_default`
// (src/runtime/tests.rs) resolve the wrong branch, a malformed `pr_view_reply`
// makes `dispatch_pr_review_task_bases_worktree_on_pr_head_branch` fall back to
// the base branch, and a malformed `rev_list_counts` is caught by the two
// start-point tests above. All three already fail on the real parse.
