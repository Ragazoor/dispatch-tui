use super::*;
use crate::process::MockProcessRunner;
use crate::setup::FakeConfirmer;

/// A `Confirmer` whose every method fails, standing in for a launch with
/// no one to answer it. Shared by the config-drift and host-label gates —
/// both ask the same question of the same trait, so a second copy would
/// only be one more place to add the next `Confirmer` method.
struct FailingConfirmer;
impl Confirmer for FailingConfirmer {
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Err(anyhow::anyhow!("stdin closed"))
    }
    fn confirm_dangerous(&self, _: &str) -> anyhow::Result<bool> {
        Err(anyhow::anyhow!("stdin closed"))
    }
    fn prompt_text(&self, _: &str, _: &str) -> anyhow::Result<String> {
        Err(anyhow::anyhow!("stdin closed"))
    }
}

// -- Launch planning (LaunchBoardInsideExistingSession /
//    LaunchBoardBySupplyingASession) --

/// Outside tmux, with no session of dispatch's own yet.
fn cold() -> LaunchContext {
    LaunchContext::Outside {
        session_exists: false,
    }
}

/// Inside `session`, in a window playing `role`.
fn inside(session: &str, role: BoardWindowRole) -> LaunchContext {
    LaunchContext::Inside {
        session: session.to_string(),
        role,
    }
}

fn argv() -> Vec<String> {
    vec!["/bin/dispatch".to_string(), "tui".to_string()]
}

#[test]
fn plan_launch_inside_tmux_continues_in_this_process() {
    let plan = plan_launch(inside("work", BoardWindowRole::NotTheBoard), argv());
    assert_eq!(
        plan,
        LaunchPlan::ContinueHere {
            session: "work".to_string()
        },
        "an operator who already had a session keeps it"
    );
}

#[test]
fn plan_launch_does_not_refuse_the_board_its_own_launch() {
    // The regression this pins: both entry paths create the board's window
    // already carrying BOARD_WINDOW_NAME, so the board's own process runs
    // this path from inside a window named TUI. A predicate that read only
    // the window's name would have every board refuse itself, and the
    // operator would attach to a session whose board had just exited with
    // "This window is already the dispatch board."
    let plan = plan_launch(inside(SESSION_NAME, BoardWindowRole::NotTheBoard), argv());
    assert_eq!(
        plan,
        LaunchPlan::ContinueHere {
            session: SESSION_NAME.to_string()
        }
    );
}

#[test]
fn plan_launch_draws_without_retiring_when_this_process_is_the_started_board() {
    // The bug this pins was only visible against a real tmux: on the
    // inside-tmux path the board retired the window named TUI in its own
    // session — which is the window it is itself running in. It closed
    // itself before drawing, and where that window was the session's only
    // one, tmux discarded the session and the whole server went with it.
    let plan = plan_launch(inside(SESSION_NAME, BoardWindowRole::StartedBoard), argv());
    assert_eq!(
        plan,
        LaunchPlan::DrawInThisWindow,
        "ALaunchNeverRetiresItsOwnWindow"
    );
}

#[test]
fn plan_launch_refuses_a_session_tmux_will_not_name() {
    let plan = plan_launch(inside("", BoardWindowRole::NotTheBoard), argv());
    assert_eq!(
        plan,
        LaunchPlan::Refuse(StartupAbort::SessionUnidentified),
        "with no session name there is nothing to scope a retire to, and an \
             unscoped one closes a window in a session dispatch may not own"
    );
}

#[test]
fn board_window_role_reads_one_name_two_ways() {
    // The board holds its window alone, so a second pane in it is somebody
    // else. The name alone cannot tell them apart: both entry paths create
    // the board's window already carrying it, so the board's own process
    // arrives here too — and a name-only reading would have every board
    // refuse, or worse retire, its own launch.
    assert_eq!(
        BoardWindowRole::of("TUI", 1),
        BoardWindowRole::StartedBoard,
        "one pane in the board's window is the board itself, mid-launch"
    );
    assert_eq!(
        BoardWindowRole::of("TUI", 2),
        BoardWindowRole::SharedWithBoard,
        "a second pane is the operator's shell, or the split-pane agent"
    );
    assert_eq!(
        BoardWindowRole::of("task-42", 2),
        BoardWindowRole::NotTheBoard,
        "a window that is not the board's is neither"
    );
}

