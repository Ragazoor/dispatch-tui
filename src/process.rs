use anyhow::{Context, Result};
#[cfg(any(test, feature = "test-support"))]
use std::collections::VecDeque;
use std::process::Output;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Canonical timeout for long-running subprocesses (git fetch, worktree add).
/// `DISPATCH_WATCHDOG_TIMEOUT` (`src/tui/mod.rs`) derives its budget from
/// this times `PROVISION_MAX_SUBPROCESS_CALLS` (`src/dispatch/worktree.rs`),
/// not a 1:1 mirror — see that constant's doc comment (#4201).
pub(crate) const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Extract stderr from a process `Output` as a trimmed `String`.
pub(crate) fn stderr_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

/// Extract stdout from a process `Output` as a trimmed `String`.
pub(crate) fn stdout_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The name this crate's own binary answers to on `PATH`.
///
/// One literal for every question of the form "which dispatch binary?":
/// [`AgentBinaries::default`] launches it, and three artefacts record it as a
/// command someone else will run — the MCP entry's `headersHelper`, the
/// statusLine command, and the tmux key binding that toggles the agent-tree
/// pane. Two copies could disagree after a rename, and the launcher would keep
/// working while a recorded artefact became a command that cannot be invoked.
///
/// It is a `macro_rules!` expanding to the literal, with the `const` beside it,
/// because `concat!` accepts a literal or a macro expansion but never a `const`
/// item — the same reason `crate::claude_paths` is shaped this way. Every
/// artefact that embeds the name in a longer fixed string composes it at
/// compile time from this macro, so the agreement is the compiler's rather than
/// a test's.
macro_rules! dispatch_program {
    () => {
        "dispatch"
    };
}

pub(crate) const DISPATCH_PROGRAM: &str = dispatch_program!();

pub(crate) use dispatch_program;

/// The `claude` and `dispatch` binaries the agent launchers in
/// `src/dispatch/agents.rs` invoke.
///
/// Production uses the bare names, resolved on `PATH` at launch time; a test can
/// name a stub instead. See [`ProcessRunner::agent_binaries`] for why this rides
/// with the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBinaries {
    /// The Claude Code CLI. Interpolated into shell command strings, so read it
    /// through [`Self::claude_quoted`] rather than directly.
    pub claude: String,
    /// This crate's own binary, launched as `dispatch agent-tree <id>` in the
    /// companion pane. Passed as argv, so it needs no quoting.
    pub dispatch: String,
}

impl Default for AgentBinaries {
    fn default() -> Self {
        Self {
            claude: "claude".to_string(),
            dispatch: DISPATCH_PROGRAM.to_string(),
        }
    }
}

impl AgentBinaries {
    /// [`Self::claude`] as one shell word, ready to interpolate into a command
    /// string.
    ///
    /// Every launcher site is arranged so that exactly one quoting layer applies
    /// — including `dispatch_with_prompt`, which passes the binary as bash's
    /// `$0` *outside* its single-quoted script body rather than inside it. Nested
    /// quoting would need the value escaped twice, and a site that got only one
    /// of the two layers right would look escaped while splitting at the first
    /// space; keeping the layer count at one everywhere removes that class of
    /// mistake instead of encapsulating it.
    pub fn claude_quoted(&self) -> String {
        shell_quote(&self.claude)
    }

    /// The stub identities the test suites substitute. One definition so the
    /// sentinel paths cannot drift between the launcher tests that assert on
    /// them.
    #[cfg(test)]
    pub fn stub() -> Self {
        Self {
            claude: "/stub/bin/claude-stub".to_string(),
            dispatch: "/stub/bin/dispatch-stub".to_string(),
        }
    }
}

/// Quote `s` for use as one word in a shell command, leaving it untouched when it
/// needs no quoting.
///
/// The pass-through case is load-bearing, not an optimisation: the default binary
/// names are plain, so the only change to production's emitted command string is
/// the `$0` indirection itself — no quoting noise on top of it.
pub(crate) fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    // POSIX single-quoting: everything inside is literal, and an embedded quote
    // is written by closing, escaping, and reopening.
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ---------------------------------------------------------------------------
// The bounded-child primitive
// ---------------------------------------------------------------------------

