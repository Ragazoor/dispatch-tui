use super::*;

// --- PR-based review worktree start point ---
//
// The mock runner doesn't actually create the worktree dir, so the post-provision
// `.claude-prompt` write fails for a fresh repo. Start-point tests use a fresh
// repo and inspect the recorded `git` calls (captured before the write fails) —
// they tolerate the resulting error. The prompt-content test pre-creates the
// worktree dir so the write succeeds.

pub(in crate::dispatch) fn pr_review_task(repo_path: &str) -> Task {
    let mut task = make_task(repo_path);
    task.tag = Some(crate::models::TaskTag::PrReview);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/7",
        crate::models::UrlType::Pr,
    ));
    task
}

/// The `git worktree add` start point (its last arg), from the recorded calls.
fn worktree_add_start_point(calls: &[(String, Vec<String>)]) -> String {
    worktree_add_call(calls)
        .1
        .last()
        .expect("start point arg")
        .clone()
}

#[test]
fn dispatch_pr_review_task_bases_worktree_on_pr_head_branch() {
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"feature-x\nfalse\n"), // gh pr view
        MockProcessRunner::ok(),                                  // git fetch origin feature-x
        MockProcessRunner::ok(),                                  // git worktree prune
        MockProcessRunner::ok(), // git worktree add origin/feature-x
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux list-windows (rollback's window-kill check)
        MockProcessRunner::ok(), // git worktree remove --force (fresh-worktree rollback)
        MockProcessRunner::ok(), // git branch -D (fresh-worktree rollback)
    ]);

    let task = pr_review_task(&repo_path);
    // Prompt write fails (mock didn't create the worktree dir) — that's fine, the
    // git calls we assert on were recorded during provisioning beforehand. The
    // failure then rolls the fresh worktree back, hence the two trailing
    // responses above.
    let _ = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    let calls = mock.recorded_calls();
    assert_eq!(
        calls[0].0, "gh",
        "first call should resolve the PR head branch"
    );
    assert_eq!(
        worktree_add_start_point(&calls),
        "origin/feature-x",
        "worktree should start from the PR head branch"
    );
}

#[test]
fn dispatch_pr_review_task_never_measures_the_pr_head_branch() {
    // End-to-end version of `provision_worktree_never_measures_a_pr_head_branch`:
    // dispatch_agent on a pr-review task with a PR URL must construct
    // `BaseRef::PrHead`, not `BaseRef::Branch`, so no ahead/behind comparison
    // (git rev-list) ever runs against the PR's head branch.
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"feature-x\nfalse\n"), // gh pr view
        MockProcessRunner::ok(),                                  // git fetch origin feature-x
        MockProcessRunner::ok(),                                  // git worktree prune
        MockProcessRunner::ok(),                                  // git worktree add
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux list-windows (rollback's window-kill check)
        MockProcessRunner::ok(), // git worktree remove --force (fresh-worktree rollback)
        MockProcessRunner::ok(), // git branch -D (fresh-worktree rollback)
    ]);

    let task = pr_review_task(&repo_path);
    // Prompt write fails (mock didn't create the worktree dir) — that's fine, the
    // calls we assert on were recorded during provisioning beforehand. The
    // failure then rolls the fresh worktree back, hence the two trailing
    // responses above.
    let _ = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.contains(&"rev-list".to_string())),
        "a PR-review dispatch must never compare the PR head branch against a local ref: {calls:?}"
    );
}

#[test]
fn dispatch_pr_review_task_prompt_rebases_onto_pr_branch() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    // A PR head base is never measured, so the script declares no rev-list —
    // the entry this vector used to carry was stale, silently absorbed by the
    // next call because new-window ignores stdout.
    let mock = DispatchScript::dispatch()
        .pr_head(PrHead::Branch("feature-x"))
        .runner();

    let task = pr_review_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let prompt =
        std::fs::read_to_string(format!("{repo_path}/.worktrees/42-fix-bug/.claude-prompt"))
            .expect("prompt file written");
    assert!(
        prompt.contains("git rebase origin/feature-x"),
        "prompt should rebase onto the PR branch, got: {prompt}"
    );
    assert!(
        !prompt.contains("git rebase main"),
        "PR review prompt must not rebase onto the base branch, got: {prompt}"
    );
}

#[test]
fn dispatch_prompt_includes_fetch_warning_when_fetch_fails() {
    // Pre-create the worktree dir so `git worktree add` is skipped (its
    // mocked response would otherwise not actually create the directory the
    // real implementation later writes `.claude-prompt` into).
    //
    // That pre-creation also puts this test on the REUSE path, so the fetch is
    // best-effort: one attempt, no `remote get-url`/`ls-remote` classification
    // probe, and a failure that warns instead of aborting. What is under test
    // either way is the threading — a fetch warning must reach the agent's own
    // prompt as a `Note:`, not just a server-side log line.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    // The reuse path is best-effort, so the script declares one attempt and no
    // classification probes — the budget the sibling test defends.
    let mock = DispatchScript::dispatch().fetch_is_unreachable().runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let prompt =
        std::fs::read_to_string(format!("{repo_path}/.worktrees/42-fix-bug/.claude-prompt"))
            .expect("prompt file written");
    assert!(
        prompt.contains("origin/main"),
        "prompt should mention the base branch that could not be fetched, got: {prompt}"
    );
    assert!(
        prompt.contains("Note:"),
        "fetch warning should be a clearly-marked note, got: {prompt}"
    );
}

