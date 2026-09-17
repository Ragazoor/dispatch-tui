//! Restoring the same snapshot twice leaves the store as the first restore
//! left it.
//!
//! See `spacetime-seed.allium`: the `RestoreSnapshot` rule's "writes match by
//! id, so a second restore is a no-op" guidance.
//!
//! This is what makes a restore interrupted halfway safe to simply run again,
//! which matters because the situation a restore is reached for is already a
//! bad one.

use super::snapshot_of_a_populated_board;
use crate::spacetime::{restore, MemoryStore, SharedStore, SharedTable};

#[tokio::test]
async fn restoring_twice_does_not_duplicate_rows() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);

    restore(&store, &snapshot).await.unwrap();
    let after_first: Vec<usize> = row_counts(&store).await;
    restore(&store, &snapshot).await.unwrap();
    let after_second: Vec<usize> = row_counts(&store).await;

    assert_eq!(
        after_first, after_second,
        "the second restore appended instead of matching by id"
    );
}

#[tokio::test]
async fn restoring_twice_leaves_every_id_unchanged() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);

    restore(&store, &snapshot).await.unwrap();
    restore(&store, &snapshot).await.unwrap();

    for extract in snapshot.extracts() {
        assert_eq!(
            store.row_ids(extract.table).await.unwrap(),
            extract.row_ids(),
            "{} changed identity on the second restore",
            extract.table.name()
        );
    }
}

/// A restore reinstates what the snapshot holds; it does not make the store
/// equal to the snapshot. A row present in the store and absent from the
/// snapshot is left alone.
///
/// This is deliberate and is the open question at the foot of the spec: a
/// restore that also deleted would be far more useful for recovering from a
/// corrupting write, and one typo away from erasing a colleague's work.
#[tokio::test]
async fn a_restore_leaves_rows_the_snapshot_does_not_mention() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    let stranger = store
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();
    restore(&store, &snapshot).await.unwrap();

    assert!(
        store
            .row_ids(SharedTable::Tasks)
            .await
            .unwrap()
            .contains(&stranger),
        "the restore deleted task {stranger}, which it was only ever asked to \
         leave alone"
    );
}

/// A row changed since the snapshot is overwritten with the snapshot's version
/// of it, not skipped. "Already present" must mean "replace", or a restore
/// after a corrupting write reinstates nothing.
#[tokio::test]
async fn a_restore_overwrites_a_row_that_changed_since() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    let mut corrupted = super::row_with_id(3);
    corrupted.insert("title".into(), serde_json::json!("clobbered"));
    store
        .upsert_rows(SharedTable::Tasks, &[corrupted])
        .await
        .unwrap();

    restore(&store, &snapshot).await.unwrap();

    let rows = store.rows(SharedTable::Tasks).await.unwrap();
    let restored = rows
        .iter()
        .find(|r| r.get("id").and_then(|v| v.as_i64()) == Some(3))
        .unwrap();
    assert_eq!(
        restored.get("title").unwrap(),
        "Oldest task",
        "the restore skipped a row it already had, instead of reinstating it"
    );
}

async fn row_counts(store: &MemoryStore) -> Vec<usize> {
    let mut counts = Vec::new();
    for table in SharedTable::ALL {
        counts.push(store.rows(table).await.unwrap().len());
    }
    counts
}
