use super::*;

// ---------------------------------------------------------------------------
// v75 — create repo_base_branches table (see docs/specs/dispatch.allium:
// RecordBaseBranch/BaseBranchPicker and docs/specs/core.allium: SavedRepoBranch)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fresh_db_creates_repo_base_branches_table() {
    let db = in_memory_db().await;
    let exists: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'repo_base_branches'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(
        exists, 1,
        "expected migration v75 to create the repo_base_branches table"
    );
}

#[test]
fn migrate_v75_creates_repo_base_branches_table_on_legacy_db() {
    use rusqlite::Connection as RawConn;
    // Simulate a v74 DB: no repo_base_branches table yet.
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 74;").unwrap();

    crate::db::migrations::migrate_v75_create_repo_base_branches(&conn).unwrap();

    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'repo_base_branches'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        exists, 1,
        "migrate_v75 should create the repo_base_branches table on a legacy DB"
    );

    // Idempotent: running again on a schema that already has the table must not error.
    crate::db::migrations::migrate_v75_create_repo_base_branches(&conn).unwrap();
}

#[test]
fn migrate_v75_repo_base_branches_schema_supports_upsert_and_recency_query() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 74;").unwrap();
    crate::db::migrations::migrate_v75_create_repo_base_branches(&conn).unwrap();

    // Columns required by RecordBaseBranch / BaseBranchPicker: repo_path,
    // branch, last_used. Identity is the (repo_path, branch) pair (see
    // core.allium: SavedRepoBranch / UniqueBranchPerRepo), so a second insert
    // of the same pair must be rejected by a uniqueness constraint (the
    // production upsert uses ON CONFLICT against exactly this constraint).
    conn.execute(
        "INSERT INTO repo_base_branches (repo_path, branch, last_used) VALUES ('/r', 'main', datetime('now'))",
        [],
    )
    .unwrap();
    let dup = conn.execute(
        "INSERT INTO repo_base_branches (repo_path, branch, last_used) VALUES ('/r', 'main', datetime('now'))",
        [],
    );
    assert!(
        dup.is_err(),
        "expected a uniqueness violation on (repo_path, branch), got: {dup:?}"
    );
}

#[tokio::test]
async fn auto_run_plan_column_defaults_to_false() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap().expect("task should exist");
    assert!(!task.auto_run_plan);
}

#[tokio::test]
async fn fresh_db_creates_task_watchers_table() {
    let db = in_memory_db().await;
    let exists: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'task_watchers'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(
        exists, 1,
        "expected migration v78 to create the task_watchers table"
    );
}

#[test]
fn migrate_v78_creates_task_watchers_table_on_legacy_db() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA user_version = 77;").unwrap();

    crate::db::migrations::migrate_v78_create_task_watchers(&conn).unwrap();

    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'task_watchers'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        exists, 1,
        "migrate_v78 should create the task_watchers table on a legacy DB"
    );

    // Idempotent: running again must not error.
    crate::db::migrations::migrate_v78_create_task_watchers(&conn).unwrap();
}

#[test]
fn migrate_v78_task_watchers_unique_constraint_rejects_duplicate_pair() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    crate::db::migrations::migrate_v78_create_task_watchers(&conn).unwrap();

    conn.execute(
        "INSERT INTO task_watchers (watcher_task_id, target_task_id) VALUES (1, 2)",
        [],
    )
    .unwrap();

    let err = conn
        .execute(
            "INSERT INTO task_watchers (watcher_task_id, target_task_id) VALUES (1, 2)",
            [],
        )
        .unwrap_err();
    assert!(format!("{err}").to_lowercase().contains("unique"));
}