/// The reuse fact provisioning establishes has to survive the trip back to the
/// caller: `dispatch_task`'s success text reads it off this result — see
/// rule-guidance.DispatchTaskViaMcp in docs/specs/mcp-task-tools.allium.
#[test]
fn dispatch_agent_carries_the_reuse_flag_into_its_result() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = DispatchScript::dispatch().runner();

    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    assert!(
        result.reused_worktree,
        "a dispatch into a pre-existing worktree directory must report the reuse"
    );
}

#[test]
fn dispatch_prompt_has_no_warning_when_fetch_succeeds() {
    // Pre-create the worktree dir — see comment in the sibling test above.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let prompt =
        std::fs::read_to_string(format!("{repo_path}/.worktrees/42-fix-bug/.claude-prompt"))
            .expect("prompt file written");
    assert!(
        !prompt.contains("Note:"),
        "no fetch warning expected when fetch succeeds, got: {prompt}"
    );
}

#[test]
fn dispatch_non_review_task_skips_gh_and_bases_worktree_on_origin() {
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::dispatch().fresh_worktree();
    let mock = script.runner();

    // make_task has tag None / url None — a plain implementation task.
    let task = make_task(&repo_path);
    let _ = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    let calls = mock.recorded_calls();
    assert!(
        calls.iter().all(|(prog, _)| prog != "gh"),
        "non-review task must not call gh"
    );
    assert_eq!(worktree_add_start_point(&calls), "origin/main");
}

#[test]
fn dispatch_review_task_pr_resolution_failure_falls_back_to_base() {
    let (_dir, repo_path) = make_test_repo();

    // An unresolvable PR leaves the dispatch on `BaseRef::Branch`, so the
    // ahead/behind measurement *does* run — unlike the PrHead::Branch shapes.
    let script = DispatchScript::dispatch()
        .pr_head(PrHead::Unresolvable)
        .fresh_worktree();
    let mock = script.runner();

    let task = pr_review_task(&repo_path);
    let _ = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    assert_eq!(
        worktree_add_start_point(&mock.recorded_calls()),
        "origin/main",
        "should fall back to the base branch when PR resolution fails"
    );
}

#[test]
fn dispatch_review_task_fork_pr_falls_back_to_base() {
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::dispatch()
        .pr_head(PrHead::Fork("patch-1"))
        .fresh_worktree();
    let mock = script.runner();

    let task = pr_review_task(&repo_path);
    let _ = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    assert_eq!(
        worktree_add_start_point(&mock.recorded_calls()),
        "origin/main",
        "fork PR should fall back to the base branch"
    );
}

#[test]
fn provision_worktree_never_measures_a_pr_head_branch() {
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git fetch origin feature-x
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git worktree add origin/feature-x
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option
        MockProcessRunner::ok(), // tmux set-hook
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::PrHead("feature-x")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.contains(&"rev-list".to_string())),
        "a PR head branch must never be compared against a local ref: {calls:?}"
    );
    assert_eq!(
        result.start_point,
        Some(StartPoint::Remote {
            base: "feature-x".to_string()
        })
    );
}

#[test]
fn provision_worktree_creates_new_when_dir_missing() {
    let (_dir, repo_path) = make_test_repo();
    // Do NOT pre-create the worktree dir — test the "create" path

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git worktree add
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].0, "git", "first call should be git worktree prune");
    assert!(calls[0].1.contains(&"prune".to_string()));
    assert_eq!(calls[1].0, "git", "then the add the prune unblocks");
    assert!(calls[1].1.contains(&"add".to_string()));
    // Call 2 is `new_window`'s own duplicate-name `list-windows` query.
    assert_eq!(calls[3].0, "tmux");
    assert_eq!(calls[3].1[0], "new-window");

    let expected_path = format!("{repo_path}/.worktrees/42-fix-bug");
    assert_eq!(result.worktree_path, expected_path);
}

