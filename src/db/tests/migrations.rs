#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn fresh_db_has_latest_schema_version() {
    let db = in_memory_db().await;
    let version: i64 = db
        .db_call(|conn| {
            conn.pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn v48_creates_retrieval_and_verdict_tables() {
    let db = in_memory_db().await;
    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('learning_retrievals','learning_verdicts')",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn v48_accepts_needs_review_status() {
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute(
            "INSERT INTO learnings (kind, summary, scope, status) VALUES ('pitfall','x','user','needs_review')",
            [],
        )
        .map(|_| ())
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn v49_renames_confirmed_columns_to_upvote() {
    let db = in_memory_db().await;
    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('learnings')
                 WHERE name IN ('upvote_count','last_upvoted_at')",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(
        count, 2,
        "expected upvote_count and last_upvoted_at columns"
    );

    let stale: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('learnings')
                 WHERE name IN ('confirmed_count','last_confirmed_at')",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(stale, 0, "old confirmed_* columns must be removed");
}

#[test]
fn migrate_v49_preserves_existing_counts() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE learnings (
             id                INTEGER PRIMARY KEY,
             kind              TEXT NOT NULL,
             summary           TEXT NOT NULL,
             scope             TEXT NOT NULL,
             status            TEXT NOT NULL,
             confirmed_count   INTEGER NOT NULL DEFAULT 0,
             last_confirmed_at TEXT
         );
         INSERT INTO learnings (kind, summary, scope, status, confirmed_count, last_confirmed_at)
         VALUES ('pitfall','one','user','approved', 7, '2026-05-09T12:00:00Z');",
    )
    .unwrap();

    crate::db::migrations::migrate_v49_rename_confirmed_to_upvote(&conn).unwrap();

    let (count, ts): (i64, String) = conn
        .query_row(
            "SELECT upvote_count, last_upvoted_at FROM learnings WHERE summary='one'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 7);
    assert_eq!(ts, "2026-05-09T12:00:00Z");
}

#[tokio::test]
async fn migration_v42_nulls_out_epic_tag() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL CHECK (status IN ('backlog','running','review','done','archived')),
             sub_status TEXT NOT NULL DEFAULT 'none',
             worktree TEXT,
             tmux_window TEXT,
             plan_path TEXT,
             epic_id INTEGER,
             pr_url TEXT,
             tag TEXT,
             sort_order INTEGER,
             base_branch TEXT NOT NULL DEFAULT 'main',
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             agent_pid INTEGER,
             agent_status TEXT,
             external_id TEXT,
             project_id INTEGER NOT NULL DEFAULT 1
         );
         INSERT INTO tasks (id, title, repo_path, status, sub_status, tag, base_branch, created_at, updated_at)
             VALUES (1, 'epic-tagged', '/r', 'backlog', 'none', 'epic', 'main', '2026-01-01', '2026-01-01');
         INSERT INTO tasks (id, title, repo_path, status, sub_status, tag, base_branch, created_at, updated_at)
             VALUES (2, 'feature-tagged', '/r', 'backlog', 'none', 'feature', 'main', '2026-01-01', '2026-01-01');
         INSERT INTO tasks (id, title, repo_path, status, sub_status, tag, base_branch, created_at, updated_at)
             VALUES (3, 'bug-tagged', '/r', 'backlog', 'none', 'bug', 'main', '2026-01-01', '2026-01-01');
         PRAGMA user_version = 41;",
    )
    .unwrap();

    crate::db::migrations::migrate_v42_drop_epic_tag(&conn).unwrap();

    let mut stmt = conn
        .prepare("SELECT id, tag FROM tasks ORDER BY id")
        .unwrap();
    let rows: Vec<(i64, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(rows[0], (1, None), "epic-tagged task should have tag NULL");
    assert_eq!(
        rows[1],
        (2, Some("feature".to_string())),
        "feature tag must be untouched"
    );
    assert_eq!(
        rows[2],
        (3, Some("bug".to_string())),
        "bug tag must be untouched"
    );
}

