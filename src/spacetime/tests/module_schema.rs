//! The module's tables and SQLite's are the same tables.
//!
//! `spacetime/module/` is not in the workspace, so nothing compiles it during a
//! `cargo test` and no type system connects it to `src/db/migrations.rs`. The
//! two schemas are kept in step by this file and by nothing else.
//!
//! **Column ORDER is the point, not just membership.** SpacetimeDB permits a
//! column to be appended and refuses one inserted anywhere else, so a module
//! whose columns merely *match* SQLite's as a set is already unmigratable: the
//! first schema change lands in the middle. A set comparison would pass on
//! exactly the state this test exists to prevent, so every assertion below is
//! positional.
//!
//! The module source is parsed as text rather than reflected over. Reflection
//! would need the crate linked into the dispatch binary, which is the coupling
//! the separate crate exists to avoid — see `spacetime/module/README.md`.

use crate::db::Database;
use crate::spacetime::dump::is_sqlite_backed;
use crate::spacetime::SharedTable;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One column, as either schema describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Column {
    name: String,
    nullable: bool,
}

fn module_source() -> String {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "spacetime",
        "module",
        "src",
        "lib.rs",
    ]
    .iter()
    .collect();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read the module source at {}: {e}", path.display()))
}

/// Every `#[spacetimedb::table(accessor = ..)]` struct, by accessor name, with
/// its fields in declaration order.
///
/// Parsed rather than reflected; see this module's header for why. The parse is
/// deliberately strict — an unrecognised field line panics rather than being
/// skipped, because a silently dropped column is the failure this whole file is
/// about.
fn module_tables() -> BTreeMap<String, Vec<Column>> {
    let source = module_source();
    let mut tables = BTreeMap::new();
    let mut lines = source.lines().peekable();

    while let Some(line) = lines.next() {
        let Some(accessor) = accessor_name(line) else {
            continue;
        };
        // Skip whatever sits between the attribute and the struct: derives,
        // doc comments, further attributes.
        let mut columns = Vec::new();
        for inner in lines.by_ref() {
            let trimmed = inner.trim();
            if trimmed.starts_with("pub struct ") {
                break;
            }
        }
        for inner in lines.by_ref() {
            let trimmed = inner.trim();
            if trimmed == "}" {
                break;
            }
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            columns.push(parse_field(&accessor, trimmed));
        }
        tables.insert(accessor, columns);
    }
    tables
}

fn accessor_name(line: &str) -> Option<String> {
    let rest = line
        .trim()
        .strip_prefix("#[spacetimedb::table(accessor = ")?;
    let end = rest.find([',', ')'])?;
    Some(rest[..end].to_string())
}

fn parse_field(accessor: &str, line: &str) -> Column {
    let body = line
        .strip_prefix("pub ")
        .unwrap_or_else(|| panic!("{accessor}: cannot parse field line `{line}`"));
    let (name, ty) = body
        .split_once(": ")
        .unwrap_or_else(|| panic!("{accessor}: cannot parse field line `{line}`"));
    Column {
        name: name.to_string(),
        nullable: ty.trim_end_matches(',').starts_with("Option<"),
    }
}