/// Regression test for the race described in task #3724: `init_schema_sync`
/// used to run a migration's DDL and its `user_version` bump as two separate
/// statements, with no shared transaction and no cross-process lock around
/// the pair. Two processes opening the same DB file concurrently could both
/// observe the old `user_version` and both apply the same migration.
///
/// This exercises the extracted `apply_pending_migrations` helper directly
/// with a synthetic, deliberately non-idempotent migration (an unconditional
/// INSERT, mirroring the real `migrate_v39_add_projects`'s unguarded
/// `INSERT INTO projects ... VALUES ('Default', ...)`), racing two real
/// file-backed connections on two OS threads. This is not timing-flaky: the
/// pre-fix bug is a structural check-then-act race (both threads decide
/// "pending" from a version read taken before either one starts writing),
/// not a narrow window — so it reproduces deterministically without the fix,
/// and is deterministically fixed once each migration is applied inside a
/// single `BEGIN IMMEDIATE` transaction that re-checks the version before
/// running the migration body.
#[test]
fn concurrent_open_applies_pending_migration_exactly_once() {
    fn insert_marker(conn: &rusqlite::Connection) -> anyhow::Result<()> {
        conn.execute("INSERT INTO marker DEFAULT VALUES", [])?;
        Ok(())
    }
    const TEST_MIGRATIONS: &[crate::db::migrations::Migration] = &[(1, insert_marker)];

    let temp = tempfile::NamedTempFile::new().unwrap();
    {
        let setup = rusqlite::Connection::open(temp.path()).unwrap();
        setup
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE marker (id INTEGER PRIMARY KEY);",
            )
            .unwrap();
    }

    let run = |path: std::path::PathBuf| {
        std::thread::spawn(move || {
            let conn = rusqlite::Connection::open(path).unwrap();
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            crate::db::apply_pending_migrations(&conn, TEST_MIGRATIONS).unwrap();
        })
    };
    let a = run(temp.path().to_path_buf());
    let b = run(temp.path().to_path_buf());
    a.join().unwrap();
    b.join().unwrap();

    let conn = rusqlite::Connection::open(temp.path()).unwrap();
    let marker_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM marker", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        marker_count, 1,
        "migration must apply exactly once even when two connections race to apply it"
    );
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

#[tokio::test]
async fn v79_backfills_sort_order_for_done_tasks_and_epics() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL,
             sort_order INTEGER,
             updated_at TEXT NOT NULL
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL,
             sort_order INTEGER,
             updated_at TEXT NOT NULL
         );
         INSERT INTO tasks (title, status, sort_order, updated_at) VALUES
           ('done-no-sort-order', 'done', NULL, '2026-01-15 12:00:00'),
           ('done-already-sorted', 'done', -999, '2026-01-15 12:00:00'),
           ('not-done', 'backlog', NULL, '2026-01-15 12:00:00');
         INSERT INTO epics (title, status, sort_order, updated_at) VALUES
           ('epic-done-no-sort-order', 'done', NULL, '2026-02-01 08:30:00'),
           ('epic-not-done', 'running', NULL, '2026-02-01 08:30:00');",
    )
    .unwrap();

    crate::db::migrations::migrate_v79_backfill_done_sort_order(&conn).unwrap();

    let mut stmt = conn
        .prepare("SELECT title, sort_order FROM tasks ORDER BY title")
        .unwrap();
    let task_rows: Vec<(String, Option<i64>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(task_rows[0].0, "done-already-sorted");
    assert_eq!(
        task_rows[0].1,
        Some(-999),
        "an already-set sort_order must not be overwritten"
    );
    assert_eq!(task_rows[1].0, "done-no-sort-order");
    assert!(
        task_rows[1].1.is_some_and(|so| so < 0),
        "a null sort_order on a Done task must be backfilled to a negative value, got {:?}",
        task_rows[1].1
    );
    assert_eq!(task_rows[2].0, "not-done");
    assert_eq!(
        task_rows[2].1, None,
        "a non-Done task's null sort_order must be left alone"
    );

    let mut estmt = conn
        .prepare("SELECT title, sort_order FROM epics ORDER BY title")
        .unwrap();
    let epic_rows: Vec<(String, Option<i64>)> = estmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(epic_rows[0].0, "epic-done-no-sort-order");
    assert!(epic_rows[0].1.is_some_and(|so| so < 0));
    assert_eq!(epic_rows[1].0, "epic-not-done");
    assert_eq!(epic_rows[1].1, None);
}

