use super::*;

// --- plugin-dir tests ---

#[test]
fn dispatch_agent_includes_plugin_dir() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        send_keys_arg.contains("--plugin-dir"),
        "dispatch_agent should include --plugin-dir, got: {send_keys_arg}"
    );
    assert!(
        send_keys_arg.contains(".claude/plugins/local/dispatch"),
        "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
    );
}

#[test]
fn resume_agent_includes_plugin_dir() {
    let (_dir, worktree_path) = make_test_repo();

    let script = DispatchScript::resume();
    let mock = script.runner();

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        send_keys_arg.contains("--plugin-dir"),
        "resume_agent should include --plugin-dir, got: {send_keys_arg}"
    );
    assert!(
        send_keys_arg.contains(".claude/plugins/local/dispatch"),
        "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
    );
}

// --- sccache env tests (task #4899:
// DispatchedAgentsShareASccacheNotATargetDir) ---

#[test]
fn dispatch_agent_sets_rustc_wrapper_when_sccache_available() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner().with_sccache_available(true);

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let new_window_args = &calls[script.index_of(Step::NewWindow)].1;
    assert!(
        new_window_args
            .windows(2)
            .any(|w| w == ["-e", "RUSTC_WRAPPER=sccache"]),
        "new-window should carry -e RUSTC_WRAPPER=sccache, got: {new_window_args:?}"
    );
}

#[test]
fn dispatch_agent_omits_rustc_wrapper_when_sccache_unavailable() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner(); // sccache_available defaults to false

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let new_window_args = &calls[script.index_of(Step::NewWindow)].1;
    assert!(
        !new_window_args.iter().any(|a| a == "-e"),
        "new-window should carry no -e flag when sccache is unavailable, got: {new_window_args:?}"
    );
}

#[test]
fn resume_agent_sets_rustc_wrapper_when_sccache_available() {
    let (_dir, worktree_path) = make_test_repo();

    let script = DispatchScript::resume();
    let mock = script.runner().with_sccache_available(true);

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let new_window_args = &calls[script.index_of(Step::NewWindow)].1;
    assert!(
        new_window_args
            .windows(2)
            .any(|w| w == ["-e", "RUSTC_WRAPPER=sccache"]),
        "new-window should carry -e RUSTC_WRAPPER=sccache, got: {new_window_args:?}"
    );
}

// --- session naming tests (task #4098: deterministic --name for native
// cross-session messaging addressing) ---

#[test]
fn dispatch_agent_names_the_session_after_the_task() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        send_keys_arg.contains("--name task-42"),
        "dispatch_agent should name the session task-<id> for native \
cross-session-messaging addressing, got: {send_keys_arg}"
    );
}

#[test]
fn resume_agent_names_the_session_after_the_task() {
    let (_dir, worktree_path) = make_test_repo();

    let script = DispatchScript::resume();
    let mock = script.runner();

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        send_keys_arg.contains("--name task-42"),
        "resume_agent should name the session task-<id>, got: {send_keys_arg}"
    );
    // Nothing may follow the flags here: the variadic `--mcp-config` would
    // swallow it instead of claude receiving it as an operand. `--continue` is
    // `-`-prefixed and ends the variadic by itself, so resume needs no `--`;
    // one that grew an operand would (see PromptIsSeparatedFromTheLaunchFlags
    // in docs/specs/dispatch.allium).
    //
    // It is also why ComposeAgentPrompt excludes mode resume
    // (docs/specs/dispatch-prompt.allium): a resume launch composes no prompt
    // at all, so no skeleton is owed to it. A prompt appearing here would fail
    // this assertion before it reached that rule.
    assert!(
        send_keys_arg.ends_with("--continue"),
        "nothing may follow the resume flags, got: {send_keys_arg}"
    );
}

// --- injected binary identities ---
//
// The launchers read `claude` / `dispatch` from `ProcessRunner::agent_binaries`
// rather than hardcoding them. These tests pin argv0, which a mock test could not
// assert while the names were literals.

fn dispatch_mock() -> MockProcessRunner {
    DispatchScript::dispatch().runner()
}

#[test]
fn dispatch_agent_launches_the_runners_claude_binary() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = dispatch_mock().with_agent_binaries(AgentBinaries::stub());

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(
        &calls,
        DispatchScript::dispatch().index_of(Step::SendKeysLiteral),
        "claude",
    );
    // The binary rides as bash's `$0`, after the script body.
    assert!(
        send_keys_arg.ends_with("/stub/bin/claude-stub"),
        "dispatch_agent must launch the runner's claude binary, got: {send_keys_arg}"
    );
}

#[test]
fn dispatch_agent_launches_the_runners_dispatch_binary_in_the_companion_pane() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = dispatch_mock().with_agent_binaries(AgentBinaries::stub());

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    // The companion pane is spawned via `split-window --`, so the binary is a
    // plain argv element rather than part of a shell string.
    let split = &mock.recorded_calls()[DispatchScript::dispatch().index_of(Step::CompanionSplit)].1;
    assert!(
        split.contains(&"/stub/bin/dispatch-stub".to_string()),
        "companion pane must exec the runner's dispatch binary, got: {split:?}"
    );
}

#[test]
fn resume_agent_launches_the_runners_claude_binary() {
    let (_dir, worktree_path) = make_test_repo();
    let script = DispatchScript::resume();
    let mock = script.runner().with_agent_binaries(AgentBinaries::stub());

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        send_keys_arg.starts_with("/stub/bin/claude-stub --plugin-dir"),
        "resume_agent must launch the runner's claude binary, got: {send_keys_arg}"
    );
    // Nothing may follow the flags here — see the sibling resume test above.
    assert!(
        send_keys_arg.ends_with("--continue"),
        "nothing may follow the resume flags, got: {send_keys_arg}"
    );
}

/// A runner that does not override `agent_binaries` must still emit the bare,
/// unquoted names — the guarantee that this seam changed no production behaviour.
#[test]
fn agent_launchers_default_to_bare_binary_names() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = dispatch_mock();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(
        &calls,
        DispatchScript::dispatch().index_of(Step::SendKeysLiteral),
        "claude",
    );
    assert!(
        send_keys_arg.ends_with("' claude"),
        "the default must be the bare, unquoted name, got: {send_keys_arg}"
    );
    let companion = DispatchScript::dispatch().index_of(Step::CompanionSplit);
    assert!(
        calls[companion].1.contains(&"dispatch".to_string()),
        "the default companion binary must be the bare name, got: {:?}",
        calls[companion].1
    );
}

// --- provision_worktree error path ---

#[test]
fn provision_worktree_nonexistent_repo_path_returns_error_without_creating_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let nonexistent = dir.path().to_str().unwrap().to_owned();
    drop(dir); // path is now guaranteed non-existent

    let mock = MockProcessRunner::new(vec![]);
    let task = make_task(&nonexistent);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT);

    assert!(
        result.is_err(),
        "nonexistent repo_path should return an error"
    );
    assert!(
        !std::path::Path::new(&nonexistent).exists(),
        "provision_worktree must not create directories for nonexistent repo_path"
    );
}

#[test]
fn provision_worktree_git_add_fails_returns_error() {
    let (_dir, repo_path) = make_test_repo();
    // No base_branch → no fetch; the prune comes first, then the add.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),                                // git worktree prune
        MockProcessRunner::fail("fatal: not a git repository"), // git worktree add
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT);

    assert!(result.is_err(), "git worktree add failure should propagate");
}

