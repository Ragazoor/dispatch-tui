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
use dispatch_tui::sync::{SharedRows, SpacetimeSdkConnector, StoreConnector, SubscriptionRequest};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long to wait for a freshly spawned standalone instance to accept a
/// connection. Generous: it is a failure timeout, not a delay — the poll below
/// returns as soon as the port answers.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// A wasm release build of the module runs inside `spacetime publish`, and a
/// cold one compiles the whole `spacetimedb` crate tree.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(600);

/// Prefix for each instance's own database. The suffix is per-[`Instance`];
/// see [`Instance::database`].
const DATABASE_PREFIX: &str = "dispatch-module-test";

/// Distinguishes the databases of two instances alive at once.
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
    database: String,
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

        let database = format!(
            "{DATABASE_PREFIX}-{}",
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
    fn database(&self) -> &str {
        &self.database
    }

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

        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", self.port)) else {
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
                self.database(),
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
            self.database().to_string(),
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
                self.database(),
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
/// Run `spacetime`, optionally pinning the cargo target directory of the build
/// it performs internally.
///
/// **`CARGO_TARGET_DIR` goes on the CHILD, never on this process.** It used to
/// be set with `std::env::set_var` around the call, which is process-global:
/// the two tests in this file are threads of one binary and run at the same
/// time, so one test's target directory silently became the other's for however
/// long the window lasted. The visible symptom was a publish of an UNCHANGED
/// module refused for "removing a column" — a migration neither test asked for,
/// reported against a wasm one test built and the other uploaded.
///
/// This is also why the call does not go through `RealProcessRunner`: that
/// runner deliberately carries no per-invocation environment, and adding one to
/// it for a test's benefit would widen a production seam. `Command` here is the
/// smaller change.
fn run(target_dir: Option<&Path>, args: &[&str]) -> std::process::Output {
    let mut command = Command::new("spacetime");
    command.args(args);
    if let Some(dir) = target_dir {
        command.env("CARGO_TARGET_DIR", dir);
    }
    command
        .output()
        .unwrap_or_else(|e| panic!("running `spacetime {}`: {e}", args.join(" ")))
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
///
/// Created here rather than left to the builder: cargo writes its lock and
/// temp files directly into the directory it is handed and does not make one,
/// so an absent path fails the publish with a bare `No such file or directory`
/// naming a random temp sibling — which reads as anything but a missing target
/// dir.
fn scratch_target_dir() -> PathBuf {
    let dir = module_path().join("target").join("committed");
    std::fs::create_dir_all(&dir).expect("scratch target dir");
    dir
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

/// What a cold start costs: process start to a subscription the board could
/// draw from.
///
/// **This is a measurement, not a gate.** The number is what Phase 4 of the
/// migration plan was asked to produce, and it is recorded in the design doc;
/// the assertion here is a ceiling so loose that only a genuine regression —
/// a hang, a retry storm, a synchronous round trip that was not there before —
/// can trip it. A tight bound would fail on a loaded CI box and teach people to
/// ignore it.
///
/// It lives beside the module harness because that is where a real standalone
/// instance already exists, and a measurement against a fake would measure the
/// fake.
///
/// **What it deliberately does NOT measure**: time to the board drawing. The
/// board draws before the connection opens (`sync.allium:
/// OpenBoardConnection`), so that number is unchanged by any of this, and
/// conflating the two is how a design that costs nothing gets reported as
/// costing a round trip.
#[test]
fn a_cold_start_reaches_a_live_subscription_promptly() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start();
    let published = instance.publish(&module_path(), None);
    assert!(
        published.status.success(),
        "publishing for the cold-start measurement: {}",
        describe(&published)
    );

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (connect, subscribe, identity) = runtime.block_on(async {
        let connector =
            SpacetimeSdkConnector::new(instance.database(), Arc::new(SharedRows::new()));

        // allow-test-sleep: this test's entire purpose is to measure elapsed
        // time against a real server. It asserts a loose ceiling, not a
        // duration, and removing the measurement removes the test.
        let started = Instant::now();
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("cold-start connect: {e}"));
        // allow-test-sleep: see above.
        let connect = started.elapsed();

        // allow-test-sleep: see above.
        let before_subscribe = Instant::now();
        connector
            .subscribe(&SubscriptionRequest::new(accepted.identity.clone(), vec![]))
            .await
            .unwrap_or_else(|e| panic!("cold-start subscribe: {e}"));
        // allow-test-sleep: see above.
        (connect, before_subscribe.elapsed(), accepted.identity)
    });

    // Printed rather than only asserted: the number is the deliverable, and
    // `cargo test -- --nocapture` is how it is read again later.
    println!(
        "cold start: connect {connect:?}, subscribe {subscribe:?}, \
         total {:?} (identity {identity})",
        connect + subscribe
    );

    assert!(
        connect + subscribe < COLD_START_CEILING,
        "a cold start took {:?}, past the {COLD_START_CEILING:?} ceiling — \
         this is a regression, not a slow machine",
        connect + subscribe
    );
}

/// The loose ceiling for the cold-start measurement above.
///
/// Set well above anything a healthy run produces, because the failure worth
/// catching is a hang or a retry storm rather than a hundred milliseconds of
/// drift. The real numbers live in the migration design doc.
const COLD_START_CEILING: Duration = Duration::from_secs(10);

/// **Test 2 of Phase 5, against a real server.** A row somebody else writes
/// reaches this board's `SharedRows` without anything here asking for it.
///
/// The in-process tests drive `SharedRows` directly, which proves the decoding
/// and the wake-up but not that the SDK ever calls it. This one closes that
/// gap: the row is written through the CLI — a different process, standing in
/// for a teammate's board — and the only thing connecting the two is the
/// subscription.
#[test]
fn a_row_written_elsewhere_arrives_through_the_subscription() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start();
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![1],
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.epics().is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        let mut woken = rows.changed();
        woken.mark_unchanged();

        // A different process writes the row. Nothing below asks for it.
        let seeded = instance.call(
            "seed_epics",
            &[&serde_json::json!([{
                "id": 1,
                "title": "Written by somebody else",
                "description": "",
                "status": "backlog",
                "plan_path": "",
                "sort_order": {"none": []},
                "created_at": "2026-09-19 10:00:00",
                "updated_at": "2026-09-19 10:00:00",
                "auto_dispatch": false,
                "parent_epic_id": 0,
                "feed_command": "",
                "feed_interval_secs": 0,
                "group_by_repo": false,
                "feed_role": "none",
                "origin": "manual",
                "feed_append_only": false,
                "completed_at": "",
            }])
            .to_string()],
        );
        assert!(seeded.status.success(), "{}", describe(&seeded));

        // No sleep and no poll: the wake-up is the assertion. If the row never
        // arrives this hangs and the harness kills it, which is louder than a
        // slept-through comparison.
        woken
            .changed()
            .await
            .expect("the subscription must deliver");

        let epics = rows.epics();
        assert_eq!(epics.len(), 1);
        assert_eq!(epics[0].title, "Written by somebody else");
    });
}

