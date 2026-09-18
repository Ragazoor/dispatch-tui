//! A snapshot covers every shared table, and an empty table is a claim rather
//! than a silence.
//!
//! See `spacetime-seed.allium`: `Snapshot.SnapshotIsComplete`,
//! `Snapshot.NoTableAppearsTwice`, and the `TakeSnapshot` rule's "every table,
//! including the empty ones" guidance.

use super::{populated_board, snapshot_of_a_populated_board};
use crate::db::Database;
use crate::spacetime::{dump_from_sqlite, SharedTable, SHARED_TABLE_COUNT};

#[test]
fn no_shared_table_is_named_twice() {
    let mut names: Vec<&str> = SharedTable::ALL.iter().map(|t| t.name()).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len(), "two variants share a table name");
}

#[tokio::test]
async fn a_dump_covers_every_shared_table() {
    let snapshot = snapshot_of_a_populated_board().await;

    for table in SharedTable::ALL {
        assert!(
            snapshot.extract(table).is_some(),
            "{} is missing from the snapshot",
            table.name()
        );
    }
    assert_eq!(snapshot.extracts().len(), SHARED_TABLE_COUNT);
}

/// An empty extract and a missing extract are different claims. The first says
/// "this table had no rows"; the second says nothing at all, and only the first
/// is safe to restore from.
#[tokio::test]
async fn an_empty_table_yields_an_empty_extract_not_a_missing_one() {
    let db = Database::open_in_memory().await.unwrap();
    let snapshot = dump_from_sqlite(&db).await.unwrap();

    assert_eq!(snapshot.extracts().len(), SHARED_TABLE_COUNT);
    for table in SharedTable::ALL {
        let extract = snapshot
            .extract(table)
            .unwrap_or_else(|| panic!("{} absent from a dump of an empty board", table.name()));
        assert!(extract.rows.is_empty() || table == SharedTable::Hosts);
    }
}

/// The host registry does not exist as a SQLite table — it is assembled from
/// this install's own identity at dump time. Asserted because a silently empty
/// `hosts` extract would restore a board on which no task's owning machine
/// resolves to anything.
#[tokio::test]
async fn the_host_registry_is_assembled_from_this_installs_identity() {
    let snapshot = snapshot_of_a_populated_board().await;
    let hosts = snapshot.extract(SharedTable::Hosts).unwrap();

    assert_eq!(hosts.rows.len(), 1, "one install, one host row");
    assert_eq!(hosts.rows[0].get("id").unwrap(), "host-1");
    assert_eq!(hosts.rows[0].get("label").unwrap(), "ragge-laptop");
    assert_eq!(
        hosts.rows[0].get("owner").unwrap(),
        &serde_json::Value::Null,
        "the fixture install has never connected, so it has no owner yet — and \
         the column is present and null rather than absent, because a missing \
         key restores as a column the far side does not know it has"
    );
}

/// The same registry, on an install that HAS connected.
///
/// `owner` is the only field here that is not minted locally, so it is the one
/// a dump can silently drop without any other test noticing: the id and label
/// would still be there, the row would still be one row, and the restored
/// board's machines would all belong to nobody.
#[tokio::test]
async fn the_host_registry_carries_the_owner_once_an_identity_is_settled() {
    use crate::db::{HostStore, IdentityCredentialStore};

    let db = super::populated_board().await;
    db.set_user_identity_token("token-a").await.unwrap();
    db.adopt_user_identity("user-a").await.unwrap();

    let snapshot = crate::spacetime::dump_from_sqlite(&db).await.unwrap();
    let hosts = snapshot.extract(SharedTable::Hosts).unwrap();

    assert_eq!(hosts.rows[0].get("owner").unwrap(), "user-a");
}

/// The assembled `hosts` row carries exactly the columns it declares.
///
/// The other end of the same rope: `module_schema.rs` checks the DECLARED list
/// against the module, and this checks the row the dump actually produces
/// against that list. Neither side is a literal, so appending a column to the
/// module fails at whichever end is forgotten rather than at neither.
#[tokio::test]
async fn the_assembled_host_row_carries_every_column_it_declares() {
    let snapshot = snapshot_of_a_populated_board().await;
    let hosts = snapshot.extract(SharedTable::Hosts).unwrap();

    let mut produced: Vec<&str> = hosts.rows[0].keys().map(String::as_str).collect();
    produced.sort_unstable();
    let mut declared: Vec<&str> = SharedTable::Hosts
        .assembled_columns()
        .iter()
        .map(|(column, _)| *column)
        .collect();
    declared.sort_unstable();

    assert_eq!(produced, declared);
    assert!(!declared.is_empty(), "sanity: hosts declares columns");
}

