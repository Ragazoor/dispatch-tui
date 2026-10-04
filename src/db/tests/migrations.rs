use super::*;

#[tokio::test]
async fn a_fresh_db_has_no_tips_state_table() {
    let db = in_memory_db().await;
    let tables: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tips_state'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(
        tables, 0,
        "v84 must leave no tips_state table behind on a fresh database"
    );
}

/// v36 created `tips_state`; v84 drops it. The historical v36 entry stays in
/// `MIGRATIONS` untouched, so an existing database still creates the table on
/// its way forward and must then lose it — including when the row carries a
/// non-default watermark and show mode.
#[tokio::test]
async fn migration_84_drops_a_populated_v36_tips_state_table() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // The v36 schema, built by migrate_v36_tips_state itself so the fixture
    // cannot drift from what shipped.
    crate::db::migrations::migrate_v36_tips_state(&conn).unwrap();
    conn.execute(
        "UPDATE tips_state SET seen_up_to = 9, show_mode = 'new_only' WHERE id = 1",
        [],
    )
    .unwrap();

    crate::db::migrations::migrate_v84_drop_tips_state(&conn).unwrap();

    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tips_state'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0, "v84 must drop the populated tips_state table");
}

/// The drop is unconditional DDL, so it must tolerate both a database that
/// never had the table and a second application against one that has already
/// lost it.
#[tokio::test]
async fn migration_84_is_idempotent_without_a_tips_state_table() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    crate::db::migrations::migrate_v84_drop_tips_state(&conn).unwrap();
    crate::db::migrations::migrate_v84_drop_tips_state(&conn).unwrap();
}

/// The four tables of the removed Review and Security boards. Nothing outside
/// `src/db/migrations.rs` reads or writes them; v103 drops them.
const LEGACY_PR_TABLES: [&str; 4] = ["my_prs", "review_prs", "bot_prs", "security_alerts"];

fn legacy_pr_tables_present(conn: &rusqlite::Connection) -> Vec<&'static str> {
    LEGACY_PR_TABLES
        .into_iter()
        .filter(|table| crate::db::migrations::table_exists(conn, table))
        .collect()
}

/// The legacy PR tables as v26 left them, built by the migrations that
/// shipped them so the fixture cannot drift from what ran in production.
fn conn_with_legacy_pr_tables() -> rusqlite::Connection {
    use crate::db::migrations as m;
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    m::migrate_v14_create_review_prs_table(&conn).unwrap();
    m::migrate_v21_create_my_prs_table(&conn).unwrap();
    m::migrate_v23_create_bot_prs_table(&conn).unwrap();
    m::migrate_v24_create_security_alerts_table(&conn).unwrap();
    m::migrate_v26_add_agent_columns(&conn).unwrap();
    conn
}

#[tokio::test]
async fn a_fresh_db_has_no_legacy_pr_tables() {
    let db = in_memory_db().await;
    let present = db
        .db_call(|conn| Ok(legacy_pr_tables_present(conn)))
        .await
        .unwrap();
    assert!(
        present.is_empty(),
        "v103 must leave no legacy PR table behind on a fresh database, found {present:?}"
    );
}

/// v14/v21/v23/v24 created the tables and v26 widened them; v103 drops them.
/// The historical entries stay in `MIGRATIONS` untouched, so an existing
/// database still creates the tables on its way forward and must then lose
/// them — including when they hold rows.
#[test]
fn migration_103_drops_populated_legacy_pr_tables() {
    let conn = conn_with_legacy_pr_tables();
    conn.execute(
        "INSERT INTO my_prs (repo, number, title, author, url, is_draft,
         created_at, updated_at, additions, deletions, review_decision, labels)
         VALUES ('acme/app', 1, 'Test', 'alice', 'https://example.com', 0,
         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', 0, 0, 'ReviewRequired', '[]')",
        [],
    )
    .unwrap();
    assert_eq!(legacy_pr_tables_present(&conn), LEGACY_PR_TABLES);

    crate::db::migrations::migrate_v103_drop_legacy_pr_tables(&conn).unwrap();

    let present = legacy_pr_tables_present(&conn);
    assert!(present.is_empty(), "v103 left {present:?} behind");
}