// ---------------------------------------------------------------------------
// Phase 6 — the mutations, and the two things only a real store can show
// ---------------------------------------------------------------------------
//
// Everything below writes through a REDUCER and reads back over SQL. The point
// is not that the reducers compile — `check-spacetime-module.sh` covers that —
// it is the two properties that need more than one writer to exist at all:
// an epic status derived once from every host's children, and two hosts
// changing one task without undoing each other.

/// A blank epic row, as `seed_epics` and `create_epic` both take it.
fn epic_json(id: i64, title: &str, status: &str, parent: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "description": "",
        "status": status,
        "plan_path": "",
        "sort_order": {"none": []},
        "created_at": "2026-09-19 10:00:00",
        "updated_at": "2026-09-19 10:00:00",
        "auto_dispatch": false,
        "parent_epic_id": parent,
        "feed_command": "",
        "feed_interval_secs": 0,
        "group_by_repo": false,
        "feed_role": "none",
        "origin": "manual",
        "feed_append_only": false,
        "completed_at": "",
    })
}

/// A task row owned by `host`, in `epic`, at `status`.
///
/// `host` is what makes these tests about two machines rather than about two
/// calls: a task's host is the machine holding its worktree, and it is the only
/// thing in the row that says which board's work it is.
fn task_json(id: i64, title: &str, status: &str, epic: i64, host: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "description": "",
        "repo_path": "/repo",
        "status": status,
        "worktree": "",
        "tmux_window": "",
        "plan_path": "",
        "epic_id": epic,
        "sub_status": "none",
        "tag": "",
        "sort_order": {"none": []},
        "created_at": "2026-09-19 10:00:00",
        "updated_at": "2026-09-19 10:00:00",
        "base_branch": "main",
        "external_id": "",
        "labels": "[]",
        "last_pre_tool_use_at": "",
        "last_notification_at": "",
        "wrap_up_mode": "",
        "url": "",
        "url_type": "",
        "pr_learnings_gate_shown_at": "",
        "auto_run_plan": false,
        "live_subagents": 0,
        "stop_pending": false,
        "stop_pending_at": "",
        "live_shells": 0,
        "oldest_live_shell_started_at": "",
        "last_peer_message_sent_at": "",
        "last_peer_message_received_at": "",
        "phoenix": false,
        "host": host,
        "owner": "",
        "completed_at": "",
    })
}

