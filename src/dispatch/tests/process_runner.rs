use super::*;

// --- ProcessRunner-based tests ---

#[test]
fn dispatch_reuses_existing_worktree() {
    // Pre-create worktree dir — simulates a re-dispatch where the worktree
    // already exists on disk from a previous dispatch cycle.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
        "git worktree add should be skipped for existing worktree"
    );
    assert_eq!(calls[script.index_of(Step::NewWindow)].0, "tmux");
    assert_eq!(calls[script.index_of(Step::NewWindow)].1[0], "new-window");
    assert_eq!(calls[script.index_of(Step::SetDispatchDir)].0, "tmux");
    assert_eq!(
        calls[script.index_of(Step::SetDispatchDir)].1[0],
        "set-option"
    );
    assert_eq!(calls[script.index_of(Step::SetSplitHook)].0, "tmux");
    assert_eq!(calls[script.index_of(Step::SetSplitHook)].1[0], "set-hook");
}

#[test]
fn dispatch_reused_worktree_prompt_carries_the_reuse_preamble() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let prompt = std::fs::read_to_string(worktree_dir.join(".claude-prompt")).unwrap();
    assert!(
        prompt.contains("reused from a previous attempt"),
        "got: {prompt}"
    );
    assert!(prompt.contains("git rebase origin/main"), "got: {prompt}");
    assert!(prompt.contains("Always work from this worktree folder"));
}

#[test]
fn dispatch_sends_claude_command() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    // The literal send-keys call carries the claude invocation
    assert!(
        calls[script.index_of(Step::SendKeysLiteral)]
            .1
            .iter()
            .any(|a| a.contains("claude")),
        "send-keys should include claude"
    );
}

#[test]
fn dispatch_agent_splits_agent_tree_companion_pane_after_send_keys() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let script = DispatchScript::dispatch();
    let mock = script.runner();

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, &LearningInjections::default()).unwrap();

    let calls = mock.recorded_calls();
    // The split, not the last call: the role marker written on the pane it
    // returns follows it (Step::CompanionRoleMark).
    let split = &calls[script.index_of(Step::CompanionSplit)];
    assert_eq!(split.0, "tmux");
    assert_eq!(split.1[0], "split-window");
    assert!(
        split.1.iter().any(|a| a == "30%"),
        "companion pane should use the 30% size, got: {:?}",
        split.1
    );
    assert_eq!(
        split.1[split.1.len() - 3..],
        vec![
            "dispatch".to_string(),
            "agent-tree".to_string(),
            "42".to_string(),
        ],
        "companion pane should run `dispatch agent-tree <task_id>`, got: {:?}",
        split.1
    );
}

#[test]
fn dispatch_agent_succeeds_even_if_companion_pane_split_fails() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = DispatchScript::dispatch()
        .fails_at(Step::CompanionSplit)
        .runner();

    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &LearningInjections::default());
    assert!(
        result.is_ok(),
        "a failed companion-pane split must not fail dispatch: {result:?}"
    );
}

/// One row of the launcher table below: a name, and a thunk that drives that
/// launcher end to end and hands back the `claude` command it sent.
type LaunchCase = (&'static str, fn() -> String);

/// Drive one `dispatch_with_prompt` launcher through the standard mock script
/// and return the `claude` command it sent to tmux.
fn dispatched_claude_cmd(launch: impl FnOnce(&Task, &MockProcessRunner)) -> String {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let script = DispatchScript::dispatch();
    let mock = script.runner();
    let task = make_task(&repo_path);

    launch(&task, &mock);

    find_call_arg(
        &mock.recorded_calls(),
        script.index_of(Step::SendKeysLiteral),
        "claude",
    )
}

/// `EveryTaskAgentLaunchesInAutoMode` in `docs/specs/dispatch.allium`. The
/// guarantee is "no exceptions", so the assertion is made once, over every
/// launcher, rather than per-launcher: a variant that reintroduces a permission
/// flag fails here even if it ships with a passing test of its own. Adding a
/// launcher means adding a row.
#[test]
fn no_task_agent_passes_a_permission_mode_flag() {
    let launchers: [LaunchCase; 4] = [
        ("dispatch_agent", || {
            dispatched_claude_cmd(|task, mock| {
                dispatch_agent(task, mock, None, &LearningInjections::default()).unwrap();
            })
        }),
        ("research_agent", || {
            dispatched_claude_cmd(|task, mock| {
                research_agent(task, mock, None).unwrap();
            })
        }),
        ("quick_dispatch_agent", || {
            dispatched_claude_cmd(|task, mock| {
                quick_dispatch_agent(task, mock, None, &LearningInjections::default()).unwrap();
            })
        }),
        // Not a dispatch_with_prompt caller — resume_agent hand-builds its own
        // claude command, which is exactly why it belongs in this table: the
        // parameter removal cannot reach it, so only an assertion can.
        ("resume_agent", || {
            let (_dir, worktree_path) = make_test_repo();
            let script = DispatchScript::resume();
            let mock = script.runner();
            resume_agent(TaskId(42), &worktree_path, &mock).unwrap();
            find_call_arg(
                &mock.recorded_calls(),
                script.index_of(Step::SendKeysLiteral),
                "claude",
            )
        }),
    ];

    for (name, launch) in launchers {
        let claude_cmd = launch();
        assert!(
            !claude_cmd.contains("--permission-mode"),
            "{name} must launch in auto mode with no --permission-mode flag, got: {claude_cmd}"
        );
    }
}