/// The premise `WorktreeIsNeverShared` rests on: the derived path is injective
/// in the task id, so two tasks can never name the same worktree.
///
/// This is the load-bearing assertion for that invariant in
/// docs/specs/tasks.allium. The surviving tripwire elsewhere
/// (`src/runtime/tests/task_exec.rs::exec_cleanup_tears_down_even_if_another_row_names_the_worktree`)
/// only asserts that consumers do not *check* for sharing; it would stay
/// green if the id prefix were dropped here. This one fails instead — pick the
/// worst case, two tasks whose titles slugify identically, so the id is the only
/// thing keeping the paths apart.
#[test]
fn provision_worktree_path_is_unique_per_task_id_even_for_identical_titles() {
    let (_dir, repo_path) = make_test_repo();

    let derive = |id: i64| {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // git worktree prune
            MockProcessRunner::ok(), // git worktree add
            MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
            MockProcessRunner::ok(), // tmux new-window
            MockProcessRunner::ok(), // tmux set-option @dispatch_dir
            MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        ]);
        let mut task = make_task(&repo_path);
        task.id = TaskId(id);
        task.title = "Same title".to_string();
        provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT)
            .unwrap()
            .worktree_path
    };

    let first = derive(7);
    let second = derive(8);

    assert_eq!(first, format!("{repo_path}/.worktrees/7-same-title"));
    assert_eq!(second, format!("{repo_path}/.worktrees/8-same-title"));
    assert_ne!(
        first, second,
        "identical titles must still yield distinct worktrees — the task id is \
         what makes the path injective, and WorktreeIsNeverShared depends on it"
    );
}

#[test]
fn provision_worktree_skips_git_when_dir_exists() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls.iter().all(|(prog, _)| prog != "git"),
        "git should be skipped"
    );
    // Call 0 is `new_window`'s own duplicate-name `list-windows` query.
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(calls[1].1[0], "new-window");
    assert_eq!(result.worktree_path, worktree_dir.to_str().unwrap());
}

#[test]
fn provision_worktree_reports_reused_worktree_false_when_dir_missing() {
    let (_dir, repo_path) = make_test_repo();
    // Do NOT pre-create the worktree dir — the "fresh" path.

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git worktree add
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap();

    assert!(
        !result.reused_worktree,
        "a freshly created worktree directory must report reused_worktree == false"
    );
}

#[test]
fn provision_worktree_reports_reused_worktree_true_when_dir_exists() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap();

    assert!(
        result.reused_worktree,
        "a pre-existing worktree directory must report reused_worktree == true"
    );
    // The reuse path skips `git worktree add` entirely, so it has no record
    // conflict to clear — and it exists partly to stay cheap when the network
    // is down, which another bounded subprocess would undo.
    assert!(
        mock.recorded_calls().iter().all(|(prog, _)| prog != "git"),
        "the reuse path must issue no git at all, prune included: {:?}",
        mock.recorded_calls()
    );
}

/// `StaleAdminRecordIsPrunedBeforeWorktreeAdd` (docs/specs/dispatch.allium).
///
/// The record `git worktree add` trips over is left by causes teardown never
/// sees — an operator's `rm -rf`, a crashed dispatch, the 104 husks removed by
/// hand in #4877 — so the prune has to sit on the path that suffers, not only
/// on the one that made the mess. Adjacency is the assertion: a prune anywhere
/// earlier would still be repo-wide, but a prune AFTER the add clears nothing
/// the add needed.
#[test]
fn provision_worktree_prunes_stale_admin_records_immediately_before_creating_it() {
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::provision().fresh_worktree();
    let mock = script.runner();

    let task = make_task(&repo_path);
    provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    script.assert_matches(&calls);

    let prune = script.index_of(Step::WorktreePrune);
    assert_eq!(
        script.index_of(Step::WorktreeAdd),
        prune + 1,
        "the prune must be the call immediately before the add it unblocks"
    );
    assert_eq!(calls[prune].0, "git");
    assert!(
        calls[prune].1.contains(&"-C".to_string())
            && calls[prune].1.contains(&repo_path.to_string()),
        "the prune is repo-wide and runs in the repo root, not the worktree: {:?}",
        calls[prune].1
    );
}

/// Best-effort means best-effort: the add that follows is the step whose
/// success decides the dispatch, and its error is the one worth reporting.
#[test]
fn provision_worktree_continues_when_the_prune_fails() {
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: not a git repository"), // git worktree prune
        MockProcessRunner::ok(),                                // git worktree add
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap();

    assert!(!result.reused_worktree);
    let calls = mock.recorded_calls();
    assert!(
        calls[0].1.contains(&"prune".to_string()),
        "the failing call must be the prune: {calls:?}"
    );
    assert!(
        calls[1].1.contains(&"add".to_string()),
        "the add must still run after the prune failed: {calls:?}"
    );
}

