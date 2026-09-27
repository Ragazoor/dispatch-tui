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

use crate::db::{
    Database, HOST_ID_KEY, HOST_LABEL_KEY, USER_IDENTITY_KEY, USER_IDENTITY_TOKEN_KEY,
};

use super::snapshot::{Row, SharedTable, Snapshot, TableExtract};

/// Read every shared table out of the board's SQLite database.
///
/// **One read, not ten.** Every table is read inside a single deferred
/// transaction on one read connection. Ten separately-timed reads would let a
/// write land between two of them and produce a snapshot holding a watch whose
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

        let mut extracts = Vec::with_capacity(SharedTable::ALL.len());
        for table in SharedTable::ALL {
            extracts.push(extract_table(&tx, table)?);
        }

        Ok(Snapshot::new(extracts))
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
    /// No SQLite representation at all — not a real table, and nothing to
    /// assemble from a single-row identity either. A dump of a local
    /// install's board is always empty for this table by construction:
    /// `poll_owners` only ever gains rows once a shared store exists for two
    /// hosts to contend a claim over (`core.allium: PollOwner`), and a
    /// standalone SQLite board has never had a second host to contend with.
    /// The first table to use this arm. `task_shells` joined it in #4965,
    /// which dropped SQLite's copy along with the shell-tracking feature it
    /// backed (see `spacetime/module/src/lib.rs`'s `TaskShell` doc comment) —
    /// nothing local writes it any more, so a dump of it is empty the same
    /// way `poll_owners`'s is, for a different structural reason. `todos`
    /// joined it the same way in #4970, with the TODO overlay, and
    /// `filter_presets` in #4972, with saved repo-filter presets.
    Empty,
    /// Assembled from the local `settings` table's generic key/value rows,
    /// stamped with this install's own host id — the rows have no `host`
    /// column of their own to read, because a SQLite file has always held
    /// exactly one machine's settings. Excludes the four identity/credential
    /// keys, which are not `Setting` rows at all (`docs/specs/settings.allium`'s
    /// Excludes) — see `read_local_settings`.
    LocalSettings,
}