/// Poll step for the bounded wait for *exit*, after the child has closed its
/// output. Short because that wait is normally already over on the first
/// `try_wait` — a child that closed stdout has usually exited — so this only
/// paces the pathological case it exists for.
const EXIT_POLL_STEP: Duration = Duration::from_millis(5);

/// Drain `pipe` to EOF on its own thread. The receiver yields the bytes once,
/// which doubles as the signal that the child stopped writing to that stream.
///
/// A channel rather than a `JoinHandle`: the caller has to be able to give up on
/// this at a deadline, and `join` cannot be bounded.
fn drain(mut pipe: impl std::io::Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        pipe.read_to_end(&mut buf).ok();
        let _ = tx.send(buf);
    });
    rx
}

/// Give up on `child`: kill it, reap it so it cannot linger as an orphan or
/// zombie, and describe the overrun. Output it had already produced is dropped
/// with it — see [`run_bounded`].
fn abandon(child: &mut std::process::Child, program: &str, timeout: Duration) -> anyhow::Error {
    let _ = child.kill();
    let _ = child.wait();
    anyhow::anyhow!("{program} timed out after {timeout:?}")
}

/// Run `program` with `args` and kill it if it has not finished within `timeout`,
/// returning its captured output.
///
/// **The one place a SYNCHRONOUS bounded child is spawned.** Both
/// [`RealProcessRunner::run_with_timeout`] (git, worktree, tmux work) and the
/// statusline decorator's chained command (`src/cli/statusline.rs`) go through
/// here; a second hand-rolled kill-on-timeout over `std::process::Command` is a
/// bug, not a variation. The one sanctioned exception is
/// `crate::feed::exec::exec_feed_command`, which bounds an ASYNC
/// `tokio::process::Command` via `tokio::time::timeout` + `kill_on_drop(true)`
/// instead — this function's threaded drain-and-poll design has no async
/// equivalent to delegate to, so that call site is a second kill-on-timeout
/// mechanism by necessity, not by drift. See feeds.allium `SerialisedFeedCycle`
/// (its "BOUNDED COST" note) for why that command in particular needed one.
///
/// `timeout` is **one** deadline spanning both of the waits below. A child that
/// produces no output and one that produces output but never exits are equally
/// abandoned at it, killed and reaped, and reported as an error — nothing is
/// returned from a child that overran, not even output it had already produced.
///
/// Three OS hazards it exists to handle:
///
/// 1. **A never-exiting child.** A subprocess can block on a lock, NFS, or a
///    network remote. The wait for output is bounded by the deadline, and so is
///    the wait for exit that follows it — a child that closes stdout and keeps
///    running (`exec 1>&-; …`, or a wrapper whose last stage exits while a
///    sibling holds the pipe) would otherwise slip past the first bound into an
///    unbounded wait.
/// 2. **Output-pipe deadlock.** Stdout and stderr are drained on background
///    threads, so a child writing more than the pipe buffer holds never blocks
///    on us. Waiting on stdout's EOF rather than polling for exit also means a
///    child that finishes early is noticed immediately — this runs on Claude
///    Code's statusline debounce, where a poll interval would be visible latency.
/// 3. **Bidirectional-pipe deadlock.** With `stdin` present, writing all of it
///    before draining stdout deadlocks against a child that echoes as it reads
///    (`cat`, `tee`, `jq .`) once the payload exceeds the pipe buffer (~64 KiB on
///    Linux): the child blocks on its full, undrained stdout while we block
///    writing to its full, undrained stdin. The payload is written from its own
///    thread, which also owns the pipe — so it closes, and the child sees EOF,
///    when the write finishes. A child that never reads stdin merely gives that
///    thread an ignored `EPIPE`.
///
/// With `stdin` absent the child's stdin is **inherited**, unchanged from what
/// every git caller here has always had.
pub(crate) fn run_bounded(
    program: &str,
    args: &[&str],
    stdin: Option<&str>,
    timeout: Duration,
) -> Result<Output> {
    use std::io::Write;

    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if stdin.is_some() {
        command.stdin(std::process::Stdio::piped());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {program}"))?;

    if let Some(payload) = stdin {
        #[allow(clippy::expect_used)] // invariant: we set Stdio::piped() above
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let payload = payload.to_string();
        // Hazard 3: this must not run on the waiting thread. Both the payload and
        // the pipe are moved in, so the pipe drops — closing the child's stdin —
        // when the write completes or fails.
        std::thread::spawn(move || {
            let _ = pipe.write_all(payload.as_bytes());
        });
    }

    #[allow(clippy::expect_used)] // invariant: we set Stdio::piped() above
    let stdout_rx = drain(child.stdout.take().expect("stdout is piped"));
    #[allow(clippy::expect_used)] // invariant: we set Stdio::piped() above
    let stderr_rx = drain(child.stderr.take().expect("stderr is piped"));

    let deadline = std::time::Instant::now() + timeout;
    let remaining = || deadline.saturating_duration_since(std::time::Instant::now());

    // Wait for the child to stop producing output. One wakeup, at EOF.
    let Ok(stdout) = stdout_rx.recv_timeout(remaining()) else {
        return Err(abandon(&mut child, program, timeout));
    };

    // Then for it to exit, on the same deadline (hazard 1).
    let status = loop {
        // Anything but "still running" — exited, or unwaitable — means done.
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(_) => return Err(abandon(&mut child, program, timeout)),
            Ok(None) => {}
        }
        let left = remaining();
        if left.is_zero() {
            return Err(abandon(&mut child, program, timeout));
        }
        std::thread::sleep(EXIT_POLL_STEP.min(left));
    };

    // The child has exited, so stderr is at EOF too in every case but a
    // grandchild holding the pipe — which the deadline covers rather than
    // waiting out.
    let stderr = stderr_rx.recv_timeout(remaining()).unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub trait ProcessRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;

    /// Run the process but kill it (and return an error) if it hasn't exited
    /// within `timeout`. Stdout and stderr are drained on a pair of background
    /// threads so the pipe buffer never fills up while we poll.
    ///
    /// The default implementation ignores `timeout` and delegates to `run()`.
    /// Override this on real runners that can actually spawn and kill children.
    fn run_with_timeout(&self, program: &str, args: &[&str], _timeout: Duration) -> Result<Output> {
        self.run(program, args)
    }

    /// Which `claude` / `dispatch` binaries the agent launchers should invoke.
    ///
    /// The default is the bare names, resolved on `PATH` at launch — what every
    /// production runner wants. Override it to name stubs instead; that is the
    /// seam `tests/tmux_harness/mod.rs` uses so its real-tmux tests cannot spawn
    /// a live agent or open the developer's database.
    ///
    /// Only `src/dispatch/agents.rs` reads this, which cuts against the
    /// trait-narrowing convention in docs/conventions.md — deliberately. That
    /// convention is scoped to the DB traits, where narrowing is what makes the
    /// mutation boundary compiler-enforced. Here the alternative is threading the
    /// value through six launcher signatures and ~50 call sites that all pass the
    /// default, to substitute it in exactly one. Don't "fix" this by narrowing.
    fn agent_binaries(&self) -> AgentBinaries {
        AgentBinaries::default()
    }

    /// Claude Code's user-global config file (`~/.claude.json`), read by
    /// `dispatch::caller_identity` to derive the per-task MCP config an agent
    /// is launched with.
    ///
    /// `None` means "no config to read from", and it is the default
    /// deliberately: the opposite default would have every fixture in the suite
    /// reaching for the developer's real file unless it remembered to opt out.
    /// Only [`RealProcessRunner`] names it.
    ///
    /// This rides with the runner for the same reason [`Self::agent_binaries`]
    /// does, and with the same caveat — see that method's note on why the
    /// trait-narrowing convention does not apply. The launch sites that read it
    /// — both of them through `agents::agent_launch_flags` — already hold a
    /// runner and nothing else that could carry a per-environment path.
    ///
    /// One asymmetry with `agent_binaries` worth naming: ITS default is the
    /// production-correct value, so a runner built without thinking still
    /// behaves. This one's default is the degraded value, so production has to
    /// opt in and forgetting is invisible. That is the price of the safe
    /// direction — the opposite default would let a test reach the operator's
    /// real file by omission, which is worse and leaves no trace either.
    fn claude_json_path(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Whether `sccache` resolves on this machine's `PATH`, consulted before
    /// every agent launch (`src/dispatch/agents.rs`, `src/dispatch/worktree.rs`)
    /// to decide whether that launch's tmux window sets
    /// `RUSTC_WRAPPER=sccache` — see `DispatchedAgentsShareASccacheNotATargetDir`
    /// in docs/specs/dispatch.allium.
    ///
    /// The default does the real scan, same asymmetry as [`Self::agent_binaries`]
    /// and for the same reason: production behaves correctly without opting in.
    /// [`MockProcessRunner`] overrides it to a fixed `false` instead — sccache
    /// is genuinely absent from most test environments, and a scan that
    /// answered "yes" only on the machines that happen to have it installed
    /// would make launch tests flaky on exactly the axis they must not depend
    /// on.
    fn sccache_available(&self) -> bool {
        sccache_on_path()
    }
}

/// Whether an executable named `sccache` exists in some `PATH` entry — the
/// same resolution a shell does before running a bare command name, checked
/// ahead of time because a missing optional tool should silently drop
/// `RUSTC_WRAPPER` rather than hand every dispatched `cargo build` a wrapper
/// that does not exist.
///
/// Memoized: whether `sccache` is installed cannot change over the life of
/// this process, but this is consulted on every dispatch and every resume, so
/// an unmemoized version would re-`stat` every `PATH` entry on each one.
fn sccache_on_path() -> bool {
    static CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        sccache_in_dirs(std::env::split_paths(&path))
    })
}

