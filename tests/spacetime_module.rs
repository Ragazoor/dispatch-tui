//! The SpacetimeDB module publishes, and keeps publishing, without losing data.
//!
//! `spacetime/module/` is outside the workspace and compiles to wasm, so
//! nothing in `cargo test` normally touches it. These tests do: they stand up a
//! throwaway standalone instance and publish the real module into it.
//!
//! **What this covers that a unit test cannot.** Whether a schema change is
//! *automigratable* is a judgement only the store makes, and it is not obvious
//! from the source. Appending a column looks like the safe case and is still
//! refused without a `#[default(..)]` annotation; that refusal was found by
//! running this, not by reading the code. Every publish below passes
//! `--delete-data=never`, which is what turns "it migrated" into an assertion
//! rather than a hope: with it, a change the store cannot automigrate aborts
//! instead of quietly destroying the database and reporting success.
//!
//! **Skipped when `spacetime` is not on `PATH`.** Unlike tmux, nothing in CI
//! installs it, so there is no CI arm that hard-fails — see
//! `tests/tmux_harness/mod.rs` for the pattern this deliberately departs from.
//! The gate script `scripts/check-spacetime-module.sh` runs the parts that need
//! no server, and runs everywhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use dispatch_tui::process::{ProcessRunner, RealProcessRunner};
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for a freshly spawned standalone instance to accept a
/// connection. Generous: it is a failure timeout, not a delay — the poll below
/// returns as soon as the port answers.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// A wasm release build of the module runs inside `spacetime publish`, and a
/// cold one compiles the whole `spacetimedb` crate tree.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(600);

const DATABASE: &str = "dispatch-module-test";

fn spacetime_available_or_skip() -> bool {
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
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("read the bound port").port()
}

/// A standalone instance of its own, on its own port, with its own data and CLI
/// config.
///
/// Isolated through `--data-dir` and `--config-path` rather than `--root-dir`:
/// that flag also relocates where the CLI looks for its own binary, so passing
/// a temp directory makes every later invocation fail with "exec failed".
struct Instance {
    child: Child,
    port: u16,
    dir: tempfile::TempDir,
}

impl Instance {
    fn start() -> Self {
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

        let instance = Instance { child, port, dir };
        instance.await_ready();
        instance
    }