#[tokio::test]
async fn migration_v39_backfills_project_id_to_default() {
    use rusqlite::Connection as RawConn;
    // Build a pre-v39 database manually (v38 schema)
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan_path TEXT,
             tag TEXT,
             epic_id INTEGER,
             sub_status TEXT NOT NULL DEFAULT 'none',
             pr_url TEXT,
             sort_order INTEGER,
             base_branch TEXT NOT NULL DEFAULT 'main',
             external_id TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             plan_path TEXT,
             sort_order INTEGER,
             auto_dispatch INTEGER NOT NULL DEFAULT 0,
             parent_epic_id INTEGER,
             feed_command TEXT,
             feed_interval_secs INTEGER,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO tasks (title, repo_path) VALUES ('Old task', '/repo');
         INSERT INTO epics (title, repo_path) VALUES ('Old epic', '/repo');
         PRAGMA user_version = 38;",
    )
    .unwrap();
    // Apply pending migrations via init_schema
    super::super::init_schema_sync(&conn).unwrap();
    // Verify project_id = 1 (Default project) was backfilled
    let task_pid: i64 = conn
        .query_row(
            "SELECT project_id FROM tasks WHERE title = 'Old task'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(task_pid, 1);
    let epic_pid: i64 = conn
        .query_row(
            "SELECT project_id FROM epics WHERE title = 'Old epic'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(epic_pid, 1);
}

#[tokio::test]
async fn legacy_db_migrates_to_latest_version() {
    // Simulate a pre-versioning DB: create tables manually including notes
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE notes (
             id INTEGER PRIMARY KEY,
             task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
             content TEXT NOT NULL,
             source TEXT NOT NULL DEFAULT 'user',
             created_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );",
    )
    .unwrap();

    // Insert a note so we can verify the table gets dropped
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path) VALUES ('T', 'D', '/r')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notes (task_id, content) VALUES (1, 'hello')",
        [],
    )
    .unwrap();

    // Run init_schema which should migrate
    super::super::init_schema_sync(&conn).unwrap();

    // Notes table should be gone
    let table_exists: bool = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='notes'")
        .unwrap()
        .exists([])
        .unwrap();
    assert!(
        !table_exists,
        "notes table should be dropped after migration"
    );

    // Verify Migration 25 renamed the plan column to plan_path
    let has_plan_path: bool = conn.prepare("SELECT plan_path FROM tasks LIMIT 1").is_ok();
    assert!(
        has_plan_path,
        "Migration 25 should have renamed plan to plan_path"
    );

    // Version should be latest
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migration_25_renames_plan_to_plan_path() {
    // Simulate a v24 DB (plan column exists, plan_path does not)
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         PRAGMA user_version=24;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             plan        TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL,
             mode       TEXT NOT NULL DEFAULT 'include'
         );
         INSERT INTO tasks (title, description, repo_path, plan)
             VALUES ('T1', 'D1', '/r', 'docs/plans/task.md');
         INSERT INTO epics (title, description, repo_path, plan)
             VALUES ('E1', 'D1', '/r', 'docs/plans/epic.md');",
    )
    .unwrap();

    // Apply migration 25
    super::super::init_schema_sync(&conn).unwrap();

    // plan_path column exists with data preserved
    let task_plan_path: Option<String> = conn
        .query_row("SELECT plan_path FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        task_plan_path.as_deref(),
        Some("docs/plans/task.md"),
        "task plan_path should be preserved after migration"
    );

    let epic_plan_path: Option<String> = conn
        .query_row("SELECT plan_path FROM epics WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        epic_plan_path.as_deref(),
        Some("docs/plans/epic.md"),
        "epic plan_path should be preserved after migration"
    );

    // Version bumped to 25
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migrate_v26_adds_agent_columns() {
    let db = in_memory_db().await;

    let (tw1, wt1, tw2, wt2): (Option<String>, Option<String>, Option<String>, Option<String>) = db
        .db_call(|conn| {
            conn.execute(
                "INSERT INTO review_prs (repo, number, title, author, url, is_draft,
                 created_at, updated_at, additions, deletions, review_decision,
                 labels, body, head_ref, ci_status, reviewers, tmux_window, worktree)
                 VALUES ('acme/app', 1, 'Test', 'alice', 'https://example.com', 0,
                 '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', 0, 0, 'ReviewRequired',
                 '[]', '', '', 'None', '[]', 'dispatch:review-1', '/tmp/wt')",
                [],
            )?;
            let (tw1, wt1): (Option<String>, Option<String>) = conn.query_row(
                "SELECT tmux_window, worktree FROM review_prs WHERE repo = 'acme/app' AND number = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            conn.execute(
                "INSERT INTO security_alerts (repo, number, kind, severity, title,
                 url, created_at, state, description, tmux_window, worktree)
                 VALUES ('acme/app', 1, 'dependabot', 'high', 'Alert',
                 'https://example.com', '2024-01-01T00:00:00Z', 'open', 'desc',
                 'dispatch:fix-1', '/tmp/wt4')",
                [],
            )?;
            let (tw2, wt2): (Option<String>, Option<String>) = conn.query_row(
                "SELECT tmux_window, worktree FROM security_alerts WHERE repo = 'acme/app'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((tw1, wt1, tw2, wt2))
        })
        .await
        .unwrap();
    assert_eq!(tw1.as_deref(), Some("dispatch:review-1"));
    assert_eq!(wt1.as_deref(), Some("/tmp/wt"));
    assert_eq!(tw2.as_deref(), Some("dispatch:fix-1"));
    assert_eq!(wt2.as_deref(), Some("/tmp/wt4"));
}

