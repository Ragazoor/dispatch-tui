//! `startup.allium`'s `StoreAddressRecord` contract, and `cli.allium`'s
//! `CliCommandsReachTheStoreWithoutManagingIt` resolution order (task #4982).
//!
//! The record is a file named `store-server` in the same folder as the
//! `--db` database file. Every test here works in its own temp directory, so
//! nothing touches the operator's real data directory.

use super::{cli_store_server, forget_store_server, record_store_server, recorded_store_server};
use std::path::{Path, PathBuf};

const MANAGED: &str = "http://127.0.0.1:3000";

/// A database path inside a fresh temp directory. The database file
/// itself never needs to exist: the record is about its folder.
fn db_in(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("dispatch.db")
}

/// Where the record lives: beside the database file
/// (`TheRecordBelongsToItsDatabase`).
fn record_file(db_path: &Path) -> PathBuf {
    db_path.parent().unwrap().join("store-server")
}

fn write_record(db_path: &Path, contents: &str) {
    std::fs::write(record_file(db_path), contents).unwrap();
}

// -- recorded_store_server ------------------------------------------------

#[test]
fn a_recorded_address_is_read_back_from_beside_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://team:3000\n");

    assert_eq!(
        recorded_store_server(&db),
        Some("http://team:3000".to_string())
    );
}

#[test]
fn no_record_reads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(recorded_store_server(&db_in(&dir)), None);
}

/// A blank record is null for the same reason a blank
/// DISPATCH_SPACETIME_SERVER is.
#[test]
fn a_blank_record_reads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "  \n\t");
    assert_eq!(recorded_store_server(&db), None);
}

// -- record_store_server --------------------------------------------------

#[test]
fn a_recorded_address_is_written_beside_the_database_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);

    assert!(record_store_server(&db, "http://team:3000"));

    let written = std::fs::read_to_string(record_file(&db))
        .expect("the record must be a file beside the database");
    assert_eq!(written.trim(), "http://team:3000");
    assert_eq!(
        recorded_store_server(&db),
        Some("http://team:3000".to_string())
    );
}

#[test]
fn recording_replaces_whatever_was_there() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://old:3000\n");

    assert!(record_store_server(&db, "http://new:3000"));

    assert_eq!(
        recorded_store_server(&db),
        Some("http://new:3000".to_string())
    );
}

/// `TheRecordBelongsToItsDatabase`: a board on a throwaway --db records
/// nothing next to the operator's real one, and a command run with the
/// throwaway --db finds the throwaway board's store and no other.
#[test]
fn the_record_belongs_to_its_database() {
    let throwaway = tempfile::tempdir().unwrap();
    let real = tempfile::tempdir().unwrap();

    assert!(record_store_server(
        &db_in(&throwaway),
        "http://scratch:3099"
    ));

    assert_eq!(recorded_store_server(&db_in(&real)), None);
    assert!(!record_file(&db_in(&real)).exists());
    assert_eq!(
        recorded_store_server(&db_in(&throwaway)),
        Some("http://scratch:3099".to_string())
    );
}

/// A record that cannot be written is reported, not raised: the caller
/// logs it and goes on (`RecordTheNamedStoreOnceItAnswers`, best-effort).
#[test]
fn a_record_that_cannot_be_written_reports_false() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    // A directory where the file would go: no write can replace it.
    std::fs::create_dir(record_file(&db)).unwrap();

    assert!(!record_store_server(&db, "http://team:3000"));
}

// -- forget_store_server --------------------------------------------------

#[test]
fn forgetting_removes_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://team:3000\n");

    assert!(forget_store_server(&db));

    assert!(!record_file(&db).exists());
    assert_eq!(recorded_store_server(&db), None);
}

#[test]
fn forgetting_a_record_that_is_not_there_is_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    assert!(forget_store_server(&db_in(&dir)));
}

// -- cli_store_server: first answer wins ----------------------------------

#[test]
fn the_flag_wins_over_the_environment_and_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://recorded:3000\n");

    assert_eq!(
        cli_store_server(
            Some("http://flag:3000".into()),
            Some("http://env:3000".into()),
            Some("http://board:3000".into()),
            &db
        ),
        "http://flag:3000"
    );
}

#[test]
fn the_environment_wins_over_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://recorded:3000\n");

    assert_eq!(
        cli_store_server(None, Some("http://env:3000".into()), None, &db),
        "http://env:3000"
    );
}

/// The point of the record: a command in a plain terminal, with nothing
/// named, reaches the store the board on this --db is using.
#[test]
fn the_record_wins_over_the_managed_address() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://recorded:3000\n");

    assert_eq!(
        cli_store_server(None, None, None, &db),
        "http://recorded:3000"
    );
}

#[test]
fn nothing_named_or_recorded_reaches_the_managed_address() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(cli_store_server(None, None, None, &db_in(&dir)), MANAGED);
}

/// `named_store`: "--spacetime-server, else DISPATCH_SPACETIME_SERVER;
/// blank counts as absent" -- a blank flag is no answer, so the
/// environment is asked next.
#[test]
fn a_blank_flag_falls_through_to_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        cli_store_server(
            Some("  ".into()),
            Some("http://env:3000".into()),
            None,
            &db_in(&dir)
        ),
        "http://env:3000"
    );
}

#[test]
fn a_blank_environment_falls_through_to_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://recorded:3000\n");

    assert_eq!(
        cli_store_server(Some(String::new()), Some(" \n".into()), None, &db),
        "http://recorded:3000"
    );
}

#[test]
fn a_blank_record_falls_through_to_the_managed_address() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "\n");

    assert_eq!(cli_store_server(None, None, None, &db), MANAGED);
}

// -- the board's published address (DISPATCH_BOARD_STORE) ------------------

/// cli.allium: the operator's variable comes before the board's.
#[test]
fn the_operators_environment_wins_over_the_boards_address() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        cli_store_server(
            None,
            Some("http://env:3000".into()),
            Some("http://board:3000".into()),
            &db_in(&dir)
        ),
        "http://env:3000"
    );
}

#[test]
fn the_boards_address_wins_over_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://recorded:3000\n");

    assert_eq!(
        cli_store_server(None, None, Some("http://board:3000".into()), &db),
        "http://board:3000"
    );
}

#[test]
fn a_blank_boards_address_falls_through_to_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_in(&dir);
    write_record(&db, "http://recorded:3000\n");

    assert_eq!(
        cli_store_server(None, None, Some("  ".into()), &db),
        "http://recorded:3000"
    );
}