/// `WorktreeExistenceIsOneQuestion` (docs/specs/dispatch.allium), the third
/// stat answer. An unreadable path is neither present nor absent, and every
/// onward choice turns on knowing which — including whether the
/// provisioning-failure rollback may delete it. Refusing is the only answer
/// that is wrong in neither direction.
#[test]
fn provision_worktree_aborts_when_the_worktree_path_cannot_be_inspected() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, repo_path) = make_test_repo();
    let worktrees_root = std::path::Path::new(&repo_path).join(".worktrees");
    std::fs::create_dir_all(&worktrees_root).unwrap();
    std::fs::set_permissions(&worktrees_root, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Root ignores the mode bits, so the precondition this test needs simply
    // does not exist there. Check rather than assume: a silently-passing
    // assertion is worse than a skipped one.
    let unreadable = std::fs::symlink_metadata(worktrees_root.join("42-fix-bug"))
        .is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound);
    if !unreadable {
        std::fs::set_permissions(&worktrees_root, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let mock = MockProcessRunner::new(vec![]);
    let task = make_task(&repo_path);
    let err = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap_err();

    std::fs::set_permissions(&worktrees_root, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        format!("{err:#}").contains("42-fix-bug"),
        "the abort must name the path it could not inspect: {err:#}"
    );
    assert!(
        mock.recorded_calls().is_empty(),
        "nothing on disk may be touched when the answer is unknown: {:?}",
        mock.recorded_calls()
    );
}

/// `PresenceIsNotYetReuse` (docs/specs/dispatch.allium): presence settles what
/// may be DELETED, not what may be worked in.
///
/// The abort is what the second question is for. Reuse is nothing but skipping
/// `git worktree add`, so an unusable path produces no downstream error of its
/// own: tmux accepts an unresolvable `-c` and starts the pane in the
/// operator's HOME, exiting 0 (probed against tmux 3.7c). Without this check
/// the dispatch SUCCEEDS with the agent outside any worktree.
///
/// Taking the fresh path instead is not the alternative it looks like: the add
/// would fail on the occupied path, but only after declaring it fresh — and a
/// fresh path is one the rollback may delete.
///
/// This is a dangling link, so it hits `worktree_is_reusable`'s UNREACHABLE
/// arm (following the link fails) rather than its "present but not a
/// directory" arm — a symlink loop or an unreadable chain would land here
/// too, and the io error is what tells the two apart. The clause promises
/// that error is carried, not just the fixed "not a usable worktree
/// directory" text, so it is asserted here rather than merely implied.
#[test]
fn provision_worktree_aborts_on_a_dangling_symlink_at_the_path() {
    let (_dir, repo_path) = make_test_repo();
    let worktrees_root = std::path::Path::new(&repo_path).join(".worktrees");
    std::fs::create_dir_all(&worktrees_root).unwrap();
    let dangling = worktrees_root.join("42-fix-bug");
    std::os::unix::fs::symlink(worktrees_root.join("nowhere-at-all"), &dangling).unwrap();

    let mock = MockProcessRunner::new(vec![]);
    let task = make_task(&repo_path);
    let err = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap_err();
    let rendered = format!("{err:#}");

    assert!(
        rendered.contains("42-fix-bug"),
        "the abort must name the path it refused: {rendered}"
    );
    assert!(
        rendered.to_lowercase().contains("no such file"),
        "an unreachable target must carry its own error, not just \
         \"not a usable worktree directory\": {rendered}"
    );
    assert!(
        mock.recorded_calls().is_empty(),
        "nothing may be touched — no add, and no window for tmux to start in \
         the operator's home: {:?}",
        mock.recorded_calls()
    );
    assert!(
        std::fs::symlink_metadata(&dangling).is_ok(),
        "the link must survive — nothing here created it, so nothing here removes it"
    );
}

/// Same clause, the other unusable shape: a plain file where the worktree
/// should be. This is `worktree_is_reusable`'s OTHER failure arm — the target
/// resolves fine, it is just not a directory — so there is no io error to
/// carry, unlike the dangling-link case above.
#[test]
fn provision_worktree_aborts_on_a_regular_file_at_the_worktree_path() {
    let (_dir, repo_path) = make_test_repo();
    let worktrees_root = std::path::Path::new(&repo_path).join(".worktrees");
    std::fs::create_dir_all(&worktrees_root).unwrap();
    std::fs::write(worktrees_root.join("42-fix-bug"), b"not a worktree").unwrap();

    let mock = MockProcessRunner::new(vec![]);
    let task = make_task(&repo_path);
    let err = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap_err();

    assert!(format!("{err:#}").contains("42-fix-bug"), "got: {err:#}");
    assert!(mock.recorded_calls().is_empty());
    assert!(
        worktrees_root.join("42-fix-bug").is_file(),
        "the file must survive"
    );
}

/// The usable half of `PresenceIsNotYetReuse`: a symlink to a REAL directory
/// is followed for the usability question and reused like any other directory.
#[test]
fn provision_worktree_reuses_a_symlink_that_resolves_to_a_directory() {
    let (dir, repo_path) = make_test_repo();
    let worktrees_root = std::path::Path::new(&repo_path).join(".worktrees");
    std::fs::create_dir_all(&worktrees_root).unwrap();
    let real = dir.path().join("elsewhere");
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, worktrees_root.join("42-fix-bug")).unwrap();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT).unwrap();

    assert!(
        result.reused_worktree,
        "a link to a real directory is a usable worktree"
    );
    assert!(
        mock.recorded_calls().iter().all(|(prog, _)| prog != "git"),
        "the reuse path issues no git at all: {:?}",
        mock.recorded_calls()
    );
}

