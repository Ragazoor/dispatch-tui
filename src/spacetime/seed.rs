//! Seeding an empty shared store from one existing board.
//!
//! Spec: `docs/specs/spacetime-seed.allium` — `SeedSharedStore`,
//! `BackfillTaskOwner`, `BackfillCreatedBy` and `RefuseSeedingANonEmptyStore`.
//!
//! A seed is a restore plus the attribution only a first seed may perform:
//! the board's history had no people in it, so every row is stamped as the
//! seeding person's before it is offered. The backfills amend the SNAPSHOT,
//! not the store — the store's own invariants decide what may be written, so
//! a row has to be right on its way in (see the `BackfillTaskOwner` rule).

use serde_json::Value;

use super::restore::{restore, RestoreError};
use super::snapshot::{Refusal, RefusalReason, SharedTable, Snapshot};
use super::store::SharedStore;

/// Seed `store` from `snapshot`, attributing every row to `operator` — this
/// install's user identity.
///
/// Refuses, before anything is written or burned, when the store already
/// holds a task or an epic: a seed happens once, into an empty store, from one
/// board. Every other check is the restore's own.
pub async fn seed(
    store: &dyn SharedStore,
    mut snapshot: Snapshot,
    operator: &str,
) -> Result<(), RestoreError> {
    for table in [SharedTable::Tasks, SharedTable::Epics] {
        let held = store.row_ids(table).await.map_err(RestoreError::Failed)?;
        if !held.is_empty() {
            return Err(RestoreError::Refused(Refusal::new(
                RefusalReason::StoreNotEmpty,
                format!(
                    "the store already holds {} {} row(s). A seed attributes every row to \
                     whoever runs it, so it only runs into an empty store; to put a backup \
                     back, use `dispatch spacetime restore` instead.",
                    held.len(),
                    table.name()
                ),
            )));
        }
    }
    backfill(&mut snapshot, operator);
    restore(store, &snapshot).await
}

/// Add every module-only column to its extract, with its seed-time value:
/// `owner` is the operator on an epic-less task (`BackfillTaskOwner`),
/// `created_by` is the operator on every task and epic (`BackfillCreatedBy`),
/// and anything else — the dead shell columns — is left absent, which the
/// store encodes as its sentinel.
fn backfill(snapshot: &mut Snapshot, operator: &str) {
    for extract in snapshot.extracts_mut() {
        let table = extract.table;
        for column in table.module_only_columns() {
            if !extract.columns.iter().any(|c| c == column) {
                extract.columns.push((*column).to_string());
            }
            for row in &mut extract.rows {
                let value = match *column {
                    "owner" if table == SharedTable::Tasks && has_no_epic(row) => {
                        Value::String(operator.to_string())
                    }
                    "created_by" => Value::String(operator.to_string()),
                    _ => Value::Null,
                };
                row.insert((*column).to_string(), value);
            }
        }
    }
}

/// A dumped task with no epic: SQLite's NULL, or the store's `0` sentinel if
/// the snapshot came from a store.
fn has_no_epic(row: &super::snapshot::Row) -> bool {
    match row.get("epic_id") {
        None | Some(Value::Null) => true,
        Some(value) => value.as_i64() == Some(0),
    }
}