/// The drop is unconditional DDL, so it must tolerate a database that never
/// had the tables and a second application against one that already lost them.
#[test]
fn migration_103_is_idempotent_without_legacy_pr_tables() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::migrate_v103_drop_legacy_pr_tables(&conn).unwrap();
    crate::db::migrations::migrate_v103_drop_legacy_pr_tables(&conn).unwrap();
}

#[tokio::test]
async fn migration_81_creates_task_subagents_and_columns() {
    let db = in_memory_db().await;
    let has = db
        .db_call(|conn| {
            let table: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_subagents'",
                [],
                |r| r.get(0),
            )?;
            let live: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='live_subagents'",
                [],
                |r| r.get(0),
            )?;
            let pending: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='stop_pending'",
                [],
                |r| r.get(0),
            )?;
            Ok((table, live, pending))
        })
        .await
        .expect("query schema");
    assert_eq!(
        has,
        (1, 1, 1),
        "migration 81 must create the table and both columns"
    );
}

/// v85 created `task_shells` and the two `tasks` columns backing it; v101
/// drops all three (#4965 retired the shell-tracking feature). A fresh
/// database must show no trace of either.
#[tokio::test]
async fn a_fresh_db_has_no_task_shells_table_or_columns() {
    let db = in_memory_db().await;
    let has = db
        .db_call(|conn| {
            let table: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_shells'",
                [],
                |r| r.get(0),
            )?;
            let live: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='live_shells'",
                [],
                |r| r.get(0),
            )?;
            let oldest: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='oldest_live_shell_started_at'",
                [],
                |r| r.get(0),
            )?;
            Ok((table, live, oldest))
        })
        .await
        .expect("query schema");
    assert_eq!(
        has,
        (0, 0, 0),
        "v101 must leave no task_shells table or tasks columns behind on a fresh database"
    );
}

/// v85 created `task_shells` and the two `tasks` columns; v101 drops all
/// three. The historical v85 entry stays in `MIGRATIONS` untouched, so an
/// existing database still creates them on its way forward and must then
/// lose them — including when the table carries a populated row.
#[tokio::test]
async fn migration_101_drops_a_populated_v85_task_shells_table_and_columns() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (id INTEGER PRIMARY KEY);
         INSERT INTO tasks (id) VALUES (1);",
    )
    .unwrap();
    crate::db::migrations::migrate_v85_create_task_shells(&conn).unwrap();
    conn.execute(
        "INSERT INTO task_shells (task_id, shell_id, session_id, started_at) \
         VALUES (1, 'bash_1', 'sess_1', '2026-09-26 10:00:00.000')",
        [],
    )
    .unwrap();
    conn.execute("UPDATE tasks SET live_shells = 1 WHERE id = 1", [])
        .unwrap();

    crate::db::migrations::migrate_v101_drop_shell_tracking(&conn).unwrap();

    let has = {
        let table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_shells'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let live: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='live_shells'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let oldest: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='oldest_live_shell_started_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (table, live, oldest)
    };
    assert_eq!(
        has,
        (0, 0, 0),
        "v101 must drop the populated task_shells table and both tasks columns"
    );
}

/// The drop is unconditional DDL guarded by `column_exists`/`IF EXISTS`, so it
/// must tolerate both a database that never had them and a second application
/// against one that has already lost them.
#[tokio::test]
async fn migration_101_is_idempotent_without_task_shells() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE tasks (id INTEGER PRIMARY KEY);")
        .unwrap();
    crate::db::migrations::migrate_v101_drop_shell_tracking(&conn).unwrap();
    crate::db::migrations::migrate_v101_drop_shell_tracking(&conn).unwrap();
}