#[test]
fn provision_worktree_with_base_branch_passes_start_point() {
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::provision().fresh_worktree();
    let mock = script.runner();

    let task = make_task(&repo_path);
    let result = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("99-prev-task")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    // call[0] = fetch
    assert_eq!(calls[0].0, "git");
    assert!(calls[0].1.contains(&"fetch".to_string()));
    assert!(calls[0].1.contains(&"99-prev-task".to_string()));
    // The start point is the add's last arg — found by verb, since the prune
    // that precedes it is a `git worktree` call too.
    let git_args = &worktree_add_call(&calls).1;
    assert_eq!(
        git_args.last().unwrap(),
        "origin/99-prev-task",
        "base branch should be origin/99-prev-task as last git arg, got: {git_args:?}"
    );

    let expected_path = format!("{repo_path}/.worktrees/42-fix-bug");
    assert_eq!(result.worktree_path, expected_path);
}

#[test]
fn provision_worktree_fetches_origin_before_create() {
    // Fetch succeeds → worktree add should use origin/<base> as start point
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::provision().fresh_worktree();
    let mock = script.runner();

    let task = make_task(&repo_path);
    provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    // call[0] = git fetch origin main
    assert_eq!(calls[0].0, "git");
    assert!(
        calls[0].1.contains(&"fetch".to_string()),
        "expected fetch, got: {:?}",
        calls[0].1
    );
    assert!(calls[0].1.contains(&"origin".to_string()));
    assert!(calls[0].1.contains(&"main".to_string()));
    assert_eq!(
        worktree_add_start_point(&calls),
        "origin/main",
        "worktree add should use origin/main as start point, got: {calls:?}"
    );
}

#[test]
fn provision_worktree_fetch_failure_falls_back_to_local_without_retry() {
    // A fetch that fails and classifies as "no origin ref" (a 404 from
    // ls-remote) is not retried, and the local branch is used — no error.
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: couldn't find remote ref main"), // git fetch
        MockProcessRunner::ok(),                  // git remote get-url origin
        MockProcessRunner::fail_with_code(2, ""), // git ls-remote --exit-code (404)
        MockProcessRunner::ok(),                  // git rev-parse --verify main (present)
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::ok(),                  // git worktree add
        MockProcessRunner::ok(),                  // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(),                  // tmux new-window
        MockProcessRunner::ok(),                  // tmux set-option @dispatch_dir
        MockProcessRunner::ok(),                  // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    // Should NOT return an error — soft fail
    let result = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    let fetch_attempts = calls
        .iter()
        .filter(|(prog, args)| prog == "git" && args.contains(&"fetch".to_string()))
        .count();
    assert_eq!(
        fetch_attempts, 1,
        "a 404-classified fetch failure must not be retried, got: {calls:?}"
    );
    // The add uses local "main", not "origin/main". The `rev-parse --verify`
    // probe before it is what licenses that choice — see
    // `ensure_local_base_resolves`.
    assert_eq!(
        worktree_add_start_point(&calls),
        "main",
        "fallback should use local main, not origin/main, got: {calls:?}"
    );
    let warning = result
        .fetch_warning
        .expect("expected a fetch_warning when there is no origin ref");
    assert!(
        warning.contains("main"),
        "warning should mention the base branch, got: {warning}"
    );
}

#[test]
fn provision_worktree_pr_head_missing_from_origin_aborts_rather_than_using_local() {
    // Mirrors provision_worktree_fetch_failure_falls_back_to_local_without_retry's
    // mock shape, but with BaseRef::PrHead: origin missing the PR's head branch
    // must abort, never fall back to a local branch of the same name.
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: couldn't find remote ref feature-x"), // git fetch
        MockProcessRunner::ok(),                  // git remote get-url origin
        MockProcessRunner::fail_with_code(2, ""), // git ls-remote --exit-code (404)
    ]);

    let task = make_task(&repo_path);
    let err = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::PrHead("feature-x")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("feature-x"),
        "error should name the missing branch, got: {err}"
    );

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.contains(&"worktree".to_string()))),
        "must not create a worktree from a stale local branch: {calls:?}"
    );
    assert!(
        calls.iter().all(|(prog, _)| prog != "tmux"),
        "must not open a tmux window when the review has no safe ref to base on: {calls:?}"
    );
}