/// [`sccache_on_path`]'s search, over an arbitrary directory list rather than
/// the process's own `PATH` — the seam that lets a test check the scan logic
/// without mutating process-global environment state.
fn sccache_in_dirs(dirs: impl Iterator<Item = std::path::PathBuf>) -> bool {
    dirs.map(|dir| dir.join("sccache"))
        .any(|c| is_executable_file(&c))
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

// ---------------------------------------------------------------------------
// Real implementation — wraps std::process::Command
// ---------------------------------------------------------------------------

/// The production runner.
///
/// `claude_json` is HANDED IN, never looked up here. Startup resolves the
/// operator's `$HOME`-derived locations once (`runtime::StartupPaths`) and
/// passes them down; a fresh lookup buried in a launcher is exactly what
/// `SettingsLocationIsAnExplicitStartupInput` (docs/specs/dispatch.allium)
/// rules out, and it is what would let a run that is not the operator's own
/// session reach the operator's configuration. [`Default`] leaves it unset, so
/// a runner built for work that never launches an agent names no file at all.
#[derive(Default)]
pub struct RealProcessRunner {
    claude_json: Option<std::path::PathBuf>,
}

impl RealProcessRunner {
    /// A runner for the launch paths, told where Claude Code's user-global
    /// config lives. The one caller is TUI startup, which has the resolved
    /// [`StartupPaths`](crate::runtime::StartupPaths).
    pub fn with_claude_json(claude_json: impl Into<std::path::PathBuf>) -> Self {
        Self {
            claude_json: Some(claude_json.into()),
        }
    }
}

impl ProcessRunner for RealProcessRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        std::process::Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {program}"))
    }

    fn claude_json_path(&self) -> Option<std::path::PathBuf> {
        self.claude_json.clone()
    }

    fn run_with_timeout(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output> {
        // No stdin payload: git and tmux subprocesses keep the inherited stdin
        // they have always had. See [`run_bounded`] for the hazards handled.
        run_bounded(program, args, None, timeout)
    }
}

