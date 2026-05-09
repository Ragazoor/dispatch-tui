//! Tests for the WP-1 async-DB foundation: the lazily-opened
//! `tokio_rusqlite::Connection` and the [`Database::db_call`] helper.
//!
//! Subsequent work packages (WP-2..WP-6) migrate individual `*Store` traits
//! onto this handle. The tests here exercise it directly so the plumbing has
//! coverage before any real impl moves.

use super::in_memory_db;

/// `db_call` runs the closure and returns its result.
#[tokio::test]
async fn db_call_returns_closure_result() {
    let db = in_memory_db();
    let value = db.db_call(|_conn| Ok(42_i64)).await.unwrap();
    assert_eq!(value, 42);
}

/// The async connection shares state with the sync connection — a row
/// inserted through the sync path is visible from `db_call`. This validates
/// the shared-cache memory URI setup in [`Database::open_in_memory`].
#[tokio::test]
async fn async_connection_sees_sync_writes() {
    use crate::db::SettingsStore;

    let db = in_memory_db();
    db.save_repo_path("/tmp/example-repo").unwrap();

    let count: i64 = db
        .db_call(|conn| {
            conn.query_row("SELECT COUNT(*) FROM repo_paths", [], |row| row.get(0))
                .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}

/// Errors from the closure surface as `anyhow::Error` (round-tripped through
/// `tokio_rusqlite::Error::Other`).
#[tokio::test]
async fn db_call_propagates_closure_errors() {
    let db = in_memory_db();
    let err = db
        .db_call(|_conn| -> anyhow::Result<()> { Err(anyhow::anyhow!("boom: 12345")) })
        .await
        .expect_err("closure error should propagate");
    assert!(
        err.to_string().contains("boom: 12345"),
        "expected boom message, got: {err}"
    );
}

/// rusqlite errors inside the closure also surface as `anyhow::Error` with the
/// SQL diagnostic preserved.
#[tokio::test]
async fn db_call_propagates_rusqlite_errors() {
    let db = in_memory_db();
    let err = db
        .db_call(|conn| {
            conn.execute("SELECT * FROM does_not_exist", [])?;
            Ok(())
        })
        .await
        .expect_err("rusqlite error should propagate");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("does_not_exist"),
        "expected SQL diagnostic in error, got: {msg}"
    );
}

/// Independent in-memory databases must not see each other's data — the
/// shared-cache URIs minted by [`Database::open_in_memory`] are unique per
/// instance.
#[tokio::test]
async fn distinct_in_memory_dbs_are_isolated() {
    use crate::db::SettingsStore;

    let db_a = in_memory_db();
    let db_b = in_memory_db();
    db_a.save_repo_path("/only-in-a").unwrap();

    let count_b: i64 = db_b
        .db_call(|conn| {
            conn.query_row("SELECT COUNT(*) FROM repo_paths", [], |row| row.get(0))
                .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count_b, 0, "db_b must not see writes made to db_a");
}