#[test]
fn provision_worktree_rolls_back_the_worktree_when_a_later_step_fails() {
    let (_dir, repo_path) = make_test_repo();
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),                      // git worktree prune
        MockProcessRunner::ok(),                      // git worktree add
        MockProcessRunner::ok(),                      // tmux list-windows (duplicate-name check)
        MockProcessRunner::fail("no server running"), // tmux new-window
        // No window-kill check in the rollback: `new-window` failed, so this
        // attempt never opened a window of its own.
        MockProcessRunner::ok(), // git worktree remove --force (rollback)
        MockProcessRunner::ok(), // git branch -D (rollback)
    ]);
    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT);
    assert!(result.is_err(), "tmux failure must abort provisioning");
    let calls = mock.recorded_calls();
    assert!(
        worktree_remove_call(&calls).is_some(),
        "the created worktree must be removed on the failure path, got: {calls:?}"
    );
}

/// dispatch.allium's "Provisioning-failure rollback": the window half of the
/// rollback is as conditional as the worktree half. A dispatch refused by
/// `TmuxWindowNamesAreUnique` never opened a window of its own, so the
/// rollback must leave the live one alone.
#[test]
fn provision_worktree_refused_for_a_duplicate_name_does_not_kill_the_live_window() {
    let (_dir, repo_path) = make_test_repo();
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git worktree add
        // The duplicate-name check finds task-42 already live, so `new-window`
        // is never issued and no window belongs to this attempt.
        MockProcessRunner::ok_with_stdout(b"board\ntask-42\n"),
        MockProcessRunner::ok(), // git worktree remove --force (rollback)
        MockProcessRunner::ok(), // git branch -D (rollback)
    ]);
    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT);
    assert!(
        result.is_err(),
        "a duplicate window name must abort a dispatch"
    );

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.first().map(String::as_str) == Some("kill-window")),
        "the rollback must not kill a window this attempt did not open, got: {calls:?}"
    );
    assert!(
        worktree_remove_call(&calls).is_some(),
        "the worktree this attempt created is still rolled back, got: {calls:?}"
    );
}

#[test]
fn provision_worktree_does_not_remove_a_reused_worktree_on_failure() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    // Reuse path: `git worktree add` is skipped entirely, so the only calls
    // before the failure are `new_window`'s duplicate-name query and the
    // `new-window` itself. The rollback issues nothing — the worktree is
    // reused, and the failed create left no window belonging to this attempt.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux list-windows (duplicate-name check)
        MockProcessRunner::fail("no server running"), // tmux new-window
    ]);
    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, SUBPROCESS_TIMEOUT);
    assert!(result.is_err(), "tmux failure must abort provisioning");
    let calls = mock.recorded_calls();
    assert!(
        worktree_remove_call(&calls).is_none(),
        "a reused (pre-existing) worktree must never be removed on failure, got: {calls:?}"
    );
}

// --- teardown_task edge cases ---

#[test]
fn cleanup_skips_kill_when_window_not_found() {
    // tmux_window is Some but has_window returns false (window already gone).
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: empty → false
        MockProcessRunner::ok(),                  // git worktree remove
        MockProcessRunner::ok(),                  // git branch -D (best-effort)
    ]);

    teardown_task(
        "/repo",
        Some("/repo/.worktrees/42-fix-bug"),
        Some(&test_tmux_window("task-42")),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "tmux" && args.iter().any(|a| a == "kill-window"))),
        "kill-window should not be called when window not found, got: {calls:?}"
    );
}

// --- finish_task edge cases ---

#[test]
fn finish_task_rebase_other_failure_aborts_and_returns_other() {
    // Rebase fails with a non-conflict stderr → maps to FinishError::Other.
    // `git rebase --abort` is still issued for cleanup.
    let (calls, result) = run_finish(&DispatchScript::finish().no_remote().rebase_fails());

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Rebase failed")),
        "non-conflict rebase failure should be FinishError::Other, got: {err}"
    );
    assert!(
        calls.last().unwrap().1.contains(&"--abort".to_string()),
        "rebase --abort must be invoked after a non-conflict rebase failure"
    );
}

#[test]
fn finish_task_ff_only_failure_returns_other() {
    let (_calls, result) = run_finish(&DispatchScript::finish().no_remote().fast_forward_fails());

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Fast-forward failed")),
        "ff-only failure should map to FinishError::Other, got: {err}"
    );
}

#[test]
fn finish_task_rev_parse_runner_error_returns_other() {
    // The runner itself errors (e.g. git binary missing) on rev-parse.
    let (_calls, result) = run_finish(&DispatchScript::finish().current_branch_cannot_run());

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to check current branch")),
        "rev-parse runner error should map to FinishError::Other, got: {err}"
    );
}

#[test]
fn finish_task_no_tmux_window_skips_tmux_entirely() {
    // tmux_window=None → no list-windows or kill-window calls.
    let (calls, result) = run_finish(&DispatchScript::finish().no_remote());

    result.unwrap();
    assert!(
        !calls.iter().any(|c| c.0 == "tmux"),
        "no tmux calls expected when tmux_window is None, got: {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// teardown_task — additional branch coverage
// ---------------------------------------------------------------------------

/// `TeardownIsOwedWheneverThereIsSomethingToRelease` in docs/specs/tasks.allium:
/// step 1 is owed on the window's presence alone. Before #4096 the archive/delete
/// wrapper skipped the whole teardown for this row shape and leaked the window.
#[test]
fn teardown_task_kills_window_when_there_is_no_worktree() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-42\n"), // has_window → true
        MockProcessRunner::ok(),                         // tmux kill-window
    ]);

    teardown_task("/repo", None, Some(&test_tmux_window("task-42")), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .any(|(prog, args)| prog == "tmux" && args.iter().any(|a| a == "kill-window")),
        "the window must be killed even with no worktree, got: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.0 == "git"),
        "no git calls expected with no worktree — no worktree means no branch \
         either, got: {calls:?}"
    );
}

/// The other half of the same clause: a row owning neither resource runs nothing.
#[test]
fn teardown_task_with_neither_worktree_nor_window_runs_no_commands() {
    let mock = MockProcessRunner::new(vec![]);

    teardown_task("/repo", None, None, &mock).unwrap();

    assert!(
        mock.recorded_calls().is_empty(),
        "a stateless row must run no commands, got: {:?}",
        mock.recorded_calls()
    );
}

// A window-only kill failure needs no test of its own: the kill runs before the
// worktree arm, so `teardown_task_kill_window_failure_propagates` below already
// drives the identical path to the same `?`. What the *wrapper* does with that
// error is the interesting half, and lives in
// src/runtime/tests/task_exec.rs::exec_cleanup_window_only_kill_failure_still_applies_the_follow_up.

#[test]
fn teardown_task_no_tmux_window_arg_skips_tmux() {
    // tmux_window=None → cleanup goes straight to worktree remove + branch -D.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git worktree remove
        MockProcessRunner::ok(), // git branch -D (best-effort)
    ]);

    teardown_task("/repo", Some("/repo/.worktrees/42-fix-bug"), None, &mock).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        !calls.iter().any(|c| c.0 == "tmux"),
        "no tmux calls expected when tmux_window is None, got: {calls:?}"
    );
    assert!(calls[0].1.contains(&"remove".to_string()));
}

