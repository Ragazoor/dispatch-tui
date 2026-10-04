//! Importing the old shared store into the managed one: an add-only copy.
//!
//! Spec: `docs/specs/spacetime-seed.allium` — `ImportOldStore`,
//! `DependentsOfDroppedRowsAreDropped`, `OrphansLeaveTheirDroppedEpic`, and
//! the `ImportNeverTouchesItsSource` / `ImportOnlyAdds` guarantees.
//!
//! The source is a [`Snapshot`] read from the old store (see
//! `cli::store_import` for how it is obtained) and is only ever borrowed. The
//! target is written to by key, never overwritten and never deleted from.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::restore::{burn_id_sequences, implausible_ceiling, schema_refusal, RestoreError};
use super::snapshot::{
    Refusal, RefusalReason, Row, SharedTable, Snapshot, TableExtract, SNAPSHOT_FORMAT_VERSION,
};
use super::store::{key_columns, row_key, SharedStore};

/// What an import did, per table. Printed to the operator.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Tasks that lost their epic and were handed to the importer, by id.
    pub orphaned: Vec<i64>,
    imported: BTreeMap<SharedTable, usize>,
    kept: BTreeMap<SharedTable, usize>,
    dropped: BTreeMap<SharedTable, usize>,
}

impl ImportReport {
    pub fn imported(&self, table: SharedTable) -> usize {
        self.imported.get(&table).copied().unwrap_or(0)
    }
    pub fn kept(&self, table: SharedTable) -> usize {
        self.kept.get(&table).copied().unwrap_or(0)
    }
    pub fn dropped(&self, table: SharedTable) -> usize {
        self.dropped.get(&table).copied().unwrap_or(0)
    }
    pub fn imported_total(&self) -> usize {
        self.imported.values().sum()
    }
    pub fn kept_total(&self) -> usize {
        self.kept.values().sum()
    }
    pub fn dropped_total(&self) -> usize {
        self.dropped.values().sum()
    }

    /// One line per table that had anything to say, then the orphans.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for table in SharedTable::ALL {
            let (i, k, d) = (self.imported(table), self.kept(table), self.dropped(table));
            if i + k + d == 0 {
                continue;
            }
            out.push_str(&format!(
                "  {:<22} imported {i:>6}   already there {k:>6}   dropped {d:>6}\n",
                table.name()
            ));
        }
        if !self.orphaned.is_empty() {
            let ids: Vec<String> = self.orphaned.iter().map(i64::to_string).collect();
            out.push_str(&format!(
                "  {} task(s) sit in an archived epic and have no epic here: {}\n",
                self.orphaned.len(),
                ids.join(", ")
            ));
        }
        out
    }
}

/// Columns, beyond the key, that make a source row "the same fact" as a target
/// row. A repo is its path, a base branch its pair, a watch its pair, a setting
/// its host and key.
fn natural_key(table: SharedTable) -> &'static [&'static str] {
    match table {
        SharedTable::RepoPaths => &["path"],
        SharedTable::RepoBaseBranches => &["repo_path", "branch"],
        SharedTable::TaskWatchers => &["watcher_task_id", "target_task_id"],
        SharedTable::Settings => &["host", "key"],
        _ => &[],
    }
}

