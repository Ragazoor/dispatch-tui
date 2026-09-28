//! A throwaway, standalone SpacetimeDB instance for integration tests.
//!
//! Shared by `tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs`
//! — both stand up a real instance, publish the module into it, and talk to it
//! over the `spacetime` CLI. This used to be two independent, hand-copied
//! implementations; the second copy is what motivated pulling this out, since
//! any future change to how the throwaway instance is started otherwise has to
//! be made twice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for a freshly spawned standalone instance to accept a
/// connection. Generous: it is a failure timeout, not a delay — the poll below
/// returns as soon as the port answers.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

pub fn spacetime_available_or_skip() -> bool {
    let present = Command::new("spacetime")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !present {
        eprintln!("skipping: spacetime not available on PATH");
    }
    present
}

/// A port nothing is listening on, released immediately so the server can take
/// it. Racy in principle; in practice the window is microseconds and the
/// alternative is a fixed port that collides with a developer's own instance.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("read the bound port").port()
}

/// A standalone instance of its own, on its own port, with its own data and CLI
/// config.
///
/// Isolated through `--data-dir` and `--config-path` rather than `--root-dir`:
/// that flag also relocates where the CLI looks for its own binary, so passing
/// a temp directory makes every later invocation fail with "exec failed".
pub struct Instance {
    child: Child,
    port: u16,
    dir: tempfile::TempDir,
    database: String,
}