#[tokio::test]
async fn migration_6_converts_ready_to_backlog() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan TEXT,
             epic_id INTEGER,
             needs_input INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             done INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         PRAGMA user_version = 5;",
    )
    .unwrap();

    // Insert a ready task
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status) VALUES ('T', 'D', '/r', 'ready')",
        [],
    ).unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    let status: String = conn
        .query_row("SELECT status FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "backlog");

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migration_13_converts_needs_input() {
    // Simulate a database at version 12 with needs_input column
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan TEXT,
             epic_id INTEGER,
             needs_input INTEGER NOT NULL DEFAULT 0,
             pr_url TEXT,
             sort_order INTEGER,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             done INTEGER NOT NULL DEFAULT 0,
             plan TEXT,
             sort_order INTEGER,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE task_usage (
             task_id            INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
             input_tokens       INTEGER NOT NULL DEFAULT 0,
             output_tokens      INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
             cache_write_tokens INTEGER NOT NULL DEFAULT 0,
             updated_at         TEXT    NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL
         );
         PRAGMA user_version = 12;",
    )
    .unwrap();

    // Insert tasks with various states
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, needs_input) VALUES ('Blocked', 'desc', '/r', 'running', 1)",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, needs_input) VALUES ('Active', 'desc', '/r', 'running', 0)",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, needs_input) VALUES ('InReview', 'desc', '/r', 'review', 0)",
        [],
    ).unwrap();

    // Run migration
    super::super::init_schema_sync(&conn).unwrap();

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);

    // Verify needs_input=1 became sub_status='needs_input'
    let ss: String = conn
        .query_row("SELECT sub_status FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(ss, "needs_input");

    // Verify running task with needs_input=0 became 'active'
    let ss: String = conn
        .query_row("SELECT sub_status FROM tasks WHERE id = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(ss, "active");

    // Verify review task became 'awaiting_review'
    let ss: String = conn
        .query_row("SELECT sub_status FROM tasks WHERE id = 3", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(ss, "awaiting_review");

    // Verify needs_input column no longer exists
    let has_needs_input = conn
        .prepare("SELECT needs_input FROM tasks LIMIT 1")
        .is_ok();
    assert!(
        !has_needs_input,
        "needs_input column should be removed after migration"
    );
}

#[tokio::test]
async fn migration_16_cleans_invalid_review_needs_input() {
    // Simulate a v15 DB that has (review, needs_input) rows from old hook behavior
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan TEXT,
             epic_id INTEGER,
             sub_status TEXT NOT NULL DEFAULT 'none',
             pr_url TEXT,
             tag TEXT,
             sort_order INTEGER,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path TEXT NOT NULL,
             done INTEGER NOT NULL DEFAULT 0,
             plan TEXT,
             sort_order INTEGER,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE task_usage (
             task_id INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_write_tokens INTEGER NOT NULL DEFAULT 0,
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE filter_presets (name TEXT PRIMARY KEY, repo_paths TEXT NOT NULL);
         CREATE TABLE review_prs (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             number INTEGER NOT NULL,
             title TEXT NOT NULL,
             url TEXT NOT NULL,
             repo TEXT NOT NULL,
             author TEXT NOT NULL,
             state TEXT NOT NULL DEFAULT 'open',
             review_decision TEXT,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL
         );
         PRAGMA user_version = 15;",
    )
    .unwrap();

    // Insert invalid rows that migration 16 must clean up
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status) \
         VALUES ('ReviewBlocked', 'desc', '/r', 'review', 'needs_input')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status) \
         VALUES ('ValidReview', 'desc', '/r', 'review', 'awaiting_review')",
        [],
    )
    .unwrap();

    // Run migrations
    super::super::init_schema_sync(&conn).unwrap();

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);

    // (review, needs_input) must be converted to (review, awaiting_review)
    let ss: String = conn
        .query_row(
            "SELECT sub_status FROM tasks WHERE title = 'ReviewBlocked'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        ss, "awaiting_review",
        "legacy (review, needs_input) must be cleaned up"
    );

    // Valid row must be unchanged
    let ss2: String = conn
        .query_row(
            "SELECT sub_status FROM tasks WHERE title = 'ValidReview'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ss2, "awaiting_review");
}

// ---------------------------------------------------------------------------
// Migration-specific tests — verify data preservation through table rebuilds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn migration_v4_preserves_epic_data_after_table_rebuild() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=3;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER REFERENCES epics(id),
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             plan        TEXT NOT NULL DEFAULT '',
             repo_path   TEXT NOT NULL,
             done        INTEGER NOT NULL DEFAULT 0,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO epics (title, description, plan, repo_path, done)
             VALUES ('Active Epic', 'Active desc', 'Original plan', '/repo/a', 0);
         INSERT INTO epics (title, description, plan, repo_path, done)
             VALUES ('Done Epic', 'Done desc', 'Done plan', '/repo/b', 1);
         INSERT INTO tasks (title, description, repo_path, epic_id)
             VALUES ('Task 1', 'Task desc', '/repo/a', 1);",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    // Epic core data preserved through v4 table rebuild
    let (title, desc, repo): (String, String, String) = conn
        .query_row(
            "SELECT title, description, repo_path FROM epics WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(title, "Active Epic");
    assert_eq!(desc, "Active desc");
    assert_eq!(repo, "/repo/a");

    // v4 dropped plan; v8 re-added it (NULL); v25 renamed to plan_path
    let plan_path: Option<String> = conn
        .query_row("SELECT plan_path FROM epics WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        plan_path.is_none(),
        "plan should be NULL after v4 dropped and v8 re-added it"
    );

    // Task-epic FK preserved through rebuild
    let epic_id: Option<i64> = conn
        .query_row("SELECT epic_id FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(epic_id, Some(1));
}

#[tokio::test]
async fn migration_v15_converts_needs_input_to_sub_status() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=14;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER,
             needs_input INTEGER NOT NULL DEFAULT 0,
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             done        INTEGER NOT NULL DEFAULT 0,
             plan        TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL
         );
         INSERT INTO tasks (title, description, repo_path, status, needs_input)
             VALUES ('Needs Input', 'desc', '/r', 'running', 1);
         INSERT INTO tasks (title, description, repo_path, status, needs_input)
             VALUES ('Running Active', 'desc', '/r', 'running', 0);
         INSERT INTO tasks (title, description, repo_path, status, needs_input)
             VALUES ('In Review', 'desc', '/r', 'review', 0);
         INSERT INTO tasks (title, description, repo_path, status, needs_input)
             VALUES ('In Backlog', 'desc', '/r', 'backlog', 0);",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    let rows: Vec<(String, String)> = conn
        .prepare("SELECT title, sub_status FROM tasks ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert_eq!(rows[0], ("Needs Input".into(), "needs_input".into()));
    assert_eq!(rows[1], ("Running Active".into(), "active".into()));
    assert_eq!(rows[2], ("In Review".into(), "awaiting_review".into()));
    assert_eq!(rows[3], ("In Backlog".into(), "none".into()));

    // needs_input column should be removed by v15 table rebuild
    assert!(
        conn.prepare("SELECT needs_input FROM tasks").is_err(),
        "needs_input column should be removed after migration"
    );
}

