//! `storage.allium`: JournalModeFollowsTheMedium and AskingForWalDoesNotMakeItSo.
//!
//! Both open paths ask for WAL. Only one gets it, and the other says nothing
//! about having settled for less — so these pin the difference where a reader
//! will find it, rather than leaving it to be rediscovered by a flaky test.

use super::in_memory_db;

async fn journal_mode(db: &crate::db::Database) -> String {
    db.db_call(|conn| {
        conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(anyhow::Error::from)
    })
    .await
    .unwrap()
}

/// A file-backed store is WAL, which is what makes a read independent of a
/// write in flight. Every real board takes this path.
#[tokio::test]
async fn a_file_backed_store_runs_in_wal_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let db = crate::db::Database::open(&tmp.path().join("board.db"))
        .await
        .unwrap();
    assert_eq!(journal_mode(&db).await, "wal");
}

/// An in-memory store is not, however it was asked. SQLite's `memdb` has
/// nowhere to put the write-ahead log's shared memory, so `PRAGMA
/// journal_mode=WAL` quietly returns `memory` instead of failing — and the
/// store that comes back is indistinguishable from a WAL one until a read and
/// a write overlap on it.
#[tokio::test]
async fn an_in_memory_store_settles_for_a_rollback_journal() {
    let db = in_memory_db().await;
    assert_eq!(
        journal_mode(&db).await,
        "memory",
        "if this ever reads 'wal', the trap this pins is gone and \
         storage.allium needs revisiting"
    );
}

/// The consequence, stated as a test so it is not only prose: on an in-memory
/// store a read issued while a write holds the store waits for that write. On
/// a WAL board it would not. A test that overlaps a read with a write on an
/// in-memory board is therefore testing different rules from the ones it ships
/// under — see `storage.allium`: ConcurrencyIsObservedOnAWalStore.
#[tokio::test]
async fn an_in_memory_read_waits_for_a_write_that_a_wal_read_would_not() {
    let db = std::sync::Arc::new(in_memory_db().await);

    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();

    // `BEGIN EXCLUSIVE` and its `COMMIT` must run on the same connection, and
    // the writer is a single actor thread — so the closure parks on a blocking
    // channel between them rather than returning. Safe for the reason
    // `read_pool.rs` gives: this closure owns a dedicated OS thread, not a
    // tokio worker.
    let holder = {
        let db = std::sync::Arc::clone(&db);
        tokio::spawn(async move {
            db.db_call(move |conn| {
                conn.execute_batch("BEGIN EXCLUSIVE;")?;
                held_tx.send(()).ok();
                let _ = release_rx.recv();
                conn.execute_batch("COMMIT;").map_err(anyhow::Error::from)
            })
            .await
        })
    };
    held_rx.await.expect("the write must be in flight");

    let reading = db.db_call_read(|conn| {
        conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .map_err(anyhow::Error::from)
    });
    // Long enough that a read which was going to proceed would have, short
    // enough to stay well inside the connection's lock wait either way.
    let outcome = tokio::time::timeout(std::time::Duration::from_millis(750), reading).await;
    assert!(
        outcome.is_err(),
        "an in-memory read must be excluded by the write; it returned {outcome:?}"
    );

    release_tx.send(()).ok();
    holder.await.unwrap().unwrap();
}

/// The same overlap on a WAL board: the read proceeds while the write is still
/// open, and sees the last committed state. This is the behaviour every board
/// actually has, and the reason the test above is a statement about fixtures
/// rather than about SQLite.
#[tokio::test]
async fn a_wal_read_proceeds_while_a_write_is_in_flight() {
    let tmp = tempfile::tempdir().unwrap();
    let db = std::sync::Arc::new(
        crate::db::Database::open(&tmp.path().join("board.db"))
            .await
            .unwrap(),
    );

    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();

    let holder = {
        let db = std::sync::Arc::clone(&db);
        tokio::spawn(async move {
            db.db_call(move |conn| {
                conn.execute_batch(
                    "BEGIN IMMEDIATE;
                     INSERT INTO repo_paths (path) VALUES ('/uncommitted');",
                )?;
                held_tx.send(()).ok();
                let _ = release_rx.recv();
                conn.execute_batch("COMMIT;").map_err(anyhow::Error::from)
            })
            .await
        })
    };
    held_rx.await.expect("the write must be in flight");

    let reading = db.db_call_read(|conn| {
        conn.query_row("SELECT COUNT(*) FROM repo_paths", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(anyhow::Error::from)
    });
    let count = tokio::time::timeout(std::time::Duration::from_secs(10), reading)
        .await
        .expect("a WAL read must not wait for the writer")
        .unwrap();
    assert_eq!(count, 0, "the read must see the last committed state");

    release_tx.send(()).ok();
    holder.await.unwrap().unwrap();
}
