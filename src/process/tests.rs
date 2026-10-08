use super::*;
use std::os::unix::fs::PermissionsExt;

/// Every artefact that records a dispatch invocation composes this name,
/// and every one of them must stay a BARE command that its runner resolves
/// on `PATH`. A name carrying a directory would make all three absolute at
/// once — the defect `startup.allium`'s `TheHelperIsTheBareCommandName`
/// exists to keep closed — so the guard belongs here rather than on each
/// composed constant.
#[test]
fn dispatch_program_names_no_directory() {
    assert!(
        !DISPATCH_PROGRAM.contains(std::path::MAIN_SEPARATOR),
        "the program name must be bare, got {DISPATCH_PROGRAM}"
    );
}

// --- Timeouts ---

/// A network hiccup during `git fetch`/`git worktree add` (slow DNS, a
/// flaky TLS handshake) must have enough room to recover before dispatch
/// gives up on it.
#[test]
fn subprocess_timeout_is_at_least_two_minutes() {
    assert!(
        SUBPROCESS_TIMEOUT >= Duration::from_secs(120),
        "SUBPROCESS_TIMEOUT is {SUBPROCESS_TIMEOUT:?}, needs to be at least 2 minutes"
    );
}

// --- AgentBinaries ---

#[test]
fn agent_binaries_defaults_to_bare_names() {
    let bins = AgentBinaries::default();
    assert_eq!(bins.claude, "claude");
    assert_eq!(bins.dispatch, "dispatch");
}

/// Production must keep resolving on `PATH`: the trait default is what every
/// real runner inherits, and no production code overrides it.
#[test]
fn real_process_runner_uses_default_agent_binaries() {
    assert_eq!(
        RealProcessRunner::default().agent_binaries(),
        AgentBinaries::default()
    );
}

/// The default must be inert. The opposite default would have every
/// fixture in the suite reading the developer's real `~/.claude.json`
/// unless it remembered to opt out, and the one that forgot would leave no
/// trace.
#[test]
fn claude_json_path_defaults_to_none() {
    assert!(MockProcessRunner::new(vec![]).claude_json_path().is_none());
}

/// Handed in, never looked up: a runner nobody told about the operator's
/// config cannot reach it. See `SettingsLocationIsAnExplicitStartupInput`.
#[test]
fn real_process_runner_names_no_claude_json_until_it_is_given_one() {
    assert!(RealProcessRunner::default().claude_json_path().is_none());
}

// --- sccache_available ---

#[test]
fn mock_sccache_available_defaults_to_false() {
    assert!(!MockProcessRunner::new(vec![]).sccache_available());
}

#[test]
fn mock_with_sccache_available_overrides_the_default() {
    let mock = MockProcessRunner::new(vec![]).with_sccache_available(true);
    assert!(mock.sccache_available());
}

#[test]
fn sccache_in_dirs_finds_an_executable_named_sccache() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("sccache");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(sccache_in_dirs(std::iter::once(dir.path().to_path_buf())));
}

#[test]
fn sccache_in_dirs_ignores_a_non_executable_file_named_sccache() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("sccache");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(!sccache_in_dirs(std::iter::once(dir.path().to_path_buf())));
}

#[test]
fn sccache_in_dirs_false_when_no_dir_has_it() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!sccache_in_dirs(std::iter::once(dir.path().to_path_buf())));
}

#[test]
fn real_process_runner_reports_the_claude_json_startup_handed_it() {
    let path = std::path::PathBuf::from("/some/where/.claude.json");
    assert_eq!(
        RealProcessRunner::with_claude_json(path.clone()).claude_json_path(),
        Some(path)
    );
}

#[test]
fn mock_process_runner_reports_the_claude_json_it_was_built_with() {
    let path = std::path::Path::new("/tmp/some-claude.json");
    let mock = MockProcessRunner::new(vec![]).with_claude_json(path);
    assert_eq!(mock.claude_json_path().as_deref(), Some(path));
}

#[test]
fn mock_process_runner_reports_the_binaries_it_was_built_with() {
    let bins = AgentBinaries::stub();
    let mock = MockProcessRunner::new(vec![]).with_agent_binaries(bins.clone());
    assert_eq!(mock.agent_binaries(), bins);
}

/// The pass-through case is what keeps the bare default names out of the
/// emitted command string unquoted, exactly as they were before.
#[test]
fn shell_quote_leaves_plain_paths_untouched() {
    for s in ["claude", "/tmp/x/claude", "./claude", "claude-1.2_beta"] {
        assert_eq!(shell_quote(s), s, "{s} should need no quoting");
    }
}

#[test]
fn shell_quote_wraps_paths_needing_it() {
    assert_eq!(shell_quote("/my dir/claude"), "'/my dir/claude'");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
}

/// An embedded single quote cannot be escaped *inside* single quotes, so it
/// has to close, escape and reopen — get this wrong and the quoting is a
/// shell injection rather than a defence against one.
#[test]
fn shell_quote_escapes_embedded_single_quotes() {
    assert_eq!(shell_quote("it's"), r"'it'\''s'");
}