/// The four scheduling columns land as nullable, so every pre-existing row
/// reads back as "not scheduled, not pinned, never processed, never checked".
#[test]
fn migration_v88_adds_scheduling_fields_to_tasks() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO tasks(title) VALUES('pre-existing');",
    )
    .unwrap();

    crate::db::migrations::migrate_v88_add_scheduling_fields(&conn).unwrap();

    let columns: Vec<(String, String, i64)> = conn
        .prepare("SELECT name, type, \"notnull\" FROM pragma_table_info('tasks')")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    for (name, ty) in [
        ("schedule_interval_secs", "INTEGER"),
        ("pinned_branch", "TEXT"),
        ("last_processed_sha", "TEXT"),
        ("last_scheduled_check_at", "TEXT"),
    ] {
        let col = columns
            .iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("{name} must be added by migration v88"));
        assert_eq!(col.1, ty, "{name} type");
        assert_eq!(col.2, 0, "{name} must be nullable");
    }

    let row: (Option<i64>, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT schedule_interval_secs, pinned_branch, last_processed_sha, \
             last_scheduled_check_at FROM tasks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (None, None, None, None),
        "existing rows stay unscheduled"
    );
}

#[test]
fn migration_v88_is_idempotent() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        // `status` is present because the real table has carried it since long
        // before v88, and the partial index is built over it.
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             schedule_interval_secs INTEGER,
             pinned_branch TEXT,
             last_processed_sha TEXT,
             last_scheduled_check_at TEXT
         );",
    )
    .unwrap();

    // Twice: both the ALTERs and the index creation must be no-ops the second
    // time (the index is what the first run leaves behind).
    crate::db::migrations::migrate_v88_add_scheduling_fields(&conn).unwrap();
    crate::db::migrations::migrate_v88_add_scheduling_fields(&conn).unwrap();
}

/// v88's four columns and its partial index go away again: the scheduling
/// feature was reverted wholesale (task #4407), so nothing reads them.
///
/// The index must be dropped BEFORE the columns — SQLite refuses `DROP COLUMN`
/// on a column any index mentions, and `idx_tasks_scheduled` is partial on
/// `schedule_interval_secs`.
#[test]
fn migration_v90_drops_the_scheduling_fields_and_index() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO tasks(title) VALUES('pre-existing');",
    )
    .unwrap();
    crate::db::migrations::migrate_v88_add_scheduling_fields(&conn).unwrap();

    crate::db::migrations::migrate_v90_drop_scheduling_fields(&conn).unwrap();

    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('tasks')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for name in [
        "schedule_interval_secs",
        "pinned_branch",
        "last_processed_sha",
        "last_scheduled_check_at",
    ] {
        assert!(
            !columns.iter().any(|c| c == name),
            "{name} must be dropped by migration v90"
        );
    }

    let indexes: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master \
             WHERE type = 'index' AND name = 'idx_tasks_scheduled'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 0, "idx_tasks_scheduled must be dropped");

    // The rows themselves survive — this drops columns, not data.
    let title: String = conn
        .query_row("SELECT title FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "pre-existing");
}

/// `DROP COLUMN` makes SQLite re-resolve EVERY trigger on the table, not only
/// the ones naming the dropped column — and a trigger body is not resolved
/// when it is created. So `tasks` carrying v72's feed-subtree triggers is the
/// interesting case for v90, and the triggers must still be there afterwards:
/// dropping four unrelated columns may not cost the uniqueness invariant they
/// enforce.
#[test]
fn migration_v90_leaves_the_feed_subtree_triggers_intact() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog',
             epic_id INTEGER,
             external_id TEXT
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             parent_epic_id INTEGER,
             feed_role TEXT NOT NULL DEFAULT 'none',
             origin TEXT NOT NULL DEFAULT 'manual'
         );",
    )
    .unwrap();
    crate::db::migrations::migrate_v88_add_scheduling_fields(&conn).unwrap();
    crate::db::migrations::migrate_v72_add_feed_task_subtree_unique_triggers(&conn).unwrap();

    crate::db::migrations::migrate_v90_drop_scheduling_fields(&conn)
        .expect("v90 must survive the triggers SQLite re-resolves during DROP COLUMN");

    let triggers: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' \
             AND name IN ('enforce_feed_task_subtree_unique_insert', \
                          'enforce_feed_task_subtree_unique_update')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(triggers, 2, "v90 must not cost the feed-subtree triggers");
}