#[test]
fn teardown_task_other_remove_failure_propagates_when_the_directory_survives() {
    // git worktree remove fails for a reason that is neither the
    // already-unregistered case nor a lock, AND the directory is still there
    // afterwards → step 2 failed, and teardown_task surfaces it.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    // The delete cannot succeed either: unlink permission lives on the parent.
    let parent = worktree.parent().unwrap().to_path_buf();
    let Some(_perm) = deny_access_or_skip(&parent, 0o555, &worktree) else {
        return;
    };

    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: some unexpected git failure",
    )]);
    let failure = teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap_err();

    let msg = format!("{failure:#}");
    assert!(
        msg.contains("git worktree remove failed"),
        "expected 'git worktree remove failed' in error chain, got: {msg}"
    );
}

#[test]
fn teardown_succeeds_when_git_fails_but_nothing_is_left_on_disk() {
    // GitsExitCodeDoesNotDecideStepTwo: git's failure is the case most in need
    // of the delete, not a reason to skip it. Once the path is absent the
    // worktree is released, whatever git's exit code said.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: some unexpected git failure"),
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git branch -D still runs
    ]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(!worktree.exists());
    let calls = mock.recorded_calls();
    assert!(
        calls.iter().any(|c| c.1.contains(&"-D".to_string())),
        "step 3 must still run for a released worktree, got: {calls:?}"
    );
}

#[test]
fn teardown_deletes_the_directory_when_git_cannot_be_run_at_all() {
    // GitsExitCodeDoesNotDecideStepTwo covers EVERY outcome of 2a, including
    // the one where there is no exit code: git missing from PATH, or a spawn
    // failure. The worktree is no less on disk for it.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        Err(anyhow::anyhow!("No such file or directory (os error 2)")),
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git branch -D
    ]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(!worktree.exists());
}

#[test]
fn a_git_failure_prunes_the_admin_record_before_deleting_the_branch() {
    // Git that removed nothing also kept `.git/worktrees/<name>`, and that
    // record makes the next dispatch of this task fail at `git worktree add`.
    // Order is load-bearing: `git branch -D` fails while the record still
    // claims the branch.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: validation failed, cannot remove working tree"),
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git branch -D
    ]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    let calls = mock.recorded_calls();
    let prune = calls
        .iter()
        .position(|c| c.1.contains(&"prune".to_string()))
        .expect("a git failure must prune the admin record it left behind");
    let branch = calls
        .iter()
        .position(|c| c.1.contains(&"-D".to_string()))
        .expect("step 3 must still run");
    assert!(
        prune < branch,
        "prune must precede the branch delete, got: {calls:?}"
    );
}

#[test]
fn a_successful_git_removal_does_not_prune() {
    // Git cleaned up its own record on the success path. A prune there would
    // be a repo-wide operation this step has no reason to run.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        !calls.iter().any(|c| c.1.contains(&"prune".to_string())),
        "no prune expected on the success path, got: {calls:?}"
    );
}

#[test]
fn an_unreadable_worktree_path_is_not_reported_as_released() {
    // "Absent" must mean NotFound. A directory that exists but cannot be
    // stat'd read as absent would let the gate clear a pointer to something
    // still on disk — the orphan WorktreeReleaseIsGated (c) prevents.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let parent = worktree.parent().unwrap().to_path_buf();
    // No execute bit on the parent: stat of the child fails with EACCES.
    let Some(_perm) = deny_access_or_skip(&parent, 0o644, &worktree) else {
        return;
    };

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let failure = teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap_err();
    assert_eq!(
        failure.worktree_left.as_deref(),
        Some(worktree.to_str().unwrap())
    );
    assert!(
        format!("{failure:#}").contains("failed to inspect leftover worktree"),
        "expected the stat failure in the error chain, got: {failure:#}"
    );
}

#[test]
fn teardown_obeys_a_lock_and_deletes_nothing() {
    // OneRefusalIsObeyed. `git worktree lock` is the operator protecting this
    // directory; dispatch neither passes `-f -f` nor deletes around it.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: cannot remove a locked working tree;\nuse 'remove -f -f' to override or unlock first",
    )]);

    let failure = teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap_err();

    assert_eq!(
        failure.worktree_left.as_deref(),
        Some(worktree.to_str().unwrap())
    );
    assert!(
        worktree.join("target/debug/huge.rlib").exists(),
        "a locked worktree must be left exactly as it was"
    );
}

#[test]
fn teardown_task_kill_window_failure_propagates() {
    // tmux kill-window fails → teardown_task returns an error and does NOT
    // attempt the worktree remove.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-42\n"), // has_window → true
        MockProcessRunner::fail("can't find window"),    // kill-window fails
    ]);

    let err = teardown_task(
        "/repo",
        Some("/repo/.worktrees/42-fix-bug"),
        Some(&test_tmux_window("task-42")),
        &mock,
    )
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("failed to kill tmux window"),
        "expected kill-window failure in error chain, got: {msg}"
    );

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.0 == "git" && c.1.contains(&"remove".to_string())),
        "git worktree remove must not run after kill-window failure, got: {calls:?}"
    );
}

// --- teardown_task has_window runner error path ---

