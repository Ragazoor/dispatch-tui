use super::*;
use crate::models::test_tmux_window;

// --- window_target scaffolding ---
//
// Every helper that takes a window *name* resolves it to a pane ID first, so
// these tests declare the fake server's windows with `with_windows` and
// assert the resolved `-t %N` target via `pane_id_of`. `with_windows` answers
// the lookup out of band, so response queues and `calls[N]` indices stay
// about the operation — see `MockProcessRunner::with_windows`.
//
// What these tests can and cannot do: they assert the argv we hand tmux, not
// what tmux does with it. The pre-fix versions pinned the vulnerable
// `-t task-42` argv and stayed green throughout. The behavioural coverage —
// that an absent name cannot reach a prefix-matched sibling — is in
// tests/tmux_window_targets.rs, against a real server.

/// The three-window topology that exposes the bug: `task-4`'s name is a
/// prefix of `task-42`'s.
const COLLIDING: [&str; 3] = ["dispatch", "task-4", "task-42"];

#[test]
fn has_window_finds_match_in_output() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"main\ntask-42\nother-window\n",
    )]);
    let result = has_window(&test_tmux_window("task-42"), &mock).unwrap();
    assert!(result);
}

#[test]
fn has_window_no_match() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"main\nother-window\n",
    )]);
    let result = has_window(&test_tmux_window("task-42"), &mock).unwrap();
    assert!(!result);
}

#[test]
fn has_window_exact_match_not_prefix() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"task-42\n")]);
    let result = has_window(&test_tmux_window("task-4"), &mock).unwrap();
    assert!(!result);
}

// --- ProcessRunner-based tests ---

use crate::process::MockProcessRunner;

#[test]
fn new_window_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // new-window
    ]);
    new_window(&test_tmux_window("task-42"), "/some/path", &[], &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(
        calls[1].1,
        vec!["new-window", "-d", "-n", "task-42", "-c", "/some/path"]
    );
}

#[test]
fn new_window_passes_env_vars_as_dash_e_flags() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // new-window
    ]);
    new_window(
        &test_tmux_window("task-42"),
        "/some/path",
        &[("RUSTC_WRAPPER", "sccache")],
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(
        calls[1].1,
        vec![
            "new-window",
            "-d",
            "-n",
            "task-42",
            "-c",
            "/some/path",
            "-e",
            "RUSTC_WRAPPER=sccache"
        ]
    );
}

// A hung tmux server must not park the calling thread forever (#4202):
// this is one of `provision_worktree`'s `post_add` calls.
#[test]
fn new_window_is_bounded_by_subprocess_timeout() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // new-window
    ]);
    new_window(&test_tmux_window("task-42"), "/some/path", &[], &mock).unwrap();
    assert_eq!(
        *mock.recorded_timeouts().last().unwrap(),
        Some(crate::process::SUBPROCESS_TIMEOUT)
    );
}

/// dispatch.allium's `TmuxWindowNamesAreUnique`: creating a window under a
/// name a live window already holds is refused, not reattached to. The
/// dispatch that reaches here would otherwise hand its prompt to a session
/// it knows nothing about, and the duplicate makes every later
/// name-targeted operation on the task ambiguous until a human intervenes.
#[test]
fn new_window_refuses_a_name_a_live_window_already_holds() {
    let mock = MockProcessRunner::new(vec![
        // has_window: a window already answers to `task-42`.
        MockProcessRunner::ok_with_stdout(b"board\ntask-42\n"),
    ]);
    let err = new_window(&test_tmux_window("task-42"), "/some/path", &[], &mock).unwrap_err();
    assert!(
        err.to_string()
            .contains("a tmux window named 'task-42' already exists"),
        "got: {err}"
    );
    assert!(
        !mock
            .recorded_calls()
            .iter()
            .any(|(_, args)| args.contains(&"new-window".to_string())),
        "new-window must not be issued"
    );
}

/// The two properties below belong to the shared guard, not to any one of
/// its four callers, so they are asserted once here rather than re-tested
/// per entry point. What each caller owns is the `_refuses_a_name_...`
/// test alongside it: that the guard is wired in at all, and that the
/// caller's own verb is not issued once it fires.
///
/// An existence check, not a prefix one: a live `task-420` must not block
/// the name `task-42`.
#[test]
fn refuse_duplicate_window_name_allows_a_name_only_a_prefix_of_a_live_one() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"task-420\n")]);
    refuse_duplicate_window_name(&test_tmux_window("task-42"), &mock).unwrap();
}

/// A query that could not answer at all reads as "no live window", so the
/// caller proceeds rather than being blocked by a tmux hiccup. Driven with
/// a runner-level `Err`: a nonzero exit is already `Ok(false)` inside
/// `has_window`, which would exercise a different default.
#[test]
fn refuse_duplicate_window_name_proceeds_when_the_query_cannot_answer() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("tmux: command not found"))]);
    refuse_duplicate_window_name(&test_tmux_window("task-42"), &mock).unwrap();
}

/// The pop-out editor's creator is bound by the same invariant. Its names
/// are nanosecond-derived so a collision is vanishingly unlikely, but the
/// guarantee is stated over every name-assigning operation, and this is
/// the one that would otherwise be the remaining hole in it.
#[test]
fn new_window_running_refuses_a_name_a_live_window_already_holds() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"dispatch-editor-7\n",
    )]);
    let err = new_window_running(
        &test_tmux_window("dispatch-editor-7"),
        "/tmp",
        &["vim", "/tmp/x"],
        &mock,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("a tmux window named 'dispatch-editor-7' already exists"),
        "got: {err}"
    );
    assert!(
        !mock
            .recorded_calls()
            .iter()
            .any(|(_, args)| args.contains(&"new-window".to_string())),
        "new-window must not be issued"
    );
}

/// The empty-command guard runs before the existence query: an invalid
/// call is rejected on its own terms, without a subprocess.
#[test]
fn new_window_running_rejects_an_empty_command_before_querying_tmux() {
    let mock = MockProcessRunner::new(vec![]);
    assert!(new_window_running(&test_tmux_window("w"), "/tmp", &[], &mock).is_err());
    assert!(mock.recorded_calls().is_empty());
}

#[test]
fn new_window_running_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // new-window
    ]);
    new_window_running(
        &test_tmux_window("dispatch-edit-1"),
        "/home/u",
        &["vim", "/tmp/foo.md"],
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(
        calls[1].1,
        vec![
            "new-window",
            "-d",
            "-n",
            "dispatch-edit-1",
            "-c",
            "/home/u",
            "--",
            "vim",
            "/tmp/foo.md"
        ]
    );
}

#[test]
fn new_window_running_keeps_argv_elements_separate() {
    // A path with spaces must be passed as its own argv element, not
    // joined into a single shell string. This is why we use the `--`
    // exec form rather than `send-keys` with a concatenated command.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // new-window
    ]);
    new_window_running(
        &test_tmux_window("edit-1"),
        "/tmp",
        &["vim", "/tmp/dir with spaces/file.md"],
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls[1].1.last().unwrap(), "/tmp/dir with spaces/file.md");
    // and the preceding element is the exec separator + program
    assert_eq!(calls[1].1[calls[1].1.len() - 3], "--");
    assert_eq!(calls[1].1[calls[1].1.len() - 2], "vim");
}

#[test]
fn new_window_running_rejects_empty_command() {
    let mock = MockProcessRunner::new(vec![]);
    let err = new_window_running(&test_tmux_window("n"), "/tmp", &[], &mock).unwrap_err();
    assert!(err.to_string().contains("command must not be empty"));
}

