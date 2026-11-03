//! A throwaway `spacetime start` for the startup smoke tests
//! (`startup_smoke.rs`), in-crate because `TuiRuntime::bootstrap` is private
//! and so the smoke tests cannot live under `tests/` beside
//! `tests/common/spacetime_instance.rs`.
//!
//! Never the operator's: a free port, a data directory, CLI config and JWT key
//! pair all named by the caller (`startup.allium`:
//! `StartupIsSmokeTestedAgainstARealStore`). Started with the production
//! command (`SpacetimeStartSpawner::command`) and judged ready by the
//! production probe (`probe_address`).

use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::spacetime::managed_store::{probe_address, ManagedAddressState, SpacetimeStartSpawner};

/// How long a started store has to answer as a store.
const START_DEADLINE: Duration = Duration::from_secs(30);

/// Whether the live-store tests run here: the same rule, and the same
/// function, as `src/spacetime/tests/managed_store_real.rs`.
pub(super) use crate::spacetime::tests::managed_store_real::spacetime_available as live_store_tests_should_run;

/// Run the `spacetime` CLI against `cli_config`, never the operator's.
pub(super) fn spacetime(cli_config: &Path, args: &[&str]) -> Output {
    Command::new("spacetime")
        .arg(format!("--config-path={}", cli_config.display()))
        .args(args)
        .output()
        .expect("run the spacetime CLI")
}

/// A standalone store on a free loopback port, killed on drop.
pub(super) struct LiveStore {
    child: Child,
    port: u16,
}

/// The port a store reports binding, from its stdout: `Starting SpacetimeDB
/// listening on 127.0.0.1:<port>`.
fn bound_port(stdout: &str) -> Option<u16> {
    let after = stdout.split("listening on ").nth(1)?;
    let address = after.split_whitespace().next()?;
    address.rsplit(':').next()?.parse().ok()
}

impl LiveStore {
    /// Start a store over `data_dir`, signing with the key pair in
    /// `keys_dir` (created there when absent), and return once it answers as
    /// a store. The store binds port 0 and reports the port it got on stdout,
    /// so no other process can take it between a pick and the bind. Panics,
    /// showing the exit status, stdout and stderr, on an early exit and on the
    /// deadline.
    pub(super) fn start(data_dir: &Path, keys_dir: &Path, cli_config: &Path) -> LiveStore {
        // One pair of output files per store: a test may hold two at once.
        static STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stem = keys_dir.with_file_name(format!("store-{n}"));
        let stdout_path = stem.with_extension("stdout");
        let stderr_path = stem.with_extension("stderr");
        let stdout_file = std::fs::File::create(&stdout_path).expect("create the stdout file");
        let stderr_file = std::fs::File::create(&stderr_path).expect("create the stderr file");
        let child = SpacetimeStartSpawner::new()
            .with_config_path(cli_config)
            .with_key_dir(keys_dir)
            .command("127.0.0.1:0", data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .expect("spawn spacetime start");
        let mut store = LiveStore { child, port: 0 };
        let report = |what: &str| {
            format!(
                "{what}\nstdout:\n{}\nstderr:\n{}",
                std::fs::read_to_string(&stdout_path).unwrap_or_default(),
                std::fs::read_to_string(&stderr_path).unwrap_or_default()
            )
        };
        let deadline = Instant::now() + START_DEADLINE;
        loop {
            // Our own child first: a probe that answers while it is dead is
            // another test's store on this port.
            if let Some(status) = store.child.try_wait().expect("poll the store") {
                panic!(
                    "{}",
                    report(&format!("the store exited during startup with {status}"))
                );
            }
            if store.port == 0 {
                let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
                store.port = bound_port(&stdout).unwrap_or(0);
            }
            if store.port != 0
                && probe_address(&store.address(), Duration::from_secs(2))
                    == ManagedAddressState::StoreAnswering
                && store.child.try_wait().expect("poll the store").is_none()
            {
                return store;
            }
            assert!(
                Instant::now() < deadline,
                "{}",
                report("the store never answered")
            );
            // allow-test-sleep: deadline-bounded poll, not a fixed wait.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `127.0.0.1:<port>`.
    pub(super) fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub(super) fn url(&self) -> String {
        format!("http://{}", self.address())
    }

    /// Stop with SIGTERM and wait for the exit, so the store flushes what it
    /// holds before its data directory is copied. Drop's SIGKILL does not.
    pub(super) fn stop(mut self) {
        let pid = self.child.id().to_string();
        let _ = Command::new("kill").args(["-TERM", &pid]).status();
        let deadline = Instant::now() + START_DEADLINE;
        while self.child.try_wait().expect("poll the store").is_none() {
            assert!(Instant::now() < deadline, "the store ignored SIGTERM");
            // allow-test-sleep: deadline-bounded poll for the exit.
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for LiveStore {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_live_store_serves_and_frees_its_port_on_drop() {
    if !live_store_tests_should_run() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let store = LiveStore::start(
        &dir.path().join("data"),
        &dir.path().join("keys"),
        &dir.path().join("cli.toml"),
    );
    let address = store.address();
    assert!(std::net::TcpStream::connect(&address).is_ok());
    assert!(dir.path().join("keys/id_ecdsa").is_file());
    drop(store);
    assert!(
        std::net::TcpListener::bind(&address).is_ok(),
        "the store still holds {address} after drop"
    );
}

#[test]
fn the_bound_port_is_read_from_the_stores_stdout() {
    assert_eq!(
        bound_port("2026-11-03T15:39:49Z  INFO start.rs:266: Starting SpacetimeDB listening on 127.0.0.1:32919\n"),
        Some(32919)
    );
    assert_eq!(bound_port("database running in data directory /x\n"), None);
    assert_eq!(bound_port(""), None);
}