#[test]
fn claude_quoted_passes_a_plain_name_through_unchanged() {
    assert_eq!(AgentBinaries::default().claude_quoted(), "claude");
}

/// Executed rather than string-compared: the value is interpolated into
/// exactly the shape `dispatch_with_prompt` emits — as bash's `$0`, *after*
/// the single-quoted script body — handed to a real shell, and the binary
/// must run as a single word with its argument intact.
///
/// This is what pins the `$0` arrangement in place. Move the binary back
/// inside the quoted body and a path with a space needs escaping twice; this
/// test fails for `"claude bin"` if anyone does.
#[test]
fn claude_quoted_survives_the_launcher_command_shape() {
    for name in ["claude bin", "cla'ude", "claude"] {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(name);
        std::fs::write(&bin, "#!/bin/sh\nprintf 'ran:%s' \"$1\"\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

        let bins = AgentBinaries {
            claude: bin.to_string_lossy().into_owned(),
            ..AgentBinaries::default()
        };
        let claude = bins.claude_quoted();
        let cmd = format!("bash -c '\"$0\" arg' {claude}");

        let out = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "ran:arg",
            "quoting failed for {name:?}; command was: {cmd} (stderr: {})",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn agent_binaries_stub_names_both_binaries_distinctly() {
    let stub = AgentBinaries::stub();
    assert_ne!(stub, AgentBinaries::default());
    assert_ne!(stub.claude, stub.dispatch);
}

// --- RealProcessRunner::run_with_timeout ---

#[test]
fn real_run_with_timeout_returns_output_on_success() {
    let runner = RealProcessRunner::default();
    let result = runner.run_with_timeout("true", &[], Duration::from_secs(5));
    assert!(result.is_ok(), "expected success, got: {result:?}");
    assert!(result.unwrap().status.success());
}

#[test]
fn real_run_with_timeout_kills_stuck_process_and_returns_error() {
    let runner = RealProcessRunner::default();
    // sleep 10 will be killed after 100ms timeout
    let result = runner.run_with_timeout("sleep", &["10"], Duration::from_millis(100));
    assert!(result.is_err(), "expected timeout error, got success");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("timed out") || msg.contains("killed"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn real_run_with_timeout_captures_stdout() {
    let runner = RealProcessRunner::default();
    let result = runner.run_with_timeout("echo", &["hello"], Duration::from_secs(5));
    assert!(result.is_ok());
    let output = result.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"), "stdout: {stdout:?}");
}

/// A missing binary (e.g. tmux not installed) must surface as a spawn error
/// with the program name in the context, not a panic or silent success.
const MISSING_BINARY: &str = "dispatch-nonexistent-binary-xyzzy";

#[test]
fn real_run_missing_binary_returns_error() {
    let runner = RealProcessRunner::default();
    let result = runner.run(MISSING_BINARY, &[]);
    assert!(result.is_err(), "expected spawn error, got: {result:?}");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains(MISSING_BINARY),
        "error should name the missing program, got: {msg}"
    );
}

#[test]
fn real_run_with_timeout_missing_binary_returns_error() {
    let runner = RealProcessRunner::default();
    let result = runner.run_with_timeout(MISSING_BINARY, &[], Duration::from_secs(5));
    assert!(result.is_err(), "expected spawn error, got: {result:?}");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains(MISSING_BINARY),
        "error should name the missing program, got: {msg}"
    );
}

// --- run_bounded ---
//
// The one bounded-child primitive: `run_with_timeout` delegates to it, and so
// does the statusline decorator's chained command (`src/cli/statusline.rs`),
// which is why the stdin-writing hazards below live here rather than there.

#[test]
fn run_bounded_writes_stdin_and_returns_stdout() {
    let out = run_bounded("cat", &[], Some("payload"), Duration::from_secs(5)).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "payload");
}

/// Writing all of stdin before draining stdout deadlocks once the payload
/// exceeds the pipe buffer (~64 KiB on Linux) against a child that echoes as
/// it reads: the child blocks on its full, undrained stdout while the parent
/// blocks writing more to the child's full, undrained stdin. The stdin writer
/// runs on its own thread precisely so neither side can stall.
#[test]
fn run_bounded_does_not_deadlock_on_a_large_payload() {
    let payload = "x".repeat(200_000);
    let out = run_bounded("cat", &[], Some(&payload), Duration::from_secs(5)).unwrap();
    assert_eq!(out.stdout.len(), payload.len());
    assert_eq!(String::from_utf8_lossy(&out.stdout), payload);
}

/// A child that never reads stdin gives the writer thread an `EPIPE`. That is
/// an ordinary outcome, not a failure: the call must still return the child's
/// output rather than hanging or propagating the write error.
#[test]
fn run_bounded_ignores_a_child_that_never_reads_stdin() {
    let out = run_bounded("echo", &["hi"], Some("unread"), Duration::from_secs(5)).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
}

/// A child that closes stdout and keeps running would slip past any bound
/// placed on *output* alone into an unbounded wait. The deadline covers the
/// exit too. See docs/specs/observability.allium: StatusLineDecorator
/// (`@guarantee ChainedCommandIsBounded`).
///
/// `run_bounded` is synchronous, so the bound is expressed by running it on
/// its own thread and waiting on its completion signal rather than by
/// asserting on measured elapsed time — see "Bounding a wait is not the
/// same as asserting on one" in docs/conventions.md.
#[test]
fn run_bounded_kills_a_child_that_closed_stdout_but_keeps_running() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = run_bounded(
            "sh",
            &["-c", "exec 1>&- ; sleep 30"],
            None,
            Duration::from_millis(100),
        );
        let _ = tx.send(result);
    });

    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the wait for the child to exit must be bounded by the 100ms deadline");
    assert!(result.is_err(), "expected a timeout error, got {result:?}");
}