#[test]
fn plan_launch_from_inside_the_board_window_refuses() {
    let plan = plan_launch(
        inside(SESSION_NAME, BoardWindowRole::SharedWithBoard),
        argv(),
    );
    assert_eq!(
        plan,
        LaunchPlan::Refuse(StartupAbort::BoardAlreadyInThisWindow),
        "retiring the board's window from a shell inside it would close that shell"
    );
}

#[test]
fn plan_launch_outside_tmux_enters_the_dispatch_session() {
    let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
    let plan = plan_launch(cold(), argv.clone());
    assert_eq!(
        plan,
        LaunchPlan::EnterSession {
            session: SESSION_NAME.to_string(),
            argv,
        },
        "the command supplies its own session rather than reporting it has none"
    );
}

#[test]
fn plan_launch_outside_tmux_restarts_the_board_in_an_existing_session() {
    let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
    let plan = plan_launch(
        LaunchContext::Outside {
            session_exists: true,
        },
        argv.clone(),
    );
    assert_eq!(
        plan,
        LaunchPlan::RestartInSession {
            session: SESSION_NAME.to_string(),
            argv,
        },
        "EveryLaunchProducesAFreshBoard: a second launch restarts, never reattaches"
    );
}

#[test]
fn plan_launch_restarts_without_asking_whether_the_old_board_is_alive() {
    // RetiringIsNotConditionalOnLiveness. The context the planner is given
    // carries no liveness signal at all, which is the point: there is no
    // input here that could make a running board take a different path
    // from an exited one.
    let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
    match plan_launch(
        LaunchContext::Outside {
            session_exists: true,
        },
        argv.clone(),
    ) {
        LaunchPlan::RestartInSession { argv: carried, .. } => assert_eq!(carried, argv),
        other => panic!("expected RestartInSession, got {other:?}"),
    }
}

#[test]
fn plan_launch_carries_the_operators_arguments_through() {
    let argv = vec![
        "/bin/dispatch".to_string(),
        "--db".to_string(),
        "/tmp/scratch.db".to_string(),
        "tui".to_string(),
        "--port".to_string(),
        "9999".to_string(),
    ];
    match plan_launch(cold(), argv.clone()) {
        LaunchPlan::EnterSession { argv: carried, .. } => assert_eq!(
            carried, argv,
            "ArgvIsCarriedThrough: the board must come up with the db and port asked for"
        ),
        other => panic!("expected EnterSession, got {other:?}"),
    }
}

// -- The tmux command line (NeverCreatesASecondSession) --

#[test]
fn session_argv_creates_or_attaches_in_one_operation() {
    let argv = session_argv(
        "dispatch",
        &["/bin/dispatch".to_string(), "tui".to_string()],
    );
    assert_eq!(
        argv,
        vec![
            "tmux",
            "new-session",
            "-A",
            "-s",
            "dispatch",
            "-n",
            "TUI",
            "--",
            "/bin/dispatch",
            "tui"
        ],
        "-A makes create-or-attach indivisible, so no second board can slip in"
    );
}

#[test]
fn session_argv_names_the_boards_window_up_front() {
    let argv = session_argv("dispatch", &["/bin/dispatch".to_string()]);
    let n = argv.iter().position(|a| a == "-n").expect("-n is passed");
    assert_eq!(
        argv[n + 1],
        BOARD_WINDOW_NAME.as_str(),
        "a board that dies before doing anything must still leave a window \
             the next launch can find and retire"
    );
}

#[test]
fn session_argv_separates_the_inner_command_from_tmux_flags() {
    let argv = session_argv(
        "dispatch",
        &["/bin/dispatch".to_string(), "--db".to_string()],
    );
    let sep = argv.iter().position(|a| a == "--").unwrap();
    assert!(
        argv[sep + 1..].iter().all(|a| a != "-s"),
        "everything after -- belongs to the inner command"
    );
    assert_eq!(argv[sep + 1], "/bin/dispatch");
}

// -- Retiring the previous board (RetireTheBoardWindow) --

