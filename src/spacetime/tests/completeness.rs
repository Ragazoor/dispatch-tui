//! A snapshot covers every shared table, and an empty table is a claim rather
//! than a silence.
//!
//! See `spacetime-seed.allium`: `Snapshot.SnapshotIsComplete`,
//! `Snapshot.NoTableAppearsTwice`, and the `TakeSnapshot` rule's "every table,
//! including the empty ones" guidance.

use crate::spacetime::SharedTable;

#[test]
fn no_shared_table_is_named_twice() {
    let mut names: Vec<&str> = SharedTable::ALL.iter().map(|t| t.name()).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len(), "two variants share a table name");
}
