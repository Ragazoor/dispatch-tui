//! The managed local store: the SpacetimeDB instance the board brings up,
//! adopts, publishes to and stops for itself when the launch names no store.
//!
//! Spec: `docs/specs/startup.allium` — the `ManagedStoreHost` contract and the
//! rules from `AbortWhenTheManagedStoreHasNoCli` through
//! `StopTheManagedStoreWhenStartupAborts`, plus the invariants
//! `ANamedStoreIsNeverManaged`, `TheManagedStoreStopsWithItsBoard` and
//! `TheManagedStoreRunsTheEmbeddedModule`.
//!
//! # Shape
//!
//! The decisions are pure functions ([`select_store`], [`address_action`],
//! [`module_action`], [`after_publish`]) and [`ManagedStore`] sequences them
//! over two small ports, so the orchestration is unit-testable with fakes:
//!
//! - [`ManagedStorePorts`] — probe the address, start, check the database
//!   exists, publish, stop. [`SpacetimeCliPublisher`] is the `spacetime publish`
//!   half of the real implementation, driven through [`ProcessRunner`].
//! - [`ModuleHashRecord`] — where the published module's hash is kept.
//!   [`FileModuleHashRecord`] keeps it in a small file in the MANAGED STORE's
//!   own data directory (task #12296's decision) -- not the board's `--db`
//!   directory -- so every board on the managed store sees the same record and
//!   a worktree build with a throwaway `--db` cannot desync it. A store
//!   missing the `dispatch` database is published to regardless of what that
//!   file says.
//!
//! # Accepted edges (task #12296)
//!
//! - A store orphaned by a board that could not run its exit (crash, SIGKILL)
//!   keeps running until the next managed board adopts it and stops it on
//!   exit. Accepted; nothing bounds its lifetime.
//! - Two managed boards on one machine share one store, and whichever exits
//!   first stops it under the other. Accepted as an edge reached on purpose.
//!
//! [`SpacetimeManagedStore`] is the real [`ManagedStorePorts`]: it starts
//! `spacetime start` detached through a [`StoreSpawner`] seam, tracks the pid of
//! a store it spawned, and stops it (or an adopted one, found through
//! `ss`/`lsof`) with SIGTERM -- the CLI has no stop command.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::process::ProcessRunner;
use crate::startup::StartupAbort;

/// `startup.allium`'s `config.managed_store_address`. Loopback only.
pub const MANAGED_STORE_ADDRESS: &str = "127.0.0.1:3000";
/// `startup.allium`'s `config.managed_database_name`.
pub const MANAGED_DATABASE_NAME: &str = "dispatch";
/// `startup.allium`'s `config.managed_store_start_timeout`.
pub const MANAGED_STORE_START_TIMEOUT: Duration = Duration::from_secs(15);
/// `startup.allium`'s `config.managed_store_stop_timeout`.
pub const MANAGED_STORE_STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// The file, in the managed store's own data directory (the `spacetime`
/// directory beside dispatch's database), that holds the hash of the module
/// last published to the managed store.
pub const MODULE_HASH_FILE: &str = "managed-module.sha256";

/// `startup.allium`'s `ManagedAddressState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedAddressState {
    StoreAnswering,
    NothingListening,
    HeldBySomethingElse,
}

/// `startup.allium`'s `ModulePublishOutcome`. The failures carry the CLI's own
/// error, which is attached to the abort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModulePublishOutcome {
    Published,
    NeedsManualMigration { error: String },
    PublishFailed { error: String },
}

/// Which store this launch uses. `Named` is never managed
/// (`ANamedStoreIsNeverManaged`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreSelection {
    Named(String),
    Managed,
}

/// What the address probe tells the board to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressAction {
    Adopt,
    Start,
    Abort(StartupAbort),
}

/// What the module check tells the board to do once the store is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleAction {
    Connect,
    Publish,
}

