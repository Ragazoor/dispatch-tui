//! The managed store's real ports and its embedded module -- the parts
//! `managed_store.rs` (the spec-driven tests) drives through fakes.
//!
//! Three groups: the embedded module against its source stamp (pure Rust, runs
//! everywhere), the real ports over a stand-in store process and a loopback
//! HTTP listener (no `spacetime` needed), and one end-to-end run against a real
//! throwaway `spacetime start` on a private port, skipped when the CLI is not on
//! `PATH` like `tests/spacetime_module.rs`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use crate::process::{MockProcessRunner, ProcessRunner};
use crate::spacetime::managed_store::{
    embedded_module_hash, module_hash, parse_lsof_pids, parse_ss_pids, FileModuleHashRecord,
    ManagedAddressState, ManagedStore, ManagedStoreLayout, ManagedStorePorts, ModuleHashRecord,
    SpacetimeManagedStore, SpacetimeStartSpawner, StoreSpawner, EMBEDDED_MODULE,
};

// ---------------------------------------------------------------------------
// The embedded module
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hmac_sha256::Hash::hash(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn module_files(dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(repo_root().join(dir)).unwrap() {
        let entry = entry.unwrap();
        let rel = dir.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            module_files(&rel, out);
        } else {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}

/// `scripts/build-managed-module.sh`'s stamp recipe, in Rust.
fn module_source_hash() -> String {
    let mut files = Vec::new();
    module_files(Path::new("spacetime/module/src"), &mut files);
    files.push("spacetime/module/Cargo.toml".to_string());
    files.push("spacetime/module/Cargo.lock".to_string());
    files.sort();
    let listing: String = files
        .iter()
        .map(|f| {
            let bytes = std::fs::read(repo_root().join(f)).unwrap();
            format!("{}  {f}\n", sha256_hex(&bytes))
        })
        .collect();
    sha256_hex(listing.as_bytes())
}

#[test]
fn the_embedded_module_is_a_wasm_binary() {
    assert!(
        EMBEDDED_MODULE.starts_with(b"\0asm"),
        "src/spacetime/module.wasm is not a wasm module"
    );
    assert_eq!(embedded_module_hash(), module_hash(EMBEDDED_MODULE));
}

/// A wasm build is not byte-reproducible across machines (it embeds the cargo
/// registry's absolute path, and `wasm-opt` is optional), so staleness is
/// gated on a stamp of the module's SOURCE instead -- see
/// `scripts/build-managed-module.sh`.
#[test]
fn the_embedded_module_is_not_stale_against_the_module_source() {
    let stamp = std::fs::read_to_string(repo_root().join("src/spacetime/module.wasm.source-hash"))
        .expect("src/spacetime/module.wasm.source-hash is committed beside the module");
    assert_eq!(
        stamp.trim(),
        module_source_hash(),
        "spacetime/module changed since src/spacetime/module.wasm was built -- \
         run ./scripts/build-managed-module.sh and commit the result"
    );
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

#[test]
fn ss_output_yields_the_listening_pids() {
    let out = "LISTEN 0 4096 0.0.0.0:3000 0.0.0.0:* users:((\"spacetimedb-sta\",pid=24470,fd=12))\n\
               LISTEN 0 4096 [::]:3000 [::]:* users:((\"spacetimedb-sta\",pid=24470,fd=13),(\"x\",pid=7,fd=1))\n";
    assert_eq!(parse_ss_pids(out), vec![24470, 7]);
    assert_eq!(parse_ss_pids("nothing here"), Vec::<u32>::new());
}

#[test]
fn lsof_output_yields_the_listening_pids() {
    assert_eq!(
        parse_lsof_pids("24470\n24470\n  9 \nnope\n"),
        vec![24470, 9]
    );
}

#[test]
fn the_store_is_started_detached_on_loopback_over_its_own_data_directory() {
    let command =
        SpacetimeStartSpawner::new().command("127.0.0.1:3000", Path::new("/data/spacetime"));
    assert_eq!(command.get_program(), "spacetime");
    let args: Vec<String> = command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "start",
            "--listen-addr=127.0.0.1:3000",
            "--data-dir=/data/spacetime",
            "--non-interactive"
        ]
    );
}

/// A test's store signs with its own key pair: left to itself,
/// `spacetime start` uses the operator's `~/.config/spacetime/id_ecdsa`.
#[test]
fn a_key_dir_is_passed_to_spacetime_start() {
    let command = SpacetimeStartSpawner::new()
        .with_key_dir("/tmp/keys")
        .command("127.0.0.1:4000", Path::new("/data/spacetime"));
    let args: Vec<String> = command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(
        args.contains(&"--jwt-priv-key-path=/tmp/keys/id_ecdsa".to_string()),
        "{args:?}"
    );
    assert!(
        args.contains(&"--jwt-pub-key-path=/tmp/keys/id_ecdsa.pub".to_string()),
        "{args:?}"
    );
}

#[test]
fn a_hash_record_is_read_back_from_the_store_data_directory_not_the_board_one() {
    let store = tempfile::tempdir().unwrap();
    let board = tempfile::tempdir().unwrap();
    assert!(FileModuleHashRecord::in_data_dir(store.path()).record_module_hash("sha256:x"));
    assert_eq!(
        FileModuleHashRecord::in_data_dir(store.path()).recorded_module_hash(),
        Some("sha256:x".to_string())
    );
    assert_eq!(
        FileModuleHashRecord::in_data_dir(board.path()).recorded_module_hash(),
        None
    );
}

// ---------------------------------------------------------------------------
// The real ports over a stand-in store
// ---------------------------------------------------------------------------

/// Answers `kill` for real and everything else (`ss`, `lsof`) with nothing, so
/// a test's in-process listener is never mistaken for a store to signal.
struct KillOnlyRunner;

impl ProcessRunner for KillOnlyRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        if program == "kill" {
            Ok(Command::new("kill").args(args).output()?)
        } else {
            Ok(Command::new("true").output()?)
        }
    }
}

