//! Tests for the snapshot escape hatch (`docs/specs/spacetime-seed.allium`).
//!
//! The load-bearing one is [`sequence_burn::a_task_created_after_a_restore_cannot_collide`].
//! Read that module's header before changing anything here — what it proves,
//! and what it deliberately does not, is the whole reason this subsystem
//! exists.

mod bindings_parity;
mod cli_store;
mod completeness;
mod idempotency;
mod import;
mod managed_store;
mod managed_store_real;
mod module_schema;
mod refusals;
mod round_trip;
mod sequence_burn;
mod snapshot_edges;

use crate::spacetime::{Row, SharedTable, Snapshot, TableExtract};

/// A row of `table` with every module column present: the module's own empty
/// value for it (`""`, `0`, `false`, or null for an `Option`), then `set`
/// applied over the top. The sentinel columns are the same empty values, so a
/// row built here reads as "nothing set" exactly where the store means it.
fn module_row(table: SharedTable, set: &[(&str, serde_json::Value)]) -> Row {
    let columns = module_schema::module_tables();
    let mut row = Row::new();
    for column in &columns[table.name()] {
        let empty = if column.nullable {
            serde_json::Value::Null
        } else if column.ty == "String" {
            serde_json::Value::from("")
        } else if column.ty == "bool" {
            serde_json::Value::from(false)
        } else {
            serde_json::Value::from(0)
        };
        row.insert(column.name.clone(), empty);
    }
    for (name, value) in set {
        assert!(
            row.contains_key(*name),
            "the module's `{}` has no column `{name}`",
            table.name()
        );
        row.insert((*name).to_string(), value.clone());
    }
    row
}

/// The extract for `table`: every module column, in the module's order, with
/// `rows` (each a `set` list for [`module_row`]).
fn module_extract(table: SharedTable, rows: &[Vec<(&str, serde_json::Value)>]) -> TableExtract {
    let columns = module_schema::module_tables()[table.name()]
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let rows = rows.iter().map(|set| module_row(table, set)).collect();
    TableExtract::new(table, columns, rows)
}

/// A snapshot of a small but structurally complete board: two epics, tasks
/// under one of them and free-standing, a watcher, a live subagent, a saved
/// repo path, a base branch and this install's host row. Every shared table is
/// present, those without rows empty.
///
/// Deliberately includes a **gap in the task ids** and a task whose id is far
/// above the rest. Both are what a real board looks like after months of
/// deletions, and both are what a restore that renumbers would silently
/// "tidy up".
pub(super) fn snapshot_of_a_populated_board_sync() -> Snapshot {
    use serde_json::json;
    let rows_for = |table: SharedTable| -> Vec<Vec<(&'static str, serde_json::Value)>> {
        match table.name() {
            "epics" => vec![
                vec![
                    ("id", json!(7)),
                    ("title", json!("Epic seven")),
                    ("description", json!("first epic")),
                    ("status", json!("running")),
                ],
                vec![
                    ("id", json!(9)),
                    ("title", json!("Epic nine")),
                    ("description", json!("second epic")),
                    ("status", json!("backlog")),
                ],
            ],
            "tasks" => [
                (
                    3,
                    "Oldest task",
                    "/repo/a",
                    "done",
                    "none",
                    7,
                    "host-1",
                    "/wt/3",
                ),
                (4, "Next task", "/repo/a", "backlog", "none", 7, "", ""),
                (
                    11,
                    "After a gap",
                    "/repo/b",
                    "running",
                    "active",
                    0,
                    "host-1",
                    "/wt/11",
                ),
                (4096, "Far ahead", "/repo/b", "backlog", "none", 9, "", ""),
            ]
            .into_iter()
            .map(|(id, title, repo, status, sub, epic, host, wt)| {
                vec![
                    ("id", json!(id)),
                    ("title", json!(title)),
                    ("description", json!(format!("body {id}"))),
                    ("repo_path", json!(repo)),
                    ("status", json!(status)),
                    ("sub_status", json!(sub)),
                    ("epic_id", json!(epic)),
                    ("host", json!(host)),
                    ("worktree", json!(wt)),
                ]
            })
            .collect(),
            "task_watchers" => vec![vec![
                ("id", json!(1)),
                ("watcher_task_id", json!(4)),
                ("target_task_id", json!(3)),
            ]],
            "task_subagents" => vec![vec![
                ("task_id", json!(11)),
                ("agent_id", json!("agent-a")),
                ("session_id", json!("session-a")),
                ("started_at", json!("2026-09-17T10:00:00Z")),
            ]],
            "repo_paths" => vec![
                vec![
                    ("id", json!(1)),
                    ("path", json!("/repo/a")),
                    ("verify_command", json!("cargo test")),
                ],
                vec![("id", json!(2)), ("path", json!("/repo/b"))],
            ],
            "repo_base_branches" => vec![
                vec![
                    ("id", json!(1)),
                    ("repo_path", json!("/repo/a")),
                    ("branch", json!("main")),
                    ("last_used", json!("2026-09-17T10:00:00Z")),
                ],
                vec![
                    ("id", json!(2)),
                    ("repo_path", json!("/repo/a")),
                    ("branch", json!("release")),
                    ("last_used", json!("2026-09-16T10:00:00Z")),
                ],
            ],
            "hosts" => vec![vec![
                ("id", json!("host-1")),
                ("label", json!("ragge-laptop")),
            ]],
            "settings" => vec![vec![
                ("key", json!("repo_filter_mode")),
                ("value", json!("include")),
            ]],
            _ => vec![],
        }
    };
    Snapshot::new(
        SharedTable::ALL
            .iter()
            .copied()
            .map(|table| module_extract(table, &rows_for(table)))
            .collect(),
    )
}

/// The snapshot under test in most of these modules.
pub(super) async fn snapshot_of_a_populated_board() -> Snapshot {
    snapshot_of_a_populated_board_sync()
}

/// A complete snapshot of a board with nothing in it. Every table present,
/// every table empty — which is a different claim from every table absent.
///
/// The column names are a stand-in, not a schema: nothing store-neutral knows
/// what a shared table's columns are, which is the whole reason a snapshot
/// records its own. Pair it with [`store_for`], which adopts whatever this
/// says, so a test aimed at the burn is not refused for a schema mismatch it
/// never asked about.
pub(super) fn empty_snapshot() -> Snapshot {
    Snapshot::new(
        SharedTable::ALL
            .iter()
            .copied()
            .map(|table| TableExtract::empty(table, vec!["id".into()]))
            .collect(),
    )
}

/// A bare task row carrying nothing but an explicit id.
pub(super) fn row_with_id(id: i64) -> crate::spacetime::Row {
    let mut row = crate::spacetime::Row::new();
    row.insert("id".into(), serde_json::Value::from(id));
    row
}

/// A [`MemoryStore`](crate::spacetime::MemoryStore) already agreeing with the
/// snapshot about the schema, so a test aimed at some other behaviour is not
/// refused for a schema mismatch it did not ask about.
pub(super) fn store_for(snapshot: &Snapshot) -> crate::spacetime::MemoryStore {
    crate::spacetime::MemoryStore::matching(snapshot)
}
