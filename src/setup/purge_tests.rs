#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task #16755: `dispatch uninstall --purge` forgets this machine's identity
//! and leaves a leftover `tasks.db` alone.
//!
//! Spec: `mcp-task-tools.allium` ("Other CLI subcommands", `uninstall`),
//! `storage.allium` (`CodeNeverTouchesLegacyDatabase`), `cli.allium`
//! (`NoSubcommandOpensSqlite`).
//!
//! The confirmer answers yes (or no) to whichever prompt the purge asks, plain
//! or dangerous, so these tests pin what is removed rather than which kind of
//! prompt guards it.

use std::fs;
use std::path::{Path, PathBuf};

use super::{run_uninstall_in, FakeConfirmer, UninstallPaths};

fn layout(root: &Path) -> (UninstallPaths, PathBuf) {
    let data_dir = root.join("dispatch");
    fs::create_dir_all(&data_dir).unwrap();
    let paths = UninstallPaths {
        mcp_path: root.join(".claude.json"),
        legacy_mcp_path: root.join(".claude").join(".mcp.json"),
        plugin_path: root.join("plugin"),
        db_path: data_dir.join("tasks.db"),
        statusline_path: root.join("statusline.json"),
    };
    (paths, data_dir)
}

fn yes_to_everything() -> FakeConfirmer {
    FakeConfirmer::new(vec![true, true, true], vec![true, true])
}

/// A real, populated, cleanly closed SQLite file plus sentinel `-wal`/`-shm`
/// companions. Returns each file and its bytes.
async fn leftover_database(data_dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let db_path = data_dir.join("tasks.db");
    let db = crate::db::Database::open(&db_path).await.unwrap();
    db.db_call(|conn| {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();
    drop(db);
    fs::write(data_dir.join("tasks.db-wal"), b"wal sentinel").unwrap();
    fs::write(data_dir.join("tasks.db-shm"), b"shm sentinel").unwrap();
    ["tasks.db", "tasks.db-wal", "tasks.db-shm"]
        .into_iter()
        .map(|name| {
            let path = data_dir.join(name);
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[tokio::test]
async fn purge_removes_the_host_file_and_log_and_leaves_the_legacy_database() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, data_dir) = layout(dir.path());
    fs::write(data_dir.join("host.json"), br#"{"host_id":"h"}"#).unwrap();
    fs::write(data_dir.join("app.log"), b"log").unwrap();
    let legacy = leftover_database(&data_dir).await;

    run_uninstall_in(&paths, &yes_to_everything(), false, true).unwrap();

    assert!(
        !data_dir.join("host.json").exists(),
        "purge forgets this machine's identity"
    );
    assert!(!data_dir.join("app.log").exists(), "purge removes the log");
    for (path, bytes) in legacy {
        assert!(
            path.exists(),
            "purge must never delete {} (CodeNeverTouchesLegacyDatabase)",
            path.display()
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "{} must be left byte-for-byte as it was",
            path.display()
        );
    }
    assert!(data_dir.exists(), "a directory still holding files is kept");
}

#[test]
fn purge_removes_the_data_directory_once_it_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, data_dir) = layout(dir.path());
    fs::write(data_dir.join("host.json"), br#"{"host_id":"h"}"#).unwrap();
    fs::write(data_dir.join("app.log"), b"log").unwrap();

    run_uninstall_in(&paths, &yes_to_everything(), false, true).unwrap();

    assert!(
        !data_dir.exists(),
        "with the host file and log gone the data directory is empty and removed"
    );
}

/// Declining the purge's confirmation keeps the identity.
#[test]
fn a_declined_purge_keeps_the_host_file() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, data_dir) = layout(dir.path());
    fs::write(data_dir.join("host.json"), br#"{"host_id":"h"}"#).unwrap();
    let confirmer = FakeConfirmer::new(vec![true, false, false], vec![false, false]);

    run_uninstall_in(&paths, &confirmer, false, true).unwrap();

    assert!(data_dir.join("host.json").exists());
}

/// Without `--purge` nothing in the data directory is touched.
#[test]
fn uninstall_without_purge_keeps_the_host_file() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, data_dir) = layout(dir.path());
    fs::write(data_dir.join("host.json"), br#"{"host_id":"h"}"#).unwrap();

    run_uninstall_in(
        &paths,
        &FakeConfirmer::new(vec![true], vec![]),
        false,
        false,
    )
    .unwrap();

    assert!(data_dir.join("host.json").exists());
}