#[tokio::test]
async fn migration_v16_cleans_invalid_status_pairs() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=15;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             done        INTEGER NOT NULL DEFAULT 0,
             plan        TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL
         );
         -- Invalid: (review, needs_input) → should become (review, awaiting_review)
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('Review NI', 'desc', '/r', 'review', 'needs_input');
         -- Invalid: (running, none) → should become (running, active)
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('Running None', 'desc', '/r', 'running', 'none');
         -- Invalid: (backlog, active) → should become (backlog, none)
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('Backlog Active', 'desc', '/r', 'backlog', 'active');
         -- Valid: (running, active) → unchanged
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('Running OK', 'desc', '/r', 'running', 'active');",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    let rows: Vec<(String, String, String)> = conn
        .prepare("SELECT title, status, sub_status FROM tasks ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert_eq!(
        rows[0],
        (
            "Review NI".into(),
            "review".into(),
            "awaiting_review".into()
        )
    );
    assert_eq!(
        rows[1],
        ("Running None".into(), "running".into(), "active".into())
    );
    assert_eq!(
        rows[2],
        ("Backlog Active".into(), "backlog".into(), "none".into())
    );
    assert_eq!(
        rows[3],
        ("Running OK".into(), "running".into(), "active".into())
    );

    // CHECK constraint should reject invalid pairs after migration
    let result = conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status)
         VALUES ('x', 'x', '/x', 'backlog', 'active')",
        [],
    );
    assert!(
        result.is_err(),
        "CHECK constraint should reject (backlog, active)"
    );
}

#[tokio::test]
async fn migration_v18_expands_tilde_paths() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=17;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed','conflict')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             done        INTEGER NOT NULL DEFAULT 0,
             plan        TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL
         );
         INSERT INTO tasks (title, description, repo_path)
             VALUES ('Tilde', 'desc', '~/project/a');
         INSERT INTO tasks (title, description, repo_path)
             VALUES ('Absolute', 'desc', '/absolute/path');
         INSERT INTO epics (title, description, repo_path)
             VALUES ('Epic', 'desc', '~/project/b');
         INSERT INTO repo_paths (path) VALUES ('~/project/c');
         INSERT INTO settings (key, value) VALUES ('repo_filter', '~/project/d');
         INSERT INTO filter_presets (name, repo_paths)
             VALUES ('preset', '~/project/e');",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    let home = std::env::var("HOME").expect("HOME must be set for this test");

    let task_path: String = conn
        .query_row("SELECT repo_path FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(task_path, format!("{home}/project/a"));

    // Absolute paths unchanged
    let abs_path: String = conn
        .query_row("SELECT repo_path FROM tasks WHERE id = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(abs_path, "/absolute/path");

    let epic_path: String = conn
        .query_row("SELECT repo_path FROM epics WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(epic_path, format!("{home}/project/b"));

    let rp: String = conn
        .query_row("SELECT path FROM repo_paths", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rp, format!("{home}/project/c"));

    // After v29, repo_filter is stored as JSON array
    let setting: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let filter_paths: Vec<String> = serde_json::from_str(&setting).unwrap();
    assert_eq!(filter_paths, vec![format!("{home}/project/d")]);

    // After v29, filter_presets.repo_paths is stored as JSON array
    let preset: String = conn
        .query_row(
            "SELECT repo_paths FROM filter_presets WHERE name = 'preset'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let preset_paths: Vec<String> = serde_json::from_str(&preset).unwrap();
    assert_eq!(preset_paths, vec![format!("{home}/project/e")]);
}

#[tokio::test]
async fn migration_v20_converts_done_boolean_to_status_enum() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=19;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed','conflict')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             done        INTEGER NOT NULL DEFAULT 0,
             plan        TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL
         );
         -- Epic 1: done=1 → status 'done'
         INSERT INTO epics (title, description, repo_path, done)
             VALUES ('Done Epic', 'desc', '/r', 1);
         -- Epic 2: done=0, no subtasks → status 'backlog'
         INSERT INTO epics (title, description, repo_path, done)
             VALUES ('Empty Epic', 'desc', '/r', 0);
         -- Epic 3: done=0, all subtasks done → status 'done'
         INSERT INTO epics (title, description, repo_path, done)
             VALUES ('All Done', 'desc', '/r', 0);
         INSERT INTO tasks (title, description, repo_path, status, sub_status, epic_id)
             VALUES ('T1', 'd', '/r', 'done', 'none', 3);
         INSERT INTO tasks (title, description, repo_path, status, sub_status, epic_id)
             VALUES ('T2', 'd', '/r', 'done', 'none', 3);
         -- Epic 4: done=0, has running subtask → status 'running'
         INSERT INTO epics (title, description, repo_path, done)
             VALUES ('Running Epic', 'desc', '/r', 0);
         INSERT INTO tasks (title, description, repo_path, status, sub_status, epic_id)
             VALUES ('T3', 'd', '/r', 'running', 'active', 4);
         INSERT INTO tasks (title, description, repo_path, status, sub_status, epic_id)
             VALUES ('T4', 'd', '/r', 'done', 'none', 4);
         -- Epic 5: done=0, review+done subtasks → status 'review'
         INSERT INTO epics (title, description, repo_path, done)
             VALUES ('Review Epic', 'desc', '/r', 0);
         INSERT INTO tasks (title, description, repo_path, status, sub_status, epic_id)
             VALUES ('T5', 'd', '/r', 'review', 'awaiting_review', 5);
         INSERT INTO tasks (title, description, repo_path, status, sub_status, epic_id)
             VALUES ('T6', 'd', '/r', 'done', 'none', 5);",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    let statuses: Vec<(String, String)> = conn
        .prepare("SELECT title, status FROM epics ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert_eq!(statuses[0], ("Done Epic".into(), "done".into()));
    assert_eq!(statuses[1], ("Empty Epic".into(), "backlog".into()));
    assert_eq!(statuses[2], ("All Done".into(), "done".into()));
    assert_eq!(statuses[3], ("Running Epic".into(), "running".into()));
    assert_eq!(statuses[4], ("Review Epic".into(), "review".into()));

    // done column should be removed (replaced by status enum)
    assert!(
        conn.prepare("SELECT done FROM epics").is_err(),
        "done column should be removed after migration"
    );
}

#[tokio::test]
async fn migration_v17_adds_conflict_sub_status() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=16;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             done        INTEGER NOT NULL DEFAULT 0,
             plan        TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL
         );
         -- Insert tasks with valid sub_status values
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('Active', 'desc', '/r', 'running', 'active');
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('Stale', 'desc', '/r', 'running', 'stale');
         INSERT INTO tasks (title, description, repo_path, status, sub_status)
             VALUES ('In Review', 'desc', '/r', 'review', 'awaiting_review');",
    )
    .unwrap();

    // Before migration, 'conflict' should be rejected by CHECK constraint
    let result = conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status)
         VALUES ('x', 'x', '/x', 'running', 'conflict')",
        [],
    );
    assert!(
        result.is_err(),
        "pre-migration CHECK should reject 'conflict'"
    );

    super::super::init_schema_sync(&conn).unwrap();

    // Existing data preserved
    let rows: Vec<(String, String, String)> = conn
        .prepare("SELECT title, status, sub_status FROM tasks ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert_eq!(
        rows[0],
        ("Active".into(), "running".into(), "active".into())
    );
    assert_eq!(rows[1], ("Stale".into(), "running".into(), "stale".into()));
    assert_eq!(
        rows[2],
        (
            "In Review".into(),
            "review".into(),
            "awaiting_review".into()
        )
    );

    // 'conflict' now accepted after migration
    let result = conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status)
         VALUES ('Conflict', 'desc', '/r', 'running', 'conflict')",
        [],
    );
    assert!(
        result.is_ok(),
        "post-migration CHECK should accept 'conflict'"
    );
}