#[test]
fn provision_worktree_retries_fetch_before_falling_back() {
    // First fetch fails and classifies as unreachable (not a 404) → retried;
    // second fetch fails too; third succeeds → no fallback, no warning.
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_succeeds_on_attempt(3);
    let mock = script.runner();

    let task = make_task(&repo_path);
    let result = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    let fetch_attempts = calls
        .iter()
        .filter(|(prog, args)| prog == "git" && args.contains(&"fetch".to_string()))
        .count();
    assert_eq!(
        fetch_attempts, FETCH_MAX_ATTEMPTS as usize,
        "expected 2 failures + 1 success, i.e. the full budget, got: {calls:?}"
    );
    assert_eq!(
        worktree_add_start_point(&calls),
        "origin/main",
        "should use origin/main once fetch eventually succeeds, got: {calls:?}"
    );
    assert!(
        result.fetch_warning.is_none(),
        "no warning expected when fetch eventually succeeds"
    );
}

/// `PROVISION_MAX_SUBPROCESS_CALLS` (`src/dispatch/worktree.rs`) is what
/// `DISPATCH_WATCHDOG_TIMEOUT` (`src/tui/mod.rs`) derives its budget from —
/// see #4201. Pin it to the real worst-case shape (a fetch that only
/// succeeds on its last allowed attempt) via `DispatchScript`'s own
/// step-counting, rather than trusting a hand-maintained literal: if
/// `fetch_origin`'s retry/classify logic ever grows another call, this fails
/// instead of silently under-sizing the watchdog again.
///
/// `index_of(Step::WorktreeAdd) + 1` counts only up through the last
/// `SUBPROCESS_TIMEOUT`-bounded call in the sequence — it says nothing about
/// (and doesn't need to, since `PROVISION_MAX_SUBPROCESS_CALLS` doesn't cover
/// them either) the unbounded tmux tail that follows worktree add.
#[test]
fn provision_max_subprocess_calls_matches_the_worst_case_shape() {
    let script = DispatchScript::provision()
        .fresh_worktree()
        .fetch_succeeds_on_attempt(FETCH_MAX_ATTEMPTS);

    assert_eq!(
        script.index_of(Step::WorktreeAdd) + 1,
        PROVISION_MAX_SUBPROCESS_CALLS as usize,
        "PROVISION_MAX_SUBPROCESS_CALLS must match the real worst-case number \
         of SUBPROCESS_TIMEOUT-bounded calls provision_worktree can issue"
    );
}

#[test]
fn provision_worktree_fetch_uses_custom_base_branch() {
    // Custom base_branch is used in both fetch and worktree add
    let (_dir, repo_path) = make_test_repo();

    let script = DispatchScript::provision().fresh_worktree();
    let mock = script.runner();

    let task = make_task(&repo_path);
    provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("develop")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls[0].1.contains(&"develop".to_string()),
        "fetch should use 'develop', got: {:?}",
        calls[0].1
    );
    assert_eq!(
        worktree_add_start_point(&calls),
        "origin/develop",
        "worktree add should use origin/develop, got: {calls:?}"
    );
}

#[test]
fn provision_worktree_still_fetches_when_dir_exists() {
    // Pre-existing worktree dir → fetch still runs (so origin/<base> stays
    // fresh for whatever rebases onto it later), but `git worktree add` is
    // skipped since the branch/dir already exist.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::provision();
    let mock = script.runner();

    let task = make_task(&repo_path);
    provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(
        calls[0].0, "git",
        "first call should be git, got: {calls:?}"
    );
    assert!(
        calls[0].1.contains(&"fetch".to_string()),
        "fetch should still run when the worktree dir already exists, got: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.contains(&"worktree".to_string()))),
        "git worktree add should be skipped when the dir already exists, got: {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// Reuse path vs an unreachable origin (#3843)
//
// On the reuse path `git worktree add` is skipped, so no ref is consumed to
// create anything: the resolved start point feeds only the rebase preamble.
// An unreachable origin therefore has nothing to corrupt, and aborting there
// costs an offline user a dispatch that needed no network at all.
// ---------------------------------------------------------------------------

#[test]
fn provision_worktree_reuse_survives_an_unreachable_origin() {
    // Dir already exists + every fetch fails ⇒ the dispatch still proceeds,
    // based on local <base>, with a Note: for the agent.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: unable to access 'origin': network is unreachable"),
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .expect("a reused worktree needs no network, so an unreachable origin must not abort");

    assert_eq!(
        result.start_point,
        Some(StartPoint::Local {
            base: "main".to_string()
        }),
        "an unfetchable origin/<base> must not be the rebase target: it would replay \
         local <base>'s unpushed commits under new SHAs"
    );
    let warning = result
        .fetch_warning
        .expect("the agent must be told in its prompt that origin could not be reached");
    assert!(
        warning.contains("main"),
        "the warning should name the base branch, got: {warning}"
    );

    let calls = mock.recorded_calls();
    assert!(
        calls.iter().any(|(prog, _)| prog == "tmux"),
        "the agent's tmux window must still be created, got: {calls:?}"
    );
}

#[test]
fn provision_worktree_reuse_does_not_retry_or_probe_an_unreachable_origin() {
    // The budget test. Both the retry loop and the ls-remote classification
    // probe exist to serve the abort decision — retries to smooth a transient
    // failure before aborting, the probe to tell a 404 (fall back) from infra
    // (abort). With no abort on this path both classes end the same way, so
    // each extra network call buys nothing and costs a full SUBPROCESS_TIMEOUT.
    // Downgrading the abort without this assertion would leave the ~4 minutes
    // of offline blocking exactly where they were.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: unable to access 'origin': network is unreachable"),
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .expect("reuse + unreachable origin must not abort");

    let calls = mock.recorded_calls();
    let fetches = calls
        .iter()
        .filter(|(prog, args)| prog == "git" && args.contains(&"fetch".to_string()))
        .count();
    assert_eq!(
        fetches, 1,
        "the reuse path keeps origin fresh with a single best-effort attempt, never a \
         retry budget, got: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .all(|(_, args)| !args.contains(&"ls-remote".to_string())),
        "classifying the failure changes nothing on the reuse path, so the probe must \
         not be run, got: {calls:?}"
    );
}

#[test]
fn provision_worktree_reuse_of_a_pr_head_keeps_the_remote_start_point() {
    // BaseRef::PrHead must never yield a Local start point — a stale local
    // branch of the same name would let a review examine the wrong code. The
    // existing worktree already holds the PR's code from the previous attempt,
    // so reuse is safe; only the preamble's rebase target is at stake.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: unable to access 'origin': network is unreachable"),
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::PrHead("feature-x")),
        SUBPROCESS_TIMEOUT,
    )
    .expect("a reused PR-review worktree already holds the PR's code");

    assert_eq!(
        result.start_point,
        Some(StartPoint::Remote {
            base: "feature-x".to_string()
        }),
        "a PR head must stay pinned to origin/<head>, never fall back to a local branch \
         of the same name"
    );
    assert!(
        result.fetch_warning.is_some(),
        "the agent must be told its rebase target may be unreachable"
    );
}