/// An empty patch, to be filled by the caller. Every field named, because a
/// missing one is a deserialisation failure rather than a `None`.
fn empty_task_patch() -> serde_json::Value {
    let mut patch = serde_json::Map::new();
    for field in [
        "title",
        "description",
        "repo_path",
        "status",
        "worktree",
        "tmux_window",
        "plan_path",
        "epic_id",
        "sub_status",
        "tag",
        "sort_order",
        "base_branch",
        "external_id",
        "labels",
        "last_pre_tool_use_at",
        "last_notification_at",
        "wrap_up_mode",
        "url",
        "url_type",
        "pr_learnings_gate_shown_at",
        "auto_run_plan",
        "live_subagents",
        "stop_pending",
        "stop_pending_at",
        "live_shells",
        "oldest_live_shell_started_at",
        "last_peer_message_sent_at",
        "last_peer_message_received_at",
        "phoenix",
        "host",
        "owner",
        "completed_at",
    ] {
        patch.insert(field.into(), serde_json::json!({"none": []}));
    }
    serde_json::Value::Object(patch)
}

/// A patch that sets one string column.
fn patch_setting(field: &str, value: &str) -> serde_json::Value {
    let mut patch = empty_task_patch();
    patch[field] = serde_json::json!({"some": value});
    patch
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
fn column(instance: &Instance, query: &str) -> String {
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
fn no_rows(instance: &Instance, query: &str) -> bool {
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

fn published_instance() -> Instance {
    let instance = Instance::start();
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));
    instance
}

/// THE FLAPPING TEST. An epic whose children live on two machines has ONE
/// status, and it is the one every child agrees on.
///
/// The set-up is the exact disagreement the 2026-09-13 design named: host-a
/// sees only a done child and would write `done`; host-b sees only a running
/// one and would write `backlog`. Neither is wrong about what it can see. The
/// store sees both, and the answer below is the only one that is right about
/// the epic.
#[test]
fn an_epics_status_is_derived_from_every_hosts_children() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Shared", "backlog", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "finished on host-a", "done", 1, "host-a"),
            task_json(2, "still going on host-b", "running", 1, "host-b"),
        ])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let recalculated = instance.call("recalculate_epic_status", &["1"]);
    assert!(recalculated.status.success(), "{}", describe(&recalculated));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog",
        "one child is still running on another host, so the epic is not done"
    );

    // The other host finishes. Now — and only now — the epic is done, and the
    // transition is driven by the child's own patch rather than by anybody
    // remembering to recalculate.
    let patched = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done"
    );
}

/// The completion stamp is the store's clock, and it lands on the transition
/// into done rather than on every write afterwards.
#[test]
fn finishing_an_epic_stamps_a_completion_from_the_stores_clock() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "running", 1, "host-a")]).to_string()],
    );

    let patched = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "done").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));

    let stamped = column(&instance, "SELECT completed_at FROM epics WHERE id = 1");
    assert_ne!(stamped, "", "finishing an epic must stamp a completion");
    // The format both stores write: `YYYY-MM-DD HH:MM:SS.mmm`.
    assert_eq!(stamped.len(), 23, "unexpected timestamp shape: {stamped}");

    // Reopening does not unmake a completion. `completed_at` records the last
    // one, and the regression writes only the status.
    let reopened = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "running").to_string()],
    );
    assert!(reopened.status.success(), "{}", describe(&reopened));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
    assert_eq!(
        column(&instance, "SELECT completed_at FROM epics WHERE id = 1"),
        stamped,
        "a regression must not clear the last completion"
    );
}

/// TWO HOSTS WRITING ONE TASK CONVERGE. Each names only the field it changed,
/// so the later write does not undo the earlier one.
///
/// This is the whole reason the patch reducers take one `Option` per field
/// rather than a row. With a full-row write each host would send a complete
/// task built from what it last saw, and the second would silently revert the
/// first's change to a field it never touched.
#[test]
fn two_hosts_patching_different_fields_of_one_task_converge() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "original", "backlog", 1, "")]).to_string()],
    );

    // Host A renames it.
    let a = instance.call(
        "patch_task",
        &[
            "1",
            &patch_setting("title", "renamed by host-a").to_string(),
        ],
    );
    assert!(a.status.success(), "{}", describe(&a));

    // Host B, which still believes the title is "original", moves it.
    let b = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/host-b").to_string()],
    );
    assert!(b.status.success(), "{}", describe(&b));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "renamed by host-a",
        "host B's write must not revert a field it never named"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "/wt/host-b"
    );
}

