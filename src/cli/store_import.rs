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
use crate::spacetime::{SharedStore, Snapshot, SpacetimeCliStore};

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

    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let address = format!("127.0.0.1:{port}");
    let child = SpacetimeStartSpawner::new()
        .spawn(&address, &copy, &scratch.path().join("store.log"))
        .context("could not run `spacetime start`")?;
    let group = child.id();

    let result = async {
        let a = address.clone();
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while probe_address(&a, Duration::from_millis(500))
                != ManagedAddressState::StoreAnswering
            {
                if Instant::now() > deadline {
                    bail!("the copy of the store did not start within 30 seconds");
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(())
        })
        .await??;
        store_for(&format!("http://{address}")).dump().await
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
    db_path: &Path,
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
            let store_data_dir = crate::default_db_path()
                .parent()
                .unwrap_or(Path::new("."))
                .join("spacetime");
            let log_dir = db_path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let managed = Arc::new(ManagedStore::for_launch(store_data_dir, &log_dir));
            let m = managed.clone();
            let ready = tokio::task::spawn_blocking(move || m.bring_up())
                .await?
                .map_err(|abort| anyhow::anyhow!("{}", abort.message()))?;
            (ready.server, Some(managed))
        }
    };

    let outcome = run(db_path, &server, &source, out).await;

    if let Some(managed) = managed {
        let _ = tokio::task::spawn_blocking(move || managed.stop_on_exit()).await;
    }
    outcome
}

async fn run(
    db_path: &Path,
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

    let connected = crate::runtime::open_cli_store(db_path, Some(server.to_string())).await?;
    let operator = crate::db::HostStore::user_identity(&*connected.database)
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