/// Where the first connection goes once the managed store is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedStoreReady {
    /// The server URL, e.g. `http://127.0.0.1:3000`.
    pub server: String,
    /// The database name, [`MANAGED_DATABASE_NAME`].
    pub database: String,
}

/// `AbortWhenTheManagedStoreHasNoCli` and the choice between the named and the
/// managed store. The CLI check is consulted only when no store is named --
/// a launch that names one is not asked for the CLI. Blank is none: a shell
/// that exports `DISPATCH_SPACETIME_SERVER=` means "unset".
pub fn select_store(
    explicit: Option<String>,
    cli_on_path: impl FnOnce() -> bool,
) -> Result<StoreSelection, StartupAbort> {
    match explicit
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(named) => Ok(StoreSelection::Named(named)),
        None if cli_on_path() => Ok(StoreSelection::Managed),
        None => Err(StartupAbort::SpacetimeCliMissing),
    }
}

/// The three `ManagedStoreRequested` rules: adopt, start, or abort.
pub fn address_action(state: ManagedAddressState) -> AddressAction {
    match state {
        ManagedAddressState::StoreAnswering => AddressAction::Adopt,
        ManagedAddressState::NothingListening => AddressAction::Start,
        ManagedAddressState::HeldBySomethingElse => {
            AddressAction::Abort(StartupAbort::ManagedStorePortTaken {
                address: MANAGED_STORE_ADDRESS.to_string(),
            })
        }
    }
}

/// `ConnectWhenTheManagedModuleIsCurrent` vs
/// `PublishTheEmbeddedModuleWhenTheStoreDiffers`. A store missing the
/// `dispatch` database is published to whatever hash is recorded.
pub fn module_action(
    recorded: Option<&str>,
    embedded: &str,
    database_exists: bool,
) -> ModuleAction {
    if database_exists && recorded == Some(embedded) {
        ModuleAction::Connect
    } else {
        ModuleAction::Publish
    }
}

/// The three `ModulePublishFinished` rules: `Ok` means connect.
pub fn after_publish(outcome: &ModulePublishOutcome) -> Result<(), StartupAbort> {
    match outcome {
        ModulePublishOutcome::Published => Ok(()),
        ModulePublishOutcome::NeedsManualMigration { error } => {
            Err(StartupAbort::ModuleNeedsManualMigration {
                reason: error.clone(),
            })
        }
        ModulePublishOutcome::PublishFailed { error } => Err(StartupAbort::ModulePublishFailed {
            reason: error.clone(),
        }),
    }
}

/// The hash `embedded_module_hash` / `recorded_module_hash` compare: a sha256
/// of the module `.wasm` bytes, as `sha256:<hex>`.
pub fn module_hash(wasm: &[u8]) -> String {
    let digest = hmac_sha256::Hash::hash(wasm);
    let mut hex = String::with_capacity("sha256:".len() + digest.len() * 2);
    hex.push_str("sha256:");
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Whether a `spacetime` executable is on `PATH`. The managed store is started,
/// published to and (for a store that was not spawned by this board) found
/// through it.
pub fn spacetime_cli_on_path() -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join("spacetime"))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// The module this binary publishes: the committed, prebuilt
/// `src/spacetime/module.wasm`, built by `scripts/build-managed-module.sh`.
pub const EMBEDDED_MODULE: &[u8] = include_bytes!("module.wasm");

/// [`module_hash`] of [`EMBEDDED_MODULE`].
pub fn embedded_module_hash() -> String {
    module_hash(EMBEDDED_MODULE)
}

// ---------------------------------------------------------------------------
// The address probe
// ---------------------------------------------------------------------------

/// What one `GET` to an address came back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpReply {
    /// Nothing accepted the connection.
    NotListening,
    /// Something accepted it and did not answer with an HTTP status line in
    /// time.
    NotHttp,
    Status(u16),
}

