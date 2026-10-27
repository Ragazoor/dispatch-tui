//! A throwaway `spacetime start` for the startup smoke tests
//! (`startup_smoke.rs`), in-crate because `TuiRuntime::bootstrap` is private
//! and so the smoke tests cannot live under `tests/` beside
//! `tests/common/spacetime_instance.rs`.
//!
//! Never the operator's: a free port, a data directory, CLI config and JWT key
//! pair all named by the caller (`startup.allium`:
//! `StartupIsSmokeTestedAgainstARealStore`).

use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long a started store has to answer HTTP.
const START_DEADLINE: Duration = Duration::from_secs(30);

/// Whether the live-store tests run here. Without the `spacetime` CLI they
/// skip, saying so, except under CI, where a missing CLI is a failure — the
/// same rule as `tests/common/spacetime_instance.rs`.
pub(super) fn live_store_tests_should_run() -> bool {
    if crate::spacetime::managed_store::spacetime_cli_on_path() {
        return true;
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "spacetime is required under CI"
    );
    eprintln!("skipping: spacetime not available on PATH");
    false
}

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
    /// `keys_dir` (created there when absent), and return once it answers
    /// HTTP. Panics on the deadline or when the store exits first.
    pub(super) fn start(data_dir: &Path, keys_dir: &Path, cli_config: &Path) -> LiveStore {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("pick a free port")
            .port();
        let child = Command::new("spacetime")
            .arg(format!("--config-path={}", cli_config.display()))
            .arg("start")
            .arg(format!("--listen-addr=127.0.0.1:{port}"))
            .arg(format!("--data-dir={}", data_dir.display()))
            .arg(format!(
                "--jwt-priv-key-path={}",
                keys_dir.join("id_ecdsa").display()
            ))
            .arg(format!(
                "--jwt-pub-key-path={}",
                keys_dir.join("id_ecdsa.pub").display()
            ))
            .arg("--non-interactive")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spacetime start");
        let mut store = LiveStore { child, port };
        let deadline = Instant::now() + START_DEADLINE;
        while !store.answers_http() {
            assert!(
                store.child.try_wait().expect("poll the store").is_none(),
                "the store on port {port} exited during startup"
            );
            assert!(
                Instant::now() < deadline,
                "the store never served HTTP on port {port}"
            );
            // allow-test-sleep: deadline-bounded poll, not a fixed wait.
            std::thread::sleep(Duration::from_millis(50));
        }
        store
    }

    /// `127.0.0.1:<port>`.
    pub(super) fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub(super) fn url(&self) -> String {
        format!("http://{}", self.address())
    }

    /// Any HTTP status line counts: a listener is up before the API is, so a
    /// bare TCP connect is not readiness (learning #806).
    fn answers_http(&self) -> bool {
        use std::io::{Read, Write};
        let Ok(mut stream) = std::net::TcpStream::connect(self.address()) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let request = format!(
            "GET /v1/identity HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address()
        );
        if stream.write_all(request.as_bytes()).is_err() {
            return false;
        }
        let mut answer = Vec::new();
        matches!(stream.read_to_end(&mut answer), Ok(n) if n > 0) && answer.starts_with(b"HTTP/")
    }
}

impl LiveStore {
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
