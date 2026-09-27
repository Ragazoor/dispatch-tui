//! Seeding a store from one existing board — Phase 12a (task #4916).
//!
//! Spec: `spacetime-seed.allium`'s `SeedSharedStore`, `BackfillTaskOwner`,
//! `BackfillCreatedBy` and `RefuseSeedingANonEmptyStore`.
//!
//! The store here has the MODULE's columns — the SQLite ones plus
//! `SharedTable::module_only_columns` — because that gap is the whole reason a
//! seed exists: a plain restore of a SQLite dump is refused for it.

use super::snapshot_of_a_populated_board;
use crate::spacetime::{
    restore, seed, MemoryStore, RefusalReason, Row, SharedStore, SharedTable, Snapshot,
};

const OPERATOR: &str = "c0ffee";

/// A store shaped like the module: every snapshot column, plus the ones only
/// the module has.
fn module_store(snapshot: &Snapshot) -> MemoryStore {
    let store = MemoryStore::matching(snapshot);
    for extract in snapshot.extracts() {
        let mut columns = extract.columns.clone();
        for column in extract.table.module_only_columns() {
            if !columns.iter().any(|c| c == column) {
                columns.push((*column).to_string());
            }
        }
        store.set_columns(extract.table, columns);
    }
    store
}

fn by_id(rows: &[Row], id: i64) -> &Row {
    rows.iter()
        .find(|r| r.get("id").and_then(|v| v.as_i64()) == Some(id))
        .unwrap_or_else(|| panic!("no row {id}"))
}

fn text<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    row.get(column).and_then(|v| v.as_str())
}

/// Why the seed exists: a SQLite dump lacks the columns only the module has,
/// so restoring it as-is is refused, and seeding it is not.
#[tokio::test]
async fn a_board_dump_is_refused_by_restore_but_accepted_by_seed() {
    let snapshot = snapshot_of_a_populated_board().await;

    let refusal = restore(&module_store(&snapshot), &snapshot)
        .await
        .unwrap_err()
        .into_refusal();
    assert_eq!(refusal.reason, RefusalReason::SchemaMismatch);

    seed(&module_store(&snapshot), snapshot, OPERATOR)
        .await
        .expect("a seed supplies the missing columns");
}

/// `BackfillTaskOwner`: an epic-less task lands on the seeder's own board; a
/// task in an epic belongs to the epic, not to a person.
#[tokio::test]
async fn a_task_with_no_epic_is_owned_by_the_seeder() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = module_store(&snapshot);

    seed(&store, snapshot, OPERATOR).await.unwrap();

    let tasks = store.rows(SharedTable::Tasks).await.unwrap();
    assert_eq!(text(by_id(&tasks, 11), "owner"), Some(OPERATOR));
    for in_an_epic in [3, 4, 4096] {
        assert_ne!(
            text(by_id(&tasks, in_an_epic), "owner"),
            Some(OPERATOR),
            "task {in_an_epic} is in an epic and must not sit on a user board"
        );
    }
}

/// `BackfillCreatedBy`: every task and epic is the seeder's creation, which is
/// what puts the epics on their board at all.
#[tokio::test]
async fn every_task_and_epic_is_created_by_the_seeder() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = module_store(&snapshot);

    seed(&store, snapshot, OPERATOR).await.unwrap();

    for table in [SharedTable::Tasks, SharedTable::Epics] {
        let rows = store.rows(table).await.unwrap();
        assert!(!rows.is_empty());
        for row in &rows {
            assert_eq!(text(row, "created_by"), Some(OPERATOR), "{row:?}");
        }
    }
}

/// A seed is a restore: ids are kept, and the counters are burned past them.
#[tokio::test]
async fn a_seed_keeps_every_id_and_burns_past_them() {
    let snapshot = snapshot_of_a_populated_board().await;
    let expected = snapshot.extract(SharedTable::Tasks).unwrap().row_ids();
    let store = module_store(&snapshot);

    seed(&store, snapshot, OPERATOR).await.unwrap();

    let mut held = store.row_ids(SharedTable::Tasks).await.unwrap();
    let mut expected = expected;
    held.sort_unstable();
    expected.sort_unstable();
    assert_eq!(held, expected);
    assert!(store.next_generated_id(SharedTable::Tasks) > 4096);
}

/// `RefuseSeedingANonEmptyStore`: a store that already holds a task is not
/// seeded, and is left exactly as it was — nothing written, nothing burned.
#[tokio::test]
async fn seeding_a_store_that_holds_tasks_is_refused_and_writes_nothing() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = module_store(&snapshot);
    let existing = store
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();
    let next = store.next_generated_id(SharedTable::Tasks);

    let refusal = seed(&store, snapshot, OPERATOR)
        .await
        .unwrap_err()
        .into_refusal();

    assert_eq!(refusal.reason, RefusalReason::StoreNotEmpty);
    assert_eq!(
        store.row_ids(SharedTable::Tasks).await.unwrap(),
        vec![existing]
    );
    assert_eq!(store.next_generated_id(SharedTable::Tasks), next);
    assert!(store.rows(SharedTable::Epics).await.unwrap().is_empty());
}