/// Safe on a database that never ran v88's ALTERs (nothing to drop) and safe
/// to run twice.
#[test]
fn migration_v90_is_idempotent_and_survives_a_missing_v88() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog'
         );",
    )
    .unwrap();

    crate::db::migrations::migrate_v90_drop_scheduling_fields(&conn).unwrap();
    crate::db::migrations::migrate_v90_drop_scheduling_fields(&conn).unwrap();
}

/// v92 arms the phoenix recurrence. Every pre-existing row must land on
/// `false` — no task written before this migration was recurring — and the
/// column has to be `NOT NULL` so `row_to_task` can read it as a plain `bool`.
#[test]
fn migration_v92_adds_phoenix_defaulting_existing_rows_to_false() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO tasks(title) VALUES('pre-existing');",
    )
    .unwrap();

    crate::db::migrations::migrate_v92_add_phoenix(&conn).unwrap();

    let phoenix: bool = conn
        .query_row(
            "SELECT phoenix FROM tasks WHERE title = 'pre-existing'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!phoenix, "a row that predates the column is not a phoenix");

    let not_null: i64 = conn
        .query_row(
            "SELECT \"notnull\" FROM pragma_table_info('tasks') WHERE name = 'phoenix'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(not_null, 1, "phoenix must be NOT NULL");
}

/// The v72 feed-subtree triggers must survive v92. `ADD COLUMN` should not make
/// SQLite re-resolve them — that is the hazard v90's `DROP COLUMN` carries, not
/// this one — but the uniqueness invariant they enforce is worth a tripwire
/// either way.
#[test]
fn migration_v92_leaves_the_feed_subtree_triggers_intact() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             epic_id INTEGER,
             external_id TEXT,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         CREATE TABLE epics (id INTEGER PRIMARY KEY, parent_epic_id INTEGER);",
    )
    .unwrap();
    crate::db::migrations::migrate_v72_add_feed_task_subtree_unique_triggers(&conn).unwrap();

    let before = trigger_names(&conn);
    crate::db::migrations::migrate_v92_add_phoenix(&conn).unwrap();
    assert_eq!(before, trigger_names(&conn));
}

fn trigger_names(conn: &rusqlite::Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

// --- v91: clamp pre-floor feed cadences (core.allium: "Interval literals") ---

/// Rows written before the floor existed are brought into line once. Clamped UP
/// rather than nulled so the value stays explicit — a later change to
/// `default_feed_interval` must not silently retarget an epic whose cadence
/// somebody once chose.
#[test]
fn v91_clamps_sub_floor_epic_feed_intervals_up_to_the_floor() {
    use crate::models::MIN_FEED_INTERVAL_SECS as FLOOR;
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             feed_interval_secs INTEGER
         );
         INSERT INTO epics (title, feed_interval_secs) VALUES
           ('busy-loop', 0),
           ('negative', -5),
           ('just-under', 59),
           ('at-floor', 60),
           ('well-above', 300),
           ('unset', NULL);",
    )
    .unwrap();

    crate::db::migrations::migrate_v91_clamp_feed_intervals(&conn).unwrap();

    let mut stmt = conn
        .prepare("SELECT title, feed_interval_secs FROM epics ORDER BY title")
        .unwrap();
    let rows: Vec<(String, Option<i64>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let get = |name: &str| -> Option<i64> {
        rows.iter()
            .find(|(t, _)| t == name)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| panic!("row {name} missing"))
    };

    assert_eq!(get("busy-loop"), Some(FLOOR), "0 must clamp to the floor");
    assert_eq!(get("negative"), Some(FLOOR), "-5 must clamp to the floor");
    assert_eq!(get("just-under"), Some(FLOOR), "59 must clamp to the floor");
    assert_eq!(
        get("at-floor"),
        Some(FLOOR),
        "a value already at the floor must be left alone"
    );
    assert_eq!(
        get("well-above"),
        Some(300),
        "a conforming value must not be dragged down to the floor"
    );
    assert_eq!(
        get("unset"),
        None,
        "NULL means 'inherit the default' and must stay NULL, not become the floor"
    );
}