/// v67 created `todos`; v102 drops it (#4970 removed the TODO subsystem).
/// A fresh database must show no trace of it.
#[tokio::test]
async fn a_fresh_db_has_no_todos_table() {
    let db = in_memory_db().await;
    let tables: i64 = db
        .db_call(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='todos'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("query schema");
    assert_eq!(
        tables, 0,
        "v102 must leave no todos table on a fresh database"
    );
}

/// An existing database arrives at v102 with a populated `todos` table, built
/// by the historical migrations that created it, a nested and a linked row
/// included. The drop takes the rows with it: the user chose removal without
/// an export.
#[tokio::test]
async fn migration_102_drops_a_populated_todos_table() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (id INTEGER PRIMARY KEY);
         CREATE TABLE epics (id INTEGER PRIMARY KEY);
         INSERT INTO tasks (id) VALUES (1);",
    )
    .unwrap();
    crate::db::migrations::migrate_v67_create_todos(&conn).unwrap();
    crate::db::migrations::migrate_v68_add_todo_links(&conn).unwrap();
    crate::db::migrations::migrate_v70_add_todo_parent_id(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO todos (id, title, task_id) VALUES (1, 'parent', 1);
         INSERT INTO todos (id, title, parent_id) VALUES (2, 'child', 1);",
    )
    .unwrap();

    crate::db::migrations::migrate_v102_drop_todos(&conn).unwrap();

    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='todos'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0, "v102 must drop the populated todos table");
}

#[tokio::test]
async fn migration_102_is_idempotent_without_todos() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    crate::db::migrations::migrate_v102_drop_todos(&conn).unwrap();
    crate::db::migrations::migrate_v102_drop_todos(&conn).unwrap();
}

/// v11 created `filter_presets`; v104 drops it (#4972 removed filter presets).
#[tokio::test]
async fn a_fresh_db_has_no_filter_presets_table() {
    let db = in_memory_db().await;
    let tables: i64 = db
        .db_call(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='filter_presets'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("query schema");
    assert_eq!(
        tables, 0,
        "v104 must leave no filter_presets table on a fresh database"
    );
}

/// An existing database arrives at v104 with saved presets. The drop takes
/// the rows with it.
#[tokio::test]
async fn migration_104_drops_a_populated_filter_presets_table() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE filter_presets (
             name TEXT PRIMARY KEY,
             repo_paths TEXT NOT NULL,
             mode TEXT NOT NULL DEFAULT 'include'
         );
         INSERT INTO filter_presets (name, repo_paths) VALUES ('backend', '[\"/repo\"]');",
    )
    .unwrap();

    crate::db::migrations::migrate_v104_drop_filter_presets(&conn).unwrap();

    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='filter_presets'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tables, 0,
        "v104 must drop the populated filter_presets table"
    );
}

#[tokio::test]
async fn migration_104_is_idempotent_without_filter_presets() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    crate::db::migrations::migrate_v104_drop_filter_presets(&conn).unwrap();
    crate::db::migrations::migrate_v104_drop_filter_presets(&conn).unwrap();
}

/// v82 is the one-shot replacement for the retired tick reconciler: a database
/// written by the older read-then-write code can still hold a task stranded in
/// `Running` + `stop_pending` + `live_subagents = 0`, and with the reconciler
/// gone nothing else would ever resolve it. The migration is idempotent and
/// conditional, so re-running it here against seeded rows is a faithful test.
#[tokio::test]
async fn migration_82_resolves_a_stranded_pending_stop() {
    let db = unattached_db().await;
    let task = create_task_returning(&db, "t", "d", "/r", None, TaskStatus::Backlog)
        .await
        .unwrap();
    db.db_call(move |conn| {
        conn.execute(
            "UPDATE tasks SET status = 'running', sub_status = 'active', \
             stop_pending = 1, live_subagents = 0, \
             last_pre_tool_use_at = '2026-01-01T00:00:00Z', \
             last_notification_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
            [task.id.0],
        )?;
        crate::db::migrations::migrate_v82_resolve_stranded_pending_stops(conn)
    })
    .await
    .unwrap();

    let reread = db.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(reread.status, TaskStatus::Review);
    assert_eq!(
        reread.sub_status,
        SubStatus::default_for(TaskStatus::Review)
    );
    assert!(!reread.stop_pending);
    assert!(reread.last_pre_tool_use_at.is_none());
    assert!(reread.last_notification_at.is_none());
}

