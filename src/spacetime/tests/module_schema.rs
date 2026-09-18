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
            continue;
        }
        assert!(
            !expected.is_empty(),
            "`dump::source` says `{name}` is a SQLite table, but SQLite has no such table"
        );

        // Module-only columns EXTEND the expected list rather than being
        // filtered out of the actual one. That keeps the comparison positional:
        // an exempt column parked in the middle still fails. Their nullability
        // is read from the module rather than assumed, so a module-only column
        // that stopped being optional is drift too.
        for column in table.module_only_columns() {
            expected.push(Column {
                name: (*column).to_string(),
                nullable: module_columns
                    .iter()
                    .find(|c| c.name == *column)
                    .map(|c| c.nullable)
                    .unwrap_or_else(|| panic!("the module has no `{name}.{column}`")),
            });
        }

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
            module_columns, &expected,
            "`{name}` has drifted from SQLite. Columns are compared in order: \
             a column added anywhere but the end is a forbidden SpacetimeDB migration."
        );
    }
}

/// Every module-only column is the last thing in its table.
///
/// The parity test above would still pass if a module-only column were simply
/// dropped from its expected position, so the position is asserted directly.
/// This is the check that says "append, never insert" for exactly the columns
/// most likely to be tucked in beside a related field.
#[test]
fn module_only_columns_sit_at_the_end_of_their_table() {
    let module = module_tables();

    for table in SharedTable::ALL {
        let Some(last_exempt) = table.module_only_columns().last() else {
            continue;
        };
        let name = table.name();
        let columns = module
            .get(name)
            .unwrap_or_else(|| panic!("the module declares no table `{name}`"));
        let last = columns
            .last()
            .unwrap_or_else(|| panic!("`{name}` has no columns"));
        assert_eq!(
            &last.name, last_exempt,
            "`{name}.{last_exempt}` is not the last column of `{name}`"
        );
    }
}