#[test]
fn new_window_running_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),        // list-windows (duplicate-name check)
        MockProcessRunner::fail("bad"), // new-window
    ]);
    let err = new_window_running(&test_tmux_window("n"), "/tmp", &["vim", "f"], &mock).unwrap_err();
    assert!(err.to_string().contains("new-window failed"));
}

#[test]
fn has_window_returns_false_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no sessions")]);
    let result = has_window(&test_tmux_window("task-42"), &mock).unwrap();
    assert!(!result);
}

// --- has_window_or_assume_present ---

#[test]
fn has_window_or_assume_present_true_when_present() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"task-42\n")]);
    assert!(has_window_or_assume_present(
        &test_tmux_window("task-42"),
        &mock
    ));
}

#[test]
fn has_window_or_assume_present_false_when_absent() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"other\n")]);
    assert!(!has_window_or_assume_present(
        &test_tmux_window("task-42"),
        &mock
    ));
}

#[test]
fn has_window_or_assume_present_true_when_query_fails() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("tmux: command not found"))]);
    assert!(has_window_or_assume_present(
        &test_tmux_window("task-42"),
        &mock
    ));
}

// --- kill_window_if_present ---

// --- is_window_absent_error ---

/// The predicate the best-effort teardowns use to tell "already gone" from
/// "the kill failed". Driven through the real `kill_window` rather than
/// hand-built error strings, so it is pinned to the error `window_target`
/// actually produces — the whole reason matching the message is defensible
/// here.
#[test]
fn kill_window_on_an_absent_window_yields_an_absent_error() {
    // `with_queued_window_lookup` so the empty listing below really answers
    // the resolver; the default permissive lookup would invent a pane and
    // the kill would succeed.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"")])
        .with_queued_window_lookup();
    let err = kill_window(&test_tmux_window("gone-window"), &mock)
        .expect_err("an absent window must fail to resolve");
    assert!(
        is_window_absent_error(&err),
        "an absent window must be recognised as absent, got: {err:#}"
    );
}

/// The direction that matters for the demotion: a kill that was attempted
/// and failed must NOT be classified as absent, or a real leaked window
/// would be logged at debug and never seen.
#[test]
fn a_failed_kill_is_not_an_absent_error() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("server exited")])
        .with_windows(&["task-7"]);
    let err =
        kill_window(&test_tmux_window("task-7"), &mock).expect_err("the kill was scripted to fail");
    assert!(
        !is_window_absent_error(&err),
        "a genuine kill failure must not be mistaken for an absent window, got: {err:#}"
    );
}

#[test]
fn kill_window_if_present_kills_when_present() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-42\n"), // has_window
        MockProcessRunner::ok(),                         // kill-window
    ])
    .with_windows(&["task-42"]);
    kill_window_if_present(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].1[0], "kill-window");
}

#[test]
fn kill_window_if_present_skips_when_absent() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"other\n")]);
    kill_window_if_present(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "kill-window should not be called");
}

#[test]
fn kill_window_if_present_skips_and_succeeds_when_query_fails() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("tmux: command not found"))]);
    kill_window_if_present(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "kill-window should not be attempted");
}

#[test]
fn kill_window_if_present_propagates_kill_failure() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-42\n"), // has_window
        MockProcessRunner::fail("no such window"),       // kill-window fails
    ])
    .with_windows(&["task-42"]);
    let err = kill_window_if_present(&test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(err.to_string().contains("kill-window failed"), "got: {err}");
}

#[test]
fn has_window_queries_across_all_sessions() {
    // has_window is used for cross-session liveness checks (cleanup,
    // finish, editor, staleness) — without -a, list-windows scopes to the
    // current/attached session only and misses windows living in another
    // session, producing a false "not found".
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"task-42\n")]);
    let _ = has_window(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec!["list-windows", "-a", "-F", "#{window_name}"]
    );
}

#[test]
fn set_window_dispatch_dir_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&COLLIDING);
    set_window_dispatch_dir(&test_tmux_window("task-42"), "/some/path", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec![
            "set-option",
            "-w",
            "-t",
            &mock.pane_id_of("task-42"),
            "@dispatch_dir",
            "/some/path",
        ]
    );
}

// A hung tmux server must not park the calling thread forever (#4202):
// this is one of `provision_worktree`'s `post_add` calls.
#[test]
fn set_window_dispatch_dir_is_bounded_by_subprocess_timeout() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&COLLIDING);
    set_window_dispatch_dir(&test_tmux_window("task-42"), "/some/path", &mock).unwrap();
    assert_eq!(
        mock.recorded_timeouts(),
        vec![Some(crate::process::SUBPROCESS_TIMEOUT)]
    );
}

#[test]
fn set_window_dispatch_dir_detects_ambiguous_windows() {
    let mock = MockProcessRunner::new(vec![]).with_windows(&["task-42", "task-42"]);
    let err =
        set_window_dispatch_dir(&test_tmux_window("task-42"), "/some/path", &mock).unwrap_err();
    assert!(err.to_string().contains("multiple tmux windows"));
    assert!(
        mock.recorded_calls().is_empty(),
        "set-option must not be attempted for an ambiguous name"
    );
}

/// Pins the hook string, including its `-t #{pane_id}` target, its
/// already-in-the-worktree guard, and the quotes around the interpolated
/// directory. Note what this test can and cannot do: it proves the argv we
/// hand tmux, not what tmux then does with it. The *behaviour* — which pane
/// is corrected, which is left alone, and that nothing is typed anywhere —
/// is only observable against a real server, so it is asserted in
/// tests/tmux_split_hook.rs. A mock-level test of this hook once asserted
/// the untargeted string verbatim and stayed green while the board was being
/// typed into.
#[test]
fn ensure_split_hook_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    ensure_split_hook(&mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec![
            "set-hook",
            "after-split-window",
            "if-shell -F '#{&&:#{@dispatch_dir},#{!=:#{pane_start_path},#{@dispatch_dir}}}' \
                 'run-shell -bC \"respawn-pane -k -t #{pane_id} -c \\\"#{@dispatch_dir}\\\"\"'",
        ]
    );
}

// A hung tmux server must not park the calling thread forever (#4202):
// this is one of `provision_worktree`'s `post_add` calls.
#[test]
fn ensure_split_hook_is_bounded_by_subprocess_timeout() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    ensure_split_hook(&mock).unwrap();
    assert_eq!(
        mock.recorded_timeouts(),
        vec![Some(crate::process::SUBPROCESS_TIMEOUT)]
    );
}

/// The correction must never be delivered as synthesised input — see
/// split-pane.allium's `SplitDirectoryIsNeverKeystrokes`. Asserted here as
/// well as behaviourally, so that reintroducing `send-keys` fails at the
/// cheapest possible layer.
#[test]
fn ensure_split_hook_never_sends_keys() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    ensure_split_hook(&mock).unwrap();
    let hook = mock.recorded_calls()[0].1[2].clone();
    assert!(
        !hook.contains("send-keys"),
        "the split correction must not type at a pane: {hook}"
    );
}

#[test]
fn current_window_name_returns_trimmed_stdout() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"dispatch\n")]);
    let result = current_window_name(None, &mock).unwrap();
    assert_eq!(result, "dispatch");
}

#[test]
fn current_window_name_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"dispatch\n")]);
    current_window_name(None, &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1, vec!["display-message", "-p", "#W"]);
}

#[test]
fn current_window_name_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no session")]);
    assert!(current_window_name(None, &mock).is_err());
}

