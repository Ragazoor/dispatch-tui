//! Reading the shared domain out of SQLite into a [`Snapshot`].
//!
//! Spec: `docs/specs/spacetime-seed.allium` — the `TakeSnapshot` rule.
//!
//! This is the seed side. The shared store's own dump lives on
//! [`crate::spacetime::SharedStore::dump`]; both produce the same artefact, and
//! [`crate::spacetime::restore`] does not care which it is handed.

use anyhow::{Context, Result};
use rusqlite::types::ValueRef;
use rusqlite::Connection;

use crate::db::Database;
use crate::db::{HOST_ID_KEY, HOST_LABEL_KEY, USER_IDENTITY_KEY};

use super::snapshot::{Row, SharedTable, Snapshot, TableExtract};

/// Read every shared table out of the board's SQLite database.
///
/// **One read, not ten.** Every table is read inside a single deferred
/// transaction on one read connection. Ten separately-timed reads would let a
/// write land between two of them and produce a snapshot holding a todo whose
/// task is absent — which restores cleanly and leaves a board with dangling
/// references, the worst outcome available here because it looks fine.
///
/// **Rows travel whole.** Columns are read by name straight out of the result
/// set, with no field dropped, defaulted, recomputed or normalised. That
/// includes the denormalised counters on `tasks`: recomputing them on restore
/// would make their restored value depend on the restore code being right, and
/// the point of a backup is to depend on as little as possible.
pub async fn dump_from_sqlite(db: &Database) -> Result<Snapshot> {
    db.db_call_read(move |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .context("Failed to open the read transaction for a snapshot")?;

        let schema_version: i64 = tx
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("Failed to read the schema version")?;

        let mut extracts = Vec::with_capacity(SharedTable::ALL.len());
        for table in SharedTable::ALL {
            extracts.push(extract_table(&tx, table)?);
        }

        Ok(Snapshot::new(schema_version, extracts))
    })
    .await
}

/// Where one shared table's rows come from.
///
/// A total mapping, so adding a shared table forces a decision here rather than
/// falling through a default arm. One of the ten has no SQLite table at all,
/// and the knowledge of which is which belongs to the dump — `SharedTable` is
/// the store-neutral vocabulary that `restore`, the CLI store and the on-disk
/// artefact all share, and a predicate about SQLite's table inventory on it
/// would be one source's concern leaking into the domain type.
enum Source {
    /// A real SQLite table of the same name.
    SqliteTable,
    /// Assembled from this install's own machine identity, because that
    /// identity is exactly what the registry is a registry of, and because
    /// every task carrying a `host` needs that host to resolve to something on
    /// the far side.
    HostIdentity,
}

fn source(table: SharedTable) -> Source {
    match table {
        SharedTable::Tasks
        | SharedTable::Epics
        | SharedTable::Todos
        | SharedTable::TaskWatchers
        | SharedTable::TaskShells
        | SharedTable::TaskSubagents
        | SharedTable::RepoPaths
        | SharedTable::RepoBaseBranches
        // A real table since migration v98. It is usually empty — an install
        // that has never reached a shared store has no identity to subscribe
        // as — but empty is a fact this reads, not one it assumes.
        | SharedTable::Subscriptions => Source::SqliteTable,
        SharedTable::Hosts => Source::HostIdentity,
    }
}

/// Whether this table exists in SQLite at all.
///
/// Derived from [`source`] rather than listed again, so the eleventh shared
/// table answers this by the same arm the compiler already forces someone to
/// write. A second list would be identical by construction today and silently
/// wrong the first time the two were edited apart — and the reader who noticed
/// would be a test failing with "add it to the list", which is self-defeating in
/// exactly the case it fires.
/// Only the parity test consumes it today; the seeding client of
/// `spacetime-seed.allium: BackfillTaskOwner` is the production caller that
/// will. Gated rather than `allow(dead_code)` so the day it has a real caller,
/// removing the gate is the change.
#[cfg(test)]
pub(crate) fn is_sqlite_backed(table: SharedTable) -> bool {
    matches!(source(table), Source::SqliteTable)
}

fn extract_table(conn: &Connection, table: SharedTable) -> Result<TableExtract> {
    match source(table) {
        Source::SqliteTable => read_sqlite_table(conn, table),
        Source::HostIdentity => read_host_identity(conn, table),
    }
}