/// One `GET path`, written by hand: the question is "does anything answer, and
/// with what status", and a client crate for one request would be the larger
/// change. Reads only as far as the status line, so a server that keeps the
/// connection open is still decided within `timeout`.
fn http_get(address: &str, path: &str, timeout: Duration) -> HttpReply {
    let Some(target) = address.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return HttpReply::NotListening;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&target, timeout) else {
        return HttpReply::NotListening;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return HttpReply::NotHttp;
    }
    let mut seen = Vec::new();
    let mut chunk = [0u8; 256];
    while !seen.windows(2).any(|w| w == b"\r\n") && seen.len() < 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&seen);
    let mut words = text.split_whitespace();
    match (
        words.next(),
        words.next().and_then(|c| c.parse::<u16>().ok()),
    ) {
        (Some(version), Some(code)) if version.starts_with("HTTP/") => HttpReply::Status(code),
        _ => HttpReply::NotHttp,
    }
}

/// What answers on `address` (`host:port`), deciding within `timeout`. A store
/// is something that answers `GET /v1/ping` with HTTP 200.
pub fn probe_address(address: &str, timeout: Duration) -> ManagedAddressState {
    match http_get(address, "/v1/ping", timeout) {
        HttpReply::NotListening => ManagedAddressState::NothingListening,
        HttpReply::Status(200) => ManagedAddressState::StoreAnswering,
        HttpReply::Status(_) | HttpReply::NotHttp => ManagedAddressState::HeldBySomethingElse,
    }
}

/// The CLI half of the `ManagedStoreHost` contract.
pub trait ManagedStorePorts: Send + Sync {
    fn probe_managed_address(&self) -> ManagedAddressState;
    /// Start detached, and return once it serves HTTP or the start timeout
    /// passes. `Err` carries the store's own failure message.
    fn start_managed_store(&self) -> Result<(), String>;
    /// Whether the store holds [`MANAGED_DATABASE_NAME`] at all.
    fn managed_database_exists(&self) -> bool;
    fn publish_embedded_module(&self) -> ModulePublishOutcome;
    /// True when it stopped within the stop timeout.
    fn stop_managed_store(&self) -> bool;
}

/// Where the published module's hash is kept.
pub trait ModuleHashRecord: Send + Sync {
    fn recorded_module_hash(&self) -> Option<String>;
    /// True when the record was kept.
    fn record_module_hash(&self, hash: &str) -> bool;
}

/// [`ModuleHashRecord`] as [`MODULE_HASH_FILE`] in the managed store's data
/// directory.
pub struct FileModuleHashRecord {
    path: PathBuf,
}

impl FileModuleHashRecord {
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(MODULE_HASH_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ModuleHashRecord for FileModuleHashRecord {
    fn recorded_module_hash(&self) -> Option<String> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        let hash = text.trim();
        (!hash.is_empty()).then(|| hash.to_string())
    }

    fn record_module_hash(&self, hash: &str) -> bool {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&self.path, format!("{hash}\n"))
        };
        match write() {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("could not record the managed module hash: {e}");
                false
            }
        }
    }
}

/// `spacetime publish` of the embedded module, through [`ProcessRunner`].
pub struct SpacetimeCliPublisher {
    runner: Arc<dyn ProcessRunner>,
    wasm_path: PathBuf,
    server: String,
    config_path: Option<PathBuf>,
}

