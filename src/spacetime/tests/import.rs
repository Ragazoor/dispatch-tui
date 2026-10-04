//! Importing the old store into the managed one — task #16385.
//!
//! Spec: `spacetime-seed.allium`'s `ImportOldStore`,
//! `DependentsOfDroppedRowsAreDropped`, `OrphansLeaveTheirDroppedEpic` and the
//! `ImportOnlyAdds` / `ImportNeverTouchesItsSource` guarantees.
//!
//! The source here is the fixture board's snapshot, edited per test to carry
//! the `archived` rows a real old store holds. The target is a
//! [`MemoryStore`](crate::spacetime::MemoryStore) agreeing with it about the
//! schema.

use super::{snapshot_of_a_populated_board, store_for};
use crate::spacetime::{
    import_old_store, restore, MemoryStore, RefusalReason, Row, SharedStore, SharedTable, Snapshot,
};
use serde_json::Value;

const OPERATOR: &str = "c0ffee";

fn set(snapshot: &mut Snapshot, table: SharedTable, id: i64, column: &str, value: Value) {
    let extract = snapshot
        .extracts_mut()
        .iter_mut()
        .find(|e| e.table == table)
        .unwrap();
    let row = extract
        .rows
        .iter_mut()
        .find(|r| r.get("id").and_then(Value::as_i64) == Some(id))
        .unwrap_or_else(|| panic!("no {} row {id}", table.name()));
    row.insert(column.to_string(), value);
}

fn archive(snapshot: &mut Snapshot, table: SharedTable, id: i64) {
    set(snapshot, table, id, "status", Value::from("archived"));
}

async fn rows(store: &MemoryStore, table: SharedTable) -> Vec<Row> {
    store.rows(table).await.unwrap()
}

fn ids(rows: &[Row]) -> Vec<i64> {
    let mut ids: Vec<i64> = rows
        .iter()
        .filter_map(|r| r.get("id").and_then(Value::as_i64))
        .collect();
    ids.sort_unstable();
    ids
}

fn by_id(rows: &[Row], id: i64) -> &Row {
    rows.iter()
        .find(|r| r.get("id").and_then(Value::as_i64) == Some(id))
        .unwrap_or_else(|| panic!("no row {id}"))
}

/// `ImportOldStore`: every shared table crosses, ids unchanged.
#[tokio::test]
async fn an_import_into_an_empty_store_carries_every_table_with_ids_kept() {
    let source = snapshot_of_a_populated_board().await;
    let target = store_for(&source);

    import_old_store(&target, &source, OPERATOR).await.unwrap();

    assert_eq!(
        target.dump().await.unwrap().canonical_rows(),
        source.canonical_rows()
    );
}

/// `ImportOnlyAdds`: a second run writes nothing and duplicates nothing.
#[tokio::test]
async fn running_the_import_twice_adds_nothing_the_second_time() {
    let source = snapshot_of_a_populated_board().await;
    let target = store_for(&source);

    let first = import_old_store(&target, &source, OPERATOR).await.unwrap();
    let after_first = target.dump().await.unwrap().canonical_rows();
    let second = import_old_store(&target, &source, OPERATOR).await.unwrap();

    assert!(first.imported_total() > 0);
    assert_eq!(second.imported_total(), 0);
    assert_eq!(second.kept_total(), first.imported_total());
    assert_eq!(target.dump().await.unwrap().canonical_rows(), after_first);
}

/// `ImportOnlyAdds`: an edit made on the managed store survives a re-run.
#[tokio::test]
async fn an_import_never_overwrites_a_row_the_target_already_holds() {
    let source = snapshot_of_a_populated_board().await;
    let target = store_for(&source);
    restore(&target, &source).await.unwrap();
    let mut edited = rows(&target, SharedTable::Tasks).await;
    let task = edited.iter_mut().find(|r| r["id"] == 3).unwrap();
    task.insert("title".into(), Value::from("Edited on the managed store"));
    target
        .upsert_rows(SharedTable::Tasks, std::slice::from_ref(task))
        .await
        .unwrap();

    import_old_store(&target, &source, OPERATOR).await.unwrap();

    let tasks = rows(&target, SharedTable::Tasks).await;
    assert_eq!(by_id(&tasks, 3)["title"], "Edited on the managed store");
}

/// `RowsWithARemovedStatusAreDropped`, as the import reads it.
#[tokio::test]
async fn archived_tasks_and_epics_are_dropped_and_counted() {
    let mut source = snapshot_of_a_populated_board().await;
    archive(&mut source, SharedTable::Tasks, 4);
    archive(&mut source, SharedTable::Epics, 7);
    let target = store_for(&source);

    let report = import_old_store(&target, &source, OPERATOR).await.unwrap();

    assert!(!ids(&rows(&target, SharedTable::Tasks).await).contains(&4));
    assert_eq!(ids(&rows(&target, SharedTable::Epics).await), vec![9]);
    assert_eq!(report.dropped(SharedTable::Tasks), 1);
    assert_eq!(report.dropped(SharedTable::Epics), 1);
}