// ---------------------------------------------------------------------------
// Mock implementation — for tests only
// ---------------------------------------------------------------------------

/// How a [`MockProcessRunner`] answers `tmux::window_target`'s exact-name
/// window lookup.
///
/// # Why this needs a policy at all
///
/// Every `tmux::` helper that takes a window *name* resolves it to a pane ID
/// first, because tmux resolves a bare `-t <name>` by **prefix** and would
/// otherwise act on a different task's window (see `tmux::window_target`). That
/// makes the lookup a precondition of nearly every tmux call in the codebase —
/// so how the mock answers it is a decision worth naming rather than burying.
#[cfg(any(test, feature = "test-support"))]
enum WindowLookup {
    /// Resolve whatever name is asked for, assigning pane IDs `%0`, `%1`, … in
    /// first-seen order. Answered out of band: not taken from the positional
    /// response queue, and not recorded in [`MockProcessRunner::recorded_calls`].
    ///
    /// The default, because for the overwhelming majority of tests the subject
    /// is the *operation* — that dispatch sends the right claude command, that
    /// cleanup kills a window — and resolution is infrastructure. Interleaving a
    /// listing response into every queue and re-numbering every `calls[N]`
    /// assertion around it would obscure those tests without testing anything
    /// new. The trade is deliberate: a mock cannot meaningfully verify target
    /// resolution anyway (it records argv, not tmux's interpretation of it), so
    /// the real coverage lives in tests/tmux_window_targets.rs against a real
    /// server, plus the `window_target` unit tests in src/tmux.rs.
    AnyName(Mutex<Vec<String>>),
    /// Resolve only these names, in this order; anything else fails as absent.
    /// Also answered out of band. Use when a test needs a *specific* topology —
    /// notably a prefix collision (`task-4` alongside `task-42`).
    OnlyNames(Vec<String>),
    /// Do not intercept: answer the lookup from the positional queue and record
    /// it like any other call. Use when the lookup itself is the subject, or
    /// when no call at all is expected (see [`MockProcessRunner::unused`]).
    Queued,
}

