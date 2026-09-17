//! Id preservation and row fidelity across a dump and a restore.
//!
//! See `spacetime-seed.allium`: the `RestoreSnapshot` rule's "ids are data, not
//! addresses" guidance, and `TableExtract.EveryRowCarriesItsId`.

use super::snapshot_of_a_populated_board;
use crate::spacetime::{restore, SharedStore, SharedTable};

/// Every id arrives on the far side as itself.
#[tokio::test]
async fn restore_preserves_every_id() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    for extract in snapshot.extracts() {
        let restored = store.row_ids(extract.table).await.unwrap();
        assert_eq!(
            restored,
            extract.row_ids(),
            "{} lost or altered an id in transit",
            extract.table.name()
        );
    }
}

/// Gaps survive. A gap is evidence that a task existed and does not, and a
/// restore that closes it makes deleted work look like it never happened.
/// The fixture has no tasks between 4 and 11.
#[tokio::test]
async fn restore_preserves_the_gaps_between_ids() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    let ids = store.row_ids(SharedTable::Tasks).await.unwrap();

    assert_eq!(
        ids,
        vec![3, 4, 11, 4096],
        "the gaps were closed — a dense renumbering is the failure this test \
         exists for, and every internal reference would still look consistent \
         afterwards"
    );
}

/// Rows travel whole. No field is dropped, defaulted, recomputed or
/// normalised on the way through, including fields that look derived.
#[tokio::test]
async fn restore_carries_every_column_unaltered() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    for extract in snapshot.extracts() {
        let mut restored = store.rows(extract.table).await.unwrap();
        let mut expected = extract.rows.clone();
        restored.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
        expected.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
        assert_eq!(
            restored,
            expected,
            "{} did not round-trip its rows verbatim",
            extract.table.name()
        );
    }
}

/// The whole-artefact statement of the same thing: re-dumping a restored store
/// yields the snapshot it was restored from. Catches a field that survives the
/// write but is read back differently, which the per-table assertions above
/// can miss when both sides share the same bug.
#[tokio::test]
async fn a_restored_store_dumps_back_to_the_same_snapshot() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    let redumped = store.dump(snapshot.schema_version).await.unwrap();

    assert_eq!(
        redumped.canonical_rows(),
        snapshot.canonical_rows(),
        "the round trip is not closed"
    );
}

/// A snapshot survives the trip through its serialised form. This is the form
/// the artefact actually exists in — a round trip that only holds in memory is
/// not a backup.
#[tokio::test]
async fn a_snapshot_survives_serialisation() {
    let snapshot = snapshot_of_a_populated_board().await;

    let encoded = serde_json::to_string(&snapshot).unwrap();
    let decoded: crate::spacetime::Snapshot = serde_json::from_str(&encoded).unwrap();

    assert_eq!(decoded.canonical_rows(), snapshot.canonical_rows());
    assert_eq!(decoded.schema_version, snapshot.schema_version);
    assert_eq!(decoded.format_version, snapshot.format_version);
}

/// Integer ids must not come back as floats or strings. JSON has one number
/// type, and a task id that round-trips as `4096.0` compares unequal to `4096`
/// everywhere it matters while looking right in a diff.
#[tokio::test]
async fn ids_survive_serialisation_as_integers() {
    let snapshot = snapshot_of_a_populated_board().await;
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let decoded: crate::spacetime::Snapshot = serde_json::from_str(&encoded).unwrap();

    let extract = decoded.extract(SharedTable::Tasks).unwrap();
    for row in &extract.rows {
        let id = row.get("id").unwrap();
        assert!(
            id.is_i64(),
            "task id decoded as {id}, which is not an integer"
        );
    }
    assert_eq!(decoded.highest_id(SharedTable::Tasks), 4096);
}

/// **The snapshot has one representation of a boolean, and it is `true`/`false`.**
///
/// SQLite stores 0 and 1; the shared store types these columns as booleans. If
/// the dump passed the integers through, the same board would produce two
/// different files depending on which store dumped it, and a diff between a
/// board backup and a server backup would show every boolean as changed.
#[tokio::test]
async fn booleans_are_canonical_in_the_snapshot_not_sqlites_integers() {
    let snapshot = snapshot_of_a_populated_board().await;

    for extract in snapshot.extracts() {
        for column in extract.table.boolean_columns() {
            for row in &extract.rows {
                let value = row
                    .get(*column)
                    .unwrap_or_else(|| panic!("{} has no column {column}", extract.table.name()));
                assert!(
                    value.is_boolean() || value.is_null(),
                    "{}.{column} is {value} — SQLite's integer leaked into the snapshot",
                    extract.table.name()
                );
            }
        }
    }
}

/// Sanity: the fixture actually exercises the conversion. A board whose boolean
/// columns held no rows would satisfy the test above vacuously.
#[tokio::test]
async fn the_fixture_has_booleans_to_convert() {
    let snapshot = snapshot_of_a_populated_board().await;
    let tasks = snapshot.extract(SharedTable::Tasks).unwrap();

    assert_eq!(
        tasks.rows[0].get("phoenix"),
        Some(&serde_json::Value::Bool(false))
    );
}