/// `DependentsOfDroppedRowsAreDropped`: no dangling watch or subagent record.
#[tokio::test]
async fn rows_that_only_describe_a_dropped_task_are_dropped_with_it() {
    let mut source = snapshot_of_a_populated_board().await;
    archive(&mut source, SharedTable::Tasks, 3); // watched by task 4
    archive(&mut source, SharedTable::Tasks, 11); // has a live subagent
    let target = store_for(&source);

    let report = import_old_store(&target, &source, OPERATOR).await.unwrap();

    assert!(rows(&target, SharedTable::TaskWatchers).await.is_empty());
    assert!(rows(&target, SharedTable::TaskSubagents).await.is_empty());
    assert_eq!(report.dropped(SharedTable::TaskWatchers), 1);
    assert_eq!(report.dropped(SharedTable::TaskSubagents), 1);
}

/// `OrphansLeaveTheirDroppedEpic`: a live task is not lost with its epic.
#[tokio::test]
async fn a_live_task_in_a_dropped_epic_arrives_with_no_epic_and_the_importer_as_owner() {
    let mut source = snapshot_of_a_populated_board().await;
    archive(&mut source, SharedTable::Epics, 9); // holds task 4096
    let target = store_for(&source);

    let report = import_old_store(&target, &source, OPERATOR).await.unwrap();

    let tasks = rows(&target, SharedTable::Tasks).await;
    let orphan = by_id(&tasks, 4096);
    assert!(matches!(orphan.get("epic_id"), Some(Value::Null)) || orphan["epic_id"] == 0);
    assert_eq!(orphan["owner"], OPERATOR);
    assert_eq!(orphan["title"], "Far ahead");
    assert_eq!(report.orphaned, vec![4096]);
    // A task whose epic survives is untouched.
    assert_eq!(by_id(&tasks, 3)["epic_id"], 7);
}

/// `ImportOldStore`: the next task the store creates cannot collide.
#[tokio::test]
async fn a_task_created_after_an_import_gets_an_id_past_every_imported_one() {
    let source = snapshot_of_a_populated_board().await;
    let target = store_for(&source);

    import_old_store(&target, &source, OPERATOR).await.unwrap();

    let next = target
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();
    assert!(next > 4096, "got {next}");
}

/// The natural key is the second "already present" test.
#[tokio::test]
async fn a_repo_the_target_already_knows_under_another_id_is_kept_not_duplicated() {
    let source = snapshot_of_a_populated_board().await;
    let target = store_for(&source);
    let mut row = Row::new();
    row.insert("id".into(), Value::from(99));
    row.insert("path".into(), Value::from("/repo/a"));
    row.insert("verify_command".into(), Value::from("managed's own"));
    target
        .upsert_rows(SharedTable::RepoPaths, &[row])
        .await
        .unwrap();

    import_old_store(&target, &source, OPERATOR).await.unwrap();

    let repos = rows(&target, SharedTable::RepoPaths).await;
    let for_a: Vec<_> = repos.iter().filter(|r| r["path"] == "/repo/a").collect();
    assert_eq!(for_a.len(), 1);
    assert_eq!(for_a[0]["verify_command"], "managed's own");
    assert_eq!(repos.len(), 2, "/repo/b still crosses");
}

/// `id_conflict`: the same number naming a different task is the one thing
/// "keep the target's row" cannot be allowed to hide.
#[tokio::test]
async fn a_different_task_under_the_same_id_is_refused_and_nothing_is_written() {
    let mut source = snapshot_of_a_populated_board().await;
    set(
        &mut source,
        SharedTable::Tasks,
        3,
        "created_at",
        Value::from("2026-01-01 00:00:00"),
    );
    let target = store_for(&source);
    let mut other = by_id(&rows_of(&source, SharedTable::Tasks), 3).clone();
    other.insert("created_at".into(), Value::from("2026-10-04 12:00:00"));
    target
        .upsert_rows(SharedTable::Tasks, &[other])
        .await
        .unwrap();

    let refusal = import_old_store(&target, &source, OPERATOR)
        .await
        .unwrap_err()
        .into_refusal();

    assert_eq!(refusal.reason, RefusalReason::IdConflict);
    assert!(refusal.detail.contains('3'), "{}", refusal.detail);
    assert_eq!(ids(&rows(&target, SharedTable::Tasks).await), vec![3]);
    assert!(rows(&target, SharedTable::Epics).await.is_empty());
}

fn rows_of(snapshot: &Snapshot, table: SharedTable) -> Vec<Row> {
    snapshot.extract(table).unwrap().rows.clone()
}

/// `NothingIsWrittenBeforeTheChecksPass`: the restore's checks apply.
#[tokio::test]
async fn a_source_whose_schema_differs_is_refused_before_anything_is_written() {
    let source = snapshot_of_a_populated_board().await;
    let target = store_for(&source);
    target.set_columns(SharedTable::Tasks, vec!["id".into()]);

    let refusal = import_old_store(&target, &source, OPERATOR)
        .await
        .unwrap_err()
        .into_refusal();

    assert_eq!(refusal.reason, RefusalReason::SchemaMismatch);
    assert!(rows(&target, SharedTable::Epics).await.is_empty());
}

/// `ImportNeverTouchesItsSource`: the source is only borrowed.
#[tokio::test]
async fn the_source_snapshot_is_unchanged_by_an_import() {
    let mut source = snapshot_of_a_populated_board().await;
    archive(&mut source, SharedTable::Epics, 9);
    let before = source.canonical_rows();

    import_old_store(&store_for(&source), &source, OPERATOR)
        .await
        .unwrap();

    assert_eq!(source.canonical_rows(), before);
}