impl SpacetimeCliPublisher {
    pub fn new(runner: Arc<dyn ProcessRunner>, wasm_path: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            wasm_path: wasm_path.into(),
            server: format!("http://{MANAGED_STORE_ADDRESS}"),
            config_path: None,
        }
    }

    /// Publish to `server` (a URL) instead of [`MANAGED_STORE_ADDRESS`].
    pub fn with_server(mut self, server: impl Into<String>) -> Self {
        self.server = server.into();
        self
    }

    /// Run the CLI against this config file instead of the operator's own.
    pub fn with_config_path(mut self, config_path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(config_path.into());
        self
    }

    /// Publish `wasm_path` to [`MANAGED_DATABASE_NAME`] on the managed store,
    /// never with an option that clears data (`PublishingNeverClearsData`).
    ///
    /// `--delete-data=never` is the load-bearing flag: without it a schema
    /// change the store cannot automigrate is "resolved" by dropping the
    /// database. It takes an optional value, so it must be attached with `=`
    /// (`--delete-data never` would name a database called `never`). `-y`
    /// skips the confirmation prompts, which have nobody to answer them.
    pub fn publish(&self) -> ModulePublishOutcome {
        let wasm = self.wasm_path.display().to_string();
        let mut argv: Vec<String> = Vec::new();
        if let Some(config) = &self.config_path {
            argv.push(format!("--config-path={}", config.display()));
        }
        argv.extend(
            [
                "publish",
                "-b",
                &wasm,
                "-s",
                &self.server,
                "-y",
                "--delete-data=never",
                MANAGED_DATABASE_NAME,
            ]
            .map(String::from),
        );
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        match self.runner.run("spacetime", &args) {
            Err(e) => ModulePublishOutcome::PublishFailed {
                error: format!("{e:#}"),
            },
            Ok(output) if output.status.success() => ModulePublishOutcome::Published,
            Ok(output) => {
                let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let error = if error.is_empty() {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                } else {
                    error
                };
                if error.to_lowercase().contains("manual migration") {
                    ModulePublishOutcome::NeedsManualMigration { error }
                } else {
                    ModulePublishOutcome::PublishFailed { error }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The real ports
// ---------------------------------------------------------------------------

/// How the store process is started. The seam that lets the start and stop
/// paths run against a stand-in process.
pub trait StoreSpawner: Send + Sync {
    fn spawn(&self, address: &str, data_dir: &Path, log_path: &Path) -> std::io::Result<Child>;
}

/// `spacetime start --listen-addr=<address> --data-dir=<dir> --non-interactive`,
/// detached: its own process group, so a signal aimed at the board's terminal
/// or foreground group (closing the pane, detaching tmux) never reaches it, and
/// stdio pointed at a log file rather than the TUI's terminal.
pub struct SpacetimeStartSpawner {
    config_path: Option<PathBuf>,
}

impl SpacetimeStartSpawner {
    pub fn new() -> Self {
        Self { config_path: None }
    }

    /// Run the CLI against this config file instead of the operator's own.
    pub fn with_config_path(mut self, config_path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(config_path.into());
        self
    }

    /// The command, unspawned, so its argv can be asserted.
    pub fn command(&self, address: &str, data_dir: &Path) -> Command {
        let mut command = Command::new("spacetime");
        if let Some(config) = &self.config_path {
            command.arg(format!("--config-path={}", config.display()));
        }
        command
            .arg("start")
            .arg(format!("--listen-addr={address}"))
            .arg(format!("--data-dir={}", data_dir.display()))
            .arg("--non-interactive");
        command
    }
}

impl Default for SpacetimeStartSpawner {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreSpawner for SpacetimeStartSpawner {
    fn spawn(&self, address: &str, data_dir: &Path, log_path: &Path) -> std::io::Result<Child> {
        use std::os::unix::process::CommandExt;

        std::fs::create_dir_all(data_dir)?;
        if let Some(dir) = log_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        let mut command = self.command(address, data_dir);
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0);
        command.spawn()
    }
}

/// Where the managed store lives on disk.
#[derive(Debug, Clone)]
pub struct ManagedStoreLayout {
    /// `host:port` the store listens on.
    pub address: String,
    /// The store's `--data-dir`, which also holds the module hash record and
    /// the extracted module.
    pub data_dir: PathBuf,
    /// Where the store's stdout and stderr go.
    pub log_path: PathBuf,
}

impl ManagedStoreLayout {
    /// The layout a real launch uses: the fixed [`MANAGED_STORE_ADDRESS`], the
    /// store's data under `store_data_dir`, its log beside `log_dir`'s
    /// `app.log`.
    pub fn for_launch(store_data_dir: PathBuf, log_dir: &Path) -> Self {
        Self {
            address: MANAGED_STORE_ADDRESS.to_string(),
            data_dir: store_data_dir,
            log_path: log_dir.join("managed-store.log"),
        }
    }
}

/// How long one probe or request may take while waiting on the store.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// Gap between polls while waiting for the store to start or stop. Only a
/// store that is not there yet pays it.
const POLL_STEP: Duration = Duration::from_millis(50);
/// The file, in the store's data directory, the extracted module is written to
/// for `spacetime publish -b`.
const EXTRACTED_MODULE_FILE: &str = "dispatch_module.wasm";

/// The real [`ManagedStorePorts`].
pub struct SpacetimeManagedStore {
    runner: Arc<dyn ProcessRunner>,
    spawner: Arc<dyn StoreSpawner>,
    layout: ManagedStoreLayout,
    wasm: &'static [u8],
    config_path: Option<PathBuf>,
    start_timeout: Duration,
    stop_timeout: Duration,
    /// The store this board spawned, if it spawned one.
    child: Mutex<Option<Child>>,
}

impl SpacetimeManagedStore {
    pub fn new(
        runner: Arc<dyn ProcessRunner>,
        spawner: Arc<dyn StoreSpawner>,
        layout: ManagedStoreLayout,
        wasm: &'static [u8],
    ) -> Self {
        Self {
            runner,
            spawner,
            layout,
            wasm,
            config_path: None,
            start_timeout: MANAGED_STORE_START_TIMEOUT,
            stop_timeout: MANAGED_STORE_STOP_TIMEOUT,
            child: Mutex::new(None),
        }
    }

    /// Publish with the CLI against this config file instead of the operator's.
    pub fn with_config_path(mut self, config_path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(config_path.into());
        self
    }

    pub fn with_timeouts(mut self, start: Duration, stop: Duration) -> Self {
        self.start_timeout = start;
        self.stop_timeout = stop;
        self
    }

    fn port(&self) -> &str {
        self.layout
            .address
            .rsplit(':')
            .next()
            .unwrap_or(&self.layout.address)
    }

    /// The pids listening on the store's port, through `ss` and then `lsof`.
    fn listening_pids(&self) -> Vec<u32> {
        let filter = format!("sport = :{}", self.port());
        if let Ok(out) = self.runner.run("ss", &["-ltnpH", &filter]) {
            let pids = parse_ss_pids(&String::from_utf8_lossy(&out.stdout));
            if !pids.is_empty() {
                return pids;
            }
        }
        let target = format!("-iTCP:{}", self.port());
        match self.runner.run("lsof", &["-t", &target, "-sTCP:LISTEN"]) {
            Ok(out) => parse_lsof_pids(&String::from_utf8_lossy(&out.stdout)),
            Err(_) => Vec::new(),
        }
    }

    fn signal(&self, signal: &str, pid: u32) -> bool {
        matches!(
            self.runner.run("kill", &[signal, &pid.to_string()]),
            Ok(out) if out.status.success()
        )
    }

    fn alive(&self, pid: u32) -> bool {
        self.signal("-0", pid)
    }
}

/// The pids in `ss -ltnp` output: every `pid=<n>` of the `users:(...)` column.
pub fn parse_ss_pids(output: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for part in output.split("pid=").skip(1) {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(pid) = digits.parse::<u32>() {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

/// The pids in `lsof -t` output: one per line.
pub fn parse_lsof_pids(output: &str) -> Vec<u32> {
    let mut pids: Vec<u32> = Vec::new();
    for pid in output.lines().filter_map(|l| l.trim().parse::<u32>().ok()) {
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
}

impl ManagedStorePorts for SpacetimeManagedStore {
    fn probe_managed_address(&self) -> ManagedAddressState {
        probe_address(&self.layout.address, PROBE_TIMEOUT)
    }

    fn start_managed_store(&self) -> Result<(), String> {
        let child = self
            .spawner
            .spawn(
                &self.layout.address,
                &self.layout.data_dir,
                &self.layout.log_path,
            )
            .map_err(|e| format!("could not run `spacetime start`: {e}"))?;
        // Tracked from the moment it exists, so a store that then misses the
        // deadline is still stopped.
        *lock(&self.child) = Some(child);

        let deadline = Instant::now() + self.start_timeout;
        loop {
            if probe_address(&self.layout.address, PROBE_TIMEOUT)
                == ManagedAddressState::StoreAnswering
            {
                return Ok(());
            }
            if let Some(child) = lock(&self.child).as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!(
                        "`spacetime start` exited with {status} (see {})",
                        self.layout.log_path.display()
                    ));
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "did not serve HTTP within {}s (see {})",
                    self.start_timeout.as_secs(),
                    self.layout.log_path.display()
                ));
            }
            std::thread::sleep(POLL_STEP);
        }
    }

    fn managed_database_exists(&self) -> bool {
        let path = format!("/v1/database/{MANAGED_DATABASE_NAME}");
        http_get(&self.layout.address, &path, PROBE_TIMEOUT) == HttpReply::Status(200)
    }

    fn publish_embedded_module(&self) -> ModulePublishOutcome {
        let wasm_path = self.layout.data_dir.join(EXTRACTED_MODULE_FILE);
        let written = std::fs::create_dir_all(&self.layout.data_dir)
            .and_then(|()| std::fs::write(&wasm_path, self.wasm));
        if let Err(e) = written {
            return ModulePublishOutcome::PublishFailed {
                error: format!("could not write the module to {}: {e}", wasm_path.display()),
            };
        }
        let mut publisher = SpacetimeCliPublisher::new(self.runner.clone(), wasm_path)
            .with_server(format!("http://{}", self.layout.address));
        if let Some(config) = &self.config_path {
            publisher = publisher.with_config_path(config);
        }
        publisher.publish()
    }

    /// SIGTERM to the store this board spawned and to whatever else listens on
    /// its port (an adopted store, or the real server behind a wrapper), then
    /// wait -- bounded by the stop timeout -- for all of them to be gone. The
    /// CLI has no stop command.
    fn stop_managed_store(&self) -> bool {
        let mut tracked = lock(&self.child).take();
        let mut pids = self.listening_pids();
        if let Some(child) = &tracked {
            if !pids.contains(&child.id()) {
                pids.push(child.id());
            }
        }
        for pid in &pids {
            self.signal("-TERM", *pid);
        }

        let deadline = Instant::now() + self.stop_timeout;
        loop {
            // Our own child is reaped rather than probed: a zombie still
            // answers `kill -0`.
            let child_gone = match tracked.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
                None => true,
            };
            let own_pid = tracked.as_ref().map(Child::id);
            let others_gone = pids
                .iter()
                .filter(|pid| Some(**pid) != own_pid)
                .all(|pid| !self.alive(*pid));
            if child_gone && others_gone {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL_STEP);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// The orchestrator
// ---------------------------------------------------------------------------

/// The orchestrator: `BringUpTheManagedStoreOnceTheHostIsNamed` through
/// `StopTheManagedStoreWhenStartupAborts`.
pub struct ManagedStore {
    ports: Arc<dyn ManagedStorePorts>,
    hashes: Arc<dyn ModuleHashRecord>,
    embedded_hash: String,
    held: AtomicBool,
    /// Set by the first stop, so an abort inside `bring_up` followed by the
    /// exit path (or a signal racing a quit) stops the store once.
    stopped: AtomicBool,
}

impl ManagedStore {
    pub fn new(
        ports: Arc<dyn ManagedStorePorts>,
        hashes: Arc<dyn ModuleHashRecord>,
        embedded_hash: impl Into<String>,
    ) -> Self {
        Self {
            ports,
            hashes,
            embedded_hash: embedded_hash.into(),
            held: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }

    /// The store a real launch runs: `spacetime start` in `store_data_dir`, its
    /// log beside `log_dir`'s `app.log`, the module hash recorded in the same
    /// data directory, and [`EMBEDDED_MODULE`] as the module.
    pub fn for_launch(store_data_dir: PathBuf, log_dir: &Path) -> Self {
        let layout = ManagedStoreLayout::for_launch(store_data_dir, log_dir);
        let hashes = Arc::new(FileModuleHashRecord::in_data_dir(&layout.data_dir));
        let ports = SpacetimeManagedStore::new(
            Arc::new(crate::process::RealProcessRunner::default()),
            Arc::new(SpacetimeStartSpawner::new()),
            layout,
            EMBEDDED_MODULE,
        );
        Self::new(Arc::new(ports), hashes, embedded_module_hash())
    }

    fn stop_once(&self) -> bool {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return true;
        }
        let in_time = self.ports.stop_managed_store();
        if !in_time {
            tracing::warn!("the managed store did not stop within its timeout");
        }
        in_time
    }

    /// Probe, adopt or start, check the module, publish if needed. Every abort
    /// it returns has already stopped a store this board holds.
    pub fn bring_up(&self) -> Result<ManagedStoreReady, StartupAbort> {
        match address_action(self.ports.probe_managed_address()) {
            AddressAction::Abort(abort) => return Err(abort),
            AddressAction::Adopt => self.held.store(true, Ordering::SeqCst),
            AddressAction::Start => {
                // Held from the moment the start is attempted: a store that
                // launched and then missed the deadline must not come up
                // unowned a moment after the abort.
                self.held.store(true, Ordering::SeqCst);
                if let Err(reason) = self.ports.start_managed_store() {
                    let stopped = self.stop_once();
                    return Err(StartupAbort::ManagedStoreDidNotStart { reason, stopped });
                }
            }
        }

        let recorded = self.hashes.recorded_module_hash();
        // The database is only worth asking about when the hash would
        // otherwise say "current": a differing hash publishes regardless.
        let database_exists = recorded.as_deref() != Some(self.embedded_hash.as_str())
            || self.ports.managed_database_exists();
        if module_action(recorded.as_deref(), &self.embedded_hash, database_exists)
            == ModuleAction::Publish
        {
            let outcome = self.ports.publish_embedded_module();
            if let Err(abort) = after_publish(&outcome) {
                self.stop_once();
                return Err(abort);
            }
            // A hash names only a module the store runs, so it is recorded
            // after the publish was accepted. A record that cannot be kept
            // costs the next launch one redundant publish.
            if !self.hashes.record_module_hash(&self.embedded_hash) {
                tracing::warn!("the managed module hash could not be recorded");
            }
        }
        Ok(ManagedStoreReady {
            server: format!("http://{MANAGED_STORE_ADDRESS}"),
            database: MANAGED_DATABASE_NAME.to_string(),
        })
    }

    /// `managed_store_held_by_this_board`.
    pub fn held_by_this_board(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }

    /// `StopTheManagedStoreWhenTheBoardExits`. Stops only a held store;
    /// returns whether a stop was attempted and completed in time (`true`
    /// when nothing was held).
    pub fn stop_on_exit(&self) -> bool {
        if !self.held_by_this_board() {
            return true;
        }
        self.stop_once()
    }

    /// `StopTheManagedStoreWhenStartupAborts`, for an abort discovered after
    /// `bring_up` (the first connection failing, say). Stops a held store and
    /// hands the reason back.
    pub fn abort_startup(&self, reason: StartupAbort) -> StartupAbort {
        self.stop_on_exit();
        reason
    }
}