impl Instance {
    /// `label` distinguishes this instance's databases from another test
    /// file's, purely for readability in `spacetime` CLI output — collisions
    /// are already ruled out by [`Self::database`]'s per-instance counter.
    pub fn start(label: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let port = free_port();
        let child = Command::new("spacetime")
            .arg(format!(
                "--config-path={}",
                dir.path().join("cli.toml").display()
            ))
            .arg("start")
            .arg(format!("--listen-addr=127.0.0.1:{port}"))
            .arg(format!("--data-dir={}", dir.path().join("data").display()))
            .arg("--non-interactive")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spacetime start");

        let database = format!(
            "dispatch-{label}-{}",
            NEXT_DATABASE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let mut instance = Instance {
            child,
            port,
            dir,
            database,
        };
        instance.await_ready();

        // Ours, not somebody else's. `await_ready` only proves that SOMETHING
        // answers on the port, and a port collision means the thing answering
        // belongs to another test. Left undetected that is not a failure, it is
        // two tests quietly sharing a server — which is how a publish of an
        // unchanged module came to be refused for removing a column.
        assert!(
            instance
                .child
                .try_wait()
                .expect("poll the instance process")
                .is_none(),
            "this test's spacetime instance exited during startup — most likely \
             another test won the race for port {}",
            instance.port
        );
        instance
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    /// Point this instance at a specific database name instead of the one
    /// [`Self::start`] generated — for the one test that needs the board's
    /// own fixed `SHARED_DATABASE_NAME` rather than a throwaway one.
    pub fn set_database(&mut self, name: impl Into<String>) {
        self.database = name.into();
    }

    /// The CLI config file path this instance's every invocation uses —
    /// for a caller that needs to point ANOTHER tool (not the `spacetime`
    /// CLI) at the same config, e.g. `SpacetimeCliStore::with_config_path`.
    pub fn config_path(&self) -> String {
        self.dir.path().join("cli.toml").display().to_string()
    }

    /// Block until the instance SERVES, or panic on the deadline.
    ///
    /// A poll against a real condition, not a fixed wait: the common path costs
    /// one failed request, and only a genuinely dead server pays the timeout.
    ///
    /// **A bare TCP connect is not enough**, and finding that out cost two
    /// flakes. The listener is up before the HTTP API is, so a connect that
    /// succeeds can be followed immediately by `publish` failing with
    /// `client error (Connect)` on `/v1/identity` — which reads as a bug in
    /// whichever test drew the short straw. So this asks for an actual
    /// response, and any HTTP status counts: what is being waited for is a
    /// server that answers, not a particular answer.
    fn await_ready(&self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if self.answers_http() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the test instance never served HTTP on port {}",
                self.port
            );
            // Backs off between attempts; only the failure path waits.
            // allow-test-sleep: deadline-bounded poll, not a fixed wait.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// One `GET /v1/identity`, written by hand.
    ///
    /// By hand rather than through an HTTP client because the whole question is
    /// "does anything answer", and pulling a client crate into the dev-tree for
    /// one request would be the larger change.
    fn answers_http(&self) -> bool {
        use std::io::{Read, Write};

        let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", self.port)) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let request = format!(
            "GET /v1/identity HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            self.port
        );
        if stream.write_all(request.as_bytes()).is_err() {
            return false;
        }
        let mut answer = Vec::new();
        // Any status line means the API is serving. A reset mid-startup gives
        // an error or an empty read, both of which are "not yet".
        matches!(stream.read_to_end(&mut answer), Ok(n) if n > 0) && answer.starts_with(b"HTTP/")
    }

    pub fn host(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn config_arg(&self) -> String {
        format!("--config-path={}", self.config_path())
    }

    /// Publish `module_path`, refusing any migration the store cannot perform
    /// without destroying data.
    ///
    /// `target_dir` pins cargo's output for the build `spacetime publish` runs
    /// inside itself; `None` leaves the module's default. Two flag placements
    /// are easy to get wrong and both make the CLI reject the invocation: `-s`
    /// and `-y` belong to the SUBCOMMAND rather than to `spacetime`, and
    /// `--delete-data` takes an optional value, so `--delete-data never` is
    /// parsed as the database name.
    pub fn publish(&self, module_path: &Path, target_dir: Option<&Path>) -> std::process::Output {
        run(
            target_dir,
            &[
                &self.config_arg(),
                "publish",
                "-p",
                &module_path.display().to_string(),
                "-s",
                &self.host(),
                "-y",
                // The load-bearing flag. Without it a schema change the store
                // cannot automigrate is "resolved" by dropping the database,
                // and the publish reports success either way.
                "--delete-data=never",
                self.database(),
            ],
        )
    }

    pub fn call(&self, reducer: &str, args: &[&str]) -> std::process::Output {
        let mut argv = vec![
            self.config_arg(),
            "call".into(),
            "-s".into(),
            self.host(),
            "-y".into(),
            self.database().to_string(),
            reducer.into(),
        ];
        argv.extend(args.iter().map(|a| (*a).to_string()));
        run(None, &argv.iter().map(String::as_str).collect::<Vec<_>>())
    }

    pub fn sql(&self, query: &str) -> String {
        let out = run(
            None,
            &[
                &self.config_arg(),
                "sql",
                "-s",
                &self.host(),
                "--format",
                "json",
                self.database(),
                query,
            ],
        );
        assert!(out.status.success(), "{}", describe(&out));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

/// Distinguishes the databases of two instances alive at once, across every
/// caller of [`Instance::start`] regardless of file.
///
/// **One shared name across parallel tests was a real, silent corruption.**
/// `free_port` binds an ephemeral port and closes it before `spacetime start`
/// takes it, so two tests racing can be handed the SAME port; the loser's
/// server dies, `await_ready` still succeeds because the winner's is answering,
/// and both tests then publish into one database. What that looked like was a
/// publish of the unchanged module being refused for "removing a column" — a
/// message about a migration neither test performed.
///
/// A per-instance name means that even where two tests end up sharing a server,
/// they do not share a database. [`Instance::start`] separately refuses to
/// return an instance whose own server has died, so the port collision itself
/// is loud rather than inferred from a confusing migration error.
static NEXT_DATABASE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `spacetime` to completion, or kill it and fail by name.
///
/// **`CARGO_TARGET_DIR` goes on the CHILD, never on this process.** It used to
/// be set with `std::env::set_var` around the call, which is process-global:
/// two tests running as threads of one binary at the same time would silently
/// make one test's target directory become the other's for however long the
/// window lasted. The visible symptom was a publish of an UNCHANGED module
/// refused for "removing a column" — a migration neither test asked for,
/// reported against a wasm one test built and the other uploaded.
pub fn run(target_dir: Option<&Path>, args: &[&str]) -> std::process::Output {
    let mut command = Command::new("spacetime");
    command.args(args);
    if let Some(dir) = target_dir {
        command.env("CARGO_TARGET_DIR", dir);
    }
    command
        .output()
        .unwrap_or_else(|e| panic!("running `spacetime {}`: {e}", args.join(" ")))
}

pub fn describe(out: &std::process::Output) -> String {
    format!(
        "status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

pub fn module_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "spacetime", "module"]
        .iter()
        .collect()
}

/// Read the single column of a single-row, single-column query, as a string.
///
/// Over SQL rather than over a subscription, deliberately: what is being
/// asserted is what the STORE holds, and a subscription is one client's view of
/// it filtered by what that client asked for.
///
/// Rows come back as positional ARRAYS rather than as objects, so the query has
/// to project exactly one column and this reads position zero. A query that
/// selects more silently asserts about whichever column came first.
pub fn column(instance: &Instance, query: &str) -> String {
    let raw = instance.sql(query);
    let line = raw
        .lines()
        .find(|l| l.trim_start().starts_with('['))
        .unwrap_or_else(|| panic!("no JSON in sql output:\n{raw}"));
    let parsed: serde_json::Value = serde_json::from_str(line).expect("parse sql output");
    let rows = parsed[0]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows in {parsed}"));
    assert_eq!(rows.len(), 1, "expected exactly one row in {parsed}");
    match &rows[0][0] {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Whether a query returned nothing.
pub fn no_rows(instance: &Instance, query: &str) -> bool {
    let raw = instance.sql(query);
    let line = raw
        .lines()
        .find(|l| l.trim_start().starts_with('['))
        .unwrap_or_else(|| panic!("no JSON in sql output:\n{raw}"));
    let parsed: serde_json::Value = serde_json::from_str(line).expect("parse sql output");
    parsed[0]["rows"]
        .as_array()
        .is_none_or(|rows| rows.is_empty())
}
