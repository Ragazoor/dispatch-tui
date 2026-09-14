//! What happens between the operator typing `dispatch tui` and the board
//! drawing its first frame: obtaining the tmux session the board requires, and
//! resolving any configuration drift before the screen is taken over.
//!
//! See `docs/specs/startup.allium`. Both halves live here because they are one
//! ordered sequence with one shared property — the board draws afterwards
//! either way — and splitting them would leave no single place that states the
//! order.

use std::path::Path;

use crate::process::{ProcessRunner, RealProcessRunner};
use crate::setup::{
    apply_config_update_in, inspect_config_drift_in, ConfigArtefact, ConfigContext, ConfigDrift,
    Confirmer, SetupPaths, StdinConfirmer,
};

/// The tmux session `dispatch tui` creates for itself when run outside one.
/// `startup.allium`'s `config.session_name`.
///
/// Fixed rather than derived: it is the name the operator reattaches to by
/// hand, and a name that varies per invocation cannot be reattached to.
pub const SESSION_NAME: &str = "dispatch";

/// Why the command could not put itself inside a tmux session. Both are fatal
/// — `startup.allium`'s `SessionFailureIsTheOnlyFatalStartup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLaunchFailure {
    /// No tmux on `PATH`.
    TmuxUnavailable,
    /// tmux was reached and refused to start or attach.
    LaunchRejected,
}

impl SessionLaunchFailure {
    /// The operator-facing message. The two cases are worded apart because the
    /// operator's next action differs: install it, or look at why the server
    /// refused (`SessionLauncher`'s `DistinguishesAbsentFromBroken`).
    pub fn message(self) -> String {
        match self {
            Self::TmuxUnavailable => "dispatch tui needs tmux, which is not on PATH. Install it \
                 (e.g. `sudo dnf install tmux`) and run `dispatch tui` again."
                .to_string(),
            Self::LaunchRejected => format!(
                "tmux refused to start or attach to the `{SESSION_NAME}` session. \
                 Check `tmux list-sessions` and the tmux server's state."
            ),
        }
    }
}

/// What the launch path decided, computed without touching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Already inside a tmux session — this process carries on and draws the
    /// board. `startup.allium`'s `LaunchBoardInsideExistingSession`.
    ContinueHere,
    /// Not inside one — replace this process with the same invocation running
    /// inside `session`. `startup.allium`'s `LaunchBoardBySupplyingASession`.
    EnterSession { session: String, argv: Vec<String> },
}

/// Decide how to obtain the session the board needs.
///
/// Takes `inside_tmux` rather than reading `$TMUX` so the decision is testable
/// without an environment the test harness shares across threads (the same
/// shape as `setup::home_dir_from_value`).
pub fn plan_launch(inside_tmux: bool, argv: Vec<String>) -> LaunchPlan {
    if inside_tmux {
        LaunchPlan::ContinueHere
    } else {
        LaunchPlan::EnterSession {
            session: SESSION_NAME.to_string(),
            argv,
        }
    }
}

/// The exact tmux command line that creates-or-attaches `session` and runs
/// `argv` inside it.
///
/// `-A` is what upholds `SessionLauncher`'s `NeverCreatesASecondSession`:
/// create-or-attach is one indivisible tmux operation, so no second board can
/// appear between a check and a create. When the session already exists tmux
/// attaches and ignores `argv` entirely, which is the spec's
/// `AttachToTheExistingSession`.
pub fn session_argv(session: &str, argv: &[String]) -> Vec<String> {
    let mut out = vec![
        "tmux".to_string(),
        "new-session".to_string(),
        "-A".to_string(),
        "-s".to_string(),
        session.to_string(),
    ];
    out.push("--".to_string());
    out.extend(argv.iter().cloned());
    out
}

/// This invocation, rebuilt so it can be handed to tmux: the resolved binary
/// path followed by the arguments the operator passed.
///
/// `argv[0]` comes from the running executable rather than from `args()`,
/// which may hold a relative path that no longer resolves once tmux has
/// changed the working directory.
pub fn current_invocation(exe: &Path, args: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = vec![exe.display().to_string()];
    out.extend(args);
    out
}

