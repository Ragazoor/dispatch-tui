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

/// How many free ports `LiveStore::start` tries before giving up.
const PORT_ATTEMPTS: usize = 5;

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

impl LiveStore {
    /// Start a store over `data_dir`, signing with the key pair in
    /// `keys_dir` (created there when absent), and return once it answers as
    /// a store. Panics on the deadline, or when every port tried was taken.
    pub(super) fn start(data_dir: &Path, keys_dir: &Path, cli_config: &Path) -> LiveStore {
        // A port picked by binding and dropping can be taken before the store
        // binds it; the store then exits, and the next attempt picks again.
        for _ in 0..PORT_ATTEMPTS {
            if let Some(store) = Self::try_start(data_dir, keys_dir, cli_config) {
                return store;
            }
        }
        panic!("no store stayed up on a free port in {PORT_ATTEMPTS} attempts");
    }

    /// One attempt on a fresh port. `None` when the store exited during
    /// startup; panics on the deadline.
    fn try_start(data_dir: &Path, keys_dir: &Path, cli_config: &Path) -> Option<LiveStore> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("pick a free port")
            .port();
        let address = format!("127.0.0.1:{port}");
        let child = SpacetimeStartSpawner::new()
            .with_config_path(cli_config)
            .with_key_dir(keys_dir)
            .command(&address, data_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spacetime start");
        let mut store = LiveStore { child, port };
        let deadline = Instant::now() + START_DEADLINE;
        loop {
            // Our own child first: a probe that answers while it is dead is
            // another test's store on this port.
            if store.child.try_wait().expect("poll the store").is_some() {
                return None;
            }
            if probe_address(&address, Duration::from_secs(2))
                == ManagedAddressState::StoreAnswering
                && store.child.try_wait().expect("poll the store").is_none()
            {
                return Some(store);
            }
            assert!(
                Instant::now() < deadline,
                "the store never answered on port {port}"
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
