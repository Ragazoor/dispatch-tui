//! The `spacetime`-CLI-backed store: what it sends, and what it makes of what
//! comes back.
//!
//! # Why the fixtures are captured, not written
//!
//! `spacetime sql --format json` emits a schema and then each row as a
//! POSITIONAL array, with optional columns as `[0, value]` or `[1, []]`. A
//! fixture written by hand is a guess at that format; these were captured from
//! a real instance (SpacetimeDB 2.10.1) running this repo's module, so a test
//! passing here is evidence about the real wire format.
//!
//! If a future CLI changes the format, recapture them — see the "SpacetimeDB"
//! section of `docs/reference.md` for the commands. Do not edit them to match
//! the code.

use std::process::Output;
use std::sync::Arc;

use crate::process::MockProcessRunner;
use crate::spacetime::{SharedStore, SharedTable, SpacetimeCliStore};

use MockProcessRunner as Mock;

const TASKS_JSON: &str = include_str!("fixtures/sql_tasks.json");
const EMPTY_JSON: &str = include_str!("fixtures/sql_repo_paths_empty.json");
/// A real `epics` schema. Kept because it is the table that still HAS an
/// optional column — `sort_order`, the one deliberate exception to the sentinel
/// rule (`SharedTable::sentinel_columns`). After that change `repo_paths` has
/// none at all, so the optional-wrapping asymmetry needs a table that does.
const EPICS_EMPTY_JSON: &str = include_str!("fixtures/sql_epics_empty.json");
const TODOS_EMPTY_JSON: &str = include_str!("fixtures/sql_todos_empty.json");
const SCHEMA_VERSION_JSON: &str = include_str!("fixtures/sql_schema_version.json");

fn ok(stdout: &str) -> anyhow::Result<Output> {
    Mock::ok_with_stdout(stdout.as_bytes())
}

/// A `spacetime sql` response carrying a real captured schema and `rows`.
///
/// The schemas in `fixtures/` are captured from a live instance rather than
/// written by hand, because what a column's shape looks like on the wire is
/// exactly the thing this module keeps getting surprised by. A test that
/// invented one would be testing its own invention.
fn response_with(schema_fixture: &str, rows: serde_json::Value) -> String {
    let mut parsed: serde_json::Value =
        serde_json::from_str(schema_fixture).expect("fixture is not JSON");
    parsed[0]["rows"] = rows;
    parsed.to_string()
}

fn failed(stderr: &str) -> anyhow::Result<Output> {
    Mock::fail(stderr)
}

fn store(responses: Vec<anyhow::Result<Output>>) -> (SpacetimeCliStore, Arc<MockProcessRunner>) {
    let runner = Arc::new(MockProcessRunner::new(responses));
    let store = SpacetimeCliStore::new(
        Arc::clone(&runner) as Arc<dyn crate::process::ProcessRunner>,
        "dispatch",
        Some("http://127.0.0.1:3099".into()),
    );
    (store, runner)
}

// ---------------------------------------------------------------------------
// Decoding what comes back
// ---------------------------------------------------------------------------

/// Positional rows become named columns, and both arms of an optional decode.
#[tokio::test]
async fn positional_rows_decode_into_named_columns() {
    let (store, _) = store(vec![ok(TASKS_JSON)]);

    let rows = store.rows(SharedTable::Tasks).await.unwrap();

    assert!(!rows.is_empty(), "fixture holds no rows");
    let first = &rows[0];
    assert!(
        first.contains_key("id") && first.contains_key("title"),
        "columns did not get their names back: {first:?}"
    );
}

/// `[1, []]` is absent, and must come back as null rather than as the literal
/// pair. A pair left undecoded looks like data and serialises into the snapshot.
#[tokio::test]
async fn an_absent_optional_decodes_to_null() {
    let (store, _) = store(vec![ok(TASKS_JSON)]);

    let rows = store.rows(SharedTable::Tasks).await.unwrap();

    let absent = rows
        .iter()
        .find(|r| r.get("title").and_then(|v| v.as_str()) == Some("t3-v2"))
        .expect("fixture has no t3-v2 row");
    assert_eq!(
        absent.get("worktree"),
        Some(&serde_json::Value::Null),
        "an absent optional did not decode to null"
    );
}