/// The two managed-feed cadences live in the settings table, not on `epics`,
/// and are copied onto managed epics by provisioning. The floor binds them too,
/// so the migration must clean them as well — `0` was blessed here as "poll
/// every tick" before the floor existed.
#[test]
fn v91_clamps_sub_floor_managed_feed_interval_settings() {
    use crate::models::MIN_FEED_INTERVAL_SECS as FLOOR;
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         INSERT INTO settings (key, value) VALUES
           ('reviews_feed_interval_secs', '0'),
           ('cve_feed_interval_secs', '900'),
           ('default_branch', 'main');",
    )
    .unwrap();

    crate::db::migrations::migrate_v91_clamp_feed_intervals(&conn).unwrap();

    let get = |key: &str| -> String {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key],
            |r| r.get(0),
        )
        .unwrap()
    };

    assert_eq!(
        get("reviews_feed_interval_secs"),
        FLOOR.to_string(),
        "a sub-floor reviews cadence must clamp to the floor"
    );
    assert_eq!(
        get("cve_feed_interval_secs"),
        "900",
        "a conforming cadence must be left alone"
    );
    assert_eq!(
        get("default_branch"),
        "main",
        "an unrelated setting must not be touched"
    );
}

/// Safe on a database missing the columns/tables entirely, and safe to run
/// twice — the second run finds nothing left to clamp.
#[test]
fn migration_v91_is_idempotent_and_survives_missing_tables() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::migrate_v91_clamp_feed_intervals(&conn).unwrap();

    conn.execute_batch(
        "CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             feed_interval_secs INTEGER
         );
         INSERT INTO epics (title, feed_interval_secs) VALUES ('busy', 0);",
    )
    .unwrap();

    crate::db::migrations::migrate_v91_clamp_feed_intervals(&conn).unwrap();
    crate::db::migrations::migrate_v91_clamp_feed_intervals(&conn).unwrap();

    let after: Option<i64> = conn
        .query_row("SELECT feed_interval_secs FROM epics", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, Some(crate::models::MIN_FEED_INTERVAL_SECS));
}

// ---------------------------------------------------------------------------
// v95 — drop the orphaned main-session directory setting
// ---------------------------------------------------------------------------

/// The main session was removed, so `main_session.dir` is a row nothing reads
/// or writes any more. Databases that ran the feature still carry it; the
/// migration clears it out and leaves every other setting alone.
#[test]
fn v95_deletes_the_orphaned_main_session_dir_setting() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         INSERT INTO settings (key, value) VALUES
           ('main_session.dir', '/home/user/code'),
           ('default_branch', 'main');",
    )
    .unwrap();

    crate::db::migrations::migrate_v95_drop_main_session_dir(&conn).unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = 'main_session.dir'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "the orphaned main-session row must be gone");

    let other: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'default_branch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(other, "main", "an unrelated setting must not be touched");
}

/// Safe on a database that never had a settings table, and safe to run twice.
#[test]
fn migration_v95_is_idempotent_and_survives_a_missing_settings_table() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::migrate_v95_drop_main_session_dir(&conn).unwrap();

    conn.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO settings (key, value) VALUES ('main_session.dir', '/home/user');",
    )
    .unwrap();

    // Twice, so the second run exercises the already-deleted path rather than
    // the empty-table one the call above already covered.
    crate::db::migrations::migrate_v95_drop_main_session_dir(&conn).unwrap();
    crate::db::migrations::migrate_v95_drop_main_session_dir(&conn).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