/// SQLite's columns for one table, in `cid` order — which is declaration order,
/// and which `ALTER TABLE ADD COLUMN` only ever appends to.
async fn sqlite_columns(db: &Database, table: &str) -> Vec<Column> {
    let table = table.to_string();
    db.db_call(move |conn| {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = stmt
            .query_map([], |row| {
                // `notnull` alone is not the answer. SQLite reports an
                // `INTEGER PRIMARY KEY` — the rowid alias every id column here
                // is — as nullable unless it was *also* declared NOT NULL,
                // which none of them were. It cannot actually hold null, so
                // reading the flag literally would report drift on every
                // table's first column. The `pk` flag is what settles it.
                let not_null = row.get::<_, i64>(3)? != 0;
                let primary_key = row.get::<_, i64>(5)? != 0;
                Ok(Column {
                    name: row.get::<_, String>(1)?,
                    nullable: !not_null && !primary_key,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
    .unwrap()
}

/// The module declares exactly the shared tables, no more and no fewer.
///
/// `schema_version` is excluded on purpose: it describes the store rather than
/// the domain, and `SharedTable` deliberately omits it (see `snapshot.rs`).
#[test]
fn the_module_declares_every_shared_table_and_nothing_else() {
    let module = module_tables();

    let mut declared: Vec<&str> = module
        .keys()
        .map(String::as_str)
        .filter(|name| *name != "schema_version")
        .collect();
    declared.sort_unstable();

    let mut expected: Vec<&str> = SharedTable::ALL.iter().map(|t| t.name()).collect();
    expected.sort_unstable();

    assert_eq!(declared, expected);
}

/// Column for column, in order, for every table SQLite also has — and no SQLite
/// table at all for the ones that are module-only.
///
/// Both halves live here so the exemption is bounded from both sides in one
/// place: a table skipped as module-only is checked to really have no SQLite
/// counterpart in the same pass that skips it.
#[tokio::test]
async fn every_shared_table_matches_sqlite_column_for_column() {
    let db = Database::open_in_memory().await.unwrap();
    let module = module_tables();

    for table in SharedTable::ALL {
        let name = table.name();
        let module_columns = module
            .get(name)
            .unwrap_or_else(|| panic!("the module declares no table `{name}`"));
        let mut expected = sqlite_columns(&db, name).await;

        if !is_sqlite_backed(table) {
            assert!(
                expected.is_empty(),
                "SQLite now has a `{name}` table, but the dump still treats it as \
                 having no SQLite source — see `dump::source`"
            );
            // No SQLite counterpart to compare against, so the check runs
            // against the declared assembly instead. This is the branch that
            // used to skip outright, which left the one table whose row is
            // BUILT rather than selected as the only one nothing verified —
            // and building it is exactly where a column gets forgotten.
            let assembled: Vec<String> = table
                .assembled_columns()
                .iter()
                .map(|(column, _)| (*column).to_string())
                .collect();
            let declared: Vec<String> = module_columns.iter().map(|c| c.name.clone()).collect();
            assert_eq!(
                assembled, declared,
                "`{name}` is assembled rather than read, and its column list has \
                 drifted from the module's. Order matters here for the same \
                 reason it does everywhere else in this test."
            );
            continue;
        }
        assert!(
            !expected.is_empty(),
            "`dump::source` says `{name}` is a SQLite table, but SQLite has no such table"
        );

        // Module-only columns are checked separately, by presence and
        // nullability, and REMOVED from the actual list before the positional
        // comparison. They cannot be pinned to a position: SpacetimeDB only
        // ever appends, so once one is published every later shared column
        // lands after it and the two orders stop lining up. What still has to
        // hold — and what this compares — is that the SHARED columns appear in
        // SQLite's own order.
        let exempt = table.module_only_columns();
        for column in exempt {
            let declared = module_columns
                .iter()
                .find(|c| c.name == *column)
                .unwrap_or_else(|| panic!("the module has no `{name}.{column}`"));
            // The sentinel list decides, here as everywhere else: a module-only
            // column something might subscribe by carries `""` or `0` and is
            // required; one nothing subscribes by stays optional, because
            // SQLite supplies no value for it on migration. Checked in both
            // directions so neither an unlisted de-nullified column nor a
            // listed `Option` slips through.
            assert_eq!(
                declared.nullable,
                table.sentinel_for(column).is_none(),
                "`{name}.{column}` is module-only; its optionality must match \
                 whether `SharedTable::sentinel_columns` lists it"
            );
        }
        let shared: Vec<Column> = module_columns
            .iter()
            .filter(|c| !exempt.contains(&c.name.as_str()))
            .cloned()
            .collect();

        // A sentinel column is nullable in SQLite and REQUIRED in the module,
        // on purpose: SpacetimeDB SQL cannot filter on an optional column, so a
        // column anything might subscribe by carries `""` or `0` instead of a
        // null. See `SharedTable::sentinel_columns`.
        //
        // Applied to the expectation rather than excused on the actual, so the
        // check stays two-directional: a module column de-nullified WITHOUT
        // being on the list fails here, and a listed column still spelled
        // `Option` fails too. The list is the single declaration both this test
        // and the dump/restore conversion read.
        for (column, _) in table.sentinel_columns() {
            // A module-only column has no SQLite counterpart to reconcile, so
            // the loop above already took its nullability from the module.
            // `tasks.owner` is both, which is not a special case — a column
            // that exists only in the shared store and that something
            // subscribes by is exactly what this migration keeps adding.
            if table.module_only_columns().contains(column) {
                continue;
            }
            let entry = expected
                .iter_mut()
                .find(|c| c.name == *column)
                .unwrap_or_else(|| {
                    panic!(
                        "`{name}.{column}` is listed as a sentinel column but SQLite has no \
                         such column"
                    )
                });
            assert!(
                entry.nullable,
                "`{name}.{column}` is listed as a sentinel column, but SQLite already requires \
                 it — there is no null for a sentinel to stand in for, so the entry is wrong"
            );
            entry.nullable = false;
        }

        assert_eq!(
            &shared, &expected,
            "`{name}` has drifted from SQLite. The shared columns are compared in \
             order: one added anywhere but the end of the SQLite table is a \
             forbidden SpacetimeDB migration."
        );
    }
}

/// Every module-only column the snapshot layer names is actually declared by
/// the module, and nothing the module declares beyond SQLite's columns is
/// missing from that list.
///
/// The parity test above filters `module_only_columns()` out of the comparison,
/// so a name that drifted out of the module — or a new module column nobody
/// registered — would simply stop being checked. This closes that hole from
/// both sides.
///
/// It deliberately does NOT assert where those columns sit. SpacetimeDB's
/// append-only rule is about the module's own published history, not about
/// SQLite's column order, and the two diverge permanently the moment a
/// module-only column exists. The authority on "append, never insert" is
/// `tests/spacetime_module.rs`, which publishes the committed module and then
/// automigrates the working tree's over it.
#[tokio::test]
async fn the_module_only_column_list_matches_the_module() {
    let db = Database::open_in_memory().await.unwrap();
    let module = module_tables();

    for table in SharedTable::ALL {
        if !is_sqlite_backed(table) {
            continue;
        }
        let name = table.name();
        let columns = module
            .get(name)
            .unwrap_or_else(|| panic!("the module declares no table `{name}`"));
        let sqlite: Vec<String> = sqlite_columns(&db, name)
            .await
            .into_iter()
            .map(|c| c.name)
            .collect();

        let mut extra: Vec<&str> = columns
            .iter()
            .map(|c| c.name.as_str())
            .filter(|n| !sqlite.iter().any(|s| s == n))
            .collect();
        extra.sort_unstable();
        let mut registered: Vec<&str> = table.module_only_columns().to_vec();
        registered.sort_unstable();

        assert_eq!(
            extra, registered,
            "`{name}`: the module's columns that SQLite has no counterpart for \
             must be exactly the ones `SharedTable::module_only_columns` names"
        );
    }
}