#[test]
fn rename_window_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // has_window: the new name is free
        MockProcessRunner::ok(), // rename-window
    ])
    .with_windows(&["dispatch", "task-42"]);
    rename_window("dispatch", &test_tmux_window("my-old-name"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(
        calls[1].1,
        vec![
            "rename-window",
            "-t",
            &mock.pane_id_of("dispatch"),
            "my-old-name",
        ]
    );
}

/// `-t` and the new name are adjacent arguments; only the target is
/// resolved. A resolver applied to the new name would reject every rename,
/// since the name being assigned does not exist yet — here
/// `brand-new-name` is not a declared window, so resolving it would fail.
///
/// The name is still *checked for existence* (the duplicate-name refusal
/// above), which is a different question: "is anything already called
/// this?" rather than "which pane does this name mean?".
#[test]
fn rename_window_does_not_resolve_the_new_name() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // has_window: the new name is free
        MockProcessRunner::ok(), // rename-window
    ])
    .with_windows(&["dispatch"]);
    rename_window("dispatch", &test_tmux_window("brand-new-name"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.last().unwrap().1.last().unwrap(), "brand-new-name");
}

/// `setup_tmux_for_tui` (src/runtime/mod.rs) renames by pane ID, and falls
/// back to `""` (tmux's "current window") when that lookup fails. Both are
/// already unambiguous and must reach tmux untouched.
#[test]
fn rename_window_passes_through_already_exact_targets() {
    for target in ["%7", ""] {
        // `with_queued_window_lookup` makes a resolution attempt observable:
        // it would consume the queued Ok and then panic for want of a second
        // response. Passing means no lookup happened at all.
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // has_window: the new name is free
            MockProcessRunner::ok(), // rename-window
        ])
        .with_queued_window_lookup();
        rename_window(target, &test_tmux_window("dispatch"), &mock).unwrap();
        let calls = mock.recorded_calls();
        // Two calls, not three: the existence check plus the rename. A
        // resolution of `target` would have consumed a third response.
        assert_eq!(calls.len(), 2, "no resolution for target {target:?}");
        assert_eq!(
            calls[1].1,
            vec!["rename-window", "-t", target, "dispatch"],
            "target {target:?} should pass through unchanged"
        );
    }
}

#[test]
fn rename_window_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),              // has_window: name is free
        MockProcessRunner::fail("no window"), // rename-window
    ])
    .with_windows(&["dispatch"]);
    assert!(rename_window("dispatch", &test_tmux_window("other"), &mock).is_err());
}

#[test]
fn rename_window_fails_when_target_window_is_absent() {
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-42"]);
    let err = rename_window("task-4", &test_tmux_window("renamed"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-4'"),
        "got: {err}"
    );
}

/// dispatch.allium's `TmuxWindowNamesAreUnique`: a rename that would leave
/// two live windows sharing a name is refused. The duplicate is not a
/// cosmetic problem — every later name-targeted operation on that task is
/// refused as ambiguous by [`window_target`] until a human closes one of
/// the two windows, so the rename that would create it must fail instead.
#[test]
fn rename_window_refuses_a_name_a_live_window_already_holds() {
    let mock = MockProcessRunner::new(vec![
        // has_window: a window already answers to `task-2`.
        MockProcessRunner::ok_with_stdout(b"board\ntask-2\ntask-3\n"),
    ])
    .with_windows(&["task-3"]);
    let err = rename_window("task-3", &test_tmux_window("task-2"), &mock).unwrap_err();
    assert!(
        err.to_string()
            .contains("a tmux window named 'task-2' already exists"),
        "got: {err}"
    );
    // The refusal must come *before* tmux is asked to rename anything.
    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|(_, args)| args.contains(&"rename-window".to_string())),
        "rename-window must not be issued, got: {calls:?}"
    );
}

/// The check is an existence check on the new name, not a prefix one: a
/// window named `task-42` must not block a rename to `task-4`.
#[test]
fn rename_window_allows_a_new_name_that_is_only_a_prefix_of_a_live_one() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-42\ntask-3\n"), // has_window
        MockProcessRunner::ok(),                                 // rename-window
    ])
    .with_windows(&["task-3"]);
    rename_window("task-3", &test_tmux_window("task-4"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.last().unwrap().1.last().unwrap(), "task-4");
}

/// A query that could not answer at all reads as "no windows" here, so
/// the rename is attempted rather than blocked by it. Driven with a
/// runner-level `Err` rather than a nonzero exit: a nonzero exit is
/// already `Ok(false)` inside `has_window`, so it would exercise that
/// default instead of this one.
#[test]
fn rename_window_proceeds_when_the_existence_query_cannot_answer() {
    let mock = MockProcessRunner::new(vec![
        Err(anyhow::anyhow!("tmux: command not found")), // has_window
        MockProcessRunner::ok(),                         // rename-window
    ])
    .with_windows(&["task-3"]);
    rename_window("task-3", &test_tmux_window("task-2"), &mock).unwrap();
    assert!(mock
        .recorded_calls()
        .iter()
        .any(|(_, args)| args.contains(&"rename-window".to_string())));
}

#[test]
fn bind_key_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    bind_key("space", "select-window -t dispatch", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec!["bind-key", "space", "select-window -t dispatch"]
    );
}

/// The store address is published on the board's own session, by exact
/// name, so every pane and window the board starts afterwards inherits it
/// (startup.allium: ConnectToTheStoreOnceTheHostIsNamed).
#[test]
fn set_session_environment_targets_the_session_exactly() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    set_session_environment(
        "dispatch",
        "DISPATCH_BOARD_STORE",
        "http://127.0.0.1:3000",
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec![
            "set-environment",
            "-t",
            "=dispatch",
            "DISPATCH_BOARD_STORE",
            "http://127.0.0.1:3000"
        ]
    );
}

#[test]
fn unbind_key_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    unbind_key("space", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1, vec!["unbind-key", "space"]);
}

#[test]
fn join_pane_issues_correct_tmux_args() {
    // Resolution *is* the pane lookup, so the separate display-message call
    // this used to make is gone: one recorded call, not two.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&COLLIDING);
    let pane_id = join_pane(&test_tmux_window("task-42"), "%1", &mock).unwrap();
    let expected = mock.pane_id_of("task-42");
    assert_eq!(pane_id, expected);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    // join-pane takes the resolved pane ID as its source, not the name.
    assert_eq!(
        calls[0].1,
        vec![
            "join-pane",
            "-h",
            "-d",
            "-s",
            &expected,
            "-t",
            "%1",
            "-l",
            "40%"
        ]
    );
}

#[test]
fn join_pane_returns_source_pane_id() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&["my-window"]);
    let result = join_pane(&test_tmux_window("my-window"), "%0", &mock).unwrap();
    assert_eq!(result, mock.pane_id_of("my-window"));
}

#[test]
fn join_pane_fails_when_source_window_is_absent() {
    // Otherwise a prefix-matched sibling's pane is torn out of its own window
    // and pulled into the board's split.
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-42"]);
    let err = join_pane(&test_tmux_window("task-4"), "%1", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-4'"),
        "got: {err}"
    );
    assert!(mock.recorded_calls().is_empty(), "join-pane must not run");
}

#[test]
fn select_pane_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    select_pane("%42", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1, vec!["select-pane", "-t", "%42"]);
}

#[test]
fn focus_events_enabled_returns_true_when_on() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
    assert!(focus_events_enabled(&mock));
}

#[test]
fn focus_events_enabled_returns_false_when_off() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"off\n")]);
    assert!(!focus_events_enabled(&mock));
}

#[test]
fn set_focus_events_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    set_focus_events(&mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1, vec!["set-option", "-g", "focus-events", "on"]);
}

#[test]
fn write_focus_events_creates_file_if_absent() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join(".tmux.conf");
    write_focus_events_to_tmux_conf_at(&conf).unwrap();
    let content = std::fs::read_to_string(&conf).unwrap();
    assert!(content.contains("set -g focus-events on"));
}