#[test]
fn retire_board_window_kills_the_board_window_and_nothing_else() {
    let mock = MockProcessRunner::new(vec![
        // list-panes -s: the session holds the board plus two agents.
        MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n1 %1 task-42\n1 %2 task-43\n"),
        MockProcessRunner::ok_with_stdout(b"4242\n"), // display-message: pane_pid
        MockProcessRunner::ok(),                      // kill-window
        // pane_exists: %0 is gone the moment it is asked about.
        MockProcessRunner::ok_with_stdout(b"%1\n%2\n"),
        MockProcessRunner::ok(), // has-session
        // list-panes -s, read back: the board's window is gone, the
        // agents' are not.
        MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n1 %2 task-43\n"),
    ])
    .with_queued_window_lookup();

    let outcome = retire_board_window_with("dispatch", &mock, &|_| false);

    assert_eq!(
        outcome,
        RetireOutcome::SessionReady,
        "the agent windows keep the session alive"
    );
    let kills: Vec<_> = mock
        .recorded_calls()
        .into_iter()
        .filter(|(_, args)| args.first().is_some_and(|a| a == "kill-window"))
        .collect();
    assert_eq!(kills.len(), 1, "exactly one window is retired");
    assert_eq!(
        kills[0].1,
        vec!["kill-window", "-t", "%0"],
        "RestartingCostsOnlyTheBoard: the agents' windows are not targets"
    );
}

#[test]
fn retire_board_window_does_nothing_when_there_is_no_board_window() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n"), // list-panes -s
        MockProcessRunner::ok(),                              // has-session
    ])
    .with_queued_window_lookup();

    let outcome = retire_board_window("dispatch", &mock);

    assert_eq!(outcome, RetireOutcome::SessionReady);
    assert!(
        !mock
            .recorded_calls()
            .iter()
            .any(|(_, args)| args.first().is_some_and(|a| a == "kill-window")),
        "RetiringAnAbsentBoardWindowSucceeds: nothing to close is not an error"
    );
    assert_eq!(
        mock.recorded_calls().len(),
        2,
        "the lookup already answered what a read-back would ask again"
    );
}

#[test]
fn retire_board_window_waits_for_the_retired_pane_to_go() {
    // `kill-window` removes the window and signals the board; the board's
    // own exit — and the release of the agent port with it — happens
    // afterwards. A launch that raced it told the operator another board
    // was holding the port, moments after they had closed it.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n1 %1 task-42\n"), // list-panes -s
        MockProcessRunner::ok_with_stdout(b"4242\n"),                   // pane_pid
        MockProcessRunner::ok(),                                        // kill-window
        MockProcessRunner::ok_with_stdout(b"%0\n%1\n"),                 // pane_exists: still there
        MockProcessRunner::ok_with_stdout(b"%1\n"),                     // pane_exists: gone
        MockProcessRunner::ok(),                                        // has-session
        MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n"),           // read back
    ])
    .with_queued_window_lookup();

    assert_eq!(
        retire_board_window_with("dispatch", &mock, &|_| false),
        RetireOutcome::SessionReady,
        "SessionReady must mean retired, not merely asked to retire"
    );
    assert_eq!(
        mock.recorded_calls()
            .iter()
            .filter(|(_, args)| args.contains(&"-a".to_string()))
            .count(),
        2,
        "the pane is re-checked until it is gone"
    );
}

#[test]
fn retire_board_window_waits_for_the_board_process_not_just_the_pane() {
    // tmux forgets the pane at once; the board is still stopping the
    // managed store. `RetiredMeansGoneNotSignalled`.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n1 %1 task-42\n"), // list-panes -s
        MockProcessRunner::ok_with_stdout(b"4242\n"),                   // pane_pid
        MockProcessRunner::ok(),                                        // kill-window
        MockProcessRunner::ok_with_stdout(b"%1\n"),                     // pane_exists: gone
        MockProcessRunner::ok_with_stdout(b"%1\n"),                     // pane_exists: gone
        MockProcessRunner::ok_with_stdout(b"%1\n"),                     // pane_exists: gone
        MockProcessRunner::ok(),                                        // has-session
        MockProcessRunner::ok_with_stdout(b"1 %1 task-42\n"),           // read back
    ])
    .with_queued_window_lookup();
    let polls = std::cell::Cell::new(0);
    let alive = |pid: u32| {
        assert_eq!(pid, 4242, "the pid read before the kill is the one watched");
        polls.set(polls.get() + 1);
        polls.get() < 3
    };

    assert_eq!(
        retire_board_window_with("dispatch", &mock, &alive),
        RetireOutcome::SessionReady
    );
    assert_eq!(polls.get(), 3, "the process is re-checked until it is gone");
}

