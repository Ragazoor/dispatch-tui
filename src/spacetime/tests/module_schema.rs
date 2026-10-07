//! What the module's tables declare, read from its source.
//!
//! `spacetime/module/` is not in the workspace, so nothing compiles it during a
//! `cargo test`. The module source is parsed as text rather than reflected
//! over: reflection would need the crate linked into the dispatch binary,
//! which is the coupling the separate crate exists to avoid — see
//! `spacetime/module/README.md`. The parse is also what the snapshot fixtures
//! (`super::snapshot_of_a_populated_board`) take their column lists from.
//!
//! Column ORDER matters: SpacetimeDB permits a column to be appended and
//! refuses one inserted anywhere else. The authority on that is
//! `tests/spacetime_module.rs`, which publishes the committed module and then
//! automigrates the working tree's over it.

use crate::spacetime::SharedTable;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One column, as either schema describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Column {
    pub(super) name: String,
    pub(super) nullable: bool,
    pub(super) ty: String,
}

/// Every `.rs` file of the module, concatenated in name order. The module is
/// split by domain, so its tables no longer sit in one file.
fn module_source() -> String {
    let dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "spacetime", "module", "src"]
        .iter()
        .collect();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read the module source at {}: {e}", dir.display()))
        .map(|entry| entry.unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display())))
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            std::fs::read_to_string(path).unwrap_or_else(|e| {
                panic!("cannot read the module source at {}: {e}", path.display())
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `#[spacetimedb::table(accessor = ..)]` struct, by accessor name, with
/// its fields in declaration order.
///
/// Parsed rather than reflected; see this module's header for why. The parse is
/// deliberately strict — an unrecognised field line panics rather than being
/// skipped, because a silently dropped column is the failure this whole file is
/// about.
pub(super) fn module_tables() -> BTreeMap<String, Vec<Column>> {
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
        ty: ty.trim_end_matches(',').to_string(),
    }
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