#[tokio::test]
async fn migration_v29_converts_newline_presets_to_json() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=28;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan_path   TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed','conflict')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             plan_path   TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL,
             mode       TEXT NOT NULL DEFAULT 'include'
         );
         -- Newline-delimited preset
         INSERT INTO filter_presets (name, repo_paths, mode)
             VALUES ('multi', '/repo/a\n/repo/b\n/repo/c', 'include');
         -- Single-path preset (no newlines)
         INSERT INTO filter_presets (name, repo_paths, mode)
             VALUES ('single', '/repo/only', 'exclude');
         -- Newline-delimited repo_filter setting
         INSERT INTO settings (key, value) VALUES ('repo_filter', '/repo/x\n/repo/y');
         -- Non-filter setting should be unaffected
         INSERT INTO settings (key, value) VALUES ('other_key', 'some\nvalue');",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    // Filter presets converted to JSON
    let multi: String = conn
        .query_row(
            "SELECT repo_paths FROM filter_presets WHERE name = 'multi'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let multi_paths: Vec<String> = serde_json::from_str(&multi).unwrap();
    assert_eq!(
        multi_paths,
        vec![
            "/repo/a".to_string(),
            "/repo/b".to_string(),
            "/repo/c".to_string()
        ]
    );

    let single: String = conn
        .query_row(
            "SELECT repo_paths FROM filter_presets WHERE name = 'single'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let single_paths: Vec<String> = serde_json::from_str(&single).unwrap();
    assert_eq!(single_paths, vec!["/repo/only".to_string()]);

    // repo_filter setting converted to JSON
    let filter: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let filter_paths: Vec<String> = serde_json::from_str(&filter).unwrap();
    assert_eq!(
        filter_paths,
        vec!["/repo/x".to_string(), "/repo/y".to_string()]
    );

    // Non-filter settings unchanged
    let other: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'other_key'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(other, "some\nvalue");
}

#[tokio::test]
async fn migration_v29_skips_already_json_presets() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=28;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan_path   TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed','conflict')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             plan_path   TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL,
             mode       TEXT NOT NULL DEFAULT 'include'
         );
         -- Already JSON — should not be double-converted
         INSERT INTO filter_presets (name, repo_paths, mode)
             VALUES ('already_json', '[\"/repo/a\",\"/repo/b\"]', 'include');
         INSERT INTO settings (key, value)
             VALUES ('repo_filter', '[\"/repo/x\"]');",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    let preset: String = conn
        .query_row(
            "SELECT repo_paths FROM filter_presets WHERE name = 'already_json'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let paths: Vec<String> = serde_json::from_str(&preset).unwrap();
    assert_eq!(paths, vec!["/repo/a".to_string(), "/repo/b".to_string()]);

    let filter: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let filter_paths: Vec<String> = serde_json::from_str(&filter).unwrap();
    assert_eq!(filter_paths, vec!["/repo/x".to_string()]);
}

