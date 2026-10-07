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
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        }
    });
    address
}

fn free_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
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
        free_address(),
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
        free_address(),
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
fn a_spacetime_that_cannot_run_fails_the_start_with_the_cause() {
    let dir = tempfile::tempdir().unwrap();
    let store = ports(
        Arc::new(KillOnlyRunner),
        Arc::new(FailingSpawner),
        free_address(),
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

fn spacetime_available() -> bool {
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

/// Never touches port 3000: the layout's address is a free private port and the
/// CLI runs against a throwaway config, so an operator's own store is safe.
#[test]
fn a_real_store_is_brought_up_published_to_and_stopped() {
    if !spacetime_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("cli.toml");
    let address = free_address();
    let layout = ManagedStoreLayout {
        address: address.clone(),
        data_dir: dir.path().join("data"),
        log_path: dir.path().join("managed-store.log"),
    };
    let real = Arc::new(
        SpacetimeManagedStore::new(
            Arc::new(crate::process::RealProcessRunner::default()),
            Arc::new(SpacetimeStartSpawner::new().with_config_path(&config)),
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
        .expect("the store comes up and takes the embedded module");
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
        free_address(),
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
        free_address(),
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