#[tokio::test]
async fn migration_82_leaves_tasks_that_are_not_stranded_alone() {
    let db = unattached_db().await;
    // A deferred Stop with a subagent still live is legitimately waiting.
    let waiting = create_task_returning(&db, "waiting", "d", "/r", None, TaskStatus::Backlog)
        .await
        .unwrap();
    // A plain running task with no Stop withheld.
    let running = create_task_returning(&db, "running", "d", "/r", None, TaskStatus::Backlog)
        .await
        .unwrap();

    db.db_call(move |conn| {
        conn.execute(
            "UPDATE tasks SET status = 'running', sub_status = 'active', \
             stop_pending = 1, live_subagents = 2 WHERE id = ?1",
            [waiting.id.0],
        )?;
        conn.execute(
            "UPDATE tasks SET status = 'running', sub_status = 'active', \
             stop_pending = 0, live_subagents = 0 WHERE id = ?1",
            [running.id.0],
        )?;
        crate::db::migrations::migrate_v82_resolve_stranded_pending_stops(conn)
    })
    .await
    .unwrap();

    let waiting = db.get_task(waiting.id).await.unwrap().unwrap();
    assert_eq!(
        waiting.status,
        TaskStatus::Running,
        "subagents are still live — the Stop is legitimately deferred"
    );
    assert!(waiting.stop_pending);

    let running = db.get_task(running.id).await.unwrap().unwrap();
    assert_eq!(running.status, TaskStatus::Running);
}

/// v83 adds the column `HookStop`'s defer branch stamps and
/// `record_user_prompt_submit` compares against. Additive and idempotent, with
/// no backfill: a null on a row that already carries `stop_pending` reads as
/// "the Stop fired before any prompt", which is the unconditional behaviour
/// those rows were written under.
#[tokio::test]
async fn migration_83_adds_stop_pending_at() {
    let db = in_memory_db().await;
    let column: i64 = db
        .db_call(|conn| {
            // Re-running it must be a no-op, not a duplicate-column error.
            crate::db::migrations::migrate_v83_add_stop_pending_at(conn)?;
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='stop_pending_at'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .expect("query schema");
    assert_eq!(column, 1);
}

#[tokio::test]
async fn new_task_defaults_to_zero_subagents_and_no_pending_stop() {
    let db = in_memory_db().await;
    let task = create_task_returning(&db, "t", "d", "/r", None, TaskStatus::Backlog)
        .await
        .expect("create task");
    assert_eq!(task.live_subagents, 0);
    assert!(!task.stop_pending);
}

#[tokio::test]
async fn fresh_db_sets_performance_pragmas() {
    let db = in_memory_db().await;
    let (synchronous, cache_size, temp_store): (i64, i64, i64) = db
        .db_call(|conn| {
            let pq = |name| {
                conn.pragma_query_value(None, name, |r| r.get::<_, i64>(0))
                    .map_err(anyhow::Error::from)
            };
            Ok((pq("synchronous")?, pq("cache_size")?, pq("temp_store")?))
        })
        .await
        .unwrap();
    assert_eq!(synchronous, 1, "synchronous should be NORMAL (1)");
    assert_eq!(cache_size, -8000, "cache_size should be -8000 (8 MB)");
    assert_eq!(temp_store, 2, "temp_store should be MEMORY (2)");
}

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
    assert_eq!(version, super::super::migrations::LATEST_SCHEMA_VERSION);
}

#[tokio::test]
async fn v64_backfills_url_type_from_pr_url() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL,
             status TEXT NOT NULL,
             sub_status TEXT NOT NULL DEFAULT 'none',
             base_branch TEXT NOT NULL DEFAULT 'main',
             pr_url TEXT,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO tasks (title, description, repo_path, status, sub_status, base_branch, pr_url)
         VALUES
           ('a','','/r','review','awaiting_review','main','https://github.com/o/r/pull/12'),
           ('b','','/r','review','awaiting_review','main','https://github.com/o/r/issues/7'),
           ('c','','/r','review','awaiting_review','main','https://example.com/x'),
           ('d','','/r','backlog','none','main',NULL);
         PRAGMA user_version = 63;",
    )
    .unwrap();

    crate::db::migrations::migrate_v64_typed_url(&conn).unwrap();

    let mut stmt = conn
        .prepare("SELECT title, url, url_type FROM tasks ORDER BY title")
        .unwrap();
    let rows: Vec<(String, Option<String>, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(
        rows[0],
        (
            "a".into(),
            Some("https://github.com/o/r/pull/12".into()),
            Some("pr".into())
        )
    );
    assert_eq!(
        rows[1],
        (
            "b".into(),
            Some("https://github.com/o/r/issues/7".into()),
            Some("issue".into())
        )
    );
    assert_eq!(
        rows[2],
        (
            "c".into(),
            Some("https://example.com/x".into()),
            Some("other".into())
        )
    );
    assert_eq!(rows[3], ("d".into(), None, None));

    let has_pr_url: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('tasks') WHERE name = 'pr_url'")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .next()
        .is_some();
    assert!(!has_pr_url, "pr_url column should be dropped");
}