#[test]
fn teardown_task_has_window_runner_error_warns_and_continues() {
    // When runner.run() itself returns Err (e.g. tmux not installed), has_window
    // propagates the error to teardown_task, which logs a warning and continues
    // rather than aborting — worktree remove must still run.
    let mock = MockProcessRunner::new(vec![
        Err(anyhow::anyhow!("tmux not installed")), // has_window → runner error
        MockProcessRunner::ok(),                    // git worktree remove
        MockProcessRunner::ok(),                    // git branch -D (best-effort)
    ]);

    teardown_task(
        "/repo",
        Some("/repo/.worktrees/42-fix-bug"),
        Some(&test_tmux_window("task-42")),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(prog, args)| prog == "tmux" && args.iter().any(|a| a == "kill-window")),
        "kill-window must not run when has_window errors, got: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|(prog, args)| prog == "git" && args.iter().any(|a| a == "remove")),
        "git worktree remove must still run after has_window error, got: {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// branch_from_worktree — pure helper
// ---------------------------------------------------------------------------

#[test]
fn branch_from_worktree_returns_last_path_component() {
    assert_eq!(
        branch_from_worktree("/repo/.worktrees/42-fix-bug"),
        Some("42-fix-bug".to_string())
    );
}

#[test]
fn branch_from_worktree_strips_trailing_slash() {
    assert_eq!(
        branch_from_worktree("/repo/.worktrees/42-fix-bug/"),
        Some("42-fix-bug".to_string())
    );
}

#[test]
fn branch_from_worktree_returns_none_for_empty() {
    assert_eq!(branch_from_worktree(""), None);
}

#[test]
fn branch_from_worktree_returns_none_for_root() {
    assert_eq!(branch_from_worktree("/"), None);
}

// ---------------------------------------------------------------------------
// provision_worktree — timeout tests
// ---------------------------------------------------------------------------

#[test]
fn provision_worktree_kills_git_fetch_on_timeout_and_aborts() {
    // git fetch times out on every attempt → classified as unreachable
    // (never a 404), retried to exhaustion, then aborts. A worktree silently
    // based on a stale local ref is worse than a dispatch that refuses to
    // start, so no tmux window is ever created.
    let (_dir, repo_path) = make_test_repo();
    let short_timeout = Duration::from_millis(10);

    let mock = MockProcessRunner::new_with_delays(vec![
        (Some(Duration::from_millis(100)), MockProcessRunner::ok()), // git fetch attempt 1 → timeout (killed)
        (None, MockProcessRunner::ok()),                             // git remote get-url origin
        (None, MockProcessRunner::ok()), // git ls-remote --exit-code (unreachable: not code 2)
        (Some(Duration::from_millis(100)), MockProcessRunner::ok()), // git fetch attempt 2 → timeout (killed)
        (Some(Duration::from_millis(100)), MockProcessRunner::ok()), // git fetch attempt 3 → timeout (killed)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, Some(BaseRef::Branch("main")), short_timeout);
    assert!(
        result.is_err(),
        "an unreachable origin must abort provisioning rather than fall back to local, got: {result:?}"
    );

    let calls = mock.recorded_calls();
    assert!(
        !calls.iter().any(|(prog, _)| prog == "tmux"),
        "aborting must happen before any tmux window is created, got: {calls:?}"
    );
}

#[test]
fn provision_worktree_kills_git_worktree_add_on_timeout() {
    // git worktree add times out → hard error (not soft-fail).
    // No base_branch → no git fetch; the prune runs first, then the add.
    // Before fix (using run): mock sleeps 100ms then succeeds → returns Ok().
    // After fix (using run_with_timeout): timeout error propagates → Err().
    let (_dir, repo_path) = make_test_repo();
    let short_timeout = Duration::from_millis(10);

    let mock = MockProcessRunner::new_with_delays(vec![
        (None, MockProcessRunner::ok()), // git worktree prune
        (
            Some(Duration::from_millis(100)), // delay > short_timeout → timeout error
            MockProcessRunner::ok(),
        ),
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None, short_timeout);
    assert!(
        result.is_err(),
        "expected error when git worktree add times out"
    );
    // anyhow error chain: use {:#} to traverse all causes
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("timed out") || msg.contains("killed"),
        "expected timeout in error chain, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// worktree-confinement invariant
//
// dispatch_with_prompt hard-codes the line
// "Always work from this worktree folder — do not `cd` to the parent repo
//  or other directories."
// into every prompt it writes. These tests drive the three public agent-spawn
// functions through a real `.claude-prompt` write (worktree dir pre-created so
// the write succeeds) and assert the invariant is present.
// ---------------------------------------------------------------------------

fn read_prompt(worktree_dir: &std::path::Path) -> String {
    std::fs::read_to_string(worktree_dir.join(".claude-prompt")).unwrap()
}

fn assert_worktree_confinement(prompt: &str) {
    assert!(
        prompt.contains("Always work from this worktree folder"),
        "prompt must include worktree-confinement instruction, got: {prompt}"
    );
    assert!(
        prompt.contains("do not `cd` to the parent repo"),
        "prompt must tell agent not to cd to parent repo, got: {prompt}"
    );
}

#[test]
fn dispatch_agent_prompt_includes_worktree_confinement() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();
    assert_worktree_confinement(&read_prompt(&worktree_dir));
}

/// `DesignStepMatchesTheReposSpecs` (task #4409), end to end: the design step
/// in the written prompt follows what the *repo* holds, not a default. The
/// check reads the parent repo, so the worktree's own contents are irrelevant.
#[test]
fn dispatch_agent_prompt_design_step_follows_the_repos_allium_specs() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let task = make_task(&repo_path);

    // A bare repo with no docs/specs: brainstorming, and no allium line.
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();
    let prompt = read_prompt(&worktree_dir);
    assert!(
        prompt.contains("superpowers:brainstorming"),
        "a repo with no specs should get the brainstorming design step, got: {prompt}"
    );
    assert!(
        !prompt.contains("allium:elicit"),
        "a repo with no specs must not be sent to elicit a spec, got: {prompt}"
    );
    assert!(
        !prompt.contains("source of truth"),
        "a repo with no specs must not be pointed at docs/specs/, got: {prompt}"
    );

    // Same repo, now keeping a spec: the spec-first sequence, no brainstorming.
    let spec_dir = std::path::Path::new(&repo_path).join("docs/specs");
    std::fs::create_dir_all(&spec_dir).unwrap();
    std::fs::write(spec_dir.join("domain.allium"), "-- allium: 3\n").unwrap();

    let script = DispatchScript::dispatch();
    let mock = script.runner();
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();
    let prompt = read_prompt(&worktree_dir);
    assert!(
        prompt.contains("allium:elicit"),
        "a spec-keeping repo should get the spec-first sequence, got: {prompt}"
    );
    assert!(
        !prompt.contains("brainstorming"),
        "a spec-keeping repo must not name brainstorming, got: {prompt}"
    );
}

/// Quick dispatch shares the branch — it is the other prompt that names a
/// design step.
#[test]
fn quick_dispatch_agent_prompt_design_step_follows_the_repos_allium_specs() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);

    quick_dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let prompt = read_prompt(&worktree_dir);
    assert!(
        prompt.contains("superpowers:brainstorming"),
        "quick dispatch into a spec-less repo should brainstorm, got: {prompt}"
    );
}

#[test]
fn research_agent_prompt_is_correct() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);
    research_agent(&task, &mock, None).unwrap();
    let prompt = read_prompt(&worktree_dir);
    assert_worktree_confinement(&prompt);
    assert!(
        prompt.contains("research agent"),
        "research_agent prompt should identify as a research agent, got: {prompt}"
    );
    // TheResearchPromptGivesItsReasonNotARule: the constraint is reasoning,
    // not a capitalised prohibition — but it still covers code changes.
    assert!(
        prompt.to_lowercase().contains("code change"),
        "research_agent prompt must still address code changes, got: {prompt}"
    );
    assert!(
        !prompt.contains("Do NOT"),
        "research_agent prompt must give its reason, not a capitalised rule, got: {prompt}"
    );
}

#[test]
fn quick_dispatch_agent_prompt_includes_worktree_confinement() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);
    quick_dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();
    assert_worktree_confinement(&read_prompt(&worktree_dir));
}

// ---------------------------------------------------------------------------
// dispatch_agent — worktree confinement (behavior-first)
//
// The prompt-text confinement tests above cover the *instruction* to the agent.
// These assert the *mechanism*: the tmux window the agent runs in is opened with
// its working directory set to the task's worktree (under `.worktrees/`), never
// the bare parent repo. This is the worktree-escape guarantee CLAUDE.md notes
// has no test.
// ---------------------------------------------------------------------------

#[test]
fn dispatch_agent_opens_tmux_window_in_worktree_not_parent_repo() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    // `tmux new-window …`; its `-c <dir>` argument sets the window cwd.
    let new_window = script.index_of(Step::NewWindow);
    assert_eq!(calls[new_window].0, "tmux");
    assert_eq!(calls[new_window].1[0], "new-window");
    let c_pos = calls[new_window]
        .1
        .iter()
        .position(|a| a == "-c")
        .expect("new-window should pass -c <working_dir>");
    let cwd = &calls[new_window].1[c_pos + 1];
    // Pinning the exact worktree path both proves the window opens *inside* the
    // worktree and (transitively) that it is not the bare parent repo — the
    // worktree-escape guarantee this test exists to lock down.
    let expected_worktree = format!("{repo_path}/.worktrees/42-fix-bug");
    assert_eq!(
        cwd, &expected_worktree,
        "agent tmux window must open inside the task worktree (never the bare parent repo {repo_path}), got cwd: {cwd}"
    );
}