// --- v97: add tasks.host and backfill it for pre-existing worktree rows ---
// (task #4812 distributed-dispatch foundations; see core.allium:
// HostTracksWorktree)

#[test]
fn migration_v97_backfills_host_for_a_preexisting_worktree_row() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             worktree TEXT,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO tasks(title, worktree) VALUES ('provisioned', '/repo/.worktrees/1-x');
         INSERT INTO tasks(title, worktree) VALUES ('never-dispatched', NULL);",
    )
    .unwrap();

    crate::db::migrations::migrate_v97_add_task_host(&conn).unwrap();

    let minted_host_id: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'host_id'",
            [],
            |r| r.get(0),
        )
        .expect("the migration must mint a host id so it has something to backfill with");
    assert!(!minted_host_id.is_empty());

    let provisioned_host: Option<String> = conn
        .query_row(
            "SELECT host FROM tasks WHERE title = 'provisioned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        provisioned_host,
        Some(minted_host_id),
        "a row that already holds a worktree must be backfilled with this install's host id, \
         or HostTracksWorktree (core.allium) is violated the moment this migration runs"
    );

    let never_dispatched_host: Option<String> = conn
        .query_row(
            "SELECT host FROM tasks WHERE title = 'never-dispatched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        never_dispatched_host, None,
        "a worktree-less row stays unowned — nothing to backfill"
    );
}

#[test]
fn migration_v97_does_not_remint_the_host_id_if_one_already_exists() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             worktree TEXT,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO settings (key, value) VALUES ('host_id', 'already-minted');
         INSERT INTO tasks(title, worktree) VALUES ('provisioned', '/repo/.worktrees/1-x');",
    )
    .unwrap();

    crate::db::migrations::migrate_v97_add_task_host(&conn).unwrap();

    let host_id: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'host_id'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(host_id, "already-minted");
    let backfilled: Option<String> = conn
        .query_row(
            "SELECT host FROM tasks WHERE title = 'provisioned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(backfilled, Some("already-minted".to_string()));
}

/// No settings table at all: the column is still added, and the backfill is
/// skipped rather than failing the migration.
#[test]
fn migration_v97_adds_the_column_and_skips_the_backfill_with_no_settings_table() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             worktree TEXT,
             status TEXT NOT NULL DEFAULT 'backlog'
         );
         INSERT INTO tasks(title, worktree) VALUES ('provisioned', '/repo/.worktrees/1-x');",
    )
    .unwrap();

    crate::db::migrations::migrate_v97_add_task_host(&conn).unwrap();

    let host: Option<String> = conn
        .query_row(
            "SELECT host FROM tasks WHERE title = 'provisioned'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(host, None);
}