#[test]
fn write_focus_events_appends_to_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join(".tmux.conf");
    std::fs::write(&conf, "set -g mouse on\n").unwrap();
    write_focus_events_to_tmux_conf_at(&conf).unwrap();
    let content = std::fs::read_to_string(&conf).unwrap();
    assert!(content.contains("set -g mouse on"));
    assert!(content.contains("set -g focus-events on"));
}

#[test]
fn write_focus_events_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join(".tmux.conf");
    std::fs::write(&conf, "set -g focus-events on\n").unwrap();
    write_focus_events_to_tmux_conf_at(&conf).unwrap();
    let content = std::fs::read_to_string(&conf).unwrap();
    assert_eq!(
        content.matches("focus-events on").count(),
        1,
        "should not duplicate the line"
    );
}

#[test]
fn list_all_window_names_parses_output() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"dispatch\ntask-42\ntask-99\n",
    )]);
    let names = list_all_window_names(&mock).unwrap();
    assert_eq!(names, vec!["dispatch", "task-42", "task-99"]);
}

#[test]
fn list_all_window_names_empty_when_no_sessions() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no server running")]);
    let names = list_all_window_names(&mock).unwrap();
    assert!(
        names.is_empty(),
        "expected empty vec when tmux not running, got: {names:?}"
    );
}

#[test]
fn list_all_window_names_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"dispatch\n")]);
    let _ = list_all_window_names(&mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec!["list-windows", "-a", "-F", "#{window_name}"]
    );
}

// --- new_window failure path ---

#[test]
fn new_window_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::fail("no server running"), // new-window
    ]);
    let err = new_window(&test_tmux_window("task-1"), "/tmp", &[], &mock).unwrap_err();
    assert!(
        err.to_string().contains("new-window failed"),
        "expected 'new-window failed', got: {err}"
    );
}

// --- send_keys ---

#[test]
fn send_keys_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // send-keys -l
        MockProcessRunner::ok(), // send-keys Enter
    ])
    .with_windows(&["dispatch", "task-1"]);
    send_keys(&test_tmux_window("task-1"), "hello world", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 2);
    let pane = mock.pane_id_of("task-1");
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec!["send-keys", "-t", &pane, "-l", "hello world"]
    );
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(calls[1].1, vec!["send-keys", "-t", &pane, "Enter"]);
}

/// Both `send-keys` calls must name the *same* resolved pane, so the payload
/// and the Enter that submits it cannot land in different places.
#[test]
fn send_keys_targets_one_pane_for_both_calls() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()])
        .with_windows(&["task-1"]);
    send_keys(&test_tmux_window("task-1"), "hello", &mock).unwrap();
    let calls = mock.recorded_calls();
    let target = |c: &(String, Vec<String>)| c.1[2].clone();
    assert_eq!(target(&calls[0]), target(&calls[1]));
    assert_eq!(target(&calls[0]), mock.pane_id_of("task-1"));
}

#[test]
fn send_keys_fails_when_window_is_absent() {
    // The worst consequence of prefix matching: `task-1`'s payload typed into
    // `task-12`'s live Claude session as user input.
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-12"]);
    let err = send_keys(&test_tmux_window("task-1"), "hello", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-1'"),
        "got: {err}"
    );
    assert!(
        mock.recorded_calls().is_empty(),
        "no send-keys may be attempted"
    );
}

#[test]
fn send_keys_fails_on_first_send_error() {
    let mock =
        MockProcessRunner::new(vec![MockProcessRunner::fail("no pane")]).with_windows(&["task-1"]);
    let err = send_keys(&test_tmux_window("task-1"), "hello", &mock).unwrap_err();
    assert!(
        err.to_string().contains("send-keys -l failed"),
        "got: {err}"
    );
}

#[test]
fn send_keys_fails_on_enter_send_error() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),              // send-keys -l succeeds
        MockProcessRunner::fail("pane gone"), // send-keys Enter fails
    ])
    .with_windows(&["task-1"]);
    let err = send_keys(&test_tmux_window("task-1"), "hello", &mock).unwrap_err();
    assert!(
        err.to_string().contains("send-keys Enter failed"),
        "got: {err}"
    );
}

// --- kill_window ---

#[test]
fn kill_window_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&COLLIDING);
    kill_window(&test_tmux_window("task-42"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec!["kill-window", "-t", &mock.pane_id_of("task-42")]
    );
}

#[test]
fn kill_window_fails_when_window_is_absent() {
    // The other worst consequence: killing a live sibling's agent because the
    // intended window had already died.
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "keep-99"]);
    let err = kill_window(&test_tmux_window("keep-9"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'keep-9'"),
        "got: {err}"
    );
    assert!(
        mock.recorded_calls().is_empty(),
        "no kill-window may be attempted"
    );
}

#[test]
fn kill_window_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no window")])
        .with_windows(&["task-42"]);
    let err = kill_window(&test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(err.to_string().contains("kill-window failed"), "got: {err}");
}

// --- select_window ---

#[test]
fn select_window_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&COLLIDING);
    select_window(&test_tmux_window("task-4"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(
        calls[0].1,
        vec!["select-window", "-t", &mock.pane_id_of("task-4")]
    );
}

#[test]
fn select_window_fails_when_window_is_absent() {
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-42"]);
    let err = select_window(&test_tmux_window("task-4"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-4'"),
        "got: {err}"
    );
}

#[test]
fn select_window_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no window")])
        .with_windows(&["task-42"]);
    let err = select_window(&test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("select-window failed"),
        "got: {err}"
    );
}

// --- ensure_split_hook failure ---

#[test]
fn ensure_split_hook_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no session")]);
    let err = ensure_split_hook(&mock).unwrap_err();
    assert!(err.to_string().contains("set-hook failed"), "got: {err}");
}

// --- set_window_dispatch_dir generic failure ---

#[test]
fn set_window_dispatch_dir_fails_on_generic_nonzero_exit() {
    // The window resolves, but `set-option` itself fails.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no session running")])
        .with_windows(&["task-42"]);
    let err =
        set_window_dispatch_dir(&test_tmux_window("task-42"), "/some/path", &mock).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("set-option failed"),
        "expected 'set-option failed', got: {msg}"
    );
    assert!(
        !msg.contains("multiple tmux windows"),
        "should not be the ambiguous-window error, got: {msg}"
    );
}

#[test]
fn set_window_dispatch_dir_fails_when_window_is_absent() {
    // The bug: with only `task-42` alive, a request for `task-4` used to set
    // @dispatch_dir on task-42, sending that task's future splits into a
    // different task's worktree.
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-42"]);
    let err =
        set_window_dispatch_dir(&test_tmux_window("task-4"), "/some/path", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-4'"),
        "got: {err}"
    );
    assert!(mock.recorded_calls().is_empty(), "set-option must not run");
}

// --- split_window_horizontal ---

#[test]
fn split_window_horizontal_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%5\n")]);
    let pane_id = split_window_horizontal("%1", &mock).unwrap();
    assert_eq!(pane_id, "%5");
    let calls = mock.recorded_calls();
    assert_eq!(
        calls[0].1,
        vec![
            "split-window",
            "-h",
            "-d",
            "-l",
            "40%",
            "-t",
            "%1",
            "-P",
            "-F",
            "#{pane_id}",
        ]
    );
}

#[test]
fn split_window_horizontal_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no target pane")]);
    let err = split_window_horizontal("%1", &mock).unwrap_err();
    assert!(
        err.to_string().contains("split-window failed"),
        "got: {err}"
    );
}

// --- split_window_horizontal_running ---