/// Replace this process with one inside `session`.
///
/// Only ever returns on failure — on success there is no "after".
#[cfg(unix)]
pub fn enter_session(session: &str, argv: &[String]) -> SessionLaunchFailure {
    use std::os::unix::process::CommandExt;

    let full = session_argv(session, argv);
    // `full` always has at least the five fixed leading elements.
    let err = std::process::Command::new(&full[0]).args(&full[1..]).exec();
    classify_launch_error(err.kind())
}

/// Which failure an exec error reports. Split out so the mapping is testable
/// without actually failing to exec.
pub fn classify_launch_error(kind: std::io::ErrorKind) -> SessionLaunchFailure {
    match kind {
        std::io::ErrorKind::NotFound => SessionLaunchFailure::TmuxUnavailable,
        _ => SessionLaunchFailure::LaunchRejected,
    }
}

/// Whether this process is already inside a tmux session.
/// `SessionLauncher.inside_tmux_session`.
///
/// The one definition, shared by the launch handoff in `src/main.rs` and by
/// `runtime::run_tui`'s guard. An empty `TMUX` counts as outside: a shell
/// spells "unset" both by omitting the variable and by setting it to nothing,
/// and only one of those spellings counting would put the two readers on
/// different sides of the same environment (the same reasoning as
/// `setup::home_dir_from_value`).
pub fn inside_tmux_session() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// The configuration check — startup.allium's CheckStartupConfig rules
// ---------------------------------------------------------------------------

/// How the startup configuration check ended.
/// `startup.allium`'s `StartupConfigOutcome`.
///
/// Every value is non-fatal: the board draws after all four
/// (`ConfigurationDriftNeverBlocksTheBoard`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupConfigOutcome {
    /// Nothing was out of date; nothing was said.
    AlreadyCurrent,
    /// The operator agreed and the artefacts were written.
    Updated,
    /// The operator was asked and said no.
    Declined,
    /// No one could be asked, so nothing was written.
    ReportedOnly,
}

/// The line shown to the operator when configuration is stale, whether they
/// are about to be asked or merely told.
///
/// Names every stale artefact: `ConfigUpdatePrompt`'s `NamesWhatWillChange`
/// forbids asking for approval of an unnamed set of writes.
fn describe_drift(drift: &ConfigDrift) -> String {
    format!(
        "Dispatch configuration is out of date: {}.",
        list_artefacts(&drift.items)
    )
}

