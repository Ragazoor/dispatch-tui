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
///
/// Both sides are compared canonically, so a symlink or a `./` does not hide
/// a store. A `spacetime start` with no `--data-dir` serves the CLI's default
/// folder, `default_store_dir`.
pub fn data_dir_in_use(
    data_dir: &Path,
    command_lines: &[String],
    default_store_dir: &Path,
) -> bool {
    let wanted = canonical_or_raw(&data_dir.to_string_lossy());
    command_lines.iter().any(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        let is_store = words.contains(&"start")
            && words
                .first()
                .and_then(|w| w.rsplit('/').next())
                .is_some_and(|program| program.contains("spacetime"));
        if !is_store {
            return false;
        }
        let mut named = false;
        for (i, word) in words.iter().enumerate() {
            let value = word.strip_prefix("--data-dir=").or_else(|| {
                (*word == "--data-dir")
                    .then(|| words.get(i + 1).copied())
                    .flatten()
            });
            if let Some(value) = value {
                named = true;
                if canonical_or_raw(value) == wanted {
                    return true;
                }
            }
        }
        !named && canonical_or_raw(&default_store_dir.to_string_lossy()) == wanted
    })
}

/// The canonical spelling of `path` when it exists, else the path without a
/// trailing slash.
fn canonical_or_raw(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.trim_end_matches('/').to_string())
}

/// Where a bare `spacetime start` keeps its data.
fn default_spacetime_data_dir() -> Result<PathBuf> {
    let data_dir = crate::default_data_dir()?;
    Ok(data_dir
        .parent()
        .map(|base| base.join("spacetime").join("data"))
        .unwrap_or_else(|| PathBuf::from("spacetime/data")))
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
async fn read_data_dir(data_dir: &Path, scratch_root: &Path) -> Result<Snapshot> {
    read_data_dir_with(
        data_dir,
        scratch_root,
        &SpacetimeStartSpawner::new(),
        START_TIMEOUT,
        &|server| Arc::new(store_for(server)),
    )
    .await
}

/// How long the copy gets to answer before the import gives up.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a stopping store gets to exit on SIGTERM before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// Stop the store `child` started, and every process in its group: SIGTERM,
/// then SIGKILL once `grace` has passed, and reap it either way. Blocks.
fn stop_store(mut child: std::process::Child, grace: Duration) {
    let group = child.id();
    // The CLI starts the real server as its child, so stop the whole group.
    crate::process::kill_process_group(group, "TERM");
    let deadline = Instant::now() + grace;
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    // The leader may be gone while the server behind it is not.
    crate::process::kill_process_group(group, "KILL");
    let _ = child.kill();
    let _ = child.wait();
}

/// `read_data_dir` with the process start, the wait and the store reached
/// through seams, so the path runs without a `spacetime` binary. The copy goes
/// under `scratch_root`, which should be on disk: a full store copy is large.
async fn read_data_dir_with(
    data_dir: &Path,
    scratch_root: &Path,
    spawner: &dyn StoreSpawner,
    start_timeout: Duration,
    store_at: &dyn Fn(&str) -> Arc<dyn SnapshotTarget>,
) -> Result<Snapshot> {
    if !data_dir.is_dir() {
        bail!("{} is not a directory", data_dir.display());
    }
    let absolute = std::fs::canonicalize(data_dir)?;
    let lines = tokio::task::spawn_blocking(running_command_lines).await?;
    if data_dir_in_use(&absolute, &lines, &default_spacetime_data_dir()?) {
        bail!(
            "a store is running over {}. Copying a folder a store is writing can capture it \
             half-written; name that store's address instead, e.g. `--from http://127.0.0.1:3001`.",
            absolute.display()
        );
    }
    std::fs::create_dir_all(scratch_root)
        .with_context(|| format!("could not make {}", scratch_root.display()))?;
    let scratch = tempfile::Builder::new()
        .prefix("import-scratch-")
        .tempdir_in(scratch_root)
        .context("could not make a scratch directory")?;
    let copy = scratch.path().join("data");
    let (from, to) = (absolute.clone(), copy.clone());
    tokio::task::spawn_blocking(move || copy_dir(&from, &to))
        .await?
        .context("could not copy the data folder")?;

    let log = scratch.path().join("store.log");
    let result = dump_from_own_copy(spawner, &copy, &log, start_timeout, store_at).await;

    if let Err(e) = result {
        // The store's log is the only account of why it failed; keep it
        // alone, not the whole copy.
        let kept = scratch_root.join("store.log");
        return Err(match std::fs::copy(&log, &kept) {
            Ok(_) => e.context(format!("the store's log is {}", kept.display())),
            Err(_) => e,
        });
    }
    result
}

/// How many ports the import tries when another store keeps taking the one it
/// picked.
const PORT_ATTEMPTS: usize = 3;

/// Start a store over `copy`, wait for it, and dump it. The port is picked by
/// bind-and-drop, so another store can win it; an answer is only trusted from
/// the process this start made, and any other listener is stopped over and the
/// start repeated on a fresh port.
async fn dump_from_own_copy(
    spawner: &dyn StoreSpawner,
    copy: &Path,
    log: &Path,
    start_timeout: Duration,
    store_at: &dyn Fn(&str) -> Arc<dyn SnapshotTarget>,
) -> Result<Snapshot> {
    for attempt in 1..=PORT_ATTEMPTS {
        let (address, child) = match spawn_on_free_port(spawner, copy, log) {
            Ok(started) => started,
            // The same lost race, seen at the start: start over on a new port.
            Err(e) if attempt < PORT_ATTEMPTS && lost_the_port(&e) => continue,
            Err(e) => return Err(e),
        };
        let answered = wait_until_answering(&address, start_timeout).await;
        let foreign = answered.is_ok() && !spawner.listener_is_ours(&address, copy);
        let result = match answered {
            Err(e) => Err(e),
            Ok(()) if !foreign => store_at(&format!("http://{address}")).dump().await,
            Ok(()) => Err(anyhow::anyhow!(
                "the store answering on {address} is not the copy this import started \
                 (another store took the port)"
            )),
        };
        // Stopped and reaped before the next start or the scratch folder goes.
        let _ = tokio::task::spawn_blocking(move || stop_store(child, STOP_GRACE)).await;
        if foreign && attempt < PORT_ATTEMPTS {
            continue;
        }
        return result;
    }
    unreachable!("the loop returns on its last attempt")
}

/// Whether a failed start was "address already in use".
fn lost_the_port(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|e| e.downcast_ref::<std::io::Error>())
        .any(|e| e.kind() == std::io::ErrorKind::AddrInUse)
}