/// Subscriptions are read from SQLite now, not assumed empty.
///
/// The table is usually empty — an install that has never reached a shared
/// store has no identity to subscribe as — but "usually empty" and "always
/// empty" differ by exactly the rows worth backing up.
#[tokio::test]
async fn subscriptions_are_dumped_rather_than_assumed_empty() {
    use crate::db::{HostStore, IdentityCredentialStore, SubscriptionStore};

    let db = super::populated_board().await;
    db.set_user_identity_token("token-a").await.unwrap();
    db.adopt_user_identity("user-a").await.unwrap();
    db.subscribe_to_epic("user-a", 7).await.unwrap();

    let snapshot = crate::spacetime::dump_from_sqlite(&db).await.unwrap();
    let subscriptions = snapshot.extract(SharedTable::Subscriptions).unwrap();

    assert_eq!(subscriptions.rows.len(), 1);
    assert_eq!(subscriptions.rows[0].get("epic_id").unwrap(), 7);
    assert_eq!(subscriptions.rows[0].get("subscriber").unwrap(), "user-a");
    assert_eq!(subscriptions.rows[0].get("id").unwrap(), "user-a/7");
}

/// Every id-carrying extract accounts for all of its rows, and every extract
/// without ids carries none at all. A partial list is the one state that means
/// nothing.
#[tokio::test]
async fn every_extract_accounts_for_all_its_rows_or_none() {
    let snapshot = snapshot_of_a_populated_board().await;

    for extract in snapshot.extracts() {
        let ids = extract.row_ids();
        assert!(
            ids.is_empty() || ids.len() == extract.rows.len(),
            "{} carries {} ids for {} rows",
            extract.table.name(),
            ids.len(),
            extract.rows.len()
        );
        assert_eq!(
            !ids.is_empty() && !extract.rows.is_empty(),
            extract.table.generates_ids() && !extract.rows.is_empty(),
            "{} disagrees with its own generates_ids()",
            extract.table.name()
        );
    }
}

/// The dump reads one consistent view rather than ten separately-timed ones.
///
/// A board written to between two table reads would otherwise produce a
/// snapshot holding a todo whose task is absent — which restores cleanly and
/// leaves a board with dangling references, the worst outcome available here
/// because it looks fine.
///
/// **The writer runs until the dump says stop**, rather than for a fixed count.
/// A fixed count finishes before the dump opens its first reader and the test
/// passes against a deliberately broken ten-read dump — which is how this test
/// was written the first time, and it proved nothing. Tying the writer's
/// lifetime to the dump's guarantees the two overlap.
#[tokio::test]
async fn a_dump_is_internally_consistent_under_concurrent_writes() {
    let db = std::sync::Arc::new(populated_board().await);
    let dump_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Each iteration adds a task and a todo pointing at it, in one transaction.
    // Any interleaving must leave the snapshot referentially whole: either both
    // rows are in it or neither is, never the todo alone.
    let writer = {
        let db = std::sync::Arc::clone(&db);
        let dump_done = std::sync::Arc::clone(&dump_done);
        tokio::spawn(async move {
            let mut n = 0i64;
            while !dump_done.load(std::sync::atomic::Ordering::Relaxed) {
                let task_id = 10_000 + n;
                n += 1;
                db.db_call(move |conn| {
                    conn.execute_batch(&format!(
                        "BEGIN IMMEDIATE;
                         INSERT INTO tasks (id, title, description, repo_path, status)
                             VALUES ({task_id}, 'racer', 'body', '/repo/a', 'backlog');
                         INSERT INTO todos (id, title, task_id)
                             VALUES ({task_id}, 'racing todo', {task_id});
                         COMMIT;"
                    ))
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap();
            }
            n
        })
    };

    let snapshot = dump_from_sqlite(&db).await.unwrap();
    dump_done.store(true, std::sync::atomic::Ordering::Relaxed);
    let written = writer.await.unwrap();
    assert!(
        written > 0,
        "the writer never ran, so this test raced nothing"
    );

    let task_ids: std::collections::HashSet<i64> = snapshot
        .extract(SharedTable::Tasks)
        .unwrap()
        .row_ids()
        .into_iter()
        .collect();

    for todo in &snapshot.extract(SharedTable::Todos).unwrap().rows {
        let Some(task_id) = todo.get("task_id").and_then(|v| v.as_i64()) else {
            continue;
        };
        assert!(
            task_ids.contains(&task_id),
            "the snapshot holds a todo pointing at task {task_id}, which the \
             snapshot does not contain — the tables were read at different \
             moments"
        );
    }
}