/// The artefact labels as the operator reads them. One formatting of the list,
/// used by the drift line and by the partial-failure warning, so the two cannot
/// punctuate the same set differently.
fn list_artefacts(items: &[ConfigArtefact]) -> String {
    items
        .iter()
        .map(|a| a.label())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve configuration drift on the way to the board, against the real
/// `$HOME`-derived locations.
///
/// Synchronous and blocking: it reads a dozen files, walks the installed plugin
/// tree, spawns a tmux subprocess, and may block on stdin waiting for an
/// answer. Callers on an async runtime must run it on a blocking thread.
pub fn resolve_startup_config(
    paths: &SetupPaths,
    data_dir: &std::path::Path,
    port: u16,
    interactive: bool,
) -> StartupConfigOutcome {
    let confirmer = StdinConfirmer;
    resolve_startup_config_in(
        paths,
        data_dir,
        port,
        &RealProcessRunner::default(),
        interactive.then_some(&confirmer as &dyn Confirmer),
    )
}

/// Injectable core of [`resolve_startup_config`]. The three-way branch is
/// `startup.allium`'s `CheckStartupConfigWhenNothingIsStale` /
/// `PromptToUpdateStaleConfig` / `ReportStaleConfigWhenNoOneCanAnswer`.
///
/// Returns an outcome rather than a `Result` on purpose. Every internal failure
/// is folded into the report it already prints, so there is no `Err` for a
/// caller to propagate and therefore no way to make configuration drift fatal —
/// `ConfigurationDriftNeverBlocksTheBoard` holds by type rather than by every
/// call site remembering to swallow the error.
///
/// `confirmer` is `None` when nobody can answer — a script, a CI job, stdin
/// redirected from nowhere. The absence is the input, not a flag beside one:
/// there is then no code path that could read a queued "yes" out of silence.
pub(crate) fn resolve_startup_config_in(
    paths: &SetupPaths,
    data_dir: &std::path::Path,
    port: u16,
    runner: &dyn ProcessRunner,
    confirmer: Option<&dyn Confirmer>,
) -> StartupConfigOutcome {
    let ctx = ConfigContext {
        paths,
        port,
        runner,
        data_dir,
        confirmer,
    };
    let drift = inspect_config_drift_in(&ctx);

    if drift.is_clean() {
        // Silent on purpose. A line reporting four current artefacts on every
        // launch trains the operator to skip the region of the screen where the
        // one line that matters appears.
        return StartupConfigOutcome::AlreadyCurrent;
    }

    let Some(confirmer) = confirmer else {
        // Treating silence as consent is how a scripted run rewrites the home
        // directory of whoever happened to start it.
        eprintln!(
            "{} Nothing was written — run `dispatch tui` from a terminal to update it.",
            describe_drift(&drift)
        );
        return StartupConfigOutcome::ReportedOnly;
    };

    // One prompt for the whole report: the artefacts are named in it, but they
    // are not separable decisions — a half-updated installation is a
    // configuration nobody chose and nothing tests.
    eprintln!("{}", describe_drift(&drift));
    match confirmer.confirm("Update it now?") {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "Configuration left as it is. The board will start; \
                 you will be asked again next launch."
            );
            return StartupConfigOutcome::Declined;
        }
        Err(e) => {
            // An unreadable stdin is an operator who could not answer, not one
            // who said yes.
            eprintln!("Warning: could not read an answer ({e:#}). Nothing was written.");
            return StartupConfigOutcome::ReportedOnly;
        }
    }

    let failed = apply_config_update_in(&drift, &ctx);
    if !failed.is_empty() {
        eprintln!(
            "Warning: could not update {}. The board will start; \
             this is retried next launch.",
            list_artefacts(&failed)
        );
    }
    StartupConfigOutcome::Updated
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::process::MockProcessRunner;
    use crate::setup::FakeConfirmer;

    // -- Launch planning (LaunchBoardInsideExistingSession /
    //    LaunchBoardBySupplyingASession) --

    #[test]
    fn plan_launch_inside_tmux_continues_in_this_process() {
        let plan = plan_launch(true, vec!["/bin/dispatch".into(), "tui".into()]);
        assert_eq!(
            plan,
            LaunchPlan::ContinueHere,
            "an operator who already had a session keeps it"
        );
    }

    #[test]
    fn plan_launch_outside_tmux_enters_the_dispatch_session() {
        let argv = vec!["/bin/dispatch".to_string(), "tui".to_string()];
        let plan = plan_launch(false, argv.clone());
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
    fn plan_launch_carries_the_operators_arguments_through() {
        let argv = vec![
            "/bin/dispatch".to_string(),
            "--db".to_string(),
            "/tmp/scratch.db".to_string(),
            "tui".to_string(),
            "--port".to_string(),
            "9999".to_string(),
        ];
        match plan_launch(false, argv.clone()) {
            LaunchPlan::EnterSession { argv: carried, .. } => assert_eq!(
                carried, argv,
                "ArgvIsCarriedThrough: the board must come up with the db and port asked for"
            ),
            other => panic!("expected EnterSession, got {other:?}"),
        }
    }

    // -- The tmux command line (NeverCreatesASecondSession /
    //    AttachToTheExistingSession) --

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
                "--",
                "/bin/dispatch",
                "tui"
            ],
            "-A makes create-or-attach indivisible, so no second board can slip in"
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
        struct FailingConfirmer;
        impl Confirmer for FailingConfirmer {
            fn confirm(&self, _: &str) -> anyhow::Result<bool> {
                Err(anyhow::anyhow!("stdin closed"))
            }
            fn confirm_dangerous(&self, _: &str) -> anyhow::Result<bool> {
                Err(anyhow::anyhow!("stdin closed"))
            }
        }

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
}
