//! Nothing is written before the checks pass.
//!
//! See `spacetime-seed.allium`: the three `Refuse*` rules and the
//! `NothingIsWrittenBeforeTheChecksPass` guarantee on `SnapshotCommandLine`.
//!
//! An operator who sees a refusal must know the store is untouched — that is
//! what makes it safe to fix the input and retry, during exactly the incident
//! where retrying is the only move left.

use super::snapshot_of_a_populated_board;
use crate::spacetime::{
    restore, MemoryStore, RefusalReason, SharedStore, SharedTable, SHARED_TABLE_COUNT,
};

/// A newer snapshot read by an older tool is the dangerous direction: the
/// fields it does not understand are the ones it would drop.
#[tokio::test]
async fn a_future_format_version_is_refused_without_writing() {
    let mut snapshot = snapshot_of_a_populated_board().await;
    snapshot.format_version += 1;
    let store = super::store_for(&snapshot);

    let refusal = restore(&store, &snapshot).await.unwrap_err().into_refusal();

    assert_eq!(refusal.reason, RefusalReason::FormatUnsupported);
    assert_store_untouched(&store).await;
}

#[tokio::test]
async fn a_mismatched_schema_version_is_refused_without_writing() {
    let mut snapshot = snapshot_of_a_populated_board().await;
    snapshot.schema_version += 1;
    let store = crate::spacetime::MemoryStore::with_schema_version(snapshot.schema_version - 1);

    let refusal = restore(&store, &snapshot).await.unwrap_err().into_refusal();

    assert_eq!(refusal.reason, RefusalReason::SchemaMismatch);
    assert_store_untouched(&store).await;
}

#[tokio::test]
async fn an_incomplete_snapshot_is_refused_without_writing() {
    let mut snapshot = snapshot_of_a_populated_board().await;
    snapshot.drop_extract(SharedTable::Todos);
    let store = super::store_for(&snapshot);

    let refusal = restore(&store, &snapshot).await.unwrap_err().into_refusal();

    assert_eq!(refusal.reason, RefusalReason::Incomplete);
    assert_store_untouched(&store).await;
}

/// "Restore failed" is not an acceptable message for an operation somebody
/// reaches for during an incident. Each refusal names what it found.
#[tokio::test]
async fn a_refusal_names_what_it_found() {
    let mut snapshot = snapshot_of_a_populated_board().await;
    snapshot.drop_extract(SharedTable::Todos);
    let store = super::store_for(&snapshot);

    let refusal = restore(&store, &snapshot).await.unwrap_err().into_refusal();

    assert!(
        refusal.detail.contains("todos"),
        "refusal detail {:?} does not say which table is missing",
        refusal.detail
    );

    let mut wrong_version = snapshot_of_a_populated_board().await;
    wrong_version.format_version = 99;
    let refusal = restore(&store, &wrong_version)
        .await
        .unwrap_err()
        .into_refusal();
    assert!(
        refusal.detail.contains("99"),
        "refusal detail {:?} does not say which version it found",
        refusal.detail
    );
}

/// The completeness check counts tables rather than trusting the file's own
/// claim about itself, so a snapshot that lists ten extracts but repeats one is
/// refused too.
#[tokio::test]
async fn a_snapshot_that_repeats_a_table_is_refused() {
    let mut snapshot = snapshot_of_a_populated_board().await;
    snapshot.duplicate_extract_for_test(SharedTable::Tasks);
    assert_eq!(snapshot.extracts().len(), SHARED_TABLE_COUNT + 1);
    let store = super::store_for(&snapshot);

    let refusal = restore(&store, &snapshot).await.unwrap_err().into_refusal();

    assert_eq!(refusal.reason, RefusalReason::Incomplete);
    assert_store_untouched(&store).await;
}

async fn assert_store_untouched(store: &MemoryStore) {
    for table in SharedTable::ALL {
        assert!(
            store.rows(table).await.unwrap().is_empty(),
            "{} was written to despite the refusal",
            table.name()
        );
        if table.generates_ids() {
            assert_eq!(
                store.next_generated_id(table),
                1,
                "{}'s counter was burned despite the refusal",
                table.name()
            );
        }
    }
}

/// A ceiling wildly out of proportion to the rows claiming it is treated as a
/// corrupt snapshot rather than a large board.
///
/// The burn is O(ceiling), not O(rows), and the ceiling comes out of a file an
/// operator may have hand-edited during an incident. One extra digit turns a
/// four-thousand-iteration loop into a four-billion-iteration one inside a
/// single open reducer transaction.
#[tokio::test]
async fn an_absurd_id_is_refused_rather_than_burned_towards() {
    let mut snapshot = snapshot_of_a_populated_board().await;
    snapshot.set_task_id_for_test(4096, 999_999_999_999);
    let store = super::store_for(&snapshot);

    let refusal = restore(&store, &snapshot).await.unwrap_err().into_refusal();

    assert_eq!(refusal.reason, RefusalReason::Incomplete);
    assert!(
        refusal.detail.contains("999999999999"),
        "refusal does not name the id it balked at: {}",
        refusal.detail
    );
    assert_store_untouched(&store).await;
}

/// The real board's own proportions must not trip the bound. A guard that
/// refuses ordinary data is worse than no guard.
#[tokio::test]
async fn an_ordinary_board_is_not_mistaken_for_a_corrupt_one() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);

    restore(&store, &snapshot).await.unwrap();
}
