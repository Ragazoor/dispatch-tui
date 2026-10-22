use super::*;

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
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no server running")])
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

// --- rejoin_pane ---

#[test]
fn rejoin_pane_succeeds_once_the_pane_has_moved_even_if_the_tag_stays() {
    // The join is what puts the pane home. A failed tag clear afterwards must
    // not read as "not rejoined": the swap would then reverse a pane that has
    // already left (split-pane.allium: SwapSplitPane).
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),                   // join-pane
        MockProcessRunner::fail("set-option err"), // set-option -u
    ])
    .with_windows(&["task-42"]);
    rejoin_pane("%7", &test_tmux_window("task-42"), &mock).unwrap();
    assert_eq!(mock.recorded_calls().len(), 2);
}

#[test]
fn rejoin_pane_fails_when_join_pane_fails() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("invalid target")])
        .with_windows(&["task-42"]);
    let err = rejoin_pane("%7", &test_tmux_window("task-42"), &mock).unwrap_err();
    assert!(err.to_string().contains("join-pane failed"), "got: {err}");
    assert_eq!(
        mock.recorded_calls().len(),
        1,
        "no tag clear after a failed join"
    );
}