#[test]
fn the_retire_wait_outlasts_the_managed_store_stop() {
    // A board being retired stops its store before it exits; a wait
    // shorter than that hands the relaunch a store mid-shutdown.
    assert!(
        RETIRED_PANE_DEADLINE
            > crate::spacetime::managed_store::MANAGED_STORE_STOP_TIMEOUT
                + std::time::Duration::from_secs(1)
    );
}

#[test]
fn pane_process_alive_sees_this_process_and_not_a_bogus_session() {
    // Under cargo test this process leads or belongs to some session; a
    // session id no process can have is never alive.
    assert!(!pane_process_alive(u32::MAX));
}

#[test]
fn retire_board_window_reports_the_session_gone_when_the_board_was_its_last_window() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n"), // list-panes -s
        MockProcessRunner::ok_with_stdout(b"4242\n"),     // pane_pid
        MockProcessRunner::ok(),                          // kill-window
        MockProcessRunner::ok_with_stdout(b""),           // pane_exists: gone
        MockProcessRunner::fail("no such session"),       // has-session
    ])
    .with_queued_window_lookup();

    assert_eq!(
        retire_board_window_with("dispatch", &mock, &|_| false),
        RetireOutcome::SessionDiscarded,
        "tmux discards a session with no windows, and the launch path must notice"
    );
}

#[test]
fn retire_board_window_reports_a_window_that_would_not_close() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n"), // list-panes -s
        MockProcessRunner::ok_with_stdout(b"4242\n"),     // pane_pid
        MockProcessRunner::fail("can't kill window"),     // kill-window
        MockProcessRunner::ok_with_stdout(b""),           // pane_exists: gone
        MockProcessRunner::ok(),                          // has-session
        MockProcessRunner::ok_with_stdout(b"1 %0 TUI\n"), // list-panes -s, read back
    ])
    .with_queued_window_lookup();

    assert_eq!(
        retire_board_window_with("dispatch", &mock, &|_| false),
        RetireOutcome::BoardWindowSurvived,
        "AFailedRetireIsNotMistakenForSuccess: starting a board here would make two"
    );
}

#[test]
fn retire_board_window_refuses_an_empty_session_name() {
    let mock = MockProcessRunner::new(vec![]).with_queued_window_lookup();

    assert_eq!(
        retire_board_window("", &mock),
        RetireOutcome::BoardWindowSurvived,
        "an unreadable session name must not become a bare `=` target that \
             matches whatever tmux feels like"
    );
    assert!(
        mock.recorded_calls().is_empty(),
        "nothing is asked of tmux without a session to ask about"
    );
}

// -- Attaching after a restart --

#[test]
fn attach_argv_targets_the_session_exactly() {
    assert_eq!(
        attach_argv("dispatch"),
        vec!["tmux", "attach-session", "-t", "=dispatch"],
        "a prefix match would attach to somebody else's session"
    );
}

#[test]
fn current_invocation_uses_the_resolved_executable_not_argv_zero() {
    let argv = current_invocation(
        Path::new("/usr/local/bin/dispatch"),
        vec!["tui".to_string()].into_iter(),
    );
    assert_eq!(argv[0], "/usr/local/bin/dispatch");
    assert_eq!(argv[1], "tui");
}

// -- Launch failure classification (DistinguishesAbsentFromBroken) --

#[test]
fn a_missing_tmux_is_reported_as_unavailable() {
    assert_eq!(
        classify_launch_error(std::io::ErrorKind::NotFound),
        SessionLaunchFailure::TmuxUnavailable
    );
}

#[test]
fn any_other_exec_failure_is_reported_as_rejected() {
    assert_eq!(
        classify_launch_error(std::io::ErrorKind::PermissionDenied),
        SessionLaunchFailure::LaunchRejected
    );
}

