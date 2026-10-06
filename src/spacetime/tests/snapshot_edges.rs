//! Edges of the snapshot model that the restore tests only reach indirectly:
//! how a refusal reads to an operator, and what a table with no generated id
//! reports for its highest id.

use crate::spacetime::{Refusal, RefusalReason, SharedTable, TableExtract};

/// An operator reads the refusal, so each reason must name itself and carry
/// the detail — the reason alone does not say which row or column.
#[test]
fn every_refusal_reason_names_itself_and_keeps_its_detail() {
    let cases = [
        (
            RefusalReason::FormatUnsupported,
            "unsupported snapshot format",
        ),
        (RefusalReason::SchemaMismatch, "schema mismatch"),
        (RefusalReason::Incomplete, "incomplete snapshot"),
        (RefusalReason::ArchivedRowsPresent, "archived rows present"),
        (RefusalReason::IdConflict, "id conflict"),
    ];

    for (reason, text) in cases {
        let refusal = Refusal::new(reason, "some detail");
        assert_eq!(refusal.to_string(), format!("{text}: some detail"));
    }
}

/// A table whose rows are not identified by a generated id has nothing to burn,
/// so its highest id is 0 whatever the rows hold.
#[test]
fn a_table_without_a_generated_id_reports_zero_as_its_highest_id() {
    let extract = TableExtract::empty(SharedTable::Hosts, vec!["id".to_string()]);

    assert_eq!(SharedTable::Hosts.id_column(), None);
    assert_eq!(extract.highest_id(), 0);
}

/// Every table answers the per-column questions the dump and restore ask,
/// including the tables that have nothing to say.
#[test]
fn every_table_answers_its_column_questions() {
    for table in SharedTable::ALL {
        let _ = table.sentinel_columns();
        let _ = table.assembled_columns();
        let _ = table.module_only_columns();
        let _ = table.boolean_columns();
        let _ = table.generates_ids();
    }
    assert!(!SharedTable::RepoPaths.sentinel_columns().is_empty());
    assert!(!SharedTable::Hosts.sentinel_columns().is_empty());
    assert!(SharedTable::Learnings.module_only_columns().is_empty());
}