    /// Block until the port answers, or panic on the deadline.
    ///
    /// A poll against a real condition, not a fixed wait: the common path costs
    /// one failed connect, and only a genuinely dead server pays the timeout.
    fn await_ready(&self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            match TcpStream::connect(("127.0.0.1", self.port)) {
                Ok(_) => return,
                Err(e) if e.kind() == ErrorKind::ConnectionRefused => {}
                Err(e) => panic!("connecting to the test instance: {e}"),
            }
            assert!(
                Instant::now() < deadline,
                "the test instance never accepted a connection on port {}",
                self.port
            );
            // Backs off between connect attempts; only the failure path waits.
            // allow-test-sleep: deadline-bounded poll, not a fixed wait.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn host(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn config_arg(&self) -> String {
        format!(
            "--config-path={}",
            self.dir.path().join("cli.toml").display()
        )
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
    fn publish(&self, module_path: &Path, target_dir: Option<&Path>) -> std::process::Output {
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
                DATABASE,
            ],
        )
    }

    fn call(&self, reducer: &str, args: &[&str]) -> std::process::Output {
        let mut argv = vec![
            self.config_arg(),
            "call".into(),
            "-s".into(),
            self.host(),
            "-y".into(),
            DATABASE.into(),
            reducer.into(),
        ];
        argv.extend(args.iter().map(|a| (*a).to_string()));
        run(None, &argv.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn sql(&self, query: &str) -> String {
        let out = run(
            None,
            &[
                &self.config_arg(),
                "sql",
                "-s",
                &self.host(),
                "--format",
                "json",
                DATABASE,
                query,
            ],
        );
        assert!(out.status.success(), "{}", describe(&out));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `spacetime` to completion, or kill it and fail by name.
///
/// [`ProcessRunner::run_with_timeout`] rather than a hand-rolled wait, for two
/// reasons that both bite here. It drains stdout and stderr on background
/// threads, which matters because a cold `spacetime publish` runs a full cargo
/// build and would otherwise fill the ~64 KiB pipe buffer and block until the
/// deadline. And it kills the child on the deadline, so a wedged publish is
/// reclaimed rather than merely reported. See `src/process.rs::run_bounded`.
///
/// `CARGO_TARGET_DIR` is set on this process rather than on the child because
/// `run_with_timeout` takes no environment. It is set and cleared around the
/// one call rather than left in place.
fn run(target_dir: Option<&Path>, args: &[&str]) -> std::process::Output {
    if let Some(dir) = target_dir {
        std::env::set_var("CARGO_TARGET_DIR", dir);
    }
    let out = RealProcessRunner::default().run_with_timeout("spacetime", args, PUBLISH_TIMEOUT);
    if target_dir.is_some() {
        std::env::remove_var("CARGO_TARGET_DIR");
    }
    out.unwrap_or_else(|e| panic!("running `spacetime {}`: {e}", args.join(" ")))
}

fn describe(out: &std::process::Output) -> String {
    format!(
        "status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn module_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "spacetime", "module"]
        .iter()
        .collect()
}

/// A build directory for the scratch module that OUTLIVES the test.
///
/// The scratch copy lives in a tempdir, so without this every run compiles the
/// whole dependency tree for wasm in release and then deletes the result.
/// Pointing it at the module's own (gitignored) target tree means a second run
/// recompiles only the module crate, because the committed and working-tree
/// manifests have identical dependencies in the normal case.
fn scratch_target_dir() -> PathBuf {
    module_path().join("target").join("committed")
}

/// The module as the last commit has it, unpacked into a scratch directory.
///
/// `git show` rather than a checked-in copy so the fixture cannot go stale: it
/// is always the previous shape, whatever that currently is. Once this branch
/// is committed the two sides are identical and the test degrades into the
/// republish case — which is the right behaviour, because from then on it
/// guards the *next* change.
fn committed_module(into: &Path) -> PathBuf {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(into.join("src")).expect("scratch module dir");
    for (tracked, dest) in [
        ("spacetime/module/Cargo.toml", "Cargo.toml"),
        ("spacetime/module/src/lib.rs", "src/lib.rs"),
    ] {
        // `git -C <dir>` rather than a cwd on the child: it is the form the rest
        // of the repo uses (`src/git.rs`, `src/repo_sync.rs`), and
        // `run_with_timeout` has no argument for a working directory.
        let out = RealProcessRunner::default()
            .run_with_timeout(
                "git",
                &[
                    "-C",
                    &repo_root.display().to_string(),
                    "show",
                    &format!("HEAD:{tracked}"),
                ],
                PUBLISH_TIMEOUT,
            )
            .expect("git show");
        assert!(out.status.success(), "{}", describe(&out));
        std::fs::write(into.join(dest), &out.stdout).expect("write scratch module file");
    }
    into.to_path_buf()
}

/// The `schema_version` row, as `[id, version, module_version]`.
fn schema_version_row(instance: &Instance) -> Vec<i64> {
    let json = instance.sql("SELECT * FROM schema_version");
    let line = json
        .lines()
        .find(|l| l.trim_start().starts_with('['))
        .unwrap_or_else(|| panic!("no JSON in sql output:\n{json}"));
    let parsed: serde_json::Value = serde_json::from_str(line).expect("parse sql output");
    parsed[0]["rows"][0]
        .as_array()
        .unwrap_or_else(|| panic!("no schema_version row in {parsed}"))
        .iter()
        .map(|v| v.as_i64().expect("integer column"))
        .collect()
}

/// Publishing the same module over itself is accepted and changes nothing.
#[test]
fn publishing_the_module_twice_automigrates() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start();
    let module = module_path();

    let first = instance.publish(&module, None);
    assert!(
        first.status.success(),
        "first publish: {}",
        describe(&first)
    );

    let second = instance.publish(&module, None);
    assert!(
        second.status.success(),
        "republishing an unchanged module must automigrate: {}",
        describe(&second)
    );
}

/// The shape in the last commit migrates into the shape in the working tree,
/// without destroying data.
///
/// This is the one that catches a column added anywhere but the end. It is also
/// what caught `#[default(..)]` being required on an appended column, which
/// nothing in the source hints at.
#[test]
fn the_committed_module_automigrates_into_the_working_tree_module() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start();
    let scratch = tempfile::tempdir().expect("temp dir");
    let previous = committed_module(scratch.path());

    let first = instance.publish(&previous, Some(&scratch_target_dir()));
    assert!(
        first.status.success(),
        "publishing the committed module: {}",
        describe(&first)
    );
    let before = schema_version_row(&instance);
    assert_eq!(before[0], 1, "init stamps exactly one schema_version row");

    let migrated = instance.publish(&module_path(), None);
    assert!(
        migrated.status.success(),
        "the working tree's module must automigrate over the committed one, \
         without --delete-data. A column added anywhere but the end fails here: {}",
        describe(&migrated)
    );

    // The row survived rather than being recreated. A publish does not re-run
    // `init`, so the SQLite version coming back unchanged is what says the
    // database was migrated rather than dropped and rebuilt.
    let after = schema_version_row(&instance);
    assert_eq!(
        after[1], before[1],
        "the SQLite schema version must survive the migration untouched"
    );

    // Re-stamping is what moves `module_version` off its migration default, and
    // it is a separate step from publishing on purpose: a publish cannot run
    // `init`, so nothing else would ever update it.
    let restamp = instance.call("set_schema_version", &["97"]);
    assert!(restamp.status.success(), "{}", describe(&restamp));
    let stamped = schema_version_row(&instance);
    assert_eq!(
        stamped[2], 1,
        "re-stamping must record the running module's own version"
    );
}