fn source(table: SharedTable) -> Source {
    match table {
        SharedTable::Tasks
        | SharedTable::Epics
        | SharedTable::TaskWatchers
        | SharedTable::TaskSubagents
        | SharedTable::RepoPaths
        | SharedTable::RepoBaseBranches
        // A real table since migration v98. It is usually empty — an install
        // that has never reached a shared store has no identity to subscribe
        // as — but empty is a fact this reads, not one it assumes.
        | SharedTable::Subscriptions
        // Real SQLite tables, unconditionally shared as of Phase 10 (task
        // #4914) — see the module's own doc comment on `Learning`.
        | SharedTable::Learnings
        | SharedTable::LearningRetrievals
        // Real SQLite table, unconditionally shared as of Phase 11 (task
        // #4915) — see the module's own doc comment on `UsageEvent`.
        | SharedTable::UsageEvents
        // Real SQLite table since migration v105 (task #4971) — see the
        // module's own doc comment on `RetiredFeedItem`.
        | SharedTable::RetiredFeedItems => Source::SqliteTable,
        SharedTable::Hosts => Source::HostIdentity,
        SharedTable::PollOwners
        | SharedTable::TaskShells
        | SharedTable::Todos
        | SharedTable::FilterPresets => Source::Empty,
        SharedTable::Settings => Source::LocalSettings,
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

/// Whether a real SQLite table of this table's own name exists, despite
/// [`is_sqlite_backed`] answering false for it.
///
/// The one case `is_sqlite_backed` alone cannot distinguish: `hosts` and
/// `poll_owners` are assembled because SQLite has NO such table at all, while
/// `settings` is assembled despite SQLite having one, because its shape (no
/// `id`, no `host`) is not the module's. The schema parity test needs to tell
/// the two apart — its "SQLite now has this table" guard would otherwise fire
/// on every run for it, rather than only when a real drift appears.
#[cfg(test)]
pub(crate) fn has_a_differently_shaped_sqlite_table(table: SharedTable) -> bool {
    matches!(source(table), Source::LocalSettings)
}

fn extract_table(conn: &Connection, table: SharedTable) -> Result<TableExtract> {
    match source(table) {
        Source::SqliteTable => read_sqlite_table(conn, table),
        Source::HostIdentity => read_host_identity(conn, table),
        Source::Empty => Ok(read_empty(table)),
        Source::LocalSettings => read_local_settings(conn, table),
    }
}

/// One `settings` value by key, or `None` if the row does not exist.
///
/// The single query both [`read_host_identity`] and [`local_host_id`] need —
/// factored out so there is exactly one place that spells it.
fn read_setting_value(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .ok()
}

/// This install's own host id, or `None` on a database that has never minted
/// one — the same absence [`read_host_identity`] treats as "nothing to
/// assemble" rather than an error.
fn local_host_id(conn: &Connection) -> Option<String> {
    read_setting_value(conn, HOST_ID_KEY)
}

/// The derived key of a `Setting` row: `"{host}/{key}"`. Must
/// agree character for character with the module's `host_scoped_id`.
fn host_scoped_id(host: &str, key: &str) -> String {
    format!("{host}/{key}")
}

/// Assemble `Setting` rows from the local `settings` table, stamped with this
/// install's own host id.
///
/// The four identity/credential keys are excluded — they are not `Setting`
/// rows (`docs/specs/settings.allium`'s Excludes) — by name rather than by
/// some structural marker, because that is genuinely all that distinguishes
/// them: same table, same two columns, no flag anywhere saying which four
/// rows are which.
fn read_local_settings(conn: &Connection, table: SharedTable) -> Result<TableExtract> {
    let names = assembled_column_names(table);
    let Some(host) = local_host_id(conn) else {
        return Ok(TableExtract::empty(table, names));
    };

    // Excluded in the query itself, not by a post-hoc filter: these four keys
    // are compile-time constants, never user input, so interpolating them into
    // the SQL is as safe as the equivalent `params![...]` binding would be.
    let sql = format!(
        "SELECT key, value FROM settings \
         WHERE key NOT IN ('{HOST_ID_KEY}', '{HOST_LABEL_KEY}', '{USER_IDENTITY_KEY}', '{USER_IDENTITY_TOKEN_KEY}') \
         ORDER BY key"
    );
    let mut stmt = conn
        .prepare(&sql)
        .context("Failed to prepare the read of settings")?;
    let rows = stmt
        .query_map([], |row| {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            let mut extracted = Row::new();
            extracted.insert(
                "id".to_string(),
                serde_json::Value::String(host_scoped_id(&host, &key)),
            );
            extracted.insert("host".to_string(), serde_json::Value::String(host.clone()));
            extracted.insert("key".to_string(), serde_json::Value::String(key));
            extracted.insert("value".to_string(), serde_json::Value::String(value));
            Ok(extracted)
        })
        .context("Failed to read settings")?
        .collect::<rusqlite::Result<Vec<Row>>>()
        .context("Failed to decode a row of settings")?;

    Ok(TableExtract::new(table, names, rows))
}

/// Always an empty extract, naming its columns from
/// [`SharedTable::assembled_columns`] the same way [`read_host_identity`]
/// does — the shared parity test compares against that list either way, and
/// giving both `Source::HostIdentity` and `Source::Empty` the same expected
/// column source is what keeps a column appended to `poll_owners` and
/// forgotten here caught by that test rather than silently dropped.
/// The bare column names of `table.assembled_columns()`, discarding the
/// settings-key half of each pair — shared by every reader that needs to
/// name a table's columns without reading any of its rows.
fn assembled_column_names(table: SharedTable) -> Vec<String> {
    table
        .assembled_columns()
        .iter()
        .map(|(column, _)| (*column).to_owned())
        .collect()
}

fn read_empty(table: SharedTable) -> TableExtract {
    TableExtract::empty(table, assembled_column_names(table))
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

    Ok(TableExtract::new(table, columns, rows))
}

/// The host registry, assembled from this install's own identity.
///
/// The keys come from the settings module rather than being spelled inline:
/// that module's own doc says a rename spelled inline "would yield a statement
/// that silently matches nothing rather than a compile error", and here the
/// consequence of matching nothing is a complete-looking snapshot with an empty
/// `hosts` extract — a restored board on which no task's owning machine exists.
fn read_host_identity(conn: &Connection, table: SharedTable) -> Result<TableExtract> {
    // No id, no host. An install that has never minted one has no machine to
    // register, and empty is the honest answer: a fabricated id would be a
    // machine that does not exist, with tasks pinned to it.
    let columns = table.assembled_columns();
    let (_, id_key) = columns
        .first()
        .ok_or_else(|| anyhow::anyhow!("{} has no assembled columns", table.name()))?;
    let names = assembled_column_names(table);
    if read_setting_value(conn, id_key).is_none() {
        // Still names its columns. An extract that named none would make no
        // claim about its schema, and a restore cannot tell "no claim" from
        // "matches".
        return Ok(TableExtract::empty(table, names));
    }

    // Built by walking the declared column list rather than field by field, so
    // a column appended to the module and forgotten here is a column this loop
    // still emits — and, failing that, one the schema parity test names. See
    // `SharedTable::assembled_columns`.
    let mut row = Row::new();
    for (column, key) in columns {
        row.insert(
            (*column).to_string(),
            read_setting_value(conn, key)
                .map_or(serde_json::Value::Null, serde_json::Value::String),
        );
    }
    Ok(TableExtract::new(table, names, vec![row]))
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
/// A BLOB becomes a JSON array of byte values — `learnings.embedding`'s shape
/// (Phase 10, task #4914) — matching the array-of-numbers form
/// `spacetime sql --format json` already uses for a `Vec<u8>` column
/// (`decode_row`'s non-optional arm passes it through unchanged), so a dump
/// taken from SQLite and one taken from the live store compare equal. Before
/// this column existed, an arriving BLOB was a schema change nobody told this
/// module about and was refused loudly rather than encoded on a guess; that
/// refusal stays the answer for anything that is not a plain byte vector,
/// should a future column need one.
fn sqlite_value_to_json(value: ValueRef<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::from(i),
        // A non-finite float has no JSON number. It cannot occur in the current
        // schema, and encoding it as null would put a silently wrong value in a
        // backup, so it is refused on the same grounds as an unrecognised blob.
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or(rusqlite::Error::InvalidQuery)?,
        // Lossy on purpose: SQLite permits text that is not valid UTF-8, and a
        // dump that aborted on one would be a backup that cannot be taken.
        // Nothing in the shared schema stores bytes as text.
        ValueRef::Text(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
        ValueRef::Blob(bytes) => {
            serde_json::Value::Array(bytes.iter().map(|b| serde_json::Value::from(*b)).collect())
        }
    })
}