// ---------------------------------------------------------------------------
// dispatch_agent — tmux spawn failure paths
//
// `dispatch_fails_fast_if_git_fails` covers the git-worktree-add failure. These
// cover the two remaining tmux spawn steps: creating the window and sending the
// launch keys. Both must propagate an error (not silently succeed) with context.
// ---------------------------------------------------------------------------

#[test]
fn dispatch_agent_propagates_tmux_new_window_failure() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch().fails_at(Step::NewWindow);
    // The script's own queue ends at the NewWindow failure; the rollback that
    // failure now triggers still checks (and would kill) the window it just
    // tried to open, so one more response is needed for that check.
    let mut responses = script.responses();
    responses.push((None, MockProcessRunner::ok()));
    let mock = MockProcessRunner::new_with_delays(responses);
    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    assert!(
        result.is_err(),
        "dispatch should propagate tmux new-window failure"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("failed to create tmux window"),
        "expected new-window context in error chain, got: {msg}"
    );
    let calls = mock.recorded_calls();
    assert!(
        worktree_remove_call(&calls).is_none(),
        "a reused (pre-existing) worktree must never be removed on failure, got: {calls:?}"
    );
}

#[test]
fn dispatch_agent_propagates_send_keys_failure() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch().fails_at(Step::SendKeysLiteral);
    // See dispatch_agent_propagates_tmux_new_window_failure: the rollback the
    // send-keys failure now triggers checks the (already-open) tmux window,
    // one call the script's own queue doesn't otherwise account for.
    let mut responses = script.responses();
    responses.push((None, MockProcessRunner::ok()));
    let mock = MockProcessRunner::new_with_delays(responses);
    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    assert!(
        result.is_err(),
        "dispatch should propagate send-keys failure"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("failed to send keys to tmux window"),
        "expected send-keys context in error chain, got: {msg}"
    );
    let calls = mock.recorded_calls();
    assert!(
        worktree_remove_call(&calls).is_none(),
        "a reused (pre-existing) worktree must never be removed on failure, got: {calls:?}"
    );
}

// A fresh (not pre-existing) worktree whose provisioning fully succeeds but
// whose post-provisioning `.claude-prompt` write then fails, because the
// mock never actually created the directory `git worktree add` claims to
// have created — see KB #351. That mismatch is exactly the "fresh path,
// later step fails" scenario the rollback exists for, and it falls out of
// the mock naturally: no faking required.
#[test]
fn dispatch_agent_rolls_back_a_fresh_worktree_when_the_prompt_write_fails() {
    let (_dir, repo_path) = make_test_repo();
    let mock = DispatchScript::dispatch().fresh_worktree().runner();
    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &LearningInjections::default());

    assert!(
        result.is_err(),
        "the prompt write should fail against a directory that was never really created"
    );
    let calls = mock.recorded_calls();
    assert!(
        worktree_remove_call(&calls).is_some(),
        "a fresh worktree must be rolled back when a later step fails, got: {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// resume_agent — failure path
// ---------------------------------------------------------------------------

#[test]
fn resume_agent_propagates_new_window_failure() {
    // Two `list-windows` queries precede the create: `resume_agent`'s own
    // liveness check, then `tmux::new_window`'s duplicate-name guard. Both
    // report no window alive; the `new-window` that follows fails, and the
    // error should bubble up.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),
        MockProcessRunner::ok(),
        MockProcessRunner::fail("no server running on /tmp/tmux-1000/default"),
    ]);

    let result = resume_agent(TaskId(42), "/repo/.worktrees/42-fix-bug", &mock);

    assert!(
        result.is_err(),
        "resume_agent should propagate new-window failure"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("failed to create tmux window for resume"),
        "expected resume context in error chain, got: {msg}"
    );
}

#[test]
fn resume_agent_reattaches_to_live_window_without_creating_duplicate() {
    let script = DispatchScript::resume().window_already_alive();
    let mock = script.runner();

    let result = resume_agent(TaskId(42), "/repo/.worktrees/42-fix-bug", &mock);
    assert!(
        result.is_ok(),
        "resume_agent should succeed by reattaching, not erroring: {result:?}"
    );
    assert_eq!(result.unwrap().tmux_window, test_tmux_window("task-42"));

    script.assert_matches(&mock.recorded_calls());
}

#[test]
fn resume_agent_has_window_runner_error_falls_back_to_creating_a_window() {
    // When runner.run() itself returns Err (e.g. tmux not installed), has_window
    // propagates the error and resume_agent falls back to its pre-existing
    // unconditional-create behaviour rather than treating the task as reattached.
    let mock = MockProcessRunner::new(vec![
        Err(anyhow::anyhow!("tmux not installed")), // has_window → runner error
        MockProcessRunner::ok(),                    // tmux list-windows (duplicate-name check)
        MockProcessRunner::ok(),                    // tmux new-window
        MockProcessRunner::ok(),                    // tmux set-option @dispatch_dir
        MockProcessRunner::ok(),                    // tmux set-hook
        MockProcessRunner::ok(),                    // tmux send-keys -l
        MockProcessRunner::ok(),                    // tmux send-keys Enter
        MockProcessRunner::ok_with_stdout(COMPANION_PANE_ID), // tmux split-window
        MockProcessRunner::ok(),                    // tmux set-option (companion role)
    ]);

    let result = resume_agent(TaskId(42), "/repo/.worktrees/42-fix-bug", &mock);
    assert!(
        result.is_ok(),
        "a has_window query failure should fall back to creating a new window: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// toggle_agent_tree_pane
// ---------------------------------------------------------------------------

#[test]
fn toggle_agent_tree_pane_is_noop_for_non_task_window() {
    let mock = MockProcessRunner::new(vec![]);
    toggle_agent_tree_pane(&test_tmux_window("TUI"), &mock).unwrap();
    assert_eq!(mock.recorded_calls().len(), 0, "should issue no tmux calls");
}

#[test]
fn toggle_agent_tree_pane_hides_when_companion_pane_present() {
    // The companion pane's id deliberately doesn't look like a positional
    // index, proving the kill target comes from the discovered pane id, not
    // an assumed index.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%3 \n%77 agent_tree\n"), // list-panes
        MockProcessRunner::ok(),                                     // kill-pane
    ]);
    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].1, vec!["kill-pane", "-t", "%77"]);
}

#[test]
fn toggle_agent_tree_pane_shows_when_no_companion_pane() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%3 \n"), // list-panes: single pane, unmarked
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options @dispatch_dir
        MockProcessRunner::ok_with_stdout(COMPANION_PANE_ID), // split-window
        MockProcessRunner::ok(),                     // set-option: the role marker
    ]);
    toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[2].0, "tmux");
    assert_eq!(calls[2].1[0], "split-window");
    assert!(
        calls[2].1.iter().any(|a| a == "30%"),
        "companion pane should use the 30% size, got: {:?}",
        calls[2].1
    );
    assert_eq!(
        calls[2].1[calls[2].1.len() - 3..],
        vec![
            "dispatch".to_string(),
            "agent-tree".to_string(),
            "42".to_string(),
        ],
        "companion pane should run `dispatch agent-tree <task_id>`, got: {:?}",
        calls[2].1
    );
    // The toggle holds only a window name, so it recovers the worktree from
    // @dispatch_dir and names it as the pane's start directory — which is what
    // keeps the correction hook from respawning the pane it just created.
    assert!(
        calls[2].1.windows(2).any(|w| w == ["-c", "/wt"]),
        "expected -c /wt in: {:?}",
        calls[2].1
    );
}