#[test]
fn split_window_horizontal_running_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%9\n")]);
    let pane_id =
        split_window_horizontal_running("%1", 30, &["dispatch", "agent-tree", "42"], None, &mock)
            .unwrap();
    assert_eq!(pane_id, "%9");
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec![
            "split-window",
            "-h",
            "-b",
            "-d",
            "-l",
            "30%",
            "-t",
            "%1",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "dispatch",
            "agent-tree",
            "42",
        ]
    );
}

/// The start directory is what keeps a dispatch-created pane out of the
/// correction hook — see `ensure_split_hook`. Whether tmux honours it is a
/// real-server question (tests/tmux_split_hook.rs); this pins that we send it.
#[test]
fn split_window_horizontal_running_passes_the_start_directory() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%9\n")]);
    split_window_horizontal_running(
        "%1",
        30,
        &["dispatch", "agent-tree", "42"],
        Some("/home/u/wt"),
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    let args = &calls[0].1;
    let at = args
        .iter()
        .position(|a| a == "-c")
        .expect("expected -c in argv");
    assert_eq!(args[at + 1], "/home/u/wt");
    // Before `--`, or tmux would read it as part of the pane's command.
    let sep = args.iter().position(|a| a == "--").expect("expected --");
    assert!(at < sep, "-c must precede the command separator: {args:?}");
}

#[test]
fn split_window_horizontal_running_keeps_argv_elements_separate() {
    // A path with spaces must stay one argv element via the `--` exec
    // form, not get joined into a single shell string.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%9\n")]);
    split_window_horizontal_running(
        "%1",
        30,
        &["vim", "/tmp/dir with spaces/file.md"],
        None,
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls[0].1.last().unwrap(), "/tmp/dir with spaces/file.md");
    assert_eq!(calls[0].1[calls[0].1.len() - 3], "--");
    assert_eq!(calls[0].1[calls[0].1.len() - 2], "vim");
}

#[test]
fn split_window_horizontal_running_rejects_empty_command() {
    let mock = MockProcessRunner::new(vec![]);
    let err = split_window_horizontal_running("%1", 30, &[], None, &mock).unwrap_err();
    assert!(err.to_string().contains("command must not be empty"));
}

#[test]
fn split_window_horizontal_running_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no target pane")]);
    let err =
        split_window_horizontal_running("%1", 30, &["dispatch", "agent-tree", "42"], None, &mock)
            .unwrap_err();
    assert!(
        err.to_string().contains("split-window failed"),
        "got: {err}"
    );
}

// --- pane_ids_with_option / pane_ids_with_option_value ---

#[test]
fn pane_ids_with_option_returns_only_marked_panes() {
    // `list-panes` rows are "<pane_id> <value>"; an unset option renders as
    // the empty string, with the separator still there.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 \n%2 agent_tree\n%3 \n",
    )]);
    let found = pane_ids_with_option("%1", PANE_ROLE_OPTION, &mock).unwrap();
    assert_eq!(found, vec!["%2".to_string()]);
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec![
            "list-panes",
            "-t",
            "%1",
            "-F",
            "#{pane_id} #{@dispatch_pane_role}",
        ]
    );
}

/// Any role at all: this is how the pin drain asks for "every pane dispatch
/// put in this window", without being taught the roles.
#[test]
fn pane_ids_with_option_returns_every_role() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 \n%2 agent_tree\n%3 diff\n",
    )]);
    let found = pane_ids_with_option("%1", PANE_ROLE_OPTION, &mock).unwrap();
    assert_eq!(found, vec!["%2".to_string(), "%3".to_string()]);
}

#[test]
fn pane_ids_with_option_is_empty_when_nothing_is_marked() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%1 \n%2 \n")]);
    assert!(pane_ids_with_option("%1", PANE_ROLE_OPTION, &mock)
        .unwrap()
        .is_empty());
}

#[test]
fn pane_ids_with_option_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no window")]);
    assert!(pane_ids_with_option("%1", "@x", &mock).is_err());
}

/// Value-matched, not presence-matched: the two roles must never stand in for
/// each other, or a file open would respawn the tree pane with an editor.
#[test]
fn pane_ids_with_option_value_returns_only_that_role() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 \n%2 agent_tree\n%3 diff\n",
    )]);
    let found = pane_ids_with_option_value("%2", PANE_ROLE_OPTION, PANE_ROLE_DIFF, &mock).unwrap();
    assert_eq!(found, vec!["%3".to_string()]);
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec![
            "list-panes",
            "-t",
            "%2",
            "-F",
            "#{pane_id} #{@dispatch_pane_role}",
        ]
    );
}

/// The whole value, not a prefix of it: a future role named `diff_split`
/// must not answer a lookup for `diff`.
#[test]
fn pane_ids_with_option_value_matches_the_whole_value() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 diff_split\n%2 diff\n",
    )]);
    let found = pane_ids_with_option_value("%1", PANE_ROLE_OPTION, PANE_ROLE_DIFF, &mock).unwrap();
    assert_eq!(found, vec!["%2".to_string()]);
}

#[test]
fn pane_ids_with_option_value_is_empty_when_no_pane_has_that_role() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"%1 \n%2 agent_tree\n",
    )]);
    assert!(
        pane_ids_with_option_value("%1", PANE_ROLE_OPTION, PANE_ROLE_DIFF, &mock)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pane_ids_with_option_value_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no window")]);
    assert!(
        pane_ids_with_option_value("%1", PANE_ROLE_OPTION, PANE_ROLE_AGENT_TREE, &mock).is_err()
    );
}

// --- split_window_below_running ---

#[test]
fn split_window_below_running_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%7\n")]);
    let pane_id =
        split_window_below_running("%3", 60, "/work/wt", &["vim", "/work/wt/src/lib.rs"], &mock)
            .unwrap();
    assert_eq!(pane_id, "%7");
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec![
            "split-window",
            "-v",
            "-d",
            "-l",
            "60%",
            "-t",
            "%3",
            "-c",
            "/work/wt",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "vim",
            "/work/wt/src/lib.rs",
        ]
    );
}

/// The ABSENCE of `-f` is what keeps the new pane inside the target's own
/// column, so the agent's pane is untouched by opening a diff; `-d` is what
/// keeps focus in the tree. Both are single-character flags, easy to add or
/// drop in a refactor and invisible in the result, so both are asserted by
/// name as well as by the argv above.
#[test]
fn split_window_below_running_subdivides_its_target_and_keeps_focus() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%7\n")]);
    split_window_below_running("%3", 66, "/work/wt", &["dispatch", "agent-diff"], &mock).unwrap();
    let args = &mock.recorded_calls()[0].1;
    assert!(
        !args.contains(&"-f".to_string()),
        "-f would span the window and steal the agent's columns; args: {args:?}"
    );
    assert!(args.contains(&"-d".to_string()), "args: {args:?}");
}

#[test]
fn split_window_below_running_keeps_argv_elements_separate() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%7\n")]);
    split_window_below_running(
        "%3",
        60,
        "/work/wt",
        &["vim", "-p", "/work/wt/dir with spaces/a.rs"],
        &mock,
    )
    .unwrap();
    let args = &mock.recorded_calls()[0].1;
    assert_eq!(args.last().unwrap(), "/work/wt/dir with spaces/a.rs");
    assert_eq!(args[args.len() - 4], "--");
    assert_eq!(args[args.len() - 3], "vim");
    assert_eq!(args[args.len() - 2], "-p");
}

#[test]
fn split_window_below_running_rejects_empty_command() {
    let mock = MockProcessRunner::new(vec![]);
    let err = split_window_below_running("%3", 60, "/w", &[], &mock).unwrap_err();
    assert!(err.to_string().contains("command must not be empty"));
}

#[test]
fn split_window_below_running_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no space")]);
    let err = split_window_below_running("%3", 60, "/w", &["vi", "a"], &mock).unwrap_err();
    assert!(
        err.to_string().contains("split-window failed"),
        "got: {err}"
    );
}