#[cfg(any(test, feature = "test-support"))]
pub struct MockProcessRunner {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    /// The timeout each recorded call was made with, positionally aligned with
    /// `calls`. Kept apart from `calls` so the `(program, args)` tuples every
    /// existing assertion destructures stay the shape they are.
    timeouts: Mutex<Vec<Option<Duration>>>,
    responses: Mutex<VecDeque<(Option<Duration>, Result<Output>)>>,
    window_lookup: WindowLookup,
    binaries: AgentBinaries,
    /// The `~/.claude.json` stand-in this runner reports, or `None` (the
    /// default) for a runner that names no file. See
    /// [`ProcessRunner::claude_json_path`] for why the default is inert.
    claude_json: Option<std::path::PathBuf>,
    /// What [`ProcessRunner::sccache_available`] answers — `false` by
    /// default. See that method's doc comment for why a fixed answer replaces
    /// the real scan here.
    sccache_available: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl MockProcessRunner {
    /// Construct a runner. tmux window-name lookups resolve permissively — see
    /// [`WindowLookup::AnyName`] for why that is the default, and
    /// [`Self::with_windows`] / [`Self::with_queued_window_lookup`] to change it.
    pub fn new(responses: Vec<Result<Output>>) -> Self {
        Self::new_with_delays(responses.into_iter().map(|r| (None, r)).collect())
    }

    /// Construct a runner whose responses are delivered after a per-response
    /// delay. Use for testing watchdog/timeout logic.
    pub fn new_with_delays(responses: Vec<(Option<Duration>, Result<Output>)>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
            window_lookup: WindowLookup::AnyName(Mutex::new(Vec::new())),
            binaries: AgentBinaries::default(),
            claude_json: None,
            sccache_available: false,
        }
    }

    /// Point this runner's caller-identity config derivation at `path`, so a
    /// test exercises the real read-and-write without going anywhere near the
    /// developer's own `~/.claude.json`.
    pub fn with_claude_json(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.claude_json = Some(path.into());
        self
    }

    /// Restrict this fake tmux server to exactly these window names, so a name
    /// that is not listed fails to resolve. Each gets one active pane, `%0`,
    /// `%1`, … in the order given; [`Self::pane_id_of`] returns the ID a
    /// resolution will yield.
    ///
    /// Use where the topology matters — above all a prefix collision, e.g.
    /// `with_windows(&["task-42"])` and then an operation on `task-4`, which
    /// must fail rather than hit `task-42`. See [`WindowLookup::OnlyNames`].
    pub fn with_windows(mut self, names: &[&str]) -> Self {
        self.window_lookup =
            WindowLookup::OnlyNames(names.iter().map(|n| (*n).to_string()).collect());
        self
    }

    /// Answer window lookups from the positional response queue and record them,
    /// instead of resolving them out of band. See [`WindowLookup::Queued`].
    pub fn with_queued_window_lookup(mut self) -> Self {
        self.window_lookup = WindowLookup::Queued;
        self
    }

