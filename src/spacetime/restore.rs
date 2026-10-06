//! Writing a snapshot back into the shared store, and clearing the id counters
//! afterwards.
//!
//! Spec: `docs/specs/spacetime-seed.allium` — the `RestoreSnapshot`,
//! `BurnIdSequences` and five `Refuse*` rules (`RefuseSeedingANonEmptyStore`
//! is `SeedSharedStore`'s, in `seed.rs`, not one of these five).

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

    if let Some(refusal) = schema_refusal(store, snapshot).await? {
        return Err(RestoreError::Refused(refusal));
    }

    if let Some(refusal) = snapshot.completeness_refusal() {
        return Err(RestoreError::Refused(refusal));
    }

    if let Some(refusal) = archived_rows_refusal(snapshot) {
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

/// Whether any extract describes a schema this store does not have.
///
/// **Asked up front, for every extract, before the burn and the first row.**
/// The alternative — noticing a stray column while encoding some row of some
/// table — arrives at a store whose id sequences are already burned and whose
/// earlier tables are already written, so the refusal lands on half a board and
/// the one promise `NothingIsWrittenBeforeTheChecksPass` makes is no longer
/// true. The per-row check in `cli_store` remains as a backstop; this is the
/// gate.
///
/// Compared as SETS. A restore writes rows by column name, so the order the
/// columns are listed in cannot change the outcome — see
/// [`super::snapshot::TableExtract::columns_differing_from`].
///
/// A table absent from the snapshot altogether is not this check's business; it
/// has no columns to disagree about, and `completeness_refusal` catches it.
pub(super) async fn schema_refusal(
    store: &dyn SharedStore,
    snapshot: &Snapshot,
) -> Result<Option<Refusal>, RestoreError> {
    for extract in snapshot.extracts() {
        let held = store
            .columns(extract.table)
            .await
            .map_err(RestoreError::Failed)?;
        let differing = extract.columns_differing_from(&held);
        if !differing.is_empty() {
            // The situation this tool is reached for is a migration the store
            // would not perform, which means the schema is precisely what
            // changed. Restoring old rows into a new schema without saying so
            // would produce exactly the quiet corruption the rebuild was meant
            // to escape.
            return Ok(Some(Refusal::new(
                RefusalReason::SchemaMismatch,
                format!(
                    "{} does not match this store's schema: {} present on one side only \
                     (snapshot has [{}], the store has [{}])",
                    extract.table.name(),
                    differing.join(", "),
                    extract.columns.join(", "),
                    held.join(", "),
                ),
            )));
        }
    }
    Ok(None)
}

/// Whether any task or epic row in the snapshot still carries the retired
/// `archived` status.
///
/// `spacetime-seed.allium: RefuseArchivedRows`. `archived` was retired by task
/// #4971 (`epics.allium: ArchivedStatusMigration`), which also rebuilds the
/// status CHECK constraints in SQLite so a live board can never write it back.
/// A store, though, CAN still hold archived rows: the module refuses new
/// writes with an unknown status, but rows an older module wrote remain, so a
/// `dump-server` of such a store produces a snapshot carrying them, as does a
/// snapshot FILE
/// written to disk by an older binary. There is deliberately no
/// store-side equivalent of that migration (see its own guidance for why a
/// second copy was declined), so the answer here is refusal rather than a
/// best-effort reconciliation:
/// restoring the row unchanged would either strand it (the client refuses to
/// decode an unrecognised status) or, with no `RetiredFeedItem` ever
/// backfilled for it, let its feed cycle re-insert it as if it were new.
///
/// Named row ids rather than a bare "archived rows present", because the
/// detail is what tells an operator which rows still need to go through
/// v106.
fn archived_rows_refusal(snapshot: &Snapshot) -> Option<Refusal> {
    const ARCHIVED_STATUS_TABLES: [SharedTable; 2] = [SharedTable::Tasks, SharedTable::Epics];

    let mut offenders: Vec<String> = Vec::new();
    for table in ARCHIVED_STATUS_TABLES {
        let Some(extract) = snapshot.extract(table) else {
            continue;
        };
        let ids: Vec<i64> = extract
            .rows
            .iter()
            .filter(|row| row.get("status").and_then(serde_json::Value::as_str) == Some("archived"))
            .filter_map(|row| row.get("id").and_then(serde_json::Value::as_i64))
            .collect();
        if !ids.is_empty() {
            let id_list = ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            offenders.push(format!("{} ({id_list})", table.name()));
        }
    }

    if offenders.is_empty() {
        return None;
    }

    Some(Refusal::new(
        RefusalReason::ArchivedRowsPresent,
        format!(
            "snapshot carries row(s) with the retired `archived` status: {}. archived was \
             removed by task #4971; run this snapshot's source through SQLite's v106 migration \
             and take a fresh dump before restoring or seeding it here",
            offenders.join("; ")
        ),
    ))
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
pub(super) fn implausible_ceiling(snapshot: &Snapshot) -> Option<Refusal> {
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
pub(super) async fn burn_id_sequences(
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