/// v99 moves the completion-recency rank out of `sort_order` and into
/// `completed_at`, and only that rank: a *positive* `sort_order` on a done row
/// is real manual or feed ordering — the very case the Done column could not
/// tell apart — so it is left alone. That row still gets a `completed_at`,
/// approximated from `updated_at` by the second pass, because a done card with
/// none sorts last for good and cannot be reordered by hand.
#[test]
fn migration_v99_moves_the_completion_rank_into_completed_at() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL,
             sort_order INTEGER,
             updated_at TEXT NOT NULL
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             status TEXT NOT NULL,
             sort_order INTEGER,
             updated_at TEXT NOT NULL
         );
         -- -1700000000123 ms == 2023-11-14 22:13:20.123 UTC
         INSERT INTO tasks (title, status, sort_order, updated_at) VALUES
           ('done-ranked',      'done',    -1700000000123, '2026-01-15 12:00:00'),
           ('done-feed-order',  'done',    7,              '2026-01-15 12:00:00'),
           ('done-unranked',    'done',    NULL,           '2026-01-15 12:00:00'),
           ('open-ordered',     'backlog', 3,              '2026-01-15 12:00:00');
         INSERT INTO epics (title, status, sort_order, updated_at) VALUES
           ('epic-done-ranked', 'done',    -1700000000000, '2026-02-01 08:30:00'),
           ('epic-open',        'running', 5,              '2026-02-01 08:30:00');",
    )
    .unwrap();

    crate::db::migrations::migrate_v99_add_completed_at(&conn).unwrap();

    let row = |table: &str, title: &str| -> (Option<i64>, Option<String>) {
        conn.query_row(
            &format!("SELECT sort_order, completed_at FROM {table} WHERE title = ?1"),
            [title],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };

    assert_eq!(
        row("tasks", "done-ranked"),
        (None, Some("2023-11-14 22:13:20.123".to_string())),
        "the rank moves across at millisecond precision, and sort_order is cleared"
    );
    assert_eq!(
        row("tasks", "done-feed-order"),
        (Some(7), Some("2026-01-15 12:00:00".to_string())),
        "a positive sort_order on a done row is manual ordering, not a rank — it \
         survives, and the row is dated from updated_at rather than left undated"
    );
    assert_eq!(
        row("tasks", "done-unranked"),
        (None, Some("2026-01-15 12:00:00".to_string())),
        "a done row that never had a rank is dated from updated_at too"
    );
    assert_eq!(
        row("tasks", "open-ordered"),
        (Some(3), None),
        "a row outside done is untouched"
    );
    assert_eq!(
        row("epics", "epic-done-ranked"),
        (None, Some("2023-11-14 22:13:20.000".to_string()))
    );
    assert_eq!(row("epics", "epic-open"), (Some(5), None));
}

/// Re-running v99 is a no-op: the ALTER is guarded, and both UPDATEs' own
/// predicates no longer match a row they already handled.
#[test]
fn migration_v99_is_idempotent() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL,
             sort_order INTEGER, updated_at TEXT NOT NULL
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL,
             sort_order INTEGER, updated_at TEXT NOT NULL
         );
         INSERT INTO tasks (title, status, sort_order, updated_at)
           VALUES ('t', 'done', -1700000000123, '2026-01-15 12:00:00');",
    )
    .unwrap();

    crate::db::migrations::migrate_v99_add_completed_at(&conn).unwrap();
    let first: Option<String> = conn
        .query_row("SELECT completed_at FROM tasks", [], |r| r.get(0))
        .unwrap();
    crate::db::migrations::migrate_v99_add_completed_at(&conn).unwrap();
    let second: Option<String> = conn
        .query_row("SELECT completed_at FROM tasks", [], |r| r.get(0))
        .unwrap();

    assert_eq!(first, second);
    assert!(first.is_some());
}

/// The value v99 writes must be readable by the same parser every other
/// timestamp column goes through, or the moved rows decode as errors. It must
/// also be byte-identical to what `format_datetime_millis` writes, so a
/// migrated row and a freshly-stamped one sort against each other correctly:
/// the column is TEXT, and the comparison is lexicographic.
#[test]
fn migration_v99_writes_the_same_timestamp_format_the_code_writes() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (
             id INTEGER PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL,
             sort_order INTEGER, updated_at TEXT NOT NULL
         );
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY, title TEXT NOT NULL, status TEXT NOT NULL,
             sort_order INTEGER, updated_at TEXT NOT NULL
         );
         INSERT INTO tasks (title, status, sort_order, updated_at)
           VALUES ('t', 'done', -1700000000123, '2026-01-15 12:00:00');",
    )
    .unwrap();

    crate::db::migrations::migrate_v99_add_completed_at(&conn).unwrap();
    let written: String = conn
        .query_row("SELECT completed_at FROM tasks", [], |r| r.get(0))
        .unwrap();

    let expected = chrono::DateTime::from_timestamp_millis(1_700_000_000_123).unwrap();
    assert_eq!(
        written,
        crate::db::queries::format_datetime_millis(expected),
        "the migration and the code must agree on the storage format"
    );
    assert_eq!(
        crate::db::queries::parse_datetime(&written).unwrap(),
        expected,
        "and the row decoder must read it back unchanged"
    );
}