/// A failed kill of the TREE is what the toggle's caller can see — the pane
/// stayed and the key looked like it did nothing — so it must reach them rather
/// than being logged away.
#[test]
fn toggle_propagates_a_failed_kill_of_the_tree_pane() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n"),
        MockProcessRunner::fail("no such pane"),
    ])
    .with_windows(&["task-42"]);

    let err = toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap_err();

    assert!(format!("{err:#}").contains("no such pane"), "got {err:#}");
}

/// ...but a stray diff pane is not worth failing a toggle over. The tree is
/// going either way.
#[test]
fn toggle_survives_a_failed_kill_of_the_diff_pane() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n%3 diff\n"),
        MockProcessRunner::fail("no such pane"), // the diff pane
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options
        MockProcessRunner::ok(),                 // the tree
    ])
    .with_windows(&["task-42"]);

    assert!(toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).is_ok());
}

/// The orphan resync used to leave behind. After `swap-pane` rewrites a
/// window's task identity, the diff pane is still rendering the PREVIOUS
/// occupant's files against the previous worktree's open set — and nothing else
/// would ever retire it, because both lifecycle paths look the window up by the
/// TREE's role and would find the fresh one this call spawns.
#[test]
fn resync_retires_the_diff_pane_along_with_the_stale_tree() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n%3 diff\n"),
        MockProcessRunner::ok(), // kill-pane: the diff pane
        MockProcessRunner::ok(), // kill-pane: the stale tree
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options: clear the open set
        MockProcessRunner::ok_with_stdout(b"/wt\n"), // show-options: the respawn's cwd
        MockProcessRunner::ok_with_stdout(b"%9\n"), // split-window
        MockProcessRunner::ok(), // set-option
    ])
    .with_windows(&["task-42"]);

    resync_agent_tree_pane(&test_tmux_window("task-42"), &mock);

    let calls = mock.recorded_calls();
    let killed: Vec<&str> = calls
        .iter()
        .filter(|(_, args)| args[0] == "kill-pane")
        .map(|(_, args)| args[2].as_str())
        .collect();
    assert_eq!(killed, vec!["%3", "%2"], "calls: {calls:?}");
    assert!(
        calls.iter().any(|(_, args)| args[0] == "split-window"),
        "a fresh tree must still be spawned; calls: {calls:?}"
    );
}

/// With no tree pane there is nothing to resync — the window never had one, or
/// the user hid it — so nothing is killed and nothing is spawned.
#[test]
fn resync_is_a_noop_for_a_window_with_no_tree_pane() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%1 \n")])
        .with_windows(&["task-42"]);

    resync_agent_tree_pane(&test_tmux_window("task-42"), &mock);

    assert_eq!(mock.recorded_calls().len(), 1, "lookup only");
}

#[test]
fn toggle_agent_tree_pane_propagates_list_panes_query_failure() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no such window")]);
    let err = toggle_agent_tree_pane(&test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(err.to_string().contains("list-panes failed"), "got: {err}");
}

/// Assert one spawn site's `tmux send-keys` payload carries both spawn flags.
///
/// Asserting on the *payload* rather than on `DISPATCH_PLUGIN_DIR` is the point:
/// a new spawn site that built its `claude` command line without interpolating
/// the constant would satisfy any assertion about the constant itself.
fn assert_spawn_flags(site: &str, payload: &str) {
    assert!(
        payload.contains("--plugin-dir ~/.claude/plugins/local/dispatch"),
        "{site} must spawn claude with the dispatch plugin dir, got: {payload}"
    );
    assert!(
        payload.contains("--settings ~/.claude/dispatch-statusline.json"),
        "{site} must spawn claude with the statusline settings overlay, got: {payload}"
    );
}

#[test]
fn all_spawn_sites_inject_the_statusline_settings_file() {
    // Every dispatch-spawned session must report budget windows, so the
    // --settings overlay has to be on the agent and resume command lines
    // alike. See docs/specs/dispatch.allium: TokenBudgetIndicator
    // and StatusLineDecorator. `claude` also refuses to start at all when that
    // settings file is absent, so a site that dropped the flag would spawn a
    // session with no budget reporting, and one that kept it while the file went
    // missing would spawn no session at all.
    //
    // These two are the whole set: `DISPATCH_PLUGIN_DIR` is interpolated in
    // exactly two places in src/dispatch/agents.rs — dispatch_with_prompt (all
    // agent dispatches funnel through it) and resume_agent.

    // 1. dispatch_agent -> dispatch_with_prompt
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    // Scripted: this vector previously omitted the `rev-list` that
    // `select_start_point` issues on the reuse path, so every later response was
    // consumed one step early and the split-window call ran off the end.
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();
    assert_spawn_flags(
        "dispatch_agent",
        &find_call_arg(
            &mock.recorded_calls(),
            script.index_of(Step::SendKeysLiteral),
            "claude",
        ),
    );

    // 2. resume_agent
    let (_resume_dir, worktree_path) = make_test_repo();
    let resume_script = DispatchScript::resume();
    let mock = resume_script.runner();
    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();
    assert_spawn_flags(
        "resume_agent",
        &find_call_arg(
            &mock.recorded_calls(),
            resume_script.index_of(Step::SendKeysLiteral),
            "claude",
        ),
    );
}

#[test]
fn spawn_constant_has_exactly_one_space_between_flags() {
    // A substring `.contains()` check only anchors on what comes AFTER
    // "--settings", so it can't catch a doubled space, a missing space, or a
    // reordering before that point — exactly the whitespace-swallowing hazard
    // a Rust string-literal line-continuation (`\`) can silently introduce.
    // Assert the full exact value so any such regression fails here instead
    // of shipping a broken `claude` command line to `tmux send-keys`.
    assert_eq!(
        crate::dispatch::prompts::DISPATCH_PLUGIN_DIR,
        "--plugin-dir ~/.claude/plugins/local/dispatch --settings ~/.claude/dispatch-statusline.json",
        "spawn constant must be exactly this string, with exactly one space between \
         the --plugin-dir and --settings flags — a doubled/missing space here breaks \
         argument splitting on the `claude` command line sent through tmux send-keys"
    );
}

#[test]
fn spawn_constant_contains_no_whitespace_hazard() {
    // The constant is interpolated into a shell command string sent through
    // tmux send_keys, so it must contain only fixed literal paths. A runtime
    // path here would break on any $HOME containing a space.
    for token in crate::dispatch::prompts::DISPATCH_PLUGIN_DIR.split_whitespace() {
        assert!(
            !token.contains('$'),
            "no runtime interpolation allowed in the spawn constant: {token}"
        );
    }
}

// ---------------------------------------------------------------------------
// Caller identity at the launch — AgentCarriesItsOwnCallerIdentity
// (docs/specs/dispatch.allium)
//
// The headersHelper cannot identify a dispatched agent: Claude Code runs a
// user-global helper from its own config directory, so `dispatch caller-headers`
// never sees the worktree and answers "session" every time. The launcher says
// it instead, through a per-task MCP config named on the `claude` command line.
// These assert the launch carries it; the file's own contents and placement are
// covered in `super::caller_identity`.
// ---------------------------------------------------------------------------

