use super::*;
use crate::models::test_pane_id;

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
    assert_eq!(found, Some(test_pane_id("%7")));
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
fn window_target_errors_on_a_transient_lookup_failure() {
    // A failure that is not "no server" must not read as an absent window,
    // or callers would take the absent-window path (skip kill, re-dispatch).
    let mock = queued(vec![MockProcessRunner::fail(
        "error connecting to /tmp/tmux-1000/default (Operation not permitted)",
    )]);
    let err = window_target("task-42", &mock).unwrap_err();
    assert!(!is_window_absent_error(&err), "got: {err}");
    assert!(
        err.to_string().contains("Operation not permitted"),
        "got: {err}"
    );
}

#[test]
fn is_no_server_rejects_permission_failures() {
    assert!(is_no_server("no server running on /tmp/tmux-1000/default"));
    assert!(is_no_server(
        "error connecting to /tmp/tmux-1000/default (No such file or directory)"
    ));
    assert!(!is_no_server(
        "error connecting to /tmp/tmux-1000/default (Permission denied)"
    ));
    assert!(!is_no_server(
        "error connecting to /tmp/tmux-1000/default (Operation not permitted)"
    ));
}

#[test]
fn refuse_duplicate_window_name_surfaces_a_query_failure() {
    let mock = queued(vec![MockProcessRunner::fail("server exited unexpectedly")]);
    let name = test_tmux_window("task-42");
    let err = refuse_duplicate_window_name(&name, &mock).unwrap_err();
    assert!(
        err.to_string().contains("server exited unexpectedly"),
        "got: {err}"
    );
}