    /// The pane ID this fake server resolves `name` to, i.e. the `-t` target the
    /// helper under test will pass to tmux.
    ///
    /// # Panics
    ///
    /// Under [`Self::with_windows`], if `name` was not declared — asserting
    /// against a window this server does not have is a test bug. Under
    /// [`Self::with_queued_window_lookup`], always: there is no fake server to
    /// ask.
    #[allow(clippy::expect_used, clippy::unwrap_used)] // test helper
    pub fn pane_id_of(&self, name: &str) -> String {
        let index = match &self.window_lookup {
            WindowLookup::AnyName(seen) => Self::index_of_or_insert(seen, name),
            WindowLookup::OnlyNames(names) => names
                .iter()
                .position(|n| n == name)
                .expect("window was not declared via with_windows"),
            WindowLookup::Queued => {
                panic!("pane_id_of has no meaning with a queued window lookup")
            }
        };
        format!("%{index}")
    }

    /// Index of `name` in `seen`, appending it first if absent. Stable within a
    /// test, so repeated resolutions of one name agree and two names differ.
    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex
    fn index_of_or_insert(seen: &Mutex<Vec<String>>, name: &str) -> usize {
        let mut seen = seen.lock().unwrap();
        if let Some(i) = seen.iter().position(|n| n == name) {
            return i;
        }
        seen.push(name.to_string());
        seen.len() - 1
    }