/// [`KillOnlyRunner`] that also remembers which programs it was asked for.
struct RecordingKillRunner(std::sync::Mutex<Vec<String>>);

impl ProcessRunner for RecordingKillRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<Output> {
        self.0.lock().unwrap().push(program.to_string());
        KillOnlyRunner.run(program, args)
    }
}

/// Spawns a shell one-liner instead of `spacetime start`.
struct ShellSpawner(&'static str);

impl StoreSpawner for ShellSpawner {
    fn spawn(&self, _: &str, _: &Path, _: &Path) -> std::io::Result<Child> {
        Command::new("sh")
            .args(["-c", self.0])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
}

/// Spawns a shell one-liner and waits for it, so the child is already gone
/// when the start begins to poll.
struct ExitedSpawner(&'static str);

impl StoreSpawner for ExitedSpawner {
    fn spawn(&self, a: &str, d: &Path, l: &Path) -> std::io::Result<Child> {
        let mut child = ShellSpawner(self.0).spawn(a, d, l)?;
        child.wait()?;
        Ok(child)
    }
}

struct FailingSpawner;

impl StoreSpawner for FailingSpawner {
    fn spawn(&self, _: &str, _: &Path, _: &Path) -> std::io::Result<Child> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "spacetime: not found",
        ))
    }
}

/// A loopback listener that answers every request `200 OK`, on a background
/// thread. Returns its address.
fn ping_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\naccess-control-allow-origin: *\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        }
    });
    address
}

/// How many free ports [`on_free_address`] tries before giving up.
pub(crate) const PORT_ATTEMPTS: usize = 5;

/// An address nothing listens on, for a test whose fake spawner never binds
/// a port. It is a placeholder, not a pick: port 1 is privileged, so no test's
/// store can ever take it, and nothing is bound or dropped to get it.
pub(crate) const UNUSED_ADDRESS: &str = "127.0.0.1:1";

/// A loopback `host:port` the OS had free a moment ago, for [`on_free_address`]
/// and the async smoke test's own retry loop. This is the one bind-and-drop
/// left, and it is immune because both callers prove, through [`listener_runs_with_arg`], that the
/// store answering on it is the one the attempt started, and start over when
/// it is not.
pub(crate) fn candidate_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// Whether the process listening on `address` was started with `arg` on its
/// command line (a store over `--data-dir=<this test's dir>`), so a store
/// another test started on a port this one lost is told apart from our own.
/// The error says "already in use" so [`on_free_address`] retries on a fresh
/// port.
pub(crate) fn listener_runs_with_arg(address: &str, arg: &str) -> Result<(), String> {
    let ours = crate::spacetime::managed_store::listener_has_arg(address, arg);
    if ours {
        Ok(())
    } else {
        Err(format!(
            "the listener on {address} is not the store this test \
             started ({arg}): another test's store holds the port, already in use"
        ))
    }
}