#[test]
fn run_bounded_reports_stderr_and_a_failing_exit_status() {
    let out = run_bounded(
        "sh",
        &["-c", "echo oops >&2; exit 3"],
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "oops");
}

/// A child abandoned at the deadline contributes nothing — not even output it
/// produced before it stopped making progress. Deliberate, and stated in
/// docs/specs/observability.allium: StatusLineDecorator
/// (`@guarantee ChainedCommandIsBounded`), whose chained command is one caller.
#[test]
fn run_bounded_discards_output_from_a_child_that_then_overruns() {
    let result = run_bounded(
        "sh",
        &["-c", "echo partial ; exec 1>&- ; sleep 30"],
        None,
        Duration::from_millis(100),
    );
    assert!(
        result.is_err(),
        "output before the overrun must not rescue it, got {result:?}"
    );
}

// --- MockProcessRunner::run_with_timeout ---

#[test]
fn mock_run_with_timeout_records_call() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    mock.run_with_timeout("git", &["fetch"], Duration::from_secs(5))
        .unwrap();
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "git");
    assert_eq!(calls[0].1, vec!["fetch"]);
}

#[test]
fn mock_run_with_timeout_succeeds_when_delay_within_timeout() {
    let mock = MockProcessRunner::new_with_delays(vec![(
        Some(Duration::from_millis(10)),
        MockProcessRunner::ok(),
    )]);
    let result = mock.run_with_timeout("git", &["fetch"], Duration::from_millis(500));
    assert!(result.is_ok(), "expected success, got: {result:?}");
}

#[test]
fn mock_run_with_timeout_returns_error_when_delay_exceeds_timeout() {
    let mock = MockProcessRunner::new_with_delays(vec![(
        Some(Duration::from_millis(200)),
        MockProcessRunner::ok(),
    )]);
    let result = mock.run_with_timeout("git", &["fetch"], Duration::from_millis(50));
    assert!(result.is_err(), "expected timeout error, got success");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("timed out") || msg.contains("killed"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn mock_run_with_timeout_no_delay_always_succeeds() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let result = mock.run_with_timeout("git", &["status"], Duration::from_millis(1));
    assert!(result.is_ok());
}

#[test]
fn fail_with_code_reports_the_requested_exit_code() {
    let out = MockProcessRunner::fail_with_code(2, "no matching ref").unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stderr), "no matching ref");
}

// Whether a call was bounded is not visible in its argv, so the mock records
// it separately — otherwise "this subprocess is bounded" can only be tested
// by letting an unbounded one hang, which fails the suite by timing out
// rather than by asserting.
#[test]
fn mock_records_the_timeout_each_call_was_made_with() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok(), MockProcessRunner::ok()]);
    mock.run("git", &["status"]).unwrap();
    mock.run_with_timeout("git", &["fetch"], Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        mock.recorded_timeouts(),
        vec![None, Some(Duration::from_secs(5))],
        "an unbounded call records no timeout, a bounded one records its own"
    );
    assert_eq!(
        mock.recorded_timeouts().len(),
        mock.recorded_calls().len(),
        "timeouts must line up positionally with the calls they belong to"
    );
}

// Out-of-band window lookups are not recorded as calls, so they must not
// shift the timeouts out of alignment with them either.
#[test]
fn mock_timeouts_stay_aligned_across_an_out_of_band_window_lookup() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let _ = mock.run(
        "tmux",
        &[
            "list-panes",
            "-a",
            "-f",
            "#{==:#{window_name},task-1}",
            "-F",
            crate::tmux::WINDOW_PANE_FORMAT,
        ],
    );
    mock.run_with_timeout("git", &["fetch"], Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        mock.recorded_timeouts(),
        vec![Some(Duration::from_secs(5))],
        "the intercepted lookup records neither a call nor a timeout"
    );
    assert_eq!(mock.recorded_calls().len(), 1);
}
