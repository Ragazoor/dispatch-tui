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

    let log = scratch.path().join("store.log");
    let (address, child) = spawn_on_free_port(spawner, &copy, &log)?;
    let group = child.id();

    let result = async {
        wait_until_answering(&address, start_timeout).await?;
        store_at(&format!("http://{address}")).dump().await
    }
    .await;

    // The CLI starts the real server as its child, so stop the whole group.
    crate::process::kill_process_group(group, "TERM");
    result
}

/// Start a store over `copy` on a free local port, returning its address and
/// process.
///
/// The port is free when picked, not when the store starts: another process
/// can take it in between. The start only spawns the process, and the child
/// binds later, so a lost race is not seen here; it shows up as the copy never
/// answering, which `wait_until_answering` reports after its timeout.
fn spawn_on_free_port(
    spawner: &dyn StoreSpawner,
    copy: &Path,
    log: &Path,
) -> Result<(String, std::process::Child)> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let address = format!("127.0.0.1:{port}");
    let child = spawner
        .spawn(&address, copy, log)
        .context("could not run `spacetime start`")?;
    Ok((address, child))
}

/// Poll `address` until a store answers, or fail after `start_timeout`.
async fn wait_until_answering(address: &str, start_timeout: Duration) -> Result<()> {
    let a = address.to_owned();
    tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + start_timeout;
        while probe_address(&a, Duration::from_millis(500)) != ManagedAddressState::StoreAnswering {
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
    .await?
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

    // It names its own target, which may be a store this install never used.
    let connected = crate::store_connection::open_cli_store_with_pin(
        data_dir,
        Some(server.to_string()),
        crate::store_connection::StorePin::Unchecked,
    )
    .await?;
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
mod tests;