/// Whether `output` (a store's stderr or log) says it lost the race for its
/// port: the only early exit worth another attempt. Any other exit is a real
/// failure to report.
///
/// `spacetime start` reports a taken port on STDERR as `Error: Port N is
/// already in use. ...` (and, without `--non-interactive`, asks on stdout
/// whether to use the next port). The OS's own `Address already in use` and
/// `AddrInUse` spellings count too.
pub(crate) fn is_address_in_use(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("already in use") || lower.contains("addrinuse")
}

/// `error`, followed by the log it points at, so [`is_address_in_use`] can see
/// what the store itself said and a panic shows it.
pub(crate) fn failure_with_log(error: impl std::fmt::Display, log: &Path) -> String {
    let logged = std::fs::read_to_string(log).unwrap_or_default();
    format!("{error}\n{logged}")
}

/// Run `attempt` on a free address, again on a fresh one when it fails because
/// the store lost the race for its port ([`is_address_in_use`] on the error),
/// and panic with the error on any other failure or after [`PORT_ATTEMPTS`].
/// The attempt's error must carry what the store said ([`failure_with_log`]).
pub(crate) fn on_free_address<T>(mut attempt: impl FnMut(String) -> Result<T, String>) -> T {
    let mut last = String::new();
    for _ in 0..PORT_ATTEMPTS {
        match attempt(candidate_address()) {
            Ok(value) => return value,
            Err(error) if is_address_in_use(&error) => last = error,
            Err(error) => panic!("the store failed to start: {error}"),
        }
    }
    panic!("no store stayed up on a free port in {PORT_ATTEMPTS} attempts; last error:\n{last}");
}

fn ports(
    runner: Arc<dyn ProcessRunner>,
    spawner: Arc<dyn StoreSpawner>,
    address: String,
    dir: &Path,
    timeout: Duration,
) -> SpacetimeManagedStore {
    SpacetimeManagedStore::new(
        runner,
        spawner,
        ManagedStoreLayout {
            address,
            data_dir: dir.join("data"),
            log_path: dir.join("logs").join("managed-store.log"),
        },
        b"\0asm test",
    )
    .with_timeouts(timeout, Duration::from_secs(5))
}

const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn a_started_store_that_serves_is_ready_and_is_then_stopped_by_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(ShellSpawner("exec sleep 300")),
        ping_server(),
        dir.path(),
        TIMEOUT,
    );

    assert_eq!(store.start_managed_store(), Ok(()));
    // The stand-in process is reaped (a zombie would fail this wait), which is
    // only so if SIGTERM reached it.
    assert!(store.stop_managed_store());
}

#[test]
fn a_store_that_exits_at_once_fails_the_start_with_its_status() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(ShellSpawner("exit 3")),
        UNUSED_ADDRESS.into(),
        dir.path(),
        TIMEOUT,
    );

    let error = store.start_managed_store().unwrap_err();
    assert!(error.contains("exited with"), "{error}");
    assert!(error.contains("managed-store.log"), "{error}");
}

#[test]
fn a_store_that_never_serves_fails_the_start_at_the_deadline_and_is_still_stoppable() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(ShellSpawner("exec sleep 300")),
        UNUSED_ADDRESS.into(),
        dir.path(),
        Duration::from_millis(300),
    );

    let error = store.start_managed_store().unwrap_err();
    assert!(error.contains("did not serve HTTP"), "{error}");
    assert!(
        store.stop_managed_store(),
        "the launched process is tracked from the start"
    );
}

#[test]
fn a_foreign_store_answering_does_not_make_a_dead_child_ready() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(ExitedSpawner("exit 3")),
        ping_server(),
        dir.path(),
        TIMEOUT,
    );

    let error = store.start_managed_store().unwrap_err();
    assert!(error.contains("exited with"), "{error}");
}

#[test]
fn stopping_a_failed_start_signals_only_the_child_never_the_port() {
    let runner = Arc::new(RecordingKillRunner(std::sync::Mutex::new(Vec::new())));
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        runner.clone(),
        Arc::new(ShellSpawner("exec sleep 300")),
        UNUSED_ADDRESS.into(),
        dir.path(),
        Duration::from_millis(200),
    );
    assert!(store.start_managed_store().is_err());

    assert!(store.stop_spawned_store());
    let programs = runner.0.lock().unwrap().clone();
    assert!(
        !programs.iter().any(|p| p == "ss" || p == "lsof"),
        "{programs:?}"
    );
    assert!(programs.iter().any(|p| p == "kill"), "{programs:?}");
}

#[test]
fn a_spacetime_that_cannot_run_fails_the_start_with_the_cause() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(FailingSpawner),
        UNUSED_ADDRESS.into(),
        dir.path(),
        TIMEOUT,
    );
    let error = store.start_managed_store().unwrap_err();
    assert!(error.contains("not found"), "{error}");
}