    /// Answer `tmux::window_target`'s lookup out of band, unless this call is not
    /// that lookup or the policy is [`WindowLookup::Queued`].
    ///
    /// The reply is what a real tmux would print for that lookup's `-f` filter:
    /// one row per *matching* pane. Under [`WindowLookup::OnlyNames`] the match
    /// is computed here over the declared windows, so a declared prefix collision
    /// (`task-4` asked for, only `task-42` declared) yields no rows and the
    /// production resolver is the thing that turns that into an error.
    fn answer_window_lookup(&self, program: &str, args: &[&str]) -> Option<Output> {
        if program != "tmux" {
            return None;
        }
        let wanted = crate::tmux::window_name_in_lookup(args)?;
        match &self.window_lookup {
            WindowLookup::Queued => None,
            WindowLookup::AnyName(seen) => {
                let index = Self::index_of_or_insert(seen, wanted);
                Some(Self::listing(&[(index, wanted.to_string())]))
            }
            WindowLookup::OnlyNames(names) => {
                let rows: Vec<(usize, String)> = names
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| name.as_str() == wanted)
                    .map(|(i, name)| (i, name.clone()))
                    .collect();
                Some(Self::listing(&rows))
            }
        }
    }

    /// A `list-panes` listing in `tmux::WINDOW_PANE_FORMAT`: one active pane per
    /// row, each carrying the pane ID its index implies.
    fn listing(rows: &[(usize, String)]) -> Output {
        let stdout: String = rows
            .iter()
            .map(|(i, name)| format!("1 %{i} {name}\n"))
            .collect();
        Output {
            status: exit_ok(),
            stdout: stdout.into_bytes(),
            stderr: vec![],
        }
    }

    /// Name distinctive `claude` / `dispatch` binaries, so a test can assert
    /// which binary an agent launcher actually invoked rather than only that it
    /// invoked *something* called `claude`.
    pub fn with_agent_binaries(mut self, binaries: AgentBinaries) -> Self {
        self.binaries = binaries;
        self
    }

    /// Make this runner report `sccache` as available, for tests exercising
    /// the launch path that sets `RUSTC_WRAPPER`.
    pub fn with_sccache_available(mut self, available: bool) -> Self {
        self.sccache_available = available;
        self
    }

    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    pub fn recorded_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    /// [`Self::recorded_calls`] rendered one `"program arg arg"` string per call,
    /// for tests that assert on a substring of the command line (e.g.
    /// `"worktree remove"`) rather than on an exact argv vector.
    pub fn flattened_calls(&self) -> Vec<String> {
        self.recorded_calls()
            .iter()
            .map(|(program, args)| format!("{program} {}", args.join(" ")))
            .collect()
    }

    /// The timeout each recorded call was made with — `None` for a plain
    /// [`ProcessRunner::run`], `Some(d)` for a
    /// [`ProcessRunner::run_with_timeout`]. Positionally aligned with
    /// [`Self::recorded_calls`].
    ///
    /// Whether a subprocess is bounded is invisible in its argv, so without this
    /// the only way to test it is to let an unbounded call hang — which fails a
    /// regression by timing the suite out rather than by asserting.
    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    pub fn recorded_timeouts(&self) -> Vec<Option<Duration>> {
        self.timeouts.lock().unwrap().clone()
    }

    /// Record a call and pop the next queued (delay, response) pair.
    /// Panics if no response is queued — same contract as `run` / `run_with_timeout`.
    #[allow(clippy::unwrap_used)] // test helper — panics on poisoned mutex (programming error)
    fn record_and_pop(
        &self,
        program: &str,
        args: &[&str],
        timeout: Option<Duration>,
    ) -> (Option<Duration>, Result<Output>) {
        // Deliberately before the recording: an out-of-band window lookup is
        // neither queued nor recorded, so `calls[N]` indices stay stable.
        if let Some(listing) = self.answer_window_lookup(program, args) {
            return (None, Ok(listing));
        }
        self.calls.lock().unwrap().push((
            program.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        self.timeouts.lock().unwrap().push(timeout);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                panic!("MockProcessRunner: no response queued for {program} {args:?}")
            })
    }

    /// A runner with no queued responses, ready to inject as a dependency.
    ///
    /// Use this wherever a test must supply a `ProcessRunner` but expects no
    /// commands to be run — the mock panics on the first call, so an
    /// accidental shell-out fails the test loudly instead of hitting the host.
    /// Queued window lookups on purpose: a window lookup is a shell-out too, and
    /// this runner's whole job is to panic on any of them.
    pub fn unused() -> Arc<dyn ProcessRunner> {
        Arc::new(Self::new(vec![]).with_queued_window_lookup())
    }

    /// Successful Output with empty stdout/stderr.
    pub fn ok() -> Result<Output> {
        Ok(Output {
            status: exit_ok(),
            stdout: vec![],
            stderr: vec![],
        })
    }

    /// Successful Output with specific stdout bytes.
    pub fn ok_with_stdout(stdout: &[u8]) -> Result<Output> {
        Ok(Output {
            status: exit_ok(),
            stdout: stdout.to_vec(),
            stderr: vec![],
        })
    }

    /// Failed Output (non-zero exit) with specific stderr.
    pub fn fail(stderr: &str) -> Result<Output> {
        Ok(Output {
            status: exit_fail(),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    /// Failed Output with a specific exit code. `fail` hardcodes 1, which cannot
    /// express the codes `git ls-remote --exit-code` uses to distinguish "no
    /// matching ref" (2) from "could not reach the remote" (128).
    pub fn fail_with_code(code: i32, stderr: &str) -> Result<Output> {
        Ok(Output {
            status: exit_code(code),
            stdout: vec![],
            stderr: stderr.as_bytes().to_vec(),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ProcessRunner for MockProcessRunner {
    fn claude_json_path(&self) -> Option<std::path::PathBuf> {
        self.claude_json.clone()
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        let (delay, response) = self.record_and_pop(program, args, None);
        if let Some(d) = delay {
            std::thread::sleep(d);
        }
        response
    }

    fn run_with_timeout(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output> {
        let (delay, response) = self.record_and_pop(program, args, Some(timeout));
        if let Some(d) = delay {
            if d >= timeout {
                anyhow::bail!("{program} timed out after {timeout:?}");
            }
            std::thread::sleep(d);
        }
        response
    }

    fn agent_binaries(&self) -> AgentBinaries {
        self.binaries.clone()
    }

    fn sccache_available(&self) -> bool {
        self.sccache_available
    }
}

// ---------------------------------------------------------------------------
// Helpers for constructing ExitStatus in tests (Unix only)
// ---------------------------------------------------------------------------

#[cfg(all(unix, any(test, feature = "test-support")))]
pub fn exit_ok() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(0)
}

#[cfg(all(unix, any(test, feature = "test-support")))]
pub fn exit_fail() -> std::process::ExitStatus {
    exit_code(1)
}

/// An `ExitStatus` carrying a specific exit code, for callers that classify on
/// the code rather than the message (e.g. `git ls-remote --exit-code`).
#[cfg(all(unix, any(test, feature = "test-support")))]
pub fn exit_code(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    // Raw status word: the exit code lives in the high byte.
    std::process::ExitStatus::from_raw(code << 8)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
