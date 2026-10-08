//! `dispatch store import --from <old store>` — bring an old store's rows into
//! the store a board runs.
//!
//! Spec: `docs/specs/spacetime-seed.allium` (`ImportOldStore`) and
//! `docs/specs/startup.allium` (`ImportBringsUpTheManagedStore`). The row
//! logic is `spacetime::import_old_store`; this is how the source is read, how
//! the target is reached, and what is printed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::spacetime::managed_store::{
    probe_address, ManagedAddressState, ManagedStore, SpacetimeStartSpawner, StoreSpawner,
};
use crate::spacetime::{Snapshot, SnapshotTarget, SpacetimeCliStore};

/// Where the old rows are read from.
#[derive(Debug, PartialEq, Eq)]
pub enum ImportSource {
    /// A running store, read through its server.
    Server(String),
    /// A data folder, copied and served from the copy.
    DataDir(PathBuf),
}

/// `http(s)://…` is a server; anything else is a data folder.
pub fn parse_source(from: &str) -> ImportSource {
    if from.starts_with("http://") || from.starts_with("https://") {
        ImportSource::Server(from.trim_end_matches('/').to_string())
    } else {
        ImportSource::DataDir(PathBuf::from(from))
    }
}

/// Whether any of these process command lines is a store serving `data_dir`.
/// Copying a folder a store is writing can capture a half-written commit log.
pub fn data_dir_in_use(data_dir: &Path, command_lines: &[String]) -> bool {
    let wanted = data_dir.to_string_lossy();
    let wanted = wanted.trim_end_matches('/');
    command_lines.iter().any(|line| {
        line.contains("start")
            && line.split_whitespace().enumerate().any(|(i, word)| {
                let mut words = line.split_whitespace().skip(i + 1);
                let value = word
                    .strip_prefix("--data-dir=")
                    .or_else(|| (word == "--data-dir").then(|| words.next()).flatten());
                value.is_some_and(|v| v.trim_end_matches('/') == wanted)
            })
    })
}

