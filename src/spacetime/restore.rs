//! Writing a snapshot back into the shared store, and clearing the id counters
//! afterwards.
//!
//! Spec: `docs/specs/spacetime-seed.allium` — the `RestoreSnapshot`,
//! `BurnIdSequences` and three `Refuse*` rules.

use super::snapshot::{Refusal, RefusalReason, SharedTable, Snapshot, SNAPSHOT_FORMAT_VERSION};
use super::store::SharedStore;

/// Why a restore did not complete.
///
/// The two arms are genuinely different events and are kept apart on purpose.
/// A [`RestoreError::Refused`] is this tool declining to write, checked before
/// the first row, and the store is provably untouched. A
/// [`RestoreError::Failed`] is the store itself failing mid-operation, where
/// what the store holds afterwards is whatever its own atomicity gave us.
/// Collapsing the two would tell an operator "restore failed" in both cases and
/// leave them unable to tell whether retrying is safe.
#[derive(Debug)]
pub enum RestoreError {
    Refused(Refusal),
    Failed(anyhow::Error),
}

impl RestoreError {
    /// The refusal, or a panic naming what happened instead. Test scaffolding:
    /// a test that meant to exercise a refusal and instead hit a store failure
    /// should say so rather than assert against a defaulted value.
    #[cfg(any(test, feature = "test-support"))]
    pub fn into_refusal(self) -> Refusal {
        match self {
            RestoreError::Refused(refusal) => refusal,
            RestoreError::Failed(e) => panic!("expected a refusal, got a store failure: {e}"),
        }
    }
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::Refused(refusal) => write!(f, "{refusal}"),
            // `{e:#}` rather than `{e}`: the whole chain, because the useful
            // part of a store failure is the reducer's own message at the
            // bottom of it, and the top is only ever "failed to seed <table>".
            RestoreError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for RestoreError {}

/// Restore a snapshot into a shared store, then leave every id counter clear of
/// the rows that just arrived.
///
/// The burn is chained here rather than left to the operator as a second
/// command, because a restore whose burn was forgotten produces a store that
/// works perfectly until somebody creates a task — and then produces a
/// collision whose cause is hours of archaeology away from its symptom.
///
/// Every check runs before the first row is written. An operator who sees a
/// refusal knows the store is untouched, which is what makes it safe to fix the
/// input and retry, during exactly the incident where retrying is the only move
/// left.
pub async fn restore(store: &dyn SharedStore, snapshot: &Snapshot) -> Result<(), RestoreError> {
    if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
        // A newer snapshot read by an older tool is the dangerous direction:
        // the fields it does not understand are the ones it would drop.
        return Err(RestoreError::Refused(Refusal::new(
            RefusalReason::FormatUnsupported,
            format!(
                "snapshot format version {}, this tool reads {SNAPSHOT_FORMAT_VERSION}",
                snapshot.format_version
            ),
        )));
    }

    let store_schema = store.schema_version().await.map_err(RestoreError::Failed)?;
    if snapshot.schema_version != store_schema {
        // The situation this tool is reached for is a migration the store would
        // not perform, which means the schema is precisely what changed.
        // Restoring old rows into a new schema without saying so would produce
        // exactly the quiet corruption the rebuild was meant to escape.
        return Err(RestoreError::Refused(Refusal::new(
            RefusalReason::SchemaMismatch,
            format!(
                "snapshot describes schema version {}, the store holds {store_schema}",
                snapshot.schema_version
            ),
        )));
    }

    if let Some(refusal) = snapshot.completeness_refusal() {
        return Err(RestoreError::Refused(refusal));
    }

    if let Some(refusal) = implausible_ceiling(snapshot) {
        return Err(RestoreError::Refused(refusal));
    }

    // BURN FIRST, THEN LOAD. The burn advances the counter by generating and
    // discarding ids 1, 2, 3 and so on — exactly the ids about to be written.
    // Run afterwards, every one of those generated ids collides with a row that
    // is now there, and a rejected insert aborts the reducer. See
    // `SharedStore::advance_id_sequence_past`.
    burn_id_sequences(store, snapshot).await?;

    for extract in snapshot.extracts() {
        store
            .upsert_rows(extract.table, &extract.rows)
            .await
            .map_err(RestoreError::Failed)?;
    }
    Ok(())
}

/// The largest id a burn will chase, per row that claims it, beyond which the
/// snapshot is assumed corrupt rather than large.
///
/// The burn is O(ceiling), not O(rows), and the ceiling comes straight out of a
/// file an operator may have hand-edited during an incident. One extra digit
/// turns a four-thousand-iteration loop into a four-billion-iteration one
/// inside a single open reducer transaction. Refused up front, alongside the
/// other refusals, rather than discovered as a reducer that never returns.
///
/// The bound is relative rather than absolute: a board may legitimately have
/// large ids, but not ids wildly out of proportion to the number of rows
/// carrying them, because a counter only advances by being used.
const MAX_CEILING_PER_ROW: i64 = 1_000_000;

/// A ceiling so far above the rows that claim it that the snapshot is more
/// likely corrupt than the board is large.
fn implausible_ceiling(snapshot: &Snapshot) -> Option<Refusal> {
    for table in SharedTable::ALL
        .iter()
        .copied()
        .filter(|t| t.generates_ids())
    {
        let ceiling = snapshot.highest_id(table);
        let rows = snapshot.extract(table).map_or(0, |e| e.rows.len()) as i64;
        if ceiling < 0 {
            return Some(Refusal::new(
                RefusalReason::Incomplete,
                format!("{} carries a negative id ({ceiling})", table.name()),
            ));
        }
        if ceiling > rows.saturating_mul(MAX_CEILING_PER_ROW) {
            return Some(Refusal::new(
                RefusalReason::Incomplete,
                format!(
                    "{} claims a highest id of {ceiling} across only {rows} row(s). Burning \
                     the id counter that far would take hours inside one transaction, so this \
                     is treated as a corrupt snapshot rather than a large board.",
                    table.name()
                ),
            ));
        }
    }
    None
}

/// Leave every generating table's counter strictly past the highest id the
/// snapshot carried for it.
///
/// Per table, because each has its own counter and burning one does nothing for
/// another. A table with no rows has ceiling 0, its counter is already past
/// that, and the burn is a no-op rather than a special case.
async fn burn_id_sequences(
    store: &dyn SharedStore,
    snapshot: &Snapshot,
) -> Result<(), RestoreError> {
    for table in SharedTable::ALL
        .iter()
        .copied()
        .filter(|t| t.generates_ids())
    {
        store
            .advance_id_sequence_past(table, snapshot.highest_id(table))
            .await
            .map_err(RestoreError::Failed)?;
    }
    Ok(())
}