#[test]
fn an_adopted_store_is_found_on_its_port_and_sigtermed() {
    let ss = "LISTEN 0 4096 0.0.0.0:3000 0.0.0.0:* users:((\"spacetimedb-sta\",pid=4242,fd=12))";
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(ss.as_bytes()),
        MockProcessRunner::ok(),                    // kill -TERM 4242
        MockProcessRunner::fail("No such process"), // kill -0 4242: gone
    ]));
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        runner.clone(),
        Arc::new(FailingSpawner),
        "127.0.0.1:3000".into(),
        dir.path(),
        TIMEOUT,
    );

    assert!(store.stop_managed_store());
    let calls = runner.recorded_calls();
    assert_eq!(calls[0].0, "ss");
    assert!(calls[0].1.iter().any(|a| a.contains(":3000")), "{calls:?}");
    assert_eq!(
        calls[1],
        ("kill".into(), vec!["-TERM".into(), "4242".into()])
    );
    assert_eq!(calls[2], ("kill".into(), vec!["-0".into(), "4242".into()]));
}

#[test]
fn nothing_to_find_on_the_port_means_nothing_to_stop() {
    let runner = Arc::new(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // ss: empty
        MockProcessRunner::ok(), // lsof: empty
    ]));
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        runner.clone(),
        Arc::new(FailingSpawner),
        "127.0.0.1:3000".into(),
        dir.path(),
        TIMEOUT,
    );
    assert!(store.stop_managed_store());
    let programs: Vec<String> = runner.recorded_calls().into_iter().map(|c| c.0).collect();
    assert_eq!(programs, ["ss", "lsof"]);
}

// ---------------------------------------------------------------------------
// End to end, against a real store on a private port
// ---------------------------------------------------------------------------

/// Whether the live test below should run. Under CI a missing CLI is a hard
/// failure, not a skip: an `eprintln!` skip is a silent pass, so a broken
/// `Install spacetime CLI` step would otherwise turn this test into a no-op.
/// The same rule as `spacetime_available_or_skip` in
/// `tests/common/spacetime_instance.rs`, which this crate cannot import.
fn live_store_test_should_run(cli_present: bool, in_ci: bool) -> bool {
    assert!(
        cli_present || !in_ci,
        "spacetime is required in CI but was not found on PATH — the workflow's \
         `Install spacetime CLI` step must run before the tests (see \
         .github/workflows/ci.yml). Refusing to skip and report green."
    );
    cli_present
}

#[test]
fn a_missing_cli_skips_the_live_test_outside_ci_only() {
    assert!(live_store_test_should_run(true, false));
    assert!(live_store_test_should_run(true, true));
    assert!(!live_store_test_should_run(false, false));
}

#[test]
#[should_panic(expected = "spacetime is required in CI")]
fn a_missing_cli_fails_the_live_test_in_ci() {
    live_store_test_should_run(false, true);
}

pub(crate) fn spacetime_available() -> bool {
    let present = Command::new("spacetime")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let run = live_store_test_should_run(present, std::env::var_os("CI").is_some());
    if !run {
        eprintln!("skipping: spacetime not available on PATH");
    }
    run
}

/// Never touches port 3000: the layout's address is a free private port and the
/// CLI runs against a throwaway config, so an operator's own store is safe.
#[test]
fn a_real_store_is_brought_up_published_to_and_stopped() {
    if !spacetime_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("cli.toml");
    // A port taken between the pick and the store's bind is a lost race: the
    // attempt starts over on a fresh one, with the store's log in the error.
    let (real, hashes, managed) = on_free_address(|address| {
        let layout = ManagedStoreLayout {
            address,
            data_dir: dir.path().join("data"),
            log_path: dir.path().join("managed-store.log"),
        };
        let real = Arc::new(
            SpacetimeManagedStore::new(
                Arc::new(crate::process::RealProcessRunner::default()),
                Arc::new(
                    SpacetimeStartSpawner::new()
                        .with_config_path(&config)
                        .with_key_dir(dir.path().join("keys")),
                ),
                layout.clone(),
                EMBEDDED_MODULE,
            )
            .with_config_path(&config)
            .with_timeouts(Duration::from_secs(60), Duration::from_secs(15)),
        );
        let hashes = Arc::new(FileModuleHashRecord::in_data_dir(&layout.data_dir));
        let managed = ManagedStore::new(real.clone(), hashes.clone(), embedded_module_hash());
        assert_eq!(
            real.probe_managed_address(),
            ManagedAddressState::NothingListening
        );
        managed
            .bring_up()
            .map_err(|e| failure_with_log(format!("{e:?}"), &layout.log_path))?;
        // A store another test started on this port would have been adopted
        // as ours; only the one over this test's data directory counts.
        let ours = format!("--data-dir={}", layout.data_dir.display());
        if let Err(e) = listener_runs_with_arg(&layout.address, &ours) {
            managed.stop_spawned_on_exit();
            return Err(e);
        }
        Ok((real, hashes, managed))
    });
    assert_eq!(
        real.probe_managed_address(),
        ManagedAddressState::StoreAnswering
    );
    assert!(real.managed_database_exists());
    assert_eq!(hashes.recorded_module_hash(), Some(embedded_module_hash()));

    assert!(managed.stop_on_exit());
    assert_eq!(
        real.probe_managed_address(),
        ManagedAddressState::NothingListening
    );
}