#[test]
fn the_two_launch_failures_are_worded_apart() {
    let unavailable = SessionLaunchFailure::TmuxUnavailable.message();
    let rejected = SessionLaunchFailure::LaunchRejected.message();
    assert_ne!(unavailable, rejected);
    assert!(
        unavailable.contains("not on PATH"),
        "a missing tmux must tell the operator to install it: {unavailable}"
    );
    assert!(
        rejected.contains(SESSION_NAME),
        "a refusal must name the session it was refused for: {rejected}"
    );
}

// -- Drift messaging (NamesWhatWillChange) --

#[test]
fn describe_drift_names_every_stale_artefact() {
    let drift = ConfigDrift {
        items: vec![ConfigArtefact::Plugin, ConfigArtefact::StatusLine],
    };
    let msg = describe_drift(&drift);
    assert!(msg.contains(ConfigArtefact::Plugin.label()), "{msg}");
    assert!(msg.contains(ConfigArtefact::StatusLine.label()), "{msg}");
}

#[test]
fn every_artefact_has_a_label_and_they_are_distinct() {
    let labels: Vec<&str> = ConfigArtefact::ALL.iter().map(|a| a.label()).collect();
    assert_eq!(
        labels.len(),
        ConfigArtefact::ALL.len(),
        "ConfigArtefact::ALL must list every variant"
    );
    for (i, a) in labels.iter().enumerate() {
        assert!(!a.is_empty(), "an unnamed artefact cannot be consented to");
        for b in &labels[i + 1..] {
            assert_ne!(
                a, b,
                "two artefacts sharing a label are one line in the prompt"
            );
        }
    }
}

// -- The configuration check: the four StartupConfigOutcome values --

/// Setup paths under a temp root, so no test can reach the operator's real
/// configuration directory.
fn layout(root: &Path) -> SetupPaths {
    let claude_dir = root.join(".claude");
    SetupPaths {
        claude_dir: claude_dir.clone(),
        mcp_path: root.join(".claude.json"),
        legacy_mcp_path: claude_dir.join(".mcp.json"),
        tmux_conf_path: root.join(".tmux.conf"),
        statusline_path: crate::setup::statusline::settings_path(&claude_dir),
        budget_snapshot_path: root.join("data").join("rate-limits.json"),
    }
}

/// Where the fixtures put `<data_dir>/scripts/`. Beside the configuration
/// directory rather than inside it, mirroring the real layout.
fn data_dir(root: &Path) -> std::path::PathBuf {
    root.join("data")
}

/// A tmux runner reporting focus-events already on, so a test that is not
/// about tmux needs to queue only the one process result.
fn focus_events_on() -> MockProcessRunner {
    MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")])
}

/// Bring every artefact current, so a test can assert on a clean check.
fn make_current(paths: &SetupPaths, data_dir: &Path, port: u16) {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    let confirmer = FakeConfirmer::never();
    let ctx = ConfigContext {
        paths,
        port,
        runner: &runner,
        data_dir,
        confirmer: Some(&confirmer),
    };
    let drift = crate::setup::inspect_config_drift_in(&ctx);
    let failed = crate::setup::apply_config_update_in(&drift, &ctx);
    assert!(failed.is_empty(), "fixture setup must not fail: {failed:?}");
}

#[test]
fn a_current_installation_resolves_silently() {
    let root = tempfile::tempdir().unwrap();
    let paths = layout(root.path());
    make_current(&paths, &data_dir(root.path()), 3142);

    // never(): AbsentWhenNothingIsStale — no prompt may fire.
    let confirmer = FakeConfirmer::never();
    let outcome = resolve_startup_config_in(
        &paths,
        &data_dir(root.path()),
        3142,
        &focus_events_on(),
        Some(&confirmer),
    );

    assert_eq!(outcome, StartupConfigOutcome::AlreadyCurrent);
    assert_eq!(confirmer.confirm_call_count(), 0);
}