#[tokio::test]
async fn migration_31_re_expands_tilde_paths() {
    // Simulate a v30 DB where tilde paths snuck in after the v18 migration
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         PRAGMA user_version=30;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan_path   TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed','conflict')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved','conflict')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             plan_path   TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE filter_presets (
             name       TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL,
             mode       TEXT NOT NULL DEFAULT 'include'
         );",
    )
    .unwrap();

    let home = std::env::var("HOME").unwrap();

    // Insert rows with tilde paths
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path) VALUES ('T1', 'D', '~/code/project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path) VALUES ('T2', 'D', '/absolute/path')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO epics (title, description, repo_path) VALUES ('E1', 'D', '~/code/epic')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO repo_paths (path) VALUES ('~/code/saved')", [])
        .unwrap();
    // filter_presets are now JSON arrays (post v29)
    conn.execute(
        r#"INSERT INTO filter_presets (name, repo_paths) VALUES ('my_preset', '["~/code/a","~/code/b","/abs/c"]')"#,
        [],
    )
    .unwrap();
    conn.execute(
        r#"INSERT INTO settings (key, value) VALUES ('repo_filter', '["~/code/x"]')"#,
        [],
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    // tasks.repo_path expanded
    let repo: String = conn
        .query_row("SELECT repo_path FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(repo, format!("{home}/code/project"));

    // Absolute path unchanged
    let repo2: String = conn
        .query_row("SELECT repo_path FROM tasks WHERE id = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(repo2, "/absolute/path");

    // epics.repo_path expanded
    let epic_repo: String = conn
        .query_row("SELECT repo_path FROM epics WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(epic_repo, format!("{home}/code/epic"));

    // repo_paths.path expanded
    let rp: String = conn
        .query_row("SELECT path FROM repo_paths WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rp, format!("{home}/code/saved"));

    // filter_presets.repo_paths (JSON) expanded
    let preset: String = conn
        .query_row(
            "SELECT repo_paths FROM filter_presets WHERE name = 'my_preset'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let paths: Vec<String> = serde_json::from_str(&preset).unwrap();
    assert_eq!(
        paths,
        vec![
            format!("{home}/code/a"),
            format!("{home}/code/b"),
            "/abs/c".to_string(),
        ]
    );

    // settings.repo_filter (JSON) expanded
    let filter: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let filter_paths: Vec<String> = serde_json::from_str(&filter).unwrap();
    assert_eq!(filter_paths, vec![format!("{home}/code/x")]);

    // Version bumped
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migrate_v32_adds_base_branch_column() {
    let conn = Connection::open_in_memory().unwrap();
    // Build a v31 schema (tasks table with CHECK constraint from v30, plus repo_paths).
    // Setting user_version = 31 ensures only v32 runs when init_schema is called.
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan_path   TEXT,
             epic_id     INTEGER,
             sub_status  TEXT NOT NULL DEFAULT 'none',
             pr_url      TEXT,
             tag         TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (status = 'backlog'  AND sub_status = 'none') OR
                 (status = 'running'  AND sub_status IN ('active','needs_input','stale','crashed','conflict')) OR
                 (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved','conflict')) OR
                 (status = 'done'     AND sub_status = 'none') OR
                 (status = 'archived' AND sub_status = 'none')
             )
         );
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             plan_path   TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE repo_paths (
             id        INTEGER PRIMARY KEY,
             path      TEXT NOT NULL UNIQUE,
             last_used TEXT NOT NULL DEFAULT (datetime('now'))
         );
         PRAGMA user_version = 31;",
    )
    .unwrap();

    // Insert a task using the v31 schema (no base_branch column yet)
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path) VALUES ('Old Task', 'pre-migration desc', '/repo')",
        [],
    )
    .unwrap();

    // Run init_schema: only v32 should run (user_version = 31)
    super::super::init_schema_sync(&conn).unwrap();

    // Existing task should have base_branch defaulted to 'main'
    let base_branch: String = conn
        .query_row("SELECT base_branch FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(base_branch, "main");

    // init_schema runs all pending migrations (v32 and v33), so final version is 39
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migration_v33_adds_auto_dispatch_to_epics() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA user_version=32;
         CREATE TABLE epics (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             plan_path   TEXT,
             sort_order  INTEGER,
             created_at  TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO epics (title, description, repo_path) VALUES ('Test', 'desc', '/r');",
    )
    .unwrap();

    crate::db::migrations::migrate_v33_add_auto_dispatch(&conn).unwrap();

    let auto_dispatch: i64 = conn
        .query_row(
            "SELECT auto_dispatch FROM epics WHERE title = 'Test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(auto_dispatch, 1);
}

#[tokio::test]
async fn migrate_v37_creates_pr_workflow_states_table() {
    let db = in_memory_db().await;

    let (count, dup_failed) = db
        .db_call(|conn| {
            // Table must exist
            let count: i64 = conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='pr_workflow_states'",
                [],
                |r| r.get(0),
            )?;

            // Primary key enforced: duplicate (repo, number, kind) must fail
            conn.execute(
                "INSERT INTO pr_workflow_states (repo, number, kind, state, updated_at)
                 VALUES ('org/repo', 1, 'reviewer_pr', 'backlog', '2026-01-01T00:00:00Z')",
                [],
            )?;
            let dup_failed = conn
                .execute(
                    "INSERT INTO pr_workflow_states (repo, number, kind, state, updated_at)
                     VALUES ('org/repo', 1, 'reviewer_pr', 'ongoing', '2026-01-01T00:00:00Z')",
                    [],
                )
                .is_err();

            // sub_state nullable: NULL is allowed
            conn.execute(
                "INSERT INTO pr_workflow_states (repo, number, kind, state, sub_state, updated_at)
                 VALUES ('org/repo', 2, 'reviewer_pr', 'ongoing', NULL, '2026-01-01T00:00:00Z')",
                [],
            )?;
            Ok((count, dup_failed))
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert!(dup_failed);
}

#[tokio::test]
async fn migration_v38_feed_epic_columns() {
    let conn = Connection::open_in_memory().unwrap();
    // Minimal v37 schema: just the tables that v38 ALTER TABLEs
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         PRAGMA user_version=37;
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             epic_id INTEGER
         );",
    )
    .unwrap();

    super::super::init_schema_sync(&conn).unwrap();

    assert!(
        conn.prepare("SELECT feed_command FROM epics LIMIT 1")
            .is_ok(),
        "feed_command column should exist on epics"
    );
    assert!(
        conn.prepare("SELECT feed_interval_secs FROM epics LIMIT 1")
            .is_ok(),
        "feed_interval_secs column should exist on epics"
    );
    assert!(
        conn.prepare("SELECT external_id FROM tasks LIMIT 1")
            .is_ok(),
        "external_id column should exist on tasks"
    );
    let index_exists: bool = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='index' AND name='tasks_epic_external_id'",
        )
        .unwrap()
        .exists([])
        .unwrap();
    assert!(index_exists, "tasks_epic_external_id index should exist");

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn fresh_db_schema_version_is_40() {
    let db = in_memory_db().await;
    let version: i64 = db
        .db_call(|conn| {
            conn.pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migration_v40_creates_learnings_table() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // Simulate a v39 database
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan_path TEXT,
             tag TEXT,
             epic_id INTEGER,
             sub_status TEXT NOT NULL DEFAULT 'none',
             pr_url TEXT,
             sort_order INTEGER,
             base_branch TEXT NOT NULL DEFAULT 'main',
             external_id TEXT,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             plan_path TEXT,
             sort_order INTEGER,
             auto_dispatch INTEGER NOT NULL DEFAULT 0,
             parent_epic_id INTEGER,
             feed_command TEXT,
             feed_interval_secs INTEGER,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE projects (
             id INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             sort_order INTEGER NOT NULL DEFAULT 0,
             is_default INTEGER NOT NULL DEFAULT 0
         );
         INSERT INTO projects (name, sort_order, is_default) VALUES ('Default', 0, 1);
         PRAGMA user_version = 39;",
    )
    .unwrap();
    super::super::init_schema_sync(&conn).unwrap();
    // learnings table must exist
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='learnings'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "learnings table should exist after migration v40");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migration_v41_drops_cost_usd_column() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // Simulate a v40 database with task_usage including cost_usd
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=OFF;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan_path TEXT,
             tag TEXT,
             epic_id INTEGER,
             sub_status TEXT NOT NULL DEFAULT 'none',
             pr_url TEXT,
             sort_order INTEGER,
             base_branch TEXT NOT NULL DEFAULT 'main',
             external_id TEXT,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE task_usage (
             task_id INTEGER NOT NULL PRIMARY KEY REFERENCES tasks(id),
             cost_usd REAL NOT NULL DEFAULT 0.0,
             input_tokens INTEGER NOT NULL DEFAULT 0,
             output_tokens INTEGER NOT NULL DEFAULT 0,
             cache_read_tokens INTEGER NOT NULL DEFAULT 0,
             cache_write_tokens INTEGER NOT NULL DEFAULT 0,
             updated_at TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE learnings (
             id                INTEGER PRIMARY KEY,
             kind              TEXT    NOT NULL,
             summary           TEXT    NOT NULL,
             detail            TEXT,
             scope             TEXT    NOT NULL,
             scope_ref         TEXT,
             tags              TEXT    NOT NULL DEFAULT '[]',
             status            TEXT    NOT NULL DEFAULT 'approved',
             source_task_id    INTEGER REFERENCES tasks(id),
             confirmed_count   INTEGER NOT NULL DEFAULT 0,
             last_confirmed_at TEXT,
             created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             updated_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (scope = 'user' AND scope_ref IS NULL)
                 OR (scope != 'user' AND scope_ref IS NOT NULL)
             )
         );
         CREATE INDEX IF NOT EXISTS idx_learnings_scope ON learnings(scope, scope_ref);
         CREATE INDEX IF NOT EXISTS idx_learnings_status ON learnings(status);
         INSERT INTO tasks (id, title, status) VALUES (999, 'test', 'backlog');
         INSERT INTO task_usage (task_id, cost_usd, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, updated_at)
             VALUES (999, 0.42, 100, 50, 10, 5, '');
         PRAGMA user_version = 40;",
    )
    .unwrap();
    super::super::init_schema_sync(&conn).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 49);
    // Original token data is preserved
    let row: (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT task_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens
             FROM task_usage WHERE task_id = 999",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(row, (999, 100, 50, 10, 5));
    // cost_usd column no longer exists
    let has_cost_usd: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('task_usage') WHERE name = 'cost_usd'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    assert!(
        !has_cost_usd,
        "cost_usd column should have been removed by migration v41"
    );
}

#[tokio::test]
async fn test_migrate_v43_proposed_to_approved() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // Build a v42 database with the learnings table using DEFAULT 'proposed'
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             worktree TEXT,
             tmux_window TEXT,
             plan_path TEXT,
             tag TEXT,
             epic_id INTEGER,
             sub_status TEXT NOT NULL DEFAULT 'none',
             pr_url TEXT,
             sort_order INTEGER,
             base_branch TEXT NOT NULL DEFAULT 'main',
             external_id TEXT,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE learnings (
             id                INTEGER PRIMARY KEY,
             kind              TEXT    NOT NULL,
             summary           TEXT    NOT NULL,
             detail            TEXT,
             scope             TEXT    NOT NULL,
             scope_ref         TEXT,
             tags              TEXT    NOT NULL DEFAULT '[]',
             status            TEXT    NOT NULL DEFAULT 'proposed',
             source_task_id    INTEGER REFERENCES tasks(id),
             confirmed_count   INTEGER NOT NULL DEFAULT 0,
             last_confirmed_at TEXT,
             created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             updated_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             CHECK (
                 (scope = 'user' AND scope_ref IS NULL)
                 OR (scope != 'user' AND scope_ref IS NOT NULL)
             )
         );
         CREATE INDEX IF NOT EXISTS idx_learnings_scope ON learnings(scope, scope_ref);
         CREATE INDEX IF NOT EXISTS idx_learnings_status ON learnings(status);
         INSERT INTO learnings (kind, summary, scope, status) VALUES ('pitfall', 'test', 'user', 'proposed');
         PRAGMA user_version = 42;",
    )
    .unwrap();

    // Apply v43 via init_schema
    super::super::init_schema_sync(&conn).unwrap();

    // Assert: the previously proposed row is now approved
    let status: String = conn
        .query_row(
            "SELECT status FROM learnings WHERE summary = 'test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "approved");

    // Assert: new inserts default to approved (not proposed)
    conn.execute(
        "INSERT INTO learnings (kind, summary, scope) VALUES ('pitfall', 'new', 'user')",
        [],
    )
    .unwrap();
    let new_status: String = conn
        .query_row(
            "SELECT status FROM learnings WHERE summary = 'new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new_status, "approved");

    // Assert: schema version bumped to 44
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 49);
}

#[tokio::test]
async fn migration_v44_converts_episodic_to_convention() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE learnings (
             id                INTEGER PRIMARY KEY,
             kind              TEXT    NOT NULL,
             summary           TEXT    NOT NULL,
             detail            TEXT,
             scope             TEXT    NOT NULL,
             scope_ref         TEXT,
             tags              TEXT    NOT NULL DEFAULT '[]',
             status            TEXT    NOT NULL DEFAULT 'approved',
             source_task_id    INTEGER,
             confirmed_count   INTEGER NOT NULL DEFAULT 0,
             last_confirmed_at TEXT,
             created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
             updated_at        TEXT    NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO learnings (kind, summary, scope, status)
             VALUES ('episodic', 'how I solved task 42', 'user', 'approved');
         INSERT INTO learnings (kind, summary, scope, status)
             VALUES ('convention', 'use Arc for shared state', 'user', 'approved');
         PRAGMA user_version = 43;",
    )
    .unwrap();

    crate::db::migrations::migrate_v44_episodic_to_convention(&conn).unwrap();

    let mut stmt = conn
        .prepare("SELECT kind FROM learnings ORDER BY id")
        .unwrap();
    let kinds: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        kinds[0], "convention",
        "episodic entry should be converted to convention"
    );
    assert_eq!(
        kinds[1], "convention",
        "non-episodic entry should be unchanged"
    );
}

#[tokio::test]
async fn migration_v45_adds_labels_column_with_default() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO tasks (title, repo_path, status)
             VALUES ('legacy task', '/repo', 'backlog');
         PRAGMA user_version = 44;",
    )
    .unwrap();

    // Pre-migration: no labels column
    assert!(
        conn.prepare("SELECT labels FROM tasks LIMIT 1").is_err(),
        "labels column should not exist before migration v45"
    );

    crate::db::migrations::migrate_v45_add_task_labels(&conn).unwrap();

    let labels: String = conn
        .query_row("SELECT labels FROM tasks WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(labels, "[]");

    conn.execute(
        "INSERT INTO tasks (title, repo_path, status) VALUES ('new task', '/repo', 'backlog')",
        [],
    )
    .unwrap();
    let new_labels: String = conn
        .query_row(
            "SELECT labels FROM tasks WHERE title = 'new task'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(new_labels, "[]");
}

#[tokio::test]
async fn migration_v45_is_idempotent() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             labels TEXT NOT NULL DEFAULT '[]'
         );",
    )
    .unwrap();

    // Running the migration on a DB that already has the column must be a no-op.
    crate::db::migrations::migrate_v45_add_task_labels(&conn).unwrap();
}
