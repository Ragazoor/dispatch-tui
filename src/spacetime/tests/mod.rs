//! Tests for the snapshot escape hatch (`docs/specs/spacetime-seed.allium`).
//!
//! The load-bearing one is [`sequence_burn::a_task_created_after_a_restore_cannot_collide`].
//! Read that module's header before changing anything here — what it proves,
//! and what it deliberately does not, is the whole reason this subsystem
//! exists.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod cli_store;
mod completeness;
mod idempotency;
mod module_schema;
mod refusals;
mod round_trip;
mod sequence_burn;

use crate::db::Database;
use crate::spacetime::{dump_from_sqlite, Snapshot};

/// A migrated in-memory database holding a small but structurally complete
/// board: two epics, tasks under one of them and free-standing, todos, a
/// watcher, a live shell and subagent, a saved repo path and a base branch.
///
/// Deliberately includes a **gap in the task ids** and a task whose id is far
/// above the rest. Both are what a real board looks like after months of
/// deletions, and both are what a restore that renumbers would silently
/// "tidy up".
pub(super) async fn populated_board() -> Database {
    let db = Database::open_in_memory().await.unwrap();
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

             INSERT INTO todos (id, title, done, sort_order, task_id) VALUES
                 (1, 'first todo',  0, 0, 3),
                 (2, 'second todo', 1, 1, 3);

             INSERT INTO task_watchers (id, watcher_task_id, target_task_id) VALUES
                 (1, 4, 3);

             INSERT INTO task_shells (task_id, shell_id, session_id, started_at) VALUES
                 (11, 'shell-a', 'session-a', '2026-09-17T10:00:00Z');

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
             -- one so the assembled hosts row is assertable.
             INSERT INTO settings (key, value) VALUES
                 ('host_id', 'host-1'),
                 ('host_label', 'ragge-laptop')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value;",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();
    db
}

/// The snapshot under test in most of these modules.
pub(super) async fn snapshot_of_a_populated_board() -> Snapshot {
    let db = populated_board().await;
    dump_from_sqlite(&db).await.unwrap()
}

/// The schema version used by the tests that build a snapshot by hand rather
/// than dumping one.
///
/// Its value does not matter — only that the hand-built snapshot and the store
/// it is restored into agree, so a test aimed at the burn is not refused for a
/// schema mismatch it never asked about. Tests that dump a real board take the
/// version from the dump instead, via [`store_for`].
pub(super) const TEST_SCHEMA_VERSION: i64 = 0;

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
    crate::spacetime::MemoryStore::with_schema_version(snapshot.schema_version)
}