#[test]
fn provision_worktree_fresh_still_spends_the_full_budget_before_aborting() {
    // Regression guard: the reuse-path shortcut above must not leak onto the
    // fresh path, where the resolved ref really does create the branch and a
    // stale local ref is the failure #3810 exists to prevent.
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: unable to access 'origin': network is unreachable"),
        MockProcessRunner::ok(), // git remote get-url origin
        MockProcessRunner::fail_with_code(128, ""), // git ls-remote --exit-code (unreachable)
        MockProcessRunner::fail("fatal: unable to access 'origin': network is unreachable"),
        MockProcessRunner::fail("fatal: unable to access 'origin': network is unreachable"),
    ]);

    let task = make_task(&repo_path);
    let err = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .expect_err("a fresh worktree must not be branched off a stale local ref");

    let calls = mock.recorded_calls();
    let fetches = calls
        .iter()
        .filter(|(prog, args)| prog == "git" && args.contains(&"fetch".to_string()))
        .count();
    assert_eq!(
        fetches, 3,
        "the fresh path still retries to exhaustion, got: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|(_, args)| args.contains(&"ls-remote".to_string())),
        "the fresh path still classifies, because 404 and infra diverge there: {calls:?}"
    );
    assert!(
        !calls.iter().any(|(prog, _)| prog == "tmux"),
        "aborting must happen before any tmux window is created, got: {calls:?}"
    );

    // The error names a next step, not just the cause and the attempt count.
    let msg = err.to_string();
    assert!(
        msg.contains("network") || msg.contains("connectivity"),
        "the error should point at what to check, got: {msg}"
    );
    assert!(
        msg.contains("retry") || msg.contains("dispatch again") || msg.contains("try again"),
        "the error should say what to do once connectivity is back, got: {msg}"
    );
}

#[test]
fn rebase_preamble_with_base_branch() {
    let sp = StartPoint::Remote {
        base: "99-prev-task".to_string(),
    };
    let preamble = reused_rebase_preamble(&sp);
    assert!(
        preamble.contains("git fetch origin 99-prev-task"),
        "should fetch the base branch first, got: {preamble}"
    );
    assert!(
        preamble.contains("git rebase origin/99-prev-task"),
        "should rebase onto the fetched origin ref, got: {preamble}"
    );
    assert!(
        !preamble.contains("origin/main"),
        "should not reference origin/main"
    );
}

#[test]
fn rebase_preamble_uses_given_target() {
    let sp = StartPoint::Remote {
        base: "develop".to_string(),
    };
    let preamble = reused_rebase_preamble(&sp);
    assert!(
        preamble.contains("git fetch origin develop"),
        "should fetch the given target, got: {preamble}"
    );
    assert!(
        preamble.contains("git rebase origin/develop"),
        "should rebase onto origin/<given target>, got: {preamble}"
    );
    assert!(
        !preamble.contains("origin/main"),
        "should not contain origin/main"
    );
}