/// `[0, value]` is present, and must decode to the bare value.
#[tokio::test]
async fn a_present_optional_decodes_to_its_value() {
    let (store, _) = store(vec![ok(TASKS_JSON)]);

    let rows = store.rows(SharedTable::Tasks).await.unwrap();

    let present = rows
        .iter()
        .find(|r| r.get("title").and_then(|v| v.as_str()) == Some("opt"))
        .expect("fixture has no row with a present optional — recapture it");
    assert_eq!(
        present.get("worktree").and_then(|v| v.as_str()),
        Some("/wt/7"),
        "a present optional did not decode to its value"
    );
}

/// Integer ids stay integers through the decode. An id that becomes a float
/// compares unequal to itself everywhere it matters while looking right.
#[tokio::test]
async fn ids_decode_as_integers() {
    let (store, _) = store(vec![ok(TASKS_JSON)]);

    let rows = store.rows(SharedTable::Tasks).await.unwrap();

    for row in &rows {
        assert!(
            row.get("id").is_some_and(serde_json::Value::is_i64),
            "id decoded as {:?}",
            row.get("id")
        );
    }
}

/// An empty table returns its schema and no rows, which is how
/// [`SpacetimeCliStore::column_shapes`] learns a shape before a restore has
/// written anything.
#[tokio::test]
async fn an_empty_result_decodes_to_no_rows_rather_than_an_error() {
    let (store, _) = store(vec![ok(EMPTY_JSON)]);

    assert!(store.rows(SharedTable::RepoPaths).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_schema_version_is_read_from_the_store() {
    let (store, _) = store(vec![ok(SCHEMA_VERSION_JSON)]);

    assert_eq!(store.schema_version().await.unwrap(), 97);
}

/// A database published from a module that has no `schema_version` table says
/// so, rather than defaulting to a number that would let a restore proceed.
#[tokio::test]
async fn a_missing_schema_version_is_an_error_not_a_default() {
    let (store, _) = store(vec![failed("table schema_version not found")]);

    let error = store.schema_version().await.unwrap_err();

    assert!(
        format!("{error:#}").contains("schema_version"),
        "unhelpful error: {error:#}"
    );
}

/// A wire format this decoder does not recognise is an error, never a defaulted
/// value. A backup that silently decodes a column as null is worse than one
/// that will not be taken.
#[tokio::test]
async fn an_unrecognised_wire_format_is_refused() {
    let (store, _) = store(vec![ok(r#"[{"schema":{"elements":[]},"rows":[[1,2]]}]"#)]);

    let error = store.rows(SharedTable::Tasks).await.unwrap_err();

    assert!(
        format!("{error:#}").contains("wire format changed"),
        "unhelpful error: {error:#}"
    );
}

// ---------------------------------------------------------------------------
// What it sends
// ---------------------------------------------------------------------------

/// The burn is one call with the table and the ceiling, not one call per id.
/// The loop lives inside the reducer, which is what makes it atomic and what
/// makes a twenty-thousand-id burn one round trip.
#[tokio::test]
async fn the_burn_is_a_single_reducer_call() {
    let (store, runner) = store(vec![ok("")]);

    store
        .advance_id_sequence_past(SharedTable::Tasks, 4096)
        .await
        .unwrap();

    let calls = runner.recorded_calls();
    assert_eq!(calls.len(), 1, "the burn should be one round trip");
    let (program, args) = &calls[0];
    assert_eq!(program, "spacetime");
    assert_eq!(
        args,
        // The flags come AFTER the subcommand: `-s` and `-y` belong to
        // `spacetime call`, not to `spacetime`, and putting them first makes
        // the CLI reject the whole invocation.
        &vec![
            "call".to_string(),
            "-s".to_string(),
            "http://127.0.0.1:3099".to_string(),
            "-y".to_string(),
            "dispatch".to_string(),
            "burn_id_sequence".to_string(),
            "\"tasks\"".to_string(),
            "4096".to_string(),
        ]
    );
}

/// A table with no generated id is not burned at all, and costs no round trip.
#[tokio::test]
async fn a_table_without_generated_ids_is_not_burned() {
    let (store, runner) = store(vec![]);

    store
        .advance_id_sequence_past(SharedTable::TaskShells, 99)
        .await
        .unwrap();

    assert!(runner.recorded_calls().is_empty());
}

/// Optionals are wrapped on the way OUT, because a reducer argument rejects the
/// bare value that a query returns. The asymmetry is the trap this test pins.
///
/// Exercised against `epics.sort_order`, which after the sentinel change is one
/// of only two optional columns left in the whole schema.
#[tokio::test]
async fn optionals_are_wrapped_for_the_reducer_but_nulls_are_not() {
    // First response is the LIMIT 0 schema probe, second is the seed call.
    let (store, runner) = store(vec![ok(EPICS_EMPTY_JSON), ok("")]);
    let present = epic_row(1, serde_json::json!(5));
    let absent = epic_row(2, serde_json::Value::Null);

    store
        .upsert_rows(SharedTable::Epics, &[present, absent])
        .await
        .unwrap();

    let calls = runner.recorded_calls();
    let argument = calls.last().expect("no seed call").1.last().unwrap();
    let sent: serde_json::Value = serde_json::from_str(argument).unwrap();

    assert_eq!(
        sent[0]["sort_order"],
        serde_json::json!({ "some": 5 }),
        "a present optional was not wrapped — the reducer rejects the bare value"
    );
    assert_eq!(
        sent[1]["sort_order"],
        serde_json::Value::Null,
        "an absent optional should stay null, not become a wrapper"
    );
    assert_eq!(
        sent[0]["title"],
        serde_json::json!("E"),
        "a column that is not optional must not be wrapped"
    );
}

/// A null on a sentinel column becomes the sentinel, unwrapped.
///
/// `repo_paths.verify_command` is nullable in SQLite and REQUIRED in the module,
/// because SpacetimeDB SQL cannot filter on an optional column. The snapshot
/// keeps the null — it is store-neutral — so this translation is the boundary
/// with the one store that has sentinels.
#[tokio::test]
async fn a_null_on_a_sentinel_column_is_sent_as_the_sentinel() {
    let (store, runner) = store(vec![ok(EMPTY_JSON), ok("")]);
    let mut set = crate::spacetime::Row::new();
    set.insert("id".into(), serde_json::json!(1));
    set.insert("path".into(), serde_json::json!("/repo/a"));
    set.insert("last_used".into(), serde_json::json!("t"));
    set.insert("verify_command".into(), serde_json::json!("cargo test"));
    let mut unset = set.clone();
    unset.insert("id".into(), serde_json::json!(2));
    unset.insert("verify_command".into(), serde_json::Value::Null);

    store
        .upsert_rows(SharedTable::RepoPaths, &[set, unset])
        .await
        .unwrap();

    let calls = runner.recorded_calls();
    let argument = calls.last().expect("no seed call").1.last().unwrap();
    let sent: serde_json::Value = serde_json::from_str(argument).unwrap();

    assert_eq!(
        sent[0]["verify_command"],
        serde_json::json!("cargo test"),
        "a present value is sent bare — the column is no longer optional, so \
         wrapping it would be rejected"
    );
    assert_eq!(
        sent[1]["verify_command"],
        serde_json::json!(""),
        "an absent value must become the sentinel; a bare null is refused by a \
         column the store requires"
    );
}

/// The inverse. A sentinel read back out is the absence it stands for.
///
/// Without this a dump from the shared store would differ from a dump of the
/// same board from SQLite on every sentinel column, and the snapshot format's
/// whole claim is that the two are the same file.
#[tokio::test]
async fn a_sentinel_read_back_out_becomes_a_null_again() {
    let response = response_with(EMPTY_JSON, serde_json::json!([[7, "/repo/a", "t", ""]]));
    let (store, _) = store(vec![ok(&response)]);

    let rows = store.rows(SharedTable::RepoPaths).await.unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("verify_command").unwrap(),
        &serde_json::Value::Null,
        "the sentinel means absent, and a snapshot spells absent as null"
    );
    assert_eq!(rows[0].get("path").unwrap(), "/repo/a");
}

/// An id column's zero sentinel is not confused with a real id.
///
/// The trap the other way round: `todos.task_id` uses 0 for "no task", and a
/// reader that skipped the mapping would report a todo attached to task 0.
#[tokio::test]
async fn a_zero_sentinel_is_absent_rather_than_a_reference_to_row_zero() {
    let response = response_with(
        TODOS_EMPTY_JSON,
        serde_json::json!([[1, "todo", false, 0, "t", 0, 9, 0]]),
    );
    let (store, _) = store(vec![ok(&response)]);

    let rows = store.rows(SharedTable::Todos).await.unwrap();

    assert_eq!(
        rows[0].get("task_id").unwrap(),
        &serde_json::Value::Null,
        "task 0 does not exist — ids start at 1, which is what makes 0 safe as \
         the sentinel"
    );
    assert_eq!(
        rows[0].get("epic_id").unwrap(),
        9,
        "a real id must survive the mapping untouched"
    );
    assert_eq!(rows[0].get("parent_id").unwrap(), &serde_json::Value::Null);
}

/// A helper for the `epics` rows above: every required column present, with
/// `sort_order` the only variable.
fn epic_row(id: i64, sort_order: serde_json::Value) -> crate::spacetime::Row {
    let mut row = crate::spacetime::Row::new();
    for (column, value) in [
        ("id", serde_json::json!(id)),
        ("title", serde_json::json!("E")),
        ("description", serde_json::json!("")),
        ("status", serde_json::json!("backlog")),
        ("plan_path", serde_json::Value::Null),
        ("sort_order", sort_order),
        ("created_at", serde_json::json!("t")),
        ("updated_at", serde_json::json!("t")),
        ("auto_dispatch", serde_json::json!(false)),
        ("parent_epic_id", serde_json::Value::Null),
        ("feed_command", serde_json::Value::Null),
        ("feed_interval_secs", serde_json::Value::Null),
        ("group_by_repo", serde_json::json!(false)),
        ("feed_role", serde_json::json!("none")),
        ("origin", serde_json::json!("manual")),
        ("feed_append_only", serde_json::json!(false)),
    ] {
        row.insert(column.to_string(), value);
    }
    row
}

/// A snapshot missing a column the store expects is refused by name, rather
/// than sent with the column absent and rejected by the server with a message
/// about a type.
#[tokio::test]
async fn a_row_missing_a_column_names_the_column() {
    let (store, _) = store(vec![ok(EMPTY_JSON)]);
    let mut row = crate::spacetime::Row::new();
    row.insert("id".into(), serde_json::json!(1));

    let error = store
        .upsert_rows(SharedTable::RepoPaths, &[row])
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("path"),
        "unhelpful error: {error:#}"
    );
}

/// An empty table costs no round trip, so the log of a restore reads as the
/// work it actually did.
#[tokio::test]
async fn an_empty_table_sends_nothing() {
    let (store, runner) = store(vec![]);

    store.upsert_rows(SharedTable::Todos, &[]).await.unwrap();

    assert!(runner.recorded_calls().is_empty());
}

/// The reducer's own error text reaches the operator. It is the only thing that
/// says which table or which row failed.
#[tokio::test]
async fn a_failing_call_surfaces_the_reducers_message() {
    let (store, _) = store(vec![failed("seed_tasks needs each task's real id")]);

    let error = store
        .advance_id_sequence_past(SharedTable::Tasks, 1)
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("real id"),
        "the reducer's message was swallowed: {error:#}"
    );
}

/// Rows are split across calls by BYTES, not by count. Nothing tested the
/// chunking loop before, and the byte limit is what it exists for: a reducer
/// argument is a single `argv` entry, capped at 128 KiB on Linux.
#[tokio::test]
async fn a_large_table_is_split_across_several_calls() {
    let filler = "x".repeat(40_000);
    let rows: Vec<crate::spacetime::Row> = (1..=6)
        .map(|id| {
            let mut row = crate::spacetime::Row::new();
            row.insert("id".into(), serde_json::json!(id));
            row.insert("path".into(), serde_json::json!(filler));
            row.insert("last_used".into(), serde_json::json!("t"));
            row.insert("verify_command".into(), serde_json::Value::Null);
            row
        })
        .collect();
    // One schema probe, then however many seed calls the chunking decides on.
    let (store, runner) = store(vec![ok(EMPTY_JSON), ok(""), ok(""), ok(""), ok("")]);

    store
        .upsert_rows(SharedTable::RepoPaths, &rows)
        .await
        .unwrap();

    let seed_calls: Vec<_> = runner
        .recorded_calls()
        .into_iter()
        .filter(|(_, args)| args.contains(&"seed_repo_paths".to_string()))
        .collect();
    assert!(
        seed_calls.len() > 1,
        "240 KB of rows went out in {} call(s) — the byte limit was not applied",
        seed_calls.len()
    );
    // Every row arrives exactly once, across however many calls it took.
    let sent: usize = seed_calls
        .iter()
        .map(|(_, args)| {
            let batch: serde_json::Value =
                serde_json::from_str(args.last().unwrap()).expect("batch is not JSON");
            batch.as_array().expect("batch is not an array").len()
        })
        .sum();
    assert_eq!(sent, 6, "rows were dropped or duplicated by the chunking");
    for (_, args) in &seed_calls {
        assert!(
            args.last().unwrap().len() <= 96 * 1024,
            "a batch exceeded the byte budget"
        );
    }
}

/// A single row too large to be an argument at all is refused by name. The
/// kernel's own "Argument list too long" says nothing about which row or which
/// table, during exactly the incident where that is the only useful fact.
#[tokio::test]
async fn a_single_oversized_row_is_refused_by_name() {
    let mut row = crate::spacetime::Row::new();
    row.insert("id".into(), serde_json::json!(77));
    row.insert("path".into(), serde_json::json!("y".repeat(200_000)));
    row.insert("last_used".into(), serde_json::json!("t"));
    row.insert("verify_command".into(), serde_json::Value::Null);
    let (store, _) = store(vec![ok(EMPTY_JSON)]);

    let error = store
        .upsert_rows(SharedTable::RepoPaths, &[row])
        .await
        .unwrap_err();

    let text = format!("{error:#}");
    assert!(
        text.contains("repo_paths"),
        "does not name the table: {text}"
    );
    assert!(text.contains("77"), "does not name the row: {text}");
}

/// A snapshot carrying a column the store does not have is refused. Left
/// unchecked, the column is simply not copied across and the restore reports
/// success — the same data-loss event the format-version refusal exists for.
#[tokio::test]
async fn a_column_the_store_does_not_have_is_refused_rather_than_dropped() {
    let mut row = crate::spacetime::Row::new();
    row.insert("id".into(), serde_json::json!(1));
    row.insert("path".into(), serde_json::json!("/repo/a"));
    row.insert("last_used".into(), serde_json::json!("t"));
    row.insert("verify_command".into(), serde_json::Value::Null);
    row.insert("invented_since".into(), serde_json::json!("surprise"));
    let (store, _) = store(vec![ok(EMPTY_JSON)]);

    let error = store
        .upsert_rows(SharedTable::RepoPaths, &[row])
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("invented_since"),
        "unhelpful error: {error:#}"
    );
}