#[tokio::test]
async fn v53_adds_wrap_up_mode_column() {
    let db = in_memory_db().await;
    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'wrap_up_mode'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn v50_adds_hook_timestamp_columns() {
    let db = in_memory_db().await;
    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') \
                 WHERE name IN ('last_pre_tool_use_at', 'last_notification_at')",
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
async fn v87_adds_peer_message_columns() {
    let db = in_memory_db().await;
    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') \
                 WHERE name IN ('last_peer_message_sent_at', 'last_peer_message_received_at')",
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
async fn latest_schema_keeps_retrievals_drops_verdicts() {
    // learning_retrievals survives; learning_verdicts is dropped by migration v74
    // (verdicts are no longer persisted — rate_learning applies only the score).
    let db = in_memory_db().await;
    let (retrievals, verdicts): (i64, i64) = db
        .db_call(|conn| {
            let retrievals: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'learning_retrievals'",
                [],
                |r| r.get(0),
            )?;
            let verdicts: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'learning_verdicts'",
                [],
                |r| r.get(0),
            )?;
            Ok((retrievals, verdicts))
        })
        .await
        .unwrap();
    assert_eq!(retrievals, 1, "learning_retrievals must still exist");
    assert_eq!(verdicts, 0, "learning_verdicts must be dropped by v74");
}

#[test]
fn migrate_v74_drops_learning_verdicts_table() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE learning_verdicts (
             id          INTEGER PRIMARY KEY,
             task_id     INTEGER NOT NULL,
             learning_id INTEGER NOT NULL,
             verdict     TEXT NOT NULL
         );
         CREATE INDEX idx_lv_learning ON learning_verdicts(learning_id);",
    )
    .unwrap();

    crate::db::migrations::migrate_v74_drop_learning_verdicts(&conn).unwrap();

    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'learning_verdicts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 0, "v74 must drop the learning_verdicts table");

    // Idempotent: running again on a schema without the table must not error.
    crate::db::migrations::migrate_v74_drop_learning_verdicts(&conn).unwrap();
}

