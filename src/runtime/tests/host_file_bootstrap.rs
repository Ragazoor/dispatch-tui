//! Task #16755: the board launch reads its identity from the host file and
//! never opens the legacy `tasks.db` beside it.
//!
//! Spec: `host.allium` (`MintHostIdentity`, `IdentityLivesInHostFile`,
//! `AdoptUserIdentity`), `startup.allium`
//! (`CheckHostLabelAfterStartupConfigResolves`,
//! `AbortWhenTheHostIdentityStoreIsUnusable`), `storage.allium`
//! (`StoreInUseNeverOpensSqlite`, `CodeNeverTouchesLegacyDatabase`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::db::TaskCrud;
use crate::host_file::{host_file_path, HostIdentity};
use crate::sync::tests::{accepted, ScriptedConnector};
use crate::sync::StoreConnector;

const TEST_STORE: &str = "http://store.test";

/// The stand-in store `misc.rs`'s bootstrap tests use: whatever database
/// bootstrap hands over, unrouted, behind a connector that accepts once.
fn test_store(database: crate::db::Database, _host: &str) -> StoreParts {
    let connector: Arc<dyn StoreConnector> =
        ScriptedConnector::new(vec![accepted("c0ffee", "secret-token")]);
    let rows = Arc::new(crate::sync::SharedRows::new());
    let settled_identity = Arc::new(crate::sync::SettledIdentity::default());
    let sdk = Arc::new(crate::sync::SpacetimeSdkConnector::new(
        "test",
        rows.clone(),
    ));
    StoreParts {
        database: Arc::new(database),
        board_reads: Arc::new(crate::sync::SubscriptionBoardReads::new(rows.clone())),
        rows,
        connector,
        reducer_caller: Arc::new(crate::sync::SdkReducerCaller::new(
            sdk,
            settled_identity.clone(),
        )),
        settled_identity,
        store_identity: |_| None,
    }
}

/// What a leftover `tasks.db` looked like before the launch: its bytes, its
/// mtime, and whether its WAL companions existed.
#[derive(Debug, PartialEq)]
struct LegacyFootprint {
    /// A digest rather than the bytes, so a failure prints one line.
    content: u64,
    modified: std::time::SystemTime,
    wal: bool,
    shm: bool,
}

fn footprint(db_path: &Path) -> LegacyFootprint {
    let companion = |suffix: &str| {
        let mut name = db_path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name).exists()
    };
    LegacyFootprint {
        content: {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::fs::read(db_path).unwrap().hash(&mut h);
            h.finish()
        },
        modified: std::fs::metadata(db_path).unwrap().modified().unwrap(),
        wal: companion("-wal"),
        shm: companion("-shm"),
    }
}