#[test]
fn stale_configuration_is_written_once_the_operator_agrees() {
    let root = tempfile::tempdir().unwrap();
    let paths = layout(root.path());
    let runner = MockProcessRunner::new(vec![
        // The inspect's read, then the apply's read and set-option.
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    let confirmer = FakeConfirmer::new(vec![true], vec![]);

    let outcome = resolve_startup_config_in(
        &paths,
        &data_dir(root.path()),
        3142,
        &runner,
        Some(&confirmer),
    );

    assert_eq!(outcome, StartupConfigOutcome::Updated);
    assert_eq!(
        confirmer.confirm_call_count(),
        1,
        "one prompt for the whole report, not one per artefact"
    );
    assert!(
        paths.mcp_path.exists(),
        "consent is the one path that writes"
    );
}

#[test]
fn declining_writes_nothing_and_still_resolves() {
    let root = tempfile::tempdir().unwrap();
    let paths = layout(root.path());
    let confirmer = FakeConfirmer::new(vec![false], vec![]);

    let outcome = resolve_startup_config_in(
        &paths,
        &data_dir(root.path()),
        3142,
        &focus_events_on(),
        Some(&confirmer),
    );

    assert_eq!(outcome, StartupConfigOutcome::Declined);
    assert!(
        !paths.mcp_path.exists(),
        "ConfigurationIsNeverWrittenWithoutConsent: a declined prompt writes nothing"
    );
    assert!(!paths.claude_dir.exists(), "not even the directory");
}

#[test]
fn a_non_interactive_launch_reports_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let paths = layout(root.path());

    // No confirmer at all: nobody can answer.
    let outcome = resolve_startup_config_in(
        &paths,
        &data_dir(root.path()),
        3142,
        &focus_events_on(),
        None,
    );

    assert_eq!(outcome, StartupConfigOutcome::ReportedOnly);
    assert!(
        !paths.mcp_path.exists(),
        "treating silence as consent is how a script rewrites someone else's home directory"
    );
    assert!(!paths.claude_dir.exists());
}

#[test]
fn a_non_interactive_launch_with_nothing_stale_is_already_current() {
    let root = tempfile::tempdir().unwrap();
    let paths = layout(root.path());
    make_current(&paths, &data_dir(root.path()), 3142);

    let outcome = resolve_startup_config_in(
        &paths,
        &data_dir(root.path()),
        3142,
        &focus_events_on(),
        None,
    );

    assert_eq!(
        outcome,
        StartupConfigOutcome::AlreadyCurrent,
        "SilentWhenClean holds whether or not anyone could be asked"
    );
}

/// An unreadable stdin is an operator who could not answer, not one who
/// said yes. Without this the `Err` arm could plausibly be written as
/// "proceed", which would write configuration nobody approved.
#[test]
fn an_unanswerable_prompt_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let paths = layout(root.path());

    let outcome = resolve_startup_config_in(
        &paths,
        &data_dir(root.path()),
        3142,
        &focus_events_on(),
        Some(&FailingConfirmer),
    );

    assert_eq!(outcome, StartupConfigOutcome::ReportedOnly);
    assert!(!paths.claude_dir.exists());
}

// -- The host-label gate (CheckHostLabel and its label-side children;
//    see the identity arm's own tests in `mod bootstrap` in
//    src/runtime/tests/misc.rs) --

#[test]
fn an_already_named_host_is_not_prompted() {
    let confirmer = FakeConfirmer::never();

    let outcome = resolve_host_label(Some("my-laptop"), "fallback-hostname", Some(&confirmer));

    assert_eq!(
        outcome,
        Ok(None),
        "ContinueWhenTheHostIsAlreadyNamed: nothing to persist when already named"
    );
    assert_eq!(confirmer.text_call_count(), 0);
}

#[test]
fn an_already_named_host_is_not_prompted_even_non_interactively() {
    // A named host must never abort, whether or not anyone could answer —
    // TheBoardNeverDrawsForAnUnnamedHost only ever blocks an UNNAMED host.
    let outcome = resolve_host_label(Some("my-laptop"), "fallback-hostname", None);

    assert_eq!(outcome, Ok(None));
}

#[test]
fn an_unnamed_host_is_prompted_with_the_hostname_prefilled() {
    let confirmer = FakeConfirmer::with_text(vec![], vec![], vec!["some-laptop".to_string()]);

    let outcome = resolve_host_label(None, "some-laptop", Some(&confirmer));

    assert_eq!(
        outcome,
        Ok(Some("some-laptop".to_string())),
        "PromptForHostLabelWhenUnnamed: the operator's answer is handed back to persist"
    );
    assert_eq!(confirmer.text_call_count(), 1);
}

