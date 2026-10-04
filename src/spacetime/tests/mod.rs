//! Tests for the snapshot escape hatch (`docs/specs/spacetime-seed.allium`).
//!
//! The load-bearing one is [`sequence_burn::a_task_created_after_a_restore_cannot_collide`].
//! Read that module's header before changing anything here — what it proves,
//! and what it deliberately does not, is the whole reason this subsystem
//! exists.
#![allow(clippy::unwrap_used, clippy::expect_used)]

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
mod seed;
mod sequence_burn;

use crate::db::Database;
use crate::spacetime::{dump_from_sqlite, SharedTable, Snapshot, TableExtract};

/// A migrated in-memory database holding a small but structurally complete
/// board: two epics, tasks under one of them and free-standing, a
/// watcher, a live subagent, a saved repo path and a base branch.
///
/// Deliberately includes a **gap in the task ids** and a task whose id is far
/// above the rest. Both are what a real board looks like after months of
/// deletions, and both are what a restore that renumbers would silently
/// "tidy up".
pub(super) async fn populated_board() -> Database {
    let db = Database::open_in_memory().await.unwrap();
    seed_board(&db).await;
    db
}

/// [`populated_board`] on a real file, which is the only way to get a WAL
/// board: an in-memory store silently settles for a rollback journal, where
/// readers and the writer exclude each other and no board ever runs. See
/// `storage.allium`: ConcurrencyIsObservedOnAWalStore.
///
/// Costs a temp directory and a full migration run, so it is for the tests
/// that actually overlap a read with a write. Everything else should stay on
/// [`populated_board`]. The returned `TempDir` must outlive the `Database` —
/// dropping it deletes the store out from under the open connections.
pub(super) async fn populated_board_on_disk() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("board.db")).await.unwrap();
    seed_board(&db).await;
    (dir, db)
}

/// The rows both fixtures above plant. Kept in one place so the on-disk board
/// cannot drift into being a different board from the in-memory one.
async fn seed_board(db: &Database) {
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status) VALUES
                 (7, 'Epic seven', 'first epic', 'running'),
                 (9, 'Epic nine', 'second epic', 'backlog');

             INSERT INTO tasks (id, title, description, repo_path, status, sub_status, epic_id, host, worktree) VALUES
                 (3,    'Oldest task',  'body 3',    '/repo/a', 'done',    'none',   7,    'host-1', '/wt/3'),
                 (4,    'Next task',    'body 4',    '/repo/a', 'backlog', 'none',   7,    NULL,     NULL),
                 (11,   'After a gap',  'body 11',   '/repo/b', 'running', 'active', NULL, 'host-1', '/wt/11'),
                 (4096, 'Far ahead',    'body 4096', '/repo/b', 'backlog', 'none',   9,    NULL,     NULL);

             INSERT INTO task_watchers (id, watcher_task_id, target_task_id) VALUES
                 (1, 4, 3);

             INSERT INTO task_subagents (task_id, agent_id, session_id, started_at) VALUES
                 (11, 'agent-a', 'session-a', '2026-09-17T10:00:00Z');

             INSERT INTO repo_paths (id, path, verify_command) VALUES
                 (1, '/repo/a', 'cargo test'),
                 (2, '/repo/b', NULL);

             INSERT INTO repo_base_branches (repo_path, branch, last_used) VALUES
                 ('/repo/a', 'main',    '2026-09-17T10:00:00Z'),
                 ('/repo/a', 'release', '2026-09-16T10:00:00Z');

             -- Replace rather than insert: migration v97 already minted a
             -- host_id for this install, and the fixture wants a predictable
             -- one so the assembled hosts row is assertable. `repo_filter_mode`
             -- is a genuine `Setting` row, planted beside the two identity
             -- keys so the dump's exclusion of those two is exercised against
             -- real data rather than an empty table.
             INSERT INTO settings (key, value) VALUES
                 ('host_id', 'host-1'),
                 ('host_label', 'ragge-laptop')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value;
             INSERT INTO settings (key, value) VALUES
                 ('repo_filter_mode', 'include');",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();
}

/// The snapshot under test in most of these modules.
pub(super) async fn snapshot_of_a_populated_board() -> Snapshot {
    let db = populated_board().await;
    dump_from_sqlite(&db).await.unwrap()
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