/// A window *name* target must be resolved rather than handed to tmux, which
/// prefix-matches names (see [`window_target`]).
#[test]
fn split_window_below_running_resolves_a_window_name() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%7\n")])
        .with_windows(&["task-42"]);
    split_window_below_running("task-42", 60, "/w", &["vi", "a"], &mock).unwrap();
    let args = &mock.recorded_calls()[0].1;
    let target = args.iter().position(|a| a == "-t").unwrap() + 1;
    assert_eq!(args[target], mock.pane_id_of("task-42"));
}

// --- respawn_pane_running ---

#[test]
fn respawn_pane_running_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    respawn_pane_running("%7", "/work/wt", &["vim", "-p", "/work/wt/a.rs"], &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1,
        vec![
            "respawn-pane",
            "-k",
            "-c",
            "/work/wt",
            "-t",
            "%7",
            "--",
            "vim",
            "-p",
            "/work/wt/a.rs",
        ]
    );
}

#[test]
fn respawn_pane_running_rejects_empty_command() {
    let mock = MockProcessRunner::new(vec![]);
    let err = respawn_pane_running("%7", "/w", &[], &mock).unwrap_err();
    assert!(err.to_string().contains("command must not be empty"));
}

#[test]
fn respawn_pane_running_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("boom")]);
    let err = respawn_pane_running("%7", "/w", &["vi", "a"], &mock).unwrap_err();
    assert!(
        err.to_string().contains("respawn-pane failed"),
        "got: {err}"
    );
}

// --- set_pane_option ---

#[test]
fn set_pane_option_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    set_pane_option("%7", PANE_ROLE_OPTION, PANE_ROLE_DIFF, &mock).unwrap();
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec![
            "set-option",
            "-p",
            "-t",
            "%7",
            "@dispatch_pane_role",
            "diff"
        ]
    );
}

#[test]
fn set_pane_option_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("bad option")]);
    let err = set_pane_option("%7", "@x", "1", &mock).unwrap_err();
    assert!(err.to_string().contains("set-option failed"), "got: {err}");
}

// --- join_pane failure paths ---

#[test]
fn join_pane_fails_when_the_window_lookup_fails() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no server running")])
        .with_queued_window_lookup();
    let err = join_pane(&test_tmux_window("task-42"), "%1", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-42'"),
        "got: {err}"
    );
}

#[test]
fn join_pane_fails_when_join_pane_command_fails() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("invalid target")])
        .with_windows(&["task-42"]);
    let err = join_pane(&test_tmux_window("task-42"), "%1", &mock).unwrap_err();
    assert!(err.to_string().contains("join-pane failed"), "got: {err}");
}

// --- break_pane_to_window ---

#[test]
fn break_pane_to_window_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::ok(), // break-pane
    ]);
    break_pane_to_window("%5", &test_tmux_window("new-win"), &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(
        calls[1].1,
        vec!["break-pane", "-d", "-s", "%5", "-n", "new-win"]
    );
}

/// split-pane.allium's `RefuseExitSplitModeOntoLiveWindow`: breaking a
/// pinned pane back out under a name a live window already holds is
/// refused, and the pane is left where it is. Killing it would destroy a
/// running agent's scrollback to settle a bookkeeping conflict.
#[test]
fn break_pane_to_window_refuses_a_name_a_live_window_already_holds() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"board\ntask-42\n")]);
    let err = break_pane_to_window("%5", &test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(
        err.to_string()
            .contains("a tmux window named 'task-42' already exists"),
        "got: {err}"
    );
    assert!(
        !mock
            .recorded_calls()
            .iter()
            .any(|(_, args)| args.contains(&"break-pane".to_string())),
        "break-pane must not be issued — the pane stays pinned"
    );
}

#[test]
fn break_pane_to_window_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // list-windows (duplicate-name check)
        MockProcessRunner::fail("no such pane"), // break-pane
    ]);
    let err = break_pane_to_window("%5", &test_tmux_window("new-win"), &mock).unwrap_err();
    assert!(err.to_string().contains("break-pane failed"), "got: {err}");
}

// --- kill_pane ---

#[test]
fn kill_pane_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    kill_pane("%42", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls[0].1, vec!["kill-pane", "-t", "%42"]);
}

#[test]
fn kill_pane_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no pane")]);
    let err = kill_pane("%42", &mock).unwrap_err();
    assert!(err.to_string().contains("kill-pane failed"), "got: {err}");
}

// --- respawn_pane ---

#[test]
fn respawn_pane_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    respawn_pane("%42", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls[0].1, vec!["respawn-pane", "-k", "-t", "%42"]);
}

#[test]
fn respawn_pane_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no such pane")]);
    let err = respawn_pane("%42", &mock).unwrap_err();
    assert!(
        err.to_string().contains("respawn-pane failed"),
        "got: {err}"
    );
}

// --- pane_id_for_window ---

#[test]
fn pane_id_for_window_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![]).with_windows(&COLLIDING);
    let short = pane_id_for_window(&test_tmux_window("task-4"), &mock).unwrap();
    let long = pane_id_for_window(&test_tmux_window("task-42"), &mock).unwrap();
    assert_eq!(short, mock.pane_id_of("task-4"));
    assert_eq!(long, mock.pane_id_of("task-42"));
    assert_ne!(short, long, "colliding names are different windows");
}

#[test]
fn pane_id_for_window_fails_on_empty_output() {
    // The case real tmux produces for a missing window: exit 0 with no
    // output. Under the old `display-message` implementation that returned
    // Ok(""), and `swap-pane -s ''` exits 0 too, so the bad id propagated
    // silently. A no-row listing must stay a hard miss.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"\n")])
        .with_queued_window_lookup();
    let err = pane_id_for_window(&test_tmux_window("task-999"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-999'"),
        "got: {err}"
    );
}

#[test]
fn pane_id_for_window_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no such window")])
        .with_queued_window_lookup();
    let err = pane_id_for_window(&test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-42'"),
        "got: {err}"
    );
}

/// The prefix case, which used to return the sibling's pane ID — a wrong ID
/// that then propagated into swaps, splits and the split-pane's tracked pane.
#[test]
fn pane_id_for_window_fails_rather_than_returning_a_siblings_pane() {
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-42"]);
    let err = pane_id_for_window(&test_tmux_window("task-4"), &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-4'"),
        "got: {err}"
    );
}

// --- swap_pane ---

#[test]
fn swap_pane_issues_correct_tmux_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    swap_pane("%1", "%2", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls[0].1, vec!["swap-pane", "-d", "-s", "%1", "-t", "%2"]);
}

#[test]
fn swap_pane_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no pane")]);
    let err = swap_pane("%1", "%2", &mock).unwrap_err();
    assert!(err.to_string().contains("swap-pane failed"), "got: {err}");
}

// --- current_pane_id ---

#[test]
fn current_pane_id_issues_correct_args() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%42\n")]);
    let result = current_pane_id(&mock).unwrap();
    assert_eq!(result, "%42");
    let calls = mock.recorded_calls();
    assert_eq!(calls[0].1, vec!["display-message", "-p", "#{pane_id}"]);
}

#[test]
fn current_pane_id_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no session")]);
    let err = current_pane_id(&mock).unwrap_err();
    assert!(
        err.to_string().contains("display-message failed"),
        "got: {err}"
    );
}

// --- pane_pid ---

#[test]
fn pane_pid_reads_the_pid_of_an_explicit_pane() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"4242\n")]);
    assert_eq!(pane_pid("%3", &mock), Some(4242));
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec!["display-message", "-p", "-t", "%3", "#{pane_pid}"]
    );
}