fn running_command_lines() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
        .map(|raw| {
            String::from_utf8_lossy(&raw)
                .split('\0')
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn store_for(server: &str) -> SpacetimeCliStore {
    SpacetimeCliStore::new(
        Arc::new(crate::process::RealProcessRunner::default()),
        crate::sync::SHARED_DATABASE_NAME.to_string(),
        Some(server.to_string()),
    )
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// A store over a COPY of `data_dir`, read and stopped. The folder named is
/// only ever read by `cp`.
async fn read_data_dir(data_dir: &Path) -> Result<Snapshot> {
    read_data_dir_with(
        data_dir,
        &SpacetimeStartSpawner::new(),
        START_TIMEOUT,
        &|server| Arc::new(store_for(server)),
    )
    .await
}

/// How many free ports the import tries before it gives up.
const PORT_ATTEMPTS: u32 = 5;

/// How long the copy gets to answer before the import gives up.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// `read_data_dir` with the process start, the wait and the store reached
/// through seams, so the path runs without a `spacetime` binary.
async fn read_data_dir_with(
    data_dir: &Path,
    spawner: &dyn StoreSpawner,
    start_timeout: Duration,
    store_at: &dyn Fn(&str) -> Arc<dyn SnapshotTarget>,
) -> Result<Snapshot> {
    if !data_dir.is_dir() {
        bail!("{} is not a directory", data_dir.display());
    }
    let absolute = std::fs::canonicalize(data_dir)?;
    let lines = tokio::task::spawn_blocking(running_command_lines).await?;
    if data_dir_in_use(&absolute, &lines) {
        bail!(
            "a store is running over {}. Copying a folder a store is writing can capture it \
             half-written; name that store's address instead, e.g. `--from http://127.0.0.1:3001`.",
            absolute.display()
        );
    }
    let scratch = tempfile::tempdir().context("could not make a scratch directory")?;
    let copy = scratch.path().join("data");
    let (from, to) = (absolute.clone(), copy.clone());
    tokio::task::spawn_blocking(move || copy_dir(&from, &to))
        .await?
        .context("could not copy the data folder")?;

    // The port is free when picked, not when the store starts: another process
    // can take it in between. Pick again when the start reports that.
    let log = scratch.path().join("store.log");
    let mut attempt = 1;
    let (address, child) = loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let address = format!("127.0.0.1:{port}");
        match spawner.spawn(&address, &copy, &log) {
            Ok(child) => break (address, child),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempt < PORT_ATTEMPTS => {
                attempt += 1;
            }
            Err(e) => return Err(e).context("could not run `spacetime start`"),
        }
    };
    let group = child.id();

    let result = async {
        let a = address.clone();
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + start_timeout;
            while probe_address(&a, Duration::from_millis(500))
                != ManagedAddressState::StoreAnswering
            {
                if Instant::now() > deadline {
                    bail!(
                        "the copy of the store did not start within {} seconds",
                        start_timeout.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(())
        })
        .await??;
        store_at(&format!("http://{address}")).dump().await
    }
    .await;

    // The CLI starts the real server as its child, so stop the whole group.
    let _ = std::process::Command::new("kill")
        .args(["-TERM", "--", &format!("-{group}")])
        .status();
    result
}

/// Read the old store into a snapshot. Read-only on the source.
pub async fn read_source(source: &ImportSource) -> Result<Snapshot> {
    match source {
        ImportSource::Server(server) => store_for(server)
            .dump()
            .await
            .with_context(|| format!("could not read the store at {server}")),
        ImportSource::DataDir(dir) => read_data_dir(dir).await,
    }
}

/// `dispatch store import`. `named_target` is `--spacetime-server`; without one
/// the managed store is brought up for the import and stopped after it.
pub async fn import_store(
    data_dir: &Path,
    named_target: Option<String>,
    from: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let source = parse_source(from);

    let (server, managed) = match named_target
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(server) => (server, None),
        None => {
            if !crate::spacetime::managed_store::spacetime_cli_on_path() {
                bail!("the `spacetime` CLI is not on PATH; install it first");
            }
            let store_data_dir = crate::default_data_dir().join("spacetime");
            let log_dir = data_dir.to_path_buf();
            let managed = Arc::new(ManagedStore::for_launch(store_data_dir, &log_dir));
            let m = managed.clone();
            let ready = tokio::task::spawn_blocking(move || m.bring_up())
                .await?
                .map_err(|abort| anyhow::anyhow!("{}", abort.message()))?;
            (ready.server, Some(managed))
        }
    };

    let outcome = run(data_dir, &server, &source, out).await;

    if let Some(managed) = managed {
        let _ = tokio::task::spawn_blocking(move || managed.stop_on_exit()).await;
    }
    outcome
}

async fn run(
    data_dir: &Path,
    server: &str,
    source: &ImportSource,
    out: &mut dyn Write,
) -> Result<()> {
    if let ImportSource::Server(from) = source {
        if from.trim_end_matches('/') == server.trim_end_matches('/') {
            bail!("the source and the target are the same store ({server})");
        }
    }
    let snapshot = read_source(source).await?;

    let connected =
        crate::store_connection::open_cli_store(data_dir, Some(server.to_string())).await?;
    let operator = crate::store::HostStore::user_identity(&*connected.database)
        .await?
        .context("connected to the store, but no user identity was stored")?;

    let target = store_for(server);
    let report = crate::spacetime::import_old_store(&target, &snapshot, &operator)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    writeln!(
        out,
        "Imported {} rows into {server}; {} were already there, {} dropped.\n{}",
        report.imported_total(),
        report.kept_total(),
        report.dropped_total(),
        report.summary()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacetime::MemoryStore;
    use std::io::Read;
    use std::net::TcpListener;
    use std::process::{Child, Command, Stdio};
    use std::sync::Mutex;

    /// Starts `sleep` in its own group instead of `spacetime start`, and
    /// records the address and data folder it was given.
    #[derive(Default)]
    struct FakeSpawner {
        seen: Mutex<Option<(String, PathBuf)>>,
        pid: Mutex<Option<ProcessGroup>>,
        /// The fake store's stdout. It reaches end-of-file only once every
        /// process in the group has exited, which is how a test waits for the
        /// stop without sleeping.
        output: Mutex<Option<std::process::ChildStdout>>,
        fail: bool,
        answer_on_start: bool,
        /// How many starts fail with "address in use" before one succeeds: a
        /// parallel process took the port between the pick and the start.
        taken_ports: Mutex<u32>,
    }

    /// A process group stopped when dropped: a backstop for a test that fails
    /// before the code under test stops it.
    struct ProcessGroup(u32);

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", self.0)])
                .status();
        }
    }

    impl StoreSpawner for FakeSpawner {
        fn spawn(&self, address: &str, data_dir: &Path, _log: &Path) -> std::io::Result<Child> {
            use std::os::unix::process::CommandExt;
            if self.fail {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no spacetime",
                ));
            }
            {
                let mut taken = self.taken_ports.lock().unwrap();
                if *taken > 0 {
                    *taken -= 1;
                    return Err(std::io::Error::from(std::io::ErrorKind::AddrInUse));
                }
            }
            *self.seen.lock().unwrap() = Some((address.to_string(), data_dir.to_path_buf()));
            if self.answer_on_start {
                let listener = TcpListener::bind(address)?;
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(mut stream) = stream else { continue };
                        let mut buf = [0u8; 512];
                        let _ = stream.read(&mut buf);
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                    }
                });
            }
            // Prints only if nothing stops it, so a test that reads its
            // output to the end learns both that it exited and how.
            let mut child = Command::new("sh")
                .args(["-c", "sleep 600; echo survived"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()?;
            *self.pid.lock().unwrap() = Some(ProcessGroup(child.id()));
            *self.output.lock().unwrap() = child.stdout.take();
            Ok(child)
        }
    }

    fn data_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.txt"), "x").unwrap();
        dir
    }

    fn memory_store(_: &str) -> Arc<dyn SnapshotTarget> {
        Arc::new(MemoryStore::new())
    }

    #[tokio::test]
    async fn a_missing_folder_is_refused_before_anything_starts() {
        let spawner = FakeSpawner::default();
        let err = read_data_dir_with(
            Path::new("/nonexistent/dispatch-store"),
            &spawner,
            Duration::from_secs(1),
            &memory_store,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("is not a directory"));
        assert!(spawner.seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_failed_start_names_the_command() {
        let dir = data_folder();
        let spawner = FakeSpawner {
            fail: true,
            ..Default::default()
        };
        let err = read_data_dir_with(dir.path(), &spawner, Duration::from_secs(1), &memory_store)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("could not run `spacetime start`"));
    }

    #[tokio::test]
    async fn a_port_taken_between_the_pick_and_the_start_is_picked_again() {
        let dir = data_folder();
        let spawner = FakeSpawner {
            answer_on_start: true,
            taken_ports: Mutex::new(2),
            ..Default::default()
        };
        let snapshot =
            read_data_dir_with(dir.path(), &spawner, Duration::from_secs(10), &memory_store)
                .await
                .unwrap();
        assert!(!snapshot.extracts().is_empty());
        assert_eq!(*spawner.taken_ports.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_port_that_stays_taken_gives_up_and_names_the_command() {
        let dir = data_folder();
        let spawner = FakeSpawner {
            taken_ports: Mutex::new(u32::MAX),
            ..Default::default()
        };
        let err = read_data_dir_with(dir.path(), &spawner, Duration::from_secs(1), &memory_store)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("could not run `spacetime start`"));
    }

    #[tokio::test]
    async fn a_copy_that_never_answers_times_out_and_is_stopped() {
        let dir = data_folder();
        let spawner = FakeSpawner::default();
        let err = read_data_dir_with(dir.path(), &spawner, Duration::ZERO, &memory_store)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not start within"));
        // End-of-file arrives when the whole group has exited: at once if the
        // code under test stopped it, and with "survived" only if it did not.
        let mut output = spawner.output.lock().unwrap().take().unwrap();
        let printed = tokio::task::spawn_blocking(move || {
            let mut text = String::new();
            output.read_to_string(&mut text).map(|_| text)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(printed, "", "the fake store process was left running");
    }

    #[tokio::test]
    async fn an_answering_copy_is_dumped_from_a_copy_not_the_original() {
        let dir = data_folder();
        let spawner = FakeSpawner {
            answer_on_start: true,
            ..Default::default()
        };
        let snapshot =
            read_data_dir_with(dir.path(), &spawner, Duration::from_secs(10), &memory_store)
                .await
                .unwrap();
        assert!(!snapshot.extracts().is_empty());
        let (_, served) = spawner.seen.lock().unwrap().clone().unwrap();
        assert_ne!(served, dir.path());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sub/a.txt")).unwrap(),
            "x"
        );
    }

    #[tokio::test]
    async fn a_server_source_naming_the_target_is_refused() {
        let mut out = Vec::new();
        let err = import_store(
            Path::new("/nonexistent/x.db"),
            Some("http://127.0.0.1:3001/".into()),
            "http://127.0.0.1:3001",
            &mut out,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("same store"));
        assert!(out.is_empty());
    }

    #[test]
    fn an_http_address_is_a_server_and_anything_else_a_folder() {
        assert_eq!(
            parse_source("http://127.0.0.1:3001/"),
            ImportSource::Server("http://127.0.0.1:3001".into())
        );
        assert_eq!(
            parse_source("/home/a/data"),
            ImportSource::DataDir(PathBuf::from("/home/a/data"))
        );
    }

    #[test]
    fn a_running_store_over_the_folder_is_detected_in_either_flag_spelling() {
        let lines = vec![
            "spacetimedb-standalone start --data-dir /home/a/data --listen-addr 127.0.0.1:3001"
                .to_string(),
            "spacetime start --data-dir=/srv/other".to_string(),
        ];
        assert!(data_dir_in_use(Path::new("/home/a/data"), &lines));
        assert!(data_dir_in_use(Path::new("/home/a/data/"), &lines));
        assert!(data_dir_in_use(Path::new("/srv/other"), &lines));
        assert!(!data_dir_in_use(Path::new("/home/a/data2"), &lines));
        assert!(!data_dir_in_use(Path::new("/home/a"), &lines));
    }
}