#[test]
fn resume_skips_git_issues_tmux_continue() {
    let (_dir, worktree_path) = make_test_repo();

    let script = DispatchScript::resume();
    let mock = script.runner();

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    script.assert_matches(&calls);
    let new_window = &calls[script.index_of(Step::NewWindow)];
    assert_eq!(new_window.0, "tmux");
    assert_eq!(new_window.1[0], "new-window");
    assert_eq!(
        calls[script.index_of(Step::SetDispatchDir)].1[0],
        "set-option"
    );
    assert_eq!(calls[script.index_of(Step::SetSplitHook)].1[0], "set-hook");
    assert!(
        calls.iter().all(|(prog, _)| prog != "git"),
        "resume should make no git calls"
    );
    assert!(calls[script.index_of(Step::SendKeysLiteral)]
        .1
        .iter()
        .any(|a| a.contains("--continue")));
}

#[test]
fn resume_agent_splits_agent_tree_companion_pane_after_send_keys() {
    let (_dir, worktree_path) = make_test_repo();

    let script = DispatchScript::resume();
    let mock = script.runner();

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let last = &calls[script.index_of(Step::CompanionSplit)];
    assert_eq!(last.0, "tmux");
    assert_eq!(last.1[0], "split-window");
    assert_eq!(
        last.1[last.1.len() - 3..],
        vec![
            "dispatch".to_string(),
            "agent-tree".to_string(),
            "42".to_string(),
        ],
        "companion pane should run `dispatch agent-tree <task_id>`, got: {:?}",
        last.1
    );
}

#[test]
fn resume_agent_succeeds_even_if_companion_pane_split_fails() {
    let (_dir, worktree_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (has_window: not alive)
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
        MockProcessRunner::fail("no target pane"), // tmux split-window fails
    ]);

    let result = resume_agent(TaskId(42), &worktree_path, &mock);
    assert!(
        result.is_ok(),
        "a failed companion-pane split must not fail resume: {result:?}"
    );
}

#[test]
fn cleanup_kills_window_and_removes_worktree() {
    let mock = MockProcessRunner::new(vec![
        // has_window: list-windows returns the window name in stdout
        MockProcessRunner::ok_with_stdout(b"task-42\n"),
        MockProcessRunner::ok(), // tmux kill-window
        MockProcessRunner::ok(), // git worktree remove
        MockProcessRunner::ok(), // git branch -D (best-effort)
    ]);

    teardown_task(
        "/repo",
        Some("/repo/.worktrees/42-fix-bug"),
        Some(&test_tmux_window("task-42")),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "list-windows");
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(calls[1].1[0], "kill-window");
    assert_eq!(calls[2].0, "git");
    // git worktree remove is invoked with -C <repo>
    assert!(calls[2].1.contains(&"-C".to_string()));
    assert!(calls[2].1.contains(&"remove".to_string()));
}

#[test]
fn cleanup_succeeds_when_worktree_already_removed() {
    // When git says "not a working tree" the teardown should still succeed,
    // not surface an error to the user.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: '/repo/.worktrees/42-fix-bug' is not a working tree"),
        MockProcessRunner::ok(), // git worktree prune (best-effort)
        MockProcessRunner::ok(), // git branch -D (best-effort)
    ]);

    teardown_task("/repo", Some("/repo/.worktrees/42-fix-bug"), None, &mock).unwrap();
}

#[test]
fn dispatch_uses_task_base_branch_in_prompt() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let mut task = make_task(&repo_path);
    task.base_branch = "master".into();
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    // Verify the prompt uses task.base_branch directly — no symbolic-ref call needed
    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("git rebase origin/master"),
        "prompt should reference task.base_branch (master), got: {prompt}"
    );
    assert!(
        !prompt.contains("git rebase origin/main"),
        "prompt should not reference main when task.base_branch is master"
    );
}

#[test]
fn dispatch_fails_fast_if_git_fails() {
    let (_dir, repo_path) = make_test_repo();

    // `fails_at` queues nothing past the failure, so a tmux call after the failed
    // worktree add panics the mock rather than passing unnoticed.
    let script = DispatchScript::dispatch()
        .fresh_worktree()
        .fails_at(Step::WorktreeAdd);
    let mock = script.runner();

    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &LearningInjections::default());
    assert!(result.is_err());
    let calls = mock.recorded_calls();
    assert_eq!(
        calls.len(),
        4,
        "only git fetch + rev-list + worktree prune + worktree add should have \
         been called (no detect_default_branch)"
    );
}

#[test]
fn quick_dispatch_reuses_existing_worktree() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    quick_dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
        "git worktree add should be skipped for existing worktree"
    );
    assert_eq!(calls[script.index_of(Step::NewWindow)].0, "tmux");
    assert_eq!(calls[script.index_of(Step::NewWindow)].1[0], "new-window");
}

#[test]
fn quick_dispatch_sends_rename_prompt() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    quick_dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("placeholder"),
        "prompt should mention placeholder title"
    );
    assert!(
        prompt.contains("update_task"),
        "prompt should mention update_task for rename"
    );
}