/// Start a store over `copy` on a free local port, returning its address and
/// process.
///
/// The port is free when picked, not when the store starts: another process
/// can take it in between. The start only spawns the process, and the child
/// binds later, so a lost race is not seen here; [`dump_from_own_copy`] checks
/// that the listener is the process it started.
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
pub async fn read_source(source: &ImportSource, scratch_root: &Path) -> Result<Snapshot> {
    match source {
        ImportSource::Server(server) => store_for(server)
            .dump()
            .await
            .with_context(|| format!("could not read the store at {server}")),
        ImportSource::DataDir(dir) => read_data_dir(dir, scratch_root).await,
    }
}

/// `dispatch store import`. `named_target` is `--spacetime-server`; without one
/// the managed store is brought up for the import and stopped after it,
/// unless it was already running: that one is left alone.
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
            let store_data_dir = crate::default_data_dir()?.join("spacetime");
            let log_dir = data_dir.to_path_buf();
            let managed = Arc::new(ManagedStore::for_launch(store_data_dir, &log_dir));
            let m = managed.clone();
            let ready = tokio::task::spawn_blocking(move || m.bring_up_for_import())
                .await?
                .map_err(|abort| anyhow::anyhow!("{}", abort.message()))?;
            (ready.server, Some(managed))
        }
    };

    let outcome = run(data_dir, &server, &source, out).await;

    if let Some(managed) = managed {
        let _ = tokio::task::spawn_blocking(move || managed.stop_spawned_on_exit()).await;
    }
    outcome
}

/// The module's refusal of a board that is not the store's publisher, with the
/// way out: a store published before the module recorded its publisher has none
/// until `claim_store_owner` runs as the CLI login that owns the database.
fn with_publisher_hint(message: &str) -> String {
    if message.contains("only the store's publisher may restore") {
        format!(
            "{message}\nThis store has no recorded publisher, or it is another login. \
             Run `spacetime call <database> claim_store_owner` as the login that \
             published the database, then import again."
        )
    } else {
        message.to_string()
    }
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
    // On disk, beside the data directory, not in `/tmp`, which is RAM on some hosts.
    let snapshot = read_source(source, &data_dir.join("import-scratch")).await?;

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
        .map_err(|e| anyhow::anyhow!("{}", with_publisher_hint(&e.to_string())))?;
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
