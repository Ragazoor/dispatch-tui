use super::*;

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
fn list_all_window_names_empty_when_cannot_connect() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "error connecting to /tmp/tmux-1000/default (No such file or directory)",
    )]);
    assert!(list_all_window_names(&mock).unwrap().is_empty());
}

#[test]
fn list_all_window_names_errors_on_other_failures() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("server exited unexpectedly")]);
    let err = list_all_window_names(&mock).unwrap_err();
    assert!(
        err.to_string().contains("server exited unexpectedly"),
        "got: {err}"
    );
}

#[test]
fn has_window_errors_when_the_listing_fails_for_another_reason() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("protocol version mismatch")]);
    assert!(has_window(&TmuxWindow::for_task(crate::models::TaskId(1)), &mock).is_err());
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