// ---------------------------------------------------------------------------
// managed_store_failed_to_save: the store's log, since this board took it on
// ---------------------------------------------------------------------------

const FAILURE_LINE: &str = "2026-10-05T06:03:35Z ERROR crates/durability/src/imp/local.rs:338: error flushing commitlog: No space left on device (os error 28)\n";

fn append(path: &Path, text: &str) {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

#[test]
fn a_failure_line_written_after_the_board_began_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(FailingSpawner),
        UNUSED_ADDRESS.into(),
        dir.path(),
        TIMEOUT,
    );
    let log = dir.path().join("logs").join("managed-store.log");

    assert!(!store.managed_store_failed_to_save());
    append(&log, "ordinary line\n");
    assert!(!store.managed_store_failed_to_save());
    append(&log, FAILURE_LINE);
    assert!(store.managed_store_failed_to_save());
}

#[test]
fn a_failure_line_from_before_the_board_began_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    append(
        &dir.path().join("logs").join("managed-store.log"),
        FAILURE_LINE,
    );
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(FailingSpawner),
        UNUSED_ADDRESS.into(),
        dir.path(),
        TIMEOUT,
    );

    assert!(!store.managed_store_failed_to_save());
}

#[test]
fn starting_the_store_moves_the_mark_past_the_old_failure() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("logs").join("managed-store.log");
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(ShellSpawner("exec sleep 300")),
        ping_server(),
        dir.path(),
        TIMEOUT,
    );
    append(&log, FAILURE_LINE);
    assert!(store.managed_store_failed_to_save());

    assert_eq!(store.start_managed_store(), Ok(()));
    assert!(
        !store.managed_store_failed_to_save(),
        "a restarted store is not failing for the line it was restarted over"
    );
    assert!(store.stop_managed_store());
}

#[test]
fn only_an_address_in_use_failure_is_retried() {
    assert!(is_address_in_use(
        "Error: Port 3000 is already in use. Please free up the port or specify a different port with --listen-addr."
    ));
    assert!(is_address_in_use(
        "Error: Address already in use (os error 98)"
    ));
    assert!(is_address_in_use("AddrInUse"));
    assert!(!is_address_in_use("Error: failed to open the database"));
    assert!(!is_address_in_use(""));
}

#[test]
fn on_free_address_retries_a_lost_race_and_stops_at_any_other_failure() {
    let mut tried = Vec::new();
    let value = on_free_address(|address| {
        tried.push(address);
        if tried.len() < 3 {
            Err("Error: Port 1 is already in use.".to_string())
        } else {
            Ok(tried.len())
        }
    });
    assert_eq!(value, 3, "two lost races, then a start");

    let result = std::panic::catch_unwind(|| {
        on_free_address(|_| -> Result<(), String> { Err("failed to open the database".into()) })
    });
    assert!(result.is_err(), "a failure that is not a lost race panics");

    let mut attempts = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        on_free_address(|_| -> Result<(), String> {
            attempts += 1;
            Err("already in use".into())
        })
    }));
    assert!(result.is_err());
    assert_eq!(attempts, PORT_ATTEMPTS, "gives up after the cap");
}

#[test]
fn a_listener_is_identified_by_an_argument_on_its_command_line() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    // This test process is the listener; its own argv[0] is on its command line.
    let argv0 = std::fs::read("/proc/self/cmdline").unwrap();
    let argv0 = String::from_utf8_lossy(argv0.split(|b| *b == 0).next().unwrap()).into_owned();
    assert_eq!(listener_runs_with_arg(&address, &argv0), Ok(()));
    let error = listener_runs_with_arg(&address, "--data-dir=/nowhere").unwrap_err();
    assert!(is_address_in_use(&error), "{error}");
}