fn read_sqlite_table(conn: &Connection, table: SharedTable) -> Result<TableExtract> {
    // Ordered by the id where there is one, so a snapshot of an unchanged board
    // is byte-identical run to run and a diff between two backups is readable.
    let order = table
        .id_column()
        .map_or(String::new(), |c| format!(" ORDER BY {c}"));
    let sql = format!("SELECT * FROM {}{order}", table.name());
    let mut stmt = conn
        .prepare(&sql)
        .with_context(|| format!("Failed to prepare the read of {}", table.name()))?;
    let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let booleans = table.boolean_columns();

    let rows = stmt
        .query_map([], |row| {
            let mut out = Row::new();
            for (index, name) in columns.iter().enumerate() {
                let value = sqlite_value_to_json(row.get_ref(index)?)?;
                let value = if booleans.contains(&name.as_str()) {
                    canonical_boolean(&value)?
                } else {
                    value
                };
                out.insert(name.clone(), value);
            }
            Ok(out)
        })
        .with_context(|| format!("Failed to read {}", table.name()))?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .with_context(|| format!("Failed to decode a row of {}", table.name()))?;

    Ok(TableExtract::new(table, rows))
}

/// The host registry, assembled from this install's own identity.
///
/// The keys come from the settings module rather than being spelled inline:
/// that module's own doc says a rename spelled inline "would yield a statement
/// that silently matches nothing rather than a compile error", and here the
/// consequence of matching nothing is a complete-looking snapshot with an empty
/// `hosts` extract — a restored board on which no task's owning machine exists.
fn read_host_identity(conn: &Connection, table: SharedTable) -> Result<TableExtract> {
    let id: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [HOST_ID_KEY],
            |row| row.get(0),
        )
        .ok();
    let Some(id) = id else {
        // An install that has never minted a host identity has no host to
        // register. Empty rather than invented: a fabricated id would be a
        // machine that does not exist, and tasks would be pinned to it.
        return Ok(TableExtract::empty(table));
    };
    let label: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [HOST_LABEL_KEY],
            |row| row.get(0),
        )
        .ok();
    // The person this machine belongs to (`core.allium: Host.owner`). Null on
    // an install that has never reached a shared store, which is a real and
    // lasting state rather than a gap: the host id is minted offline on first
    // run, and no identity exists at that moment.
    let owner: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [USER_IDENTITY_KEY],
            |row| row.get(0),
        )
        .ok();
    let mut row = Row::new();
    row.insert("id".into(), serde_json::Value::String(id));
    row.insert(
        "label".into(),
        label.map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    row.insert(
        "owner".into(),
        owner.map_or(serde_json::Value::Null, serde_json::Value::String),
    );
    Ok(TableExtract::new(table, vec![row]))
}

/// SQLite's 0 and 1 as the snapshot's canonical `false` and `true`.
///
/// Narrow on purpose: a column the shared store types as a boolean, holding
/// anything but 0, 1 or null, is a corrupt row. Reading it as `true` would
/// write that corruption into the backup under a different name.
fn canonical_boolean(value: &serde_json::Value) -> rusqlite::Result<serde_json::Value> {
    match value {
        serde_json::Value::Null => Ok(serde_json::Value::Null),
        serde_json::Value::Bool(_) => Ok(value.clone()),
        _ => match value.as_i64() {
            Some(0) => Ok(serde_json::Value::Bool(false)),
            Some(1) => Ok(serde_json::Value::Bool(true)),
            _ => Err(rusqlite::Error::InvalidQuery),
        },
    }
}

/// SQLite's five storage classes, as JSON.
///
/// Integers stay integers. JSON has one number type, and an id that round-trips
/// as `4096.0` compares unequal to `4096` everywhere it matters while looking
/// right in a diff.
///
/// A BLOB has no JSON representation and no shared table currently stores one,
/// so one arriving here is a schema change nobody told this module about. It is
/// refused loudly rather than encoded on a guess — a backup that silently
/// mangles a column is worse than one that will not be taken.
fn sqlite_value_to_json(value: ValueRef<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::from(i),
        // A non-finite float has no JSON number. It cannot occur in the current
        // schema, and encoding it as null would put a silently wrong value in a
        // backup, so it is refused on the same grounds as a blob.
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or(rusqlite::Error::InvalidQuery)?,
        // Lossy on purpose: SQLite permits text that is not valid UTF-8, and a
        // dump that aborted on one would be a backup that cannot be taken.
        // Nothing in the shared schema stores bytes as text.
        ValueRef::Text(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
        ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
    })
}