#[test]
fn pane_pid_is_none_when_tmux_cannot_say() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no pane")]);
    assert_eq!(pane_pid("%3", &mock), None);
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"junk\n")]);
    assert_eq!(pane_pid("%3", &mock), None);
}

// --- pane_exists ---

#[test]
fn pane_exists_finds_the_pane_in_the_listing() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%1\n%42\n%7\n")]);
    assert!(pane_exists("%42", &mock));
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec!["list-panes", "-a", "-F", "#{pane_id}"],
        "must query the pane listing, not display-message — see pane_exists' docs"
    );
}

#[test]
fn pane_exists_is_false_when_the_pane_is_absent_from_the_listing() {
    // The case the old implementation could never detect: tmux exits 0 for an
    // unknown pane target, so only a membership test sees a closed pane.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%1\n%7\n")]);
    assert!(!pane_exists("%42", &mock));
}

#[test]
fn pane_exists_does_not_match_a_pane_id_prefix() {
    // `%4` must not satisfy a query for `%42`, nor the reverse.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%4\n")]);
    assert!(!pane_exists("%42", &mock));
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%42\n")]);
    assert!(!pane_exists("%4", &mock));
}

#[test]
fn pane_exists_returns_false_when_no_server_is_running() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no server running")]);
    assert!(!pane_exists("%42", &mock));
}

#[test]
fn pane_exists_returns_false_on_runner_error() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("binary not found"))]);
    assert!(!pane_exists("%42", &mock));
}

// --- capture_pane ---

#[test]
fn capture_pane_issues_correct_args_against_the_resolved_target() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"hello\n")])
        .with_windows(&["task-42"]);
    let result = capture_pane(&mock.pane_id_of("task-42"), &mock).unwrap();
    assert_eq!(result, "hello");
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1,
        vec!["capture-pane", "-p", "-t", &mock.pane_id_of("task-42")]
    );
}

// --- set_focus_events failure ---

#[test]
fn set_focus_events_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no server running")]);
    let err = set_focus_events(&mock).unwrap_err();
    assert!(
        err.to_string().contains("set-option focus-events failed"),
        "got: {err}"
    );
}

// --- focus_events_enabled runner error ---

#[test]
fn focus_events_enabled_returns_false_on_runner_error() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("tmux not found"))]);
    assert!(!focus_events_enabled(&mock));
}

// --- bind_key / unbind_key failure paths ---

#[test]
fn bind_key_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("invalid key")]);
    let err = bind_key("space", "select-window -t dispatch", &mock).unwrap_err();
    assert!(err.to_string().contains("bind-key failed"), "got: {err}");
}

#[test]
fn unbind_key_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no key bound")]);
    let err = unbind_key("space", &mock).unwrap_err();
    assert!(err.to_string().contains("unbind-key failed"), "got: {err}");
}

// --- select_pane failure ---

#[test]
fn select_pane_fails_on_nonzero_exit() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no such pane")]);
    let err = select_pane("%42", &mock).unwrap_err();
    assert!(err.to_string().contains("select-pane failed"), "got: {err}");
}

// --- pane lookups by window NAME ---
//
// The `%N`-target cases live with the other pane-lookup tests above; these two
// cover what only a *name* target exercises, and are what the deleted
// `inactive_pane_id` tests used to guarantee.

/// The window is named to `list-panes` by its resolved pane, so the panes
/// returned cannot be a prefix-matched sibling's.
#[test]
fn pane_ids_with_option_resolves_a_window_name_to_its_own_panes() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"%3 \n%7 1\n")])
        .with_windows(&COLLIDING);
    let found = pane_ids_with_option("task-42", PANE_ROLE_OPTION, &mock).unwrap();
    assert_eq!(found, vec!["%7".to_string()]);
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec![
            "list-panes",
            "-t",
            &mock.pane_id_of("task-42"),
            "-F",
            "#{pane_id} #{@dispatch_pane_role}",
        ]
    );
}

/// An absent window must error rather than reporting "no marked pane" after
/// inspecting a prefix-matched sibling's panes.
#[test]
fn pane_ids_with_option_fails_when_window_is_absent() {
    let mock = MockProcessRunner::new(vec![]).with_windows(&["dispatch", "task-42"]);
    let err = pane_ids_with_option("task-4", "@x", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-4'"),
        "got: {err}"
    );
}

// --- window_target ---
//
// These assert the lookup itself, so they queue its response positionally
// (`with_queued_window_lookup`) rather than letting the mock resolve out of
// band — the listing bytes *are* the fixture here.

/// A runner whose window lookup is answered from `responses`, for the tests
/// whose subject is resolution.
fn queued(responses: Vec<Result<Output>>) -> MockProcessRunner {
    MockProcessRunner::new(responses).with_queued_window_lookup()
}

#[test]
fn window_target_asks_tmux_for_an_exact_name_match() {
    let mock = queued(vec![MockProcessRunner::ok_with_stdout(b"1 %1 task-4\n")]);
    window_target("task-4", &mock).unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(
        calls[0].1,
        vec![
            "list-panes",
            "-a",
            "-f",
            "#{==:#{window_name},task-4}",
            "-F",
            WINDOW_PANE_FORMAT,
        ],
        "the filter must compare window_name for equality, not prefix"
    );
}

#[test]
fn window_target_resolves_an_exact_name_to_its_active_pane() {
    // Two panes in task-42, only one active: the active pane is the one every
    // command this module issues means by "the window".
    let mock = queued(vec![MockProcessRunner::ok_with_stdout(
        b"0 %5 task-42\n1 %6 task-42\n",
    )]);
    assert_eq!(window_target("task-42", &mock).unwrap(), "%6");
}

/// The local name comparison is not redundant with tmux's `-f` filter: a name
/// carrying `,` or `}` could confuse `#{==:…}` into returning a row for a
/// different window. Re-checking here turns that into a miss, never a wrong
/// hit — so a hostile listing cannot make resolution target the wrong pane.
#[test]
fn window_target_rechecks_the_name_the_filter_returned() {
    let mock = queued(vec![MockProcessRunner::ok_with_stdout(b"1 %9 task-3782\n")]);
    let err = window_target("task-378", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-378'"),
        "a row for the wrong window must not resolve, got: {err}"
    );
}

#[test]
fn window_target_rejects_duplicate_names() {
    let mock = queued(vec![MockProcessRunner::ok_with_stdout(
        b"1 %1 task-42\n1 %2 task-42\n",
    )]);
    let err = window_target("task-42", &mock).unwrap_err();
    assert!(
        err.to_string().contains("multiple tmux windows"),
        "got: {err}"
    );
}

/// Window names may contain spaces (the board window inherits whatever the
/// user had), so the name must be the parsed remainder, not one field.
#[test]
fn window_target_handles_names_containing_spaces() {
    let mock = queued(vec![MockProcessRunner::ok_with_stdout(
        b"1 %3 my project shell\n",
    )]);
    assert_eq!(window_target("my project shell", &mock).unwrap(), "%3");
}

#[test]
fn window_target_passes_through_pane_ids_and_empty_targets() {
    for target in ["%42", "%0", ""] {
        let mock = queued(vec![]);
        assert_eq!(window_target(target, &mock).unwrap(), target);
        assert!(
            mock.recorded_calls().is_empty(),
            "an already-exact target must not be looked up: {target:?}"
        );
    }
}