#[test]
fn dispatch_launches_claude_with_the_per_task_mcp_config() {
    let (dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let (_worktree, admin) = make_linked_worktree(dir.path(), "42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script
        .runner()
        .with_claude_json(claude_json_with_dispatch_entry(dir.path()));
    let task = make_task(&repo_path);

    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let launch = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        launch.contains(&format!(
            "--mcp-config {}",
            admin.join("dispatch-mcp.json").display()
        )),
        "the launch must name the task's own MCP config, got: {launch}"
    );
    // Asserted on the composed command, not only on the flag builder: strict
    // mode could be added to this format string without the unit test noticing.
    assert!(
        !launch.contains("--strict-mcp-config"),
        "strict mode strips every other MCP server the operator configured, got: {launch}"
    );
}

#[test]
fn resume_launches_claude_with_the_per_task_mcp_config() {
    // Resume is the one launch path that never provisions, so it writes the
    // config itself rather than inheriting one.
    let dir = tempfile::TempDir::new().unwrap();
    let (worktree_path, admin) = make_linked_worktree(dir.path(), "42-fix-bug");
    let script = DispatchScript::resume();
    let mock = script
        .runner()
        .with_claude_json(claude_json_with_dispatch_entry(dir.path()));

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let launch = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        launch.contains(&format!(
            "--mcp-config {}",
            admin.join("dispatch-mcp.json").display()
        )),
        "resume must carry caller identity too, got: {launch}"
    );
}

#[test]
fn a_dispatch_that_cannot_write_the_config_launches_without_the_flag() {
    // Degrading to the old behaviour (no caller identity) is acceptable.
    // Naming a file that does not exist is not — it breaks the launch itself.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);

    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let launch = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        !launch.contains("--mcp-config"),
        "no config written means no flag, got: {launch}"
    );
}

// ---------------------------------------------------------------------------
// The prompt is an operand, not a flag value —
// PromptIsSeparatedFromTheLaunchFlags (docs/specs/dispatch.allium)
// ---------------------------------------------------------------------------

/// Executed rather than string-compared, on the same terms as
/// `claude_quoted_survives_the_launcher_command_shape` in `src/process.rs`: the
/// hazard is in how a real shell splits this command and how a real CLI then
/// parses the argv it produces, and neither is visible in the format string.
///
/// `--mcp-config` is variadic (`<configs...>`) — it consumes every following
/// word up to the next `-`-prefixed one. `agent_launch_flags` ends with it, so
/// without the separator the prompt becomes a second MCP configuration, Claude
/// Code resolves it as a file path relative to the worktree, and the launch
/// fails before the agent ever starts.
#[test]
fn the_prompt_reaches_claude_as_an_operand_not_as_a_flag_value() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("claude-stub");
    std::fs::write(&stub, "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\"; done\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let prompt = "This worktree was reused from a previous attempt";
    std::fs::write(dir.path().join(".claude-prompt"), prompt).unwrap();

    // The exact flag tail `agent_launch_flags` produces: the variadic one last.
    let cmd = prompt_launch_command(
        &crate::process::shell_quote(&stub.to_string_lossy()),
        "--name task-42 --mcp-config /tmp/dispatch-mcp.json",
    );
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(&cmd)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "launcher command failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let argv: Vec<&str> = stdout.lines().collect();

    assert_eq!(
        argv,
        vec![
            "--name",
            "task-42",
            "--mcp-config",
            "/tmp/dispatch-mcp.json",
            "--",
            prompt,
        ],
        "the prompt must arrive as one operand behind a `--` separator, got: {argv:?}"
    );
}

/// The separator has to survive into the command the launch actually sends, not
/// just the builder: the format string in `dispatch_with_prompt` is where it
/// would be dropped.
#[test]
fn the_dispatch_launch_command_separates_the_prompt_from_the_flags() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);

    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    let launch = find_call_arg(&calls, script.index_of(Step::SendKeysLiteral), "claude");
    assert!(
        launch.contains(r#" -- "$prompt""#),
        "the prompt must be passed behind a `--` separator, got: {launch}"
    );
}

// -----------------------------------------------------------------------
// WorktreeDirectoryMustNotSurviveTeardown (docs/specs/tasks.allium)
//
// Step 2 releases a DIRECTORY, not git's registration of one. These tests run
// against a real filesystem with a mocked `git`, because the behaviour under
// test is precisely what happens on disk *after* git has had its turn — the
// three outcomes probed in #4882, two of which leave the directory whole.
// -----------------------------------------------------------------------

/// A worktree directory on disk under `<repo>/.worktrees/<slug>`, holding the
/// gitignored build output that made the leak expensive. Returns the tempdir
/// guard, the repo path and the worktree path.
fn leftover_worktree(slug: &str) -> (tempfile::TempDir, String, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let worktree = dir.path().join(".worktrees").join(slug);
    std::fs::create_dir_all(worktree.join("target/debug")).unwrap();
    std::fs::write(worktree.join("target/debug/huge.rlib"), b"build output").unwrap();
    std::fs::write(worktree.join("CLAUDE.md"), b"tracked file").unwrap();
    (dir, repo, worktree)
}

/// Drop-restores a directory's permissions.
///
/// The restore must not be a line at the end of the test: an assertion that
/// panics between the `chmod` and that line leaves a directory `TempDir`'s own
/// drop cannot remove, silently leaking a temp directory per failing run.
struct PermGuard {
    dir: std::path::PathBuf,
}

impl Drop for PermGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o755));
    }
}

/// Strip `mode` bits from `dir`, and confirm the process is actually bound by
/// the result. `None` means the caller must skip: root ignores directory
/// permissions, so the scenario cannot be staged there at all.
///
/// Probing rather than reading the uid asks the same question without a libc
/// dependency, and asks it of the actual filesystem — a mount option could
/// defeat the permission too.
///
/// Under CI a skip is a hard failure, for the reason
/// `tests/tmux_harness/mod.rs::tmux_available_or_skip` gives: `eprintln!` is
/// swallowed by the default harness, so a silent skip would let these tests
/// quietly stop covering anything while still reporting green.
#[must_use]
fn deny_access_or_skip(
    dir: &std::path::Path,
    mode: u32,
    probe: &std::path::Path,
) -> Option<PermGuard> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    let guard = PermGuard {
        dir: dir.to_path_buf(),
    };
    if std::fs::symlink_metadata(probe).is_ok() && std::fs::write(dir.join(".probe"), b"").is_ok() {
        let _ = std::fs::remove_file(dir.join(".probe"));
        assert!(
            std::env::var_os("CI").is_none(),
            "this process is not bound by directory permissions (running as \
             root?), so these teardown tests cannot be staged. Refusing to \
             skip and report green in CI."
        );
        eprintln!("skipping: this process is not bound by directory permissions");
        return None;
    }
    Some(guard)
}

#[test]
fn teardown_deletes_the_directory_git_left_behind() {
    // git reports success but the directory is still there (the observed
    // outcome when its own recursive delete does not get everything).
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(
        !worktree.exists(),
        "teardown must leave nothing at the worktree path"
    );
}

#[test]
fn teardown_deletes_the_directory_when_git_says_it_is_not_a_working_tree() {
    // The silent leak: git's admin record is gone, so git removes NOTHING and
    // fails with "is not a working tree". That is a released REGISTRATION, not
    // a released directory — teardown still owes the delete.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: '.worktrees/42-fix-bug' is not a working tree"),
        MockProcessRunner::ok(), // git worktree prune
        MockProcessRunner::ok(), // git branch -D, best-effort
    ]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(
        !worktree.exists(),
        "an already-unregistered worktree must still have its directory removed"
    );
}