#[tokio::test]
async fn v48_schema_stores_arbitrary_status_text() {
    // The learnings schema has no CHECK constraint on `status`, so the historical
    // 'needs_review' string can still be written and read back at the raw TEXT
    // level. The LearningStatus enum no longer recognises it (migration v73
    // converts any surviving rows to 'approved'), but the storage layer is
    // schema-tolerant. Assert the raw column round-trips.
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
    let status: String = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT status FROM learnings WHERE summary = 'x'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(status, "needs_review");
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

#[test]
fn migrate_v62_drops_only_unused_verdicts() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE learning_verdicts (
             id          INTEGER PRIMARY KEY,
             task_id     INTEGER NOT NULL,
             learning_id INTEGER NOT NULL,
             verdict     TEXT NOT NULL,
             recorded_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO learning_verdicts (task_id, learning_id, verdict) VALUES
             (1, 1, 'unused'),
             (1, 2, 'helped'),
             (1, 3, 'wrong'),
             (2, 1, 'unused');",
    )
    .unwrap();

    crate::db::migrations::migrate_v62_drop_unused_verdicts(&conn).unwrap();

    let unused: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM learning_verdicts WHERE verdict = 'unused'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unused, 0, "all 'unused' verdict rows must be deleted");

    let kept: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM learning_verdicts WHERE verdict IN ('helped','wrong')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        kept, 2,
        "'helped' and 'wrong' verdict rows must be preserved"
    );
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
async fn migration_v39_and_v60_round_trip_project_columns() {
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
    // Apply all migrations including v39 (adds project_id) then v60 (drops it)
    super::super::init_schema_sync(&conn).unwrap();
    // After v60, project_id should no longer exist on tasks or epics
    let tasks_has_project_id: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='project_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tasks_has_project_id, 0,
        "v60 should drop project_id from tasks"
    );
    let epics_has_project_id: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('epics') WHERE name='project_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        epics_has_project_id, 0,
        "v60 should drop project_id from epics"
    );
    // Data is preserved
    let title: String = conn
        .query_row("SELECT title FROM tasks WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "Old task");
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
    assert_eq!(version, super::super::migrations::LATEST_SCHEMA_VERSION);
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
    assert_eq!(version, super::super::migrations::LATEST_SCHEMA_VERSION);
}

/// v103 drops these tables, so a fresh database cannot show v26's columns;
/// the tables are built here by the migrations that shipped them.
#[test]
fn migrate_v26_adds_agent_columns() {
    let conn = conn_with_legacy_pr_tables();
    conn.execute(
        "INSERT INTO review_prs (repo, number, title, author, url, is_draft,
         created_at, updated_at, additions, deletions, review_decision,
         labels, body, head_ref, ci_status, reviewers, tmux_window, worktree)
         VALUES ('acme/app', 1, 'Test', 'alice', 'https://example.com', 0,
         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z', 0, 0, 'ReviewRequired',
         '[]', '', '', 'None', '[]', 'dispatch:review-1', '/tmp/wt')",
        [],
    )
    .unwrap();
    let (tw1, wt1): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT tmux_window, worktree FROM review_prs WHERE repo = 'acme/app' AND number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO security_alerts (repo, number, kind, severity, title,
         url, created_at, state, description, tmux_window, worktree)
         VALUES ('acme/app', 1, 'dependabot', 'high', 'Alert',
         'https://example.com', '2024-01-01T00:00:00Z', 'open', 'desc',
         'dispatch:fix-1', '/tmp/wt4')",
        [],
    )
    .unwrap();
    let (tw2, wt2): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT tmux_window, worktree FROM security_alerts WHERE repo = 'acme/app'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
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
    assert_eq!(version, super::super::migrations::LATEST_SCHEMA_VERSION);
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
    assert_eq!(version, super::super::migrations::LATEST_SCHEMA_VERSION);

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
    assert_eq!(version, super::super::migrations::LATEST_SCHEMA_VERSION);

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