#[test]
fn accepting_the_prefilled_hostname_names_the_host() {
    // StdinConfirmer::prompt_text substitutes the default for empty input,
    // so a confirmer honouring that contract returns the hostname back —
    // exercised here via a fake standing in for "operator pressed enter".
    let confirmer =
        FakeConfirmer::with_text(vec![], vec![], vec!["my-machine-hostname".to_string()]);

    let outcome = resolve_host_label(None, "my-machine-hostname", Some(&confirmer));

    assert_eq!(outcome, Ok(Some("my-machine-hostname".to_string())));
}

#[test]
fn a_blank_answer_is_asked_again_rather_than_accepted() {
    // ThereIsNoWayPast: a confirmer that hands back whitespace is asked
    // again rather than being treated as a way past the gate.
    let confirmer = FakeConfirmer::with_text(
        vec![],
        vec![],
        vec!["   ".to_string(), "real-name".to_string()],
    );

    let outcome = resolve_host_label(None, "fallback-hostname", Some(&confirmer));

    assert_eq!(outcome, Ok(Some("real-name".to_string())));
    assert_eq!(
        confirmer.text_call_count(),
        2,
        "a blank answer must not resolve the gate on the first ask"
    );
}

#[test]
fn a_non_interactive_launch_aborts_on_an_unnamed_host() {
    // AbortWhenTheHostIsUnnamedAndNoOneCanAnswer: no confirmer at all —
    // a script, a CI job, stdin redirected from nowhere.
    let outcome = resolve_host_label(None, "fallback-hostname", None);

    assert_eq!(outcome, Err(StartupAbort::HostUnnamed));
}

#[test]
fn an_unanswerable_host_label_prompt_aborts_rather_than_proceeding_unnamed() {
    // Unlike the configuration prompt's unreadable-stdin case (which
    // degrades to ReportedOnly), a host that cannot be named must abort —
    // TheBoardNeverDrawsForAnUnnamedHost has no non-fatal counterpart.
    let outcome = resolve_host_label(None, "fallback-hostname", Some(&FailingConfirmer));

    assert_eq!(outcome, Err(StartupAbort::HostUnnamed));
}

#[test]
fn host_unnamed_is_worded_distinctly_and_names_the_remedy() {
    let msg = StartupAbort::HostUnnamed.message();
    assert!(
        msg.contains("dispatch tui"),
        "the remedy — one interactive launch — must be named: {msg}"
    );
    for other in [
        StartupAbort::TmuxUnavailable,
        StartupAbort::LaunchRejected,
        StartupAbort::BoardAlreadyInThisWindow,
        StartupAbort::PreviousBoardNotRetired,
        StartupAbort::SessionUnidentified,
        StartupAbort::AgentPortUnavailable { port: 1234 },
        StartupAbort::HostIdentityUnavailable,
    ] {
        assert_ne!(msg, other.message());
    }
}

/// startup.allium: `AbortWhenTheHostIdentityStoreIsUnusable`. This is a
/// different failure from `HostUnnamed` — either the identity could not
/// be read or minted at all, or an unnamed machine's label could not be
/// written back, so there is no label question worth asking and "run
/// `dispatch tui` interactively to name it" would not help. The message
/// must say so distinctly rather than reusing `HostUnnamed`'s remedy.
#[test]
fn host_identity_unavailable_is_worded_distinctly_and_names_the_remedy() {
    let msg = StartupAbort::HostIdentityUnavailable.message();
    assert!(
        !msg.contains("dispatch tui interactively"),
        "nothing is asking for a name, so the remedy must not be the \
             one-time naming prompt: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("identity") || msg.to_lowercase().contains("settings"),
        "the message must explain that the host's identity itself could \
             not be determined, not that it merely lacks a label: {msg}"
    );
    for other in [
        StartupAbort::TmuxUnavailable,
        StartupAbort::LaunchRejected,
        StartupAbort::BoardAlreadyInThisWindow,
        StartupAbort::PreviousBoardNotRetired,
        StartupAbort::SessionUnidentified,
        StartupAbort::AgentPortUnavailable { port: 1234 },
        StartupAbort::HostUnnamed,
    ] {
        assert_ne!(msg, other.message());
    }
}