fn natural_identity(table: SharedTable, row: &Row) -> Option<String> {
    let columns = natural_key(table);
    if columns.is_empty() {
        return None;
    }
    Some(
        columns
            .iter()
            .map(|c| row.get(*c).map(Value::to_string).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\u{1}"),
    )
}

fn id_of(row: &Row, column: &str) -> Option<i64> {
    row.get(column).and_then(Value::as_i64)
}

fn is_dropped_status(row: &Row) -> bool {
    row.get("status")
        .and_then(Value::as_str)
        .is_some_and(crate::models::is_removed_status)
}

/// Copy every row `source` has and `target` lacks into `target`.
///
/// Spec: `ImportOldStore`. Every check runs before the burn and the first row,
/// so a refusal leaves the target untouched.
pub async fn import_old_store(
    target: &dyn SharedStore,
    source: &Snapshot,
    operator: &str,
) -> Result<ImportReport, RestoreError> {
    if source.format_version != SNAPSHOT_FORMAT_VERSION {
        return Err(RestoreError::Refused(Refusal::new(
            RefusalReason::FormatUnsupported,
            format!(
                "source snapshot format version {}, this tool reads {SNAPSHOT_FORMAT_VERSION}",
                source.format_version
            ),
        )));
    }
    if let Some(refusal) = schema_refusal(target, source).await? {
        return Err(RestoreError::Refused(refusal));
    }
    if let Some(refusal) = source.completeness_refusal() {
        return Err(RestoreError::Refused(refusal));
    }

    let mut report = ImportReport::default();
    let prepared = prepare(source, operator, &mut report);

    if let Some(refusal) = implausible_ceiling(&prepared) {
        return Err(RestoreError::Refused(refusal));
    }

    // Decide what is missing before writing anything, so an id conflict is a
    // refusal and not a half-finished import.
    let mut to_write: Vec<(SharedTable, Vec<Row>)> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for extract in prepared.extracts() {
        let table = extract.table;
        let held = target.rows(table).await.map_err(RestoreError::Failed)?;
        let by_key: BTreeMap<String, &Row> = held.iter().map(|r| (row_key(table, r), r)).collect();
        let natural: BTreeSet<String> = held
            .iter()
            .filter_map(|r| natural_identity(table, r))
            .collect();

        let mut missing = Vec::new();
        for row in &extract.rows {
            if let Some(existing) = by_key.get(&row_key(table, row)) {
                if differs_in_creation(table, existing, row) {
                    conflicts.push(format!("{} {}", table.name(), row_key_display(table, row)));
                }
                *report.kept.entry(table).or_default() += 1;
            } else if natural_identity(table, row).is_some_and(|n| natural.contains(&n)) {
                *report.kept.entry(table).or_default() += 1;
            } else {
                missing.push(row.clone());
            }
        }
        report.imported.insert(table, missing.len());
        to_write.push((table, missing));
    }
    if !conflicts.is_empty() {
        return Err(RestoreError::Refused(Refusal::new(
            RefusalReason::IdConflict,
            format!(
                "the target already uses these ids for different rows: {}. Nothing was \
                 written. Import into a store that has not been used yet, or decide which \
                 store's rows win first.",
                conflicts.join(", ")
            ),
        )));
    }

    burn_id_sequences(target, &prepared).await?;
    for (table, rows) in &to_write {
        if rows.is_empty() {
            continue;
        }
        target
            .upsert_rows(*table, rows)
            .await
            .map_err(RestoreError::Failed)?;
    }
    report.imported.retain(|_, n| *n > 0);
    Ok(report)
}

/// Same key, different `created_at`: two different things sharing a number.
/// Only tables that carry the column can say; the rest are taken as the same.
fn differs_in_creation(table: SharedTable, existing: &Row, incoming: &Row) -> bool {
    if !matches!(
        table,
        SharedTable::Tasks | SharedTable::Epics | SharedTable::Learnings
    ) {
        return false;
    }
    match (existing.get("created_at"), incoming.get("created_at")) {
        (Some(a), Some(b)) if !a.is_null() && !b.is_null() => a != b,
        _ => false,
    }
}

fn row_key_display(table: SharedTable, row: &Row) -> String {
    key_columns(table)
        .iter()
        .map(|c| row.get(*c).map(Value::to_string).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("/")
}

/// The source with removed-status rows and their dependents gone, and orphans
/// re-homed. A copy: the source is never altered.
fn prepare(source: &Snapshot, operator: &str, report: &mut ImportReport) -> Snapshot {
    let ids_of = |table: SharedTable| -> BTreeSet<i64> {
        source
            .extract(table)
            .map(|e| {
                e.rows
                    .iter()
                    .filter(|r| is_dropped_status(r))
                    .filter_map(|r| id_of(r, "id"))
                    .collect()
            })
            .unwrap_or_default()
    };
    let dropped_tasks = ids_of(SharedTable::Tasks);
    let dropped_epics = ids_of(SharedTable::Epics);

    let mut extracts = Vec::new();
    for extract in source.extracts() {
        let table = extract.table;
        let mut rows = Vec::with_capacity(extract.rows.len());
        for row in &extract.rows {
            let drop = match table {
                SharedTable::Tasks | SharedTable::Epics => is_dropped_status(row),
                SharedTable::TaskSubagents => {
                    id_of(row, "task_id").is_some_and(|t| dropped_tasks.contains(&t))
                }
                SharedTable::TaskWatchers => {
                    [id_of(row, "watcher_task_id"), id_of(row, "target_task_id")]
                        .into_iter()
                        .flatten()
                        .any(|t| dropped_tasks.contains(&t))
                }
                SharedTable::Subscriptions => {
                    id_of(row, "epic_id").is_some_and(|e| dropped_epics.contains(&e))
                }
                SharedTable::PollOwners => {
                    row.get("scope").and_then(Value::as_str) == Some("epic")
                        && id_of(row, "scope_id").is_some_and(|e| dropped_epics.contains(&e))
                }
                _ => false,
            };
            if drop {
                *report.dropped.entry(table).or_default() += 1;
                continue;
            }
            let mut row = row.clone();
            match table {
                SharedTable::Tasks => {
                    if id_of(&row, "epic_id").is_some_and(|e| dropped_epics.contains(&e)) {
                        row.insert("epic_id".into(), Value::from(0));
                        row.insert("owner".into(), Value::from(operator));
                        if let Some(id) = id_of(&row, "id") {
                            report.orphaned.push(id);
                        }
                    }
                }
                SharedTable::Epics
                    if id_of(&row, "parent_epic_id")
                        .is_some_and(|e| dropped_epics.contains(&e)) =>
                {
                    row.insert("parent_epic_id".into(), Value::from(0));
                }
                _ => {}
            }
            rows.push(row);
        }
        extracts.push(TableExtract::new(table, extract.columns.clone(), rows));
    }
    report.orphaned.sort_unstable();
    Snapshot::new(extracts)
}