/// A window can genuinely be named `%foo`, so only `%`-plus-digits is a pane
/// ID. Anything else takes the lookup path and gets this module's clear
/// "no tmux window named" error rather than tmux's "can't find pane".
#[test]
fn window_target_looks_up_names_that_merely_start_with_percent() {
    for target in ["%foo", "%", "%1a"] {
        let mock = queued(vec![MockProcessRunner::ok_with_stdout(b"")]);
        let err = window_target(target, &mock).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("no tmux window named '{target}'")),
            "{target:?} should be looked up as a name, got: {err}"
        );
        assert_eq!(
            mock.recorded_calls().len(),
            1,
            "{target:?} should have been looked up"
        );
    }
}

#[test]
fn window_target_treats_a_failed_lookup_as_not_found() {
    // No server running: there is genuinely no such window. Mirrors
    // `list_all_window_names`, which maps the same failure to an empty list.
    let mock = queued(vec![MockProcessRunner::fail("no server running")]);
    let err = window_target("task-42", &mock).unwrap_err();
    assert!(
        err.to_string().contains("no tmux window named 'task-42'"),
        "got: {err}"
    );
}

#[test]
fn window_target_propagates_a_runner_error() {
    let mock = queued(vec![Err(anyhow::anyhow!("tmux: command not found"))]);
    let err = window_target("task-42", &mock).unwrap_err();
    assert!(err.to_string().contains("command not found"), "got: {err}");
}

/// `window_name_in_lookup` must invert `window_filter` exactly, or
/// `MockProcessRunner` silently stops recognising the lookup and every mock
/// test that relies on out-of-band resolution starts failing obscurely.
#[test]
fn window_name_in_lookup_inverts_the_filter_this_module_builds() {
    let filter = window_filter("task-42");
    let args = ["list-panes", "-a", "-f", &filter, "-F", WINDOW_PANE_FORMAT];
    assert_eq!(window_name_in_lookup(&args), Some("task-42"));
}

#[test]
fn window_name_in_lookup_ignores_other_calls() {
    assert_eq!(
        window_name_in_lookup(&["list-windows", "-a", "-F", "#{window_name}"]),
        None
    );
    assert_eq!(
        window_name_in_lookup(&["list-panes", "-t", "%1", "-F", "#{pane_id}"]),
        None
    );
}

// -- Session-scoped board-window targeting (startup.allium's
//    RetiringTouchesOnlyTheBoardWindow) --

#[test]
fn pane_id_of_window_in_session_scopes_the_query_to_that_session() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"1 %0 shell\n1 %7 TUI\n",
    )])
    .with_queued_window_lookup();
    let found =
        pane_id_of_window_in_session("dispatch", &TmuxWindow::from_static("TUI"), &mock).unwrap();
    assert_eq!(found.as_deref(), Some("%7"));
    let calls = mock.recorded_calls();
    assert_eq!(
        calls[0].1,
        vec![
            "list-panes",
            "-s",
            "-t",
            "=dispatch",
            "-F",
            WINDOW_PANE_FORMAT
        ],
        "the listing must be scoped to the named session, exactly matched"
    );
}

#[test]
fn pane_id_of_window_in_session_matches_the_name_exactly() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"1 %0 TUI-notes\n1 %1 task-42\n",
    )])
    .with_queued_window_lookup();
    let found =
        pane_id_of_window_in_session("dispatch", &TmuxWindow::from_static("TUI"), &mock).unwrap();
    assert_eq!(
        found, None,
        "a window whose name merely starts with the board's must not be retired"
    );
}

#[test]
fn pane_id_of_window_in_session_reports_absent_when_the_session_is_gone() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no such session")])
        .with_queued_window_lookup();
    let found =
        pane_id_of_window_in_session("dispatch", &TmuxWindow::from_static("TUI"), &mock).unwrap();
    assert_eq!(
        found, None,
        "a session that is not there has no board window to retire"
    );
}

#[test]
fn session_exists_targets_the_name_exactly() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    assert!(session_exists("dispatch", &mock));
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec!["has-session", "-t", "=dispatch"],
        "a prefix match would find a session the operator named for something else"
    );
}

#[test]
fn session_exists_is_false_when_tmux_says_no() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no such session")]);
    assert!(!session_exists("dispatch", &mock));
}

#[test]
fn new_window_in_session_running_names_and_selects_the_window() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b""), // duplicate-name check
        MockProcessRunner::ok(),                // new-window
    ]);
    new_window_in_session_running(
        "dispatch",
        &TmuxWindow::from_static("TUI"),
        &["/bin/dispatch", "tui"],
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    let (_, args) = calls.last().unwrap();
    assert_eq!(
        args,
        &vec![
            "new-window",
            "-t",
            "=dispatch:",
            "-n",
            "TUI",
            "--",
            "/bin/dispatch",
            "tui"
        ],
        "the board's window is named up front and left selected so the attach lands on it"
    );
    assert!(
        !args.contains(&"-d".to_string()),
        "`-d` would leave the operator attaching to whatever window was current"
    );
}

#[test]
fn display_message_targets_the_callers_own_pane_when_it_knows_it() {
    // `display-message -p` with no `-t` answers about the session's ACTIVE
    // pane, not the caller's — verified against tmux 3.5a in
    // tests/tmux_board_restart.rs. A process in a background window then
    // reads another window's name as its own, with no sign anything is
    // wrong.
    assert_eq!(
        display_message_args(Some("%3"), "#W"),
        vec!["display-message", "-p", "-t", "%3", "#W"]
    );
    assert_eq!(
        display_message_args(None, "#W"),
        vec!["display-message", "-p", "#W"],
        "with no pane to name, the untargeted query is still the best available"
    );
}

#[test]
fn current_window_context_parses_panes_session_and_name() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"2 dispatch my window\n",
    )]);
    let ctx = current_window_context(Some("%3"), &mock).unwrap();
    assert_eq!(ctx.window_panes, 2);
    assert_eq!(ctx.session_name, "dispatch");
    assert_eq!(
        ctx.window_name, "my window",
        "the window name is the remainder, so a space in it survives"
    );
    assert_eq!(
        mock.recorded_calls()[0].1,
        vec![
            "display-message",
            "-p",
            "-t",
            "%3",
            "#{window_panes} #{session_name} #{window_name}"
        ]
    );
}

#[test]
fn current_window_context_rejects_an_unparseable_reply() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"\n")]);
    assert!(
        current_window_context(Some("%3"), &mock).is_err(),
        "a reply that cannot be read must not become a confident zero"
    );
}

#[test]
fn new_window_in_session_running_refuses_a_name_already_in_that_session() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n")])
        .with_queued_window_lookup();
    let err = new_window_in_session_running(
        "dispatch",
        &TmuxWindow::from_static("TUI"),
        &["/bin/dispatch"],
        &mock,
    )
    .expect_err("a duplicate name inside the target session is refused");
    assert!(
        err.to_string().contains("TUI"),
        "the refusal must name the window, got: {err}"
    );
}

#[test]
fn new_window_in_session_running_scopes_the_duplicate_check_to_the_session() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b""), // duplicate check
        MockProcessRunner::ok(),                // new-window
    ])
    .with_queued_window_lookup();
    new_window_in_session_running(
        "dispatch",
        &TmuxWindow::from_static("TUI"),
        &["/bin/dispatch"],
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    assert!(
        calls[0].1.contains(&"-s".to_string()) || calls[0].1.contains(&"=dispatch".to_string()),
        "the check must ask about this session, not the whole server: {:?}",
        calls[0].1
    );
    assert!(
        !calls[0].1.contains(&"-a".to_string()),
        "a server-wide check would let a window in a session dispatch does not \
             own refuse the replacement board: {:?}",
        calls[0].1
    );
}

#[test]
fn new_window_in_session_running_rejects_an_empty_command() {
    let mock = MockProcessRunner::new(vec![]);
    assert!(
        new_window_in_session_running("dispatch", &TmuxWindow::from_static("TUI"), &[], &mock)
            .is_err()
    );
}