/// The validator runs at the store, and its refusal changes nothing.
///
/// `core.allium: OwnerTracksUserBoardTask`, enforced by the module's
/// `write_task`. Asserted here rather than only as a unit test because what
/// matters is that a REDUCER refuses — a validator that lived on the client
/// could be skipped by any other client.
#[test]
fn the_store_refuses_a_user_board_task_with_no_owner() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let refused = instance.call(
        "create_task",
        &[&task_json(0, "nobody's task", "backlog", 0, "").to_string()],
    );
    assert!(
        !refused.status.success(),
        "a task with no epic and no owner must be refused, got {}",
        describe(&refused)
    );

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "a refused create must leave no row behind"
    );

    // The same row WITH an owner is accepted, so the refusal is the rule rather
    // than the reducer being broken.
    let accepted = instance.call(
        "create_task",
        &[&{
            let mut row = task_json(0, "my task", "backlog", 0, "");
            row["owner"] = serde_json::json!("user-me");
            row
        }
        .to_string()],
    );
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// ONE CLAIM, ONE WINNER — and the loser is TOLD.
///
/// `dispatch.allium: DispatchClaimExclusive`. A reducer returns no value, so
/// "did I win?" is carried by the only channel it has: the second claim is
/// REFUSED. Reading the row back instead would not work — a claim does not
/// stamp the winner's name on it, so both hosts would see `running` and both
/// would believe they won.
///
/// Sequential here rather than concurrent, and that is the honest limit of this
/// test: it shows the claim excludes an already-claimed task, not that two
/// simultaneous calls cannot interleave. The latter is the reducer's
/// transaction, which is the store's guarantee rather than something a test on
/// this side can observe.
#[test]
fn a_claimed_task_is_not_claimed_twice() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "the only one", "backlog", 1, "")]).to_string()],
    );

    let first = instance.call("claim_backlog_task", &["1", "host-a"]);
    assert!(first.status.success(), "{}", describe(&first));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );

    let second = instance.call("claim_backlog_task", &["1", "host-b"]);
    assert!(
        !second.status.success(),
        "the second claim must be refused, not silently ignored: {}",
        describe(&second)
    );
    assert!(
        describe(&second).contains("already claimed"),
        "the refusal must say why: {}",
        describe(&second)
    );
}

/// A foreign-owned backlog task is refused, not claimed. Its worktree is on
/// another machine, so dispatching an agent here would give it nowhere to work.
#[test]
fn a_task_owned_by_another_host_cannot_be_claimed() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "host-a's", "backlog", 1, "host-a")]).to_string()],
    );

    let refused = instance.call("claim_backlog_task", &["1", "host-b"]);
    assert!(
        !refused.status.success(),
        "a task whose worktree is elsewhere must be refused: {}",
        describe(&refused)
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog",
        "a refused claim must change nothing"
    );

    // Its owner can still take it, so the refusal is the rule rather than the
    // reducer being broken.
    let won = instance.call("claim_backlog_task", &["1", "host-a"]);
    assert!(won.status.success(), "{}", describe(&won));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
}

/// Releasing a claim that already has a worktree is refused. That claim is a
/// dispatch in progress, and releasing it would put a running agent's task back
/// in the backlog for another host to claim underneath it.
#[test]
fn a_claim_with_a_worktree_cannot_be_released() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "provisioned", "running", 1, "host-a"),
            task_json(2, "not yet", "running", 1, "host-a"),
        ])
        .to_string()],
    );
    let gave_it_a_worktree = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/1").to_string()],
    );
    assert!(
        gave_it_a_worktree.status.success(),
        "{}",
        describe(&gave_it_a_worktree)
    );

    let refused = instance.call("release_backlog_claim", &["1"]);
    assert!(
        !refused.status.success(),
        "a provisioned claim must not be releasable: {}",
        describe(&refused)
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );

    // One that never got a worktree goes back to the backlog, which is what
    // the release is for.
    let released = instance.call("release_backlog_claim", &["2"]);
    assert!(released.status.success(), "{}", describe(&released));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 2"),
        "backlog"
    );
}

/// Deleting a task takes its watchers with it, in the same transaction. A watch
/// pointing at a task that no longer exists is not a row anybody can act on.
#[test]
fn deleting_a_task_takes_its_watchers_with_it() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "watched", "backlog", 1, ""),
            task_json(2, "watcher", "backlog", 1, ""),
        ])
        .to_string()],
    );
    let seeded = instance.call(
        "seed_task_watchers",
        &[&serde_json::json!([{
            "id": 1,
            "watcher_task_id": 2,
            "target_task_id": 1,
            "created_at": "2026-09-19 10:00:00",
        }])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM task_watchers"),
        "the watch must go with the task it pointed at"
    );
}