/// A data directory holding a populated pre-#16755 `tasks.db` — a real
/// SQLite file with a task in it and NO host identity — closed cleanly so no
/// `-wal`/`-shm` remain. Any launch that opens it shows: opening it in WAL
/// mode recreates the companions, and minting an identity into it changes its
/// bytes.
async fn legacy_install() -> (tempfile::TempDir, PathBuf, StartupPaths) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("tasks.db");
    {
        let db = crate::db::Database::open(&db_path).await.unwrap();
        db.create_task(crate::db::CreateTaskRequest {
            title: "Left behind",
            description: "",
            repo_path: "/tmp/legacy-repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
        // Leave WAL mode on the way out, so the file is self-contained and its
        // companions are gone the moment this returns — dropping the handle
        // closes it on a background thread, which a test cannot wait for.
        db.db_call(|conn| {
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
                .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    }
    let paths = StartupPaths {
        claude_dir: dir.path().join("claude"),
        claude_json_path: dir.path().join(".claude.json"),
    };
    let before = footprint(&db_path);
    assert!(
        !before.wal && !before.shm,
        "fixture: the legacy database must be closed cleanly"
    );
    (dir, db_path, paths)
}

fn write_host_file(data_dir: &Path, identity: &HostIdentity) {
    use std::os::unix::fs::PermissionsExt;
    let path = host_file_path(data_dir);
    std::fs::write(&path, serde_json::to_vec(identity).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn read_host_file(data_dir: &Path) -> HostIdentity {
    serde_json::from_slice(&std::fs::read(host_file_path(data_dir)).unwrap())
        .expect("host.json must hold a parseable identity")
}

/// An install upgrading past #16755 has no host file, so its first launch is a
/// first run: it mints and writes host.json (label null) before the label gate
/// — which then aborts, because `cargo test`'s stdin cannot answer — and the
/// old identity in tasks.db is neither read nor carried over.
#[tokio::test]
async fn a_launch_with_no_host_file_mints_one_and_leaves_the_legacy_database_alone() {
    let (dir, db_path, paths) = legacy_install().await;
    let before = footprint(&db_path);

    let result =
        TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false).await;

    match result {
        Ok(_) => panic!("a freshly minted host is unnamed and nobody can name it here"),
        Err(err) => assert_eq!(
            err.to_string(),
            crate::startup::StartupAbort::HostUnnamed.message(),
            "the identity step succeeds and the label gate is what stops a \
             non-interactive first run"
        ),
    }
    let path = host_file_path(dir.path());
    assert!(path.exists(), "a first run writes host.json beside --db");
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let minted = read_host_file(dir.path());
    assert!(!minted.host_id.is_empty());
    assert_eq!(minted.label, None);
    assert_eq!(
        footprint(&db_path),
        before,
        "the leftover tasks.db must not be opened, written, or have WAL files created"
    );
}

/// A host file that will not parse aborts the launch with
/// host_identity_unavailable, and is not overwritten.
#[tokio::test]
async fn a_launch_with_an_unparseable_host_file_aborts_and_keeps_it() {
    let (dir, db_path, paths) = legacy_install().await;
    let path = host_file_path(dir.path());
    std::fs::write(&path, b"not json at all").unwrap();
    let before = footprint(&db_path);

    let result =
        TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false).await;

    match result {
        Ok(_) => panic!("a damaged host file must abort the launch"),
        Err(err) => assert_eq!(
            err.to_string(),
            crate::startup::StartupAbort::HostIdentityUnavailable.message()
        ),
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"not json at all");
    assert_eq!(
        footprint(&db_path),
        before,
        "tasks.db is not consulted instead"
    );
}

/// A named host file is the whole identity: the board starts from it, adopts
/// the store's user identity INTO it (owner and credential together), and at
/// no point — not even while the board is up — opens the tasks.db beside it.
#[tokio::test]
async fn a_launch_with_a_named_host_file_never_opens_the_legacy_database() {
    let (dir, db_path, paths) = legacy_install().await;
    write_host_file(
        dir.path(),
        &HostIdentity {
            host_id: "host-from-file".to_string(),
            label: Some("laptop".to_string()),
            user_identity: None,
            credential: None,
        },
    );
    let before = footprint(&db_path);

    let bootstrap =
        match TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
            .await
        {
            Ok(b) => b,
            Err(err) => panic!("a named host file must let the board start: {err}"),
        };

    assert_eq!(
        footprint(&db_path),
        before,
        "with the board up, the leftover tasks.db must be untouched and have no WAL files"
    );
    assert_eq!(
        read_host_file(dir.path()),
        HostIdentity {
            host_id: "host-from-file".to_string(),
            label: Some("laptop".to_string()),
            user_identity: Some("c0ffee".to_string()),
            credential: Some("secret-token".to_string()),
        },
        "the identity the store handed out is adopted into the host file, with its credential"
    );
    drop(bootstrap);
    assert_eq!(footprint(&db_path), before, "nor after it shuts down");
    assert!(db_path.exists(), "nothing deletes the leftover");
}