#[test]
fn teardown_of_an_absent_directory_is_success() {
    // "Absent" is the success condition, so a path that was never there is
    // already satisfied. This is also what keeps every mock-only teardown test
    // in this file — which names paths that do not exist — passing.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let worktree = dir.path().join(".worktrees").join("42-fix-bug");
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]);

    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(!worktree.exists());
}

#[test]
fn teardown_unlinks_symlinks_inside_the_worktree_without_touching_their_targets() {
    // SymlinksAreUnlinkedNeverFollowed: a worktree may link out into the
    // operator's wider filesystem, and none of those targets belong to the task.
    let (dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let outside_dir = dir.path().join("outside");
    std::fs::create_dir_all(&outside_dir).unwrap();
    let outside_file = outside_dir.join("precious.txt");
    std::fs::write(&outside_file, b"keep me").unwrap();
    std::os::unix::fs::symlink(&outside_dir, worktree.join("linked-dir")).unwrap();
    std::os::unix::fs::symlink(&outside_file, worktree.join("linked-file")).unwrap();

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]);
    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(!worktree.exists(), "the worktree itself must be gone");
    assert!(
        outside_file.exists(),
        "a symlink's target outside the worktree must survive teardown"
    );
}

#[test]
fn teardown_unlinks_a_symlinked_worktree_path_without_deleting_its_target() {
    // The worktree path is ITSELF a symlink. Only the link is removed; the
    // recursive delete never descends through it.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let target = dir.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("precious.txt"), b"keep me").unwrap();
    std::fs::create_dir_all(dir.path().join(".worktrees")).unwrap();
    let worktree = dir.path().join(".worktrees").join("42-fix-bug");
    std::os::unix::fs::symlink(&target, &worktree).unwrap();

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]);
    teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap();

    assert!(
        std::fs::symlink_metadata(&worktree).is_err(),
        "the symlink at the worktree path must be unlinked"
    );
    assert!(
        target.join("precious.txt").exists(),
        "the symlink's target must survive teardown"
    );
}

#[test]
fn teardown_refuses_to_delete_a_path_outside_the_worktrees_directory() {
    // DeletionIsBoundedToTheWorktreesDirectory: a mis-set pointer is surfaced,
    // never acted on.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("main.rs"), b"fn main() {}").unwrap();

    // One response: the refusal returns before `git branch -D` ever runs.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let failure = teardown_task(&repo, Some(src.to_str().unwrap()), None, &mock).unwrap_err();

    assert_eq!(
        failure.worktree_left.as_deref(),
        Some(src.to_str().unwrap()),
        "the refused path must still be reported as left on disk"
    );
    assert!(
        src.join("main.rs").exists(),
        "a path outside .worktrees must not be deleted"
    );
}

#[test]
fn teardown_refuses_to_delete_the_worktrees_directory_itself() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let worktrees = dir.path().join(".worktrees");
    std::fs::create_dir_all(worktrees.join("7-other-task")).unwrap();

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let failure = teardown_task(&repo, Some(worktrees.to_str().unwrap()), None, &mock).unwrap_err();

    assert!(failure.worktree_left.is_some());
    assert!(
        worktrees.join("7-other-task").exists(),
        "another task's live worktree must not be collateral"
    );
}

#[test]
fn teardown_refuses_a_traversal_out_of_the_worktrees_directory() {
    // The bound is lexical, so `..` must be resolved as text before the test —
    // otherwise the prefix check passes on a path that escapes.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("main.rs"), b"fn main() {}").unwrap();
    let escaping = format!("{repo}/.worktrees/../src");

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let failure = teardown_task(&repo, Some(&escaping), None, &mock).unwrap_err();

    assert!(failure.worktree_left.is_some());
    assert!(
        src.join("main.rs").exists(),
        "a `..` traversal must not escape the bound"
    );
}

#[test]
fn a_failed_delete_fails_teardown_and_reports_the_path() {
    // A failed 2b fails step 2 exactly as a failed 2a does, so
    // WorktreeReleaseIsGated's (a), (b) and (c) apply unchanged.
    let (_dir, repo, worktree) = leftover_worktree("42-fix-bug");
    let parent = worktree.parent().unwrap().to_path_buf();
    // Unlink permission lives on the PARENT directory, so this makes the
    // recursive delete of `worktree` fail without making it unreadable.
    let Some(_perm) = deny_access_or_skip(&parent, 0o555, &worktree) else {
        return;
    };

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let failure = teardown_task(&repo, Some(worktree.to_str().unwrap()), None, &mock).unwrap_err();
    assert_eq!(
        failure.worktree_left.as_deref(),
        Some(worktree.to_str().unwrap()),
        "a failed delete must report the worktree as still on disk"
    );
    assert!(
        format!("{failure:#}").contains("failed to delete leftover worktree"),
        "expected the delete failure in the error chain, got: {failure:#}"
    );
}

#[test]
fn provision_worktree_refuses_a_base_that_resolves_neither_remotely_nor_locally() {
    // UnresolvableBaseIsRefusedByName (docs/specs/dispatch.allium). The
    // 404-class branch picks local <base> on no evidence that it exists, so a
    // task whose base_branch names a branch the repo never had — "main"
    // against a "master" repo — reached `git worktree add` and died on git's
    // own "fatal: invalid reference: main". Probe it, and refuse by name.
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: couldn't find remote ref main"), // git fetch
        MockProcessRunner::ok(),                  // git remote get-url origin
        MockProcessRunner::fail_with_code(2, ""), // git ls-remote --exit-code (404)
        MockProcessRunner::fail_with_code(128, ""), // git rev-parse --verify main (absent)
    ]);

    let task = make_task(&repo_path);
    let err = provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap_err();

    let msg = format!("{err:#}");
    assert!(
        msg.contains("origin/main"),
        "the refusal must name the remote ref it tried, got: {msg}"
    );
    assert!(
        msg.contains("main"),
        "the refusal must name the base branch, got: {msg}"
    );

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.contains(&"worktree".to_string()))),
        "must not hand an unresolvable ref to `git worktree add`: {calls:?}"
    );
    assert!(
        calls.iter().all(|(prog, _)| prog != "tmux"),
        "must not open a tmux window for a dispatch that cannot be based on anything: {calls:?}"
    );
}

#[test]
fn provision_worktree_does_not_probe_a_start_point_the_fetch_already_proved() {
    // The other half of UnresolvableBaseIsRefusedByName: the probe is scoped
    // to the one unproven choice. A successful fetch proves origin/<base>, so
    // spending a further subprocess to re-establish it on the hot path of
    // every dispatch is exactly what the spec's proof structure rules out.
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),                    // git fetch origin main
        MockProcessRunner::fail_with_code(128, ""), // git rev-list (local main absent)
        MockProcessRunner::ok(),                    // git worktree prune
        MockProcessRunner::ok(),                    // git worktree add
        MockProcessRunner::ok(),                    // tmux list-windows
        MockProcessRunner::ok(),                    // tmux new-window
        MockProcessRunner::ok(),                    // tmux set-option @dispatch_dir
        MockProcessRunner::ok(),                    // tmux set-hook
    ]);

    let task = make_task(&repo_path);
    provision_worktree(
        &task,
        &mock,
        Some(BaseRef::Branch("main")),
        SUBPROCESS_TIMEOUT,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.contains(&"rev-parse".to_string()))),
        "a fetched origin/<base> is proven by the fetch; no probe belongs here: {calls:?}"
    );
}
