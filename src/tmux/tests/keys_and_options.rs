use super::*;

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
