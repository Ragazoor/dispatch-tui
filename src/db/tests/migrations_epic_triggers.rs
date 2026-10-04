use super::*;

// ---------------------------------------------------------------------------
// v80 — drop the orphaned agent_status column
// ---------------------------------------------------------------------------

#[test]
fn migrate_v80_drops_agent_status_and_keeps_sibling_columns() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // Pre-v80 shape of the four tables v27/v28 added agent_status to.
    conn.execute_batch(
        "CREATE TABLE review_prs (repo TEXT, number INTEGER, tmux_window TEXT,
                                  worktree TEXT, agent_status TEXT);
         CREATE TABLE bot_prs (repo TEXT, number INTEGER, agent_status TEXT);
         CREATE TABLE security_alerts (repo TEXT, number INTEGER, agent_status TEXT);
         CREATE TABLE my_prs (repo TEXT, number INTEGER, agent_status TEXT);
         INSERT INTO review_prs (repo, number, tmux_window, worktree, agent_status)
             VALUES ('kognic/x', 7, 'win', '/wt', 'reviewing');
         PRAGMA user_version = 79;",
    )
    .unwrap();

    crate::db::migrations::migrate_v80_drop_agent_status(&conn).unwrap();

    for table in ["review_prs", "bot_prs", "security_alerts", "my_prs"] {
        let cols: Vec<String> = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            !cols.contains(&"agent_status".to_string()),
            "agent_status must be gone from {table}, got {cols:?}"
        );
    }

    // Sibling columns and their data survive the drop.
    let (window, worktree): (String, String) = conn
        .query_row("SELECT tmux_window, worktree FROM review_prs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((window.as_str(), worktree.as_str()), ("win", "/wt"));
}

#[test]
fn migrate_v80_is_idempotent_and_tolerates_missing_tables() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // No agent_status column, and three of the four tables absent entirely.
    conn.execute_batch("CREATE TABLE my_prs (repo TEXT, number INTEGER);")
        .unwrap();
    crate::db::migrations::migrate_v80_drop_agent_status(&conn).unwrap();
    crate::db::migrations::migrate_v80_drop_agent_status(&conn).unwrap();
}

#[tokio::test]
async fn fresh_db_has_no_agent_status_column_anywhere() {
    let db = in_memory_db().await;
    let hits: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master m
                 JOIN pragma_table_info(m.name) c
                 WHERE m.type = 'table' AND c.name = 'agent_status'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(hits, 0, "no table may carry agent_status after v80");
}

#[tokio::test]
async fn migrate_v56_drops_task_usage_table() {
    let db = in_memory_db().await;

    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='task_usage'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count, 0, "task_usage should be dropped by v56");
}

// ---------------------------------------------------------------------------
// v57 — epic project consistency triggers + data fix
// ---------------------------------------------------------------------------

#[test]
fn migrate_v57_fixes_sub_epic_project_id_violations() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    // Minimal pre-v57 schema: two projects, parent epic in project 2, sub-epic mistakenly in project 1.
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0, is_default INTEGER NOT NULL DEFAULT 0);
         INSERT INTO projects VALUES (1, 'Default', 0, 1);
         INSERT INTO projects VALUES (2, 'Work', 1, 0);
         CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             plan_path TEXT,
             sort_order INTEGER,
             auto_dispatch BOOLEAN NOT NULL DEFAULT 1,
             parent_epic_id INTEGER,
             feed_command TEXT,
             feed_interval_secs INTEGER,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE tasks (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             sub_status TEXT NOT NULL DEFAULT 'none',
             base_branch TEXT NOT NULL DEFAULT 'main',
             epic_id INTEGER,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         -- parent epic in project 2
         INSERT INTO epics (id, title, project_id) VALUES (1, 'Parent', 2);
         -- sub-epic in wrong project (1 instead of 2)
         INSERT INTO epics (id, title, project_id, parent_epic_id) VALUES (2, 'Sub', 1, 1);
         -- grandchild also wrong
         INSERT INTO epics (id, title, project_id, parent_epic_id) VALUES (3, 'Grandchild', 1, 2);
         -- task under sub-epic in wrong project
         INSERT INTO tasks (id, title, epic_id, project_id) VALUES (1, 'Task', 2, 1);
         PRAGMA user_version = 56;",
    )
    .unwrap();

    crate::db::migrations::migrate_v57_enforce_epic_project_consistency(&conn).unwrap();

    let sub_pid: i64 = conn
        .query_row("SELECT project_id FROM epics WHERE id = 2", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        sub_pid, 2,
        "sub-epic project_id must be fixed to match parent"
    );

    let grandchild_pid: i64 = conn
        .query_row("SELECT project_id FROM epics WHERE id = 3", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        grandchild_pid, 2,
        "grandchild project_id must be fixed (multi-level)"
    );

    let task_pid: i64 = conn
        .query_row("SELECT project_id FROM tasks WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(task_pid, 2, "task project_id must be fixed to match epic");
}

// ---------------------------------------------------------------------------
// v58 — reset intermediate epic statuses to backlog
// ---------------------------------------------------------------------------

#[test]
fn migrate_v58_resets_running_and_review_epics_to_backlog() {
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE epics (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL DEFAULT '',
             description TEXT NOT NULL DEFAULT '',
             repo_path TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL DEFAULT 'backlog',
             plan_path TEXT,
             sort_order INTEGER,
             auto_dispatch BOOLEAN NOT NULL DEFAULT 1,
             parent_epic_id INTEGER,
             feed_command TEXT,
             feed_interval_secs INTEGER,
             project_id INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at TEXT NOT NULL DEFAULT (datetime('now')),
             group_by_repo BOOLEAN NOT NULL DEFAULT 0
         );
         INSERT INTO epics (id, title, status) VALUES (1, 'A', 'running');
         INSERT INTO epics (id, title, status) VALUES (2, 'B', 'review');
         INSERT INTO epics (id, title, status) VALUES (3, 'C', 'backlog');
         INSERT INTO epics (id, title, status) VALUES (4, 'D', 'done');",
    )
    .unwrap();

    crate::db::migrations::migrate_v58_reset_intermediate_epic_statuses(&conn).unwrap();

    let status = |id: i64| -> String {
        conn.query_row("SELECT status FROM epics WHERE id = ?", [id], |row| {
            row.get(0)
        })
        .unwrap()
    };

    assert_eq!(status(1), "backlog"); // running → backlog
    assert_eq!(status(2), "backlog"); // review → backlog
    assert_eq!(status(3), "backlog"); // backlog unchanged
    assert_eq!(status(4), "done"); // done unchanged
}

#[tokio::test]
async fn migration_v60_drops_projects_table_and_columns() {
    let db = in_memory_db().await;
    let (projects_gone, tasks_no_project_id, epics_no_project_id): (i64, i64, i64) = db
        .db_call(|conn| {
            let projects_gone: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects'",
                    [],
                    |r| r.get(0),
                )
                .map_err(anyhow::Error::from)?;
            let tasks_no_project_id: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name='project_id'",
                    [],
                    |r| r.get(0),
                )
                .map_err(anyhow::Error::from)?;
            let epics_no_project_id: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('epics') WHERE name='project_id'",
                    [],
                    |r| r.get(0),
                )
                .map_err(anyhow::Error::from)?;
            Ok((projects_gone, tasks_no_project_id, epics_no_project_id))
        })
        .await
        .unwrap();
    assert_eq!(projects_gone, 0, "projects table should be dropped by v60");
    assert_eq!(
        tasks_no_project_id, 0,
        "tasks.project_id should be dropped by v60"
    );
    assert_eq!(
        epics_no_project_id, 0,
        "epics.project_id should be dropped by v60"
    );
}

#[test]
fn migration_v60_migrates_project_namespaced_repo_filter_settings() {
    use crate::db::migrations::migrate_v60_drop_projects;
    use rusqlite::{Connection as RawConn, OptionalExtension};
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', repo_path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'backlog', sub_status TEXT NOT NULL DEFAULT 'none', base_branch TEXT NOT NULL DEFAULT 'main', project_id INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE epics (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', repo_path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'backlog', project_id INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE learnings (id INTEGER PRIMARY KEY, scope TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending');
         CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
         INSERT INTO settings VALUES ('repo_filter:1', 'myrepo');
         INSERT INTO settings VALUES ('repo_filter_mode:1', 'include');
         INSERT INTO settings VALUES ('other_key', 'untouched');",
    )
    .unwrap();

    migrate_v60_drop_projects(&conn).unwrap();

    let filter: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(
        filter.as_deref(),
        Some("myrepo"),
        "repo_filter:1 should be promoted to repo_filter"
    );

    let mode: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter_mode'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(
        mode.as_deref(),
        Some("include"),
        "repo_filter_mode:1 should be promoted to repo_filter_mode"
    );

    let old_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'repo_filter:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        old_count, 0,
        "namespaced repo_filter keys should be deleted"
    );

    let other: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'other_key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        other, "untouched",
        "unrelated settings should not be touched"
    );
}

#[test]
fn migration_v60_repo_filter_does_not_overwrite_existing_unnamespaced_key() {
    use crate::db::migrations::migrate_v60_drop_projects;
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', repo_path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'backlog', sub_status TEXT NOT NULL DEFAULT 'none', base_branch TEXT NOT NULL DEFAULT 'main', project_id INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE epics (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', repo_path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'backlog', project_id INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE learnings (id INTEGER PRIMARY KEY, scope TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending');
         CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
         INSERT INTO settings VALUES ('repo_filter', 'already-set');
         INSERT INTO settings VALUES ('repo_filter:1', 'old-value');",
    )
    .unwrap();

    migrate_v60_drop_projects(&conn).unwrap();

    let filter: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        filter, "already-set",
        "existing unnamespaced key must not be overwritten"
    );
}

// ---------------------------------------------------------------------------
// Table-existence guard: skip gracefully when target table is absent
// ---------------------------------------------------------------------------

#[test]
fn migrate_v43_skips_when_learnings_absent() {
    use crate::db::migrations::migrate_v43_proposed_to_approved;
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    migrate_v43_proposed_to_approved(&conn).unwrap();
}

#[test]
fn migrate_v46_skips_when_learnings_absent() {
    use crate::db::migrations::migrate_v46_learning_source_task_set_null;
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    migrate_v46_learning_source_task_set_null(&conn).unwrap();
}

#[test]
fn migrate_v47_skips_when_task_usage_absent() {
    use crate::db::migrations::migrate_v47_task_usage_restore_cascade;
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    migrate_v47_task_usage_restore_cascade(&conn).unwrap();
}

#[test]
fn migrate_v55_skips_when_learnings_absent() {
    use crate::db::migrations::migrate_v55_add_learning_embedding;
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    migrate_v55_add_learning_embedding(&conn).unwrap();
}

#[test]
fn migration_v60_deletes_project_scoped_learnings() {
    use crate::db::migrations::migrate_v60_drop_projects;
    use rusqlite::Connection as RawConn;
    let conn = RawConn::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', repo_path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'backlog', sub_status TEXT NOT NULL DEFAULT 'none', base_branch TEXT NOT NULL DEFAULT 'main', project_id INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE epics (id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', repo_path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'backlog', project_id INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE learnings (id INTEGER PRIMARY KEY, scope TEXT NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending');
         CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0);
         INSERT INTO learnings (scope, content, status) VALUES ('project', 'old learning', 'approved');
         INSERT INTO learnings (scope, content, status) VALUES ('user', 'keep me', 'approved');
         INSERT INTO learnings (scope, content, status) VALUES ('repo', 'keep me too', 'pending');",
    )
    .unwrap();

    migrate_v60_drop_projects(&conn).unwrap();

    let project_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM learnings WHERE scope = 'project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        project_count, 0,
        "project-scoped learnings should be deleted by v60"
    );

    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM learnings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 2, "non-project learnings should be preserved");
}

#[tokio::test]
async fn migration_v63_adds_idx_tasks_status_and_epic_id() {
    let db = in_memory_db().await;
    let (status_idx, epic_id_idx): (i64, i64) = db
        .db_call(|conn| {
            let status_idx: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_tasks_status'",
                    [],
                    |r| r.get(0),
                )
                .map_err(anyhow::Error::from)?;
            let epic_id_idx: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_tasks_epic_id'",
                    [],
                    |r| r.get(0),
                )
                .map_err(anyhow::Error::from)?;
            Ok((status_idx, epic_id_idx))
        })
        .await
        .unwrap();
    assert_eq!(
        status_idx, 1,
        "idx_tasks_status must exist after migration v63"
    );
    assert_eq!(
        epic_id_idx, 1,
        "idx_tasks_epic_id must exist after migration v63"
    );
}

#[tokio::test]
async fn v69_adds_origin_column_and_repo_group_index() {
    let db = unattached_db().await;

    // Verify the origin column exists.
    let has_origin: bool = db
        .db_call(|conn| {
            conn.prepare("SELECT 1 FROM pragma_table_info('epics') WHERE name = 'origin'")
                .and_then(|mut stmt| {
                    stmt.query_map([], |_| Ok(()))
                        .map(|mut rows| rows.next().is_some())
                })
                .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert!(has_origin, "origin column should exist on epics table");

    // Two RepoGroup sub-epics with DIFFERENT titles under one parent: allowed.
    let root = db.create_epic("root", "", None).await.unwrap();
    let result = set_origin_raw(&db, root.id.0, "manual").await;
    assert!(result.is_ok(), "setting manual origin should succeed");

    let a = db.create_epic("repo-a", "", Some(root.id)).await.unwrap();
    let result = set_origin_raw(&db, a.id.0, "repo-group").await;
    assert!(result.is_ok(), "setting repo-group origin should succeed");

    let b = db.create_epic("repo-b", "", Some(root.id)).await.unwrap();
    let result = set_origin_raw(&db, b.id.0, "repo-group").await;
    assert!(
        result.is_ok(),
        "different repo-group titles under same parent should be allowed"
    );

    // A SECOND RepoGroup sub-epic with the SAME title under the same parent: rejected.
    let c = db.create_epic("repo-a", "", Some(root.id)).await.unwrap();
    let dup = set_origin_raw(&db, c.id.0, "repo-group").await;
    assert!(
        dup.is_err(),
        "duplicate RepoGroup (parent,title) must violate the unique index"
    );

    // A Manual sub-epic with the same title coexists (index only constrains repo-group).
    let d = db.create_epic("repo-a", "", Some(root.id)).await.unwrap();
    let result = set_origin_raw(&db, d.id.0, "manual").await;
    assert!(
        result.is_ok(),
        "manual origin with duplicate title should be allowed (index is partial)"
    );
}

/// Set `epics.origin` directly via SQL so the migration's unique index can be exercised.
async fn set_origin_raw(db: &Database, epic_id: i64, origin: &str) -> anyhow::Result<()> {
    let origin = origin.to_string();
    db.db_call(move |conn| {
        conn.execute(
            "UPDATE epics SET origin = ?1 WHERE id = ?2",
            rusqlite::params![origin, epic_id],
        )
        .map(|_| ())
        .map_err(anyhow::Error::from)
    })
    .await
}

/// v71 removes repo-sub-epic task copies when the same external_id
/// already exists directly on the parent role sub-epic.
#[tokio::test]
async fn v71_dedup_removes_repo_sub_epic_shadow_tasks() {
    // in_memory_db() runs ALL migrations including v72, which installs the
    // BEFORE INSERT trigger.  Two tasks with the same external_id can't be
    // inserted directly, so we insert both with external_id=NULL and then
    // UPDATE to set it — the BEFORE UPDATE OF epic_id trigger doesn't fire
    // for external_id-only updates, so this path is safe.
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (3, 'myrepo', '', 'backlog', 'none', 'repo-group', 2);
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id)
             VALUES (10, 'PR #1', '', '/r', 'backlog', 'main', 2);
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id)
             VALUES (11, 'PR #1 dup', '', '/r', 'backlog', 'main', 3);
             UPDATE tasks SET external_id = 'pr-1' WHERE id IN (10, 11);",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();

    db.db_call(|conn| crate::db::migrations::migrate_v71_dedup_role_subtree_tasks(conn))
        .await
        .unwrap();

    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE external_id = 'pr-1'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count, 1, "duplicate repo-sub-epic copy must be deleted");

    let surviving_epic: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT epic_id FROM tasks WHERE external_id = 'pr-1'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(surviving_epic, 2, "surviving task must be on role sub-epic");
}

/// v71 leaves tasks untouched when there is no matching copy on the parent.
#[tokio::test]
async fn v71_dedup_preserves_solo_repo_sub_epic_tasks() {
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (3, 'myrepo', '', 'backlog', 'none', 'repo-group', 2);
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id, external_id)
             VALUES (10, 'PR #2', '', '/r', 'backlog', 'main', 3, 'pr-2');",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();

    db.db_call(|conn| crate::db::migrations::migrate_v71_dedup_role_subtree_tasks(conn))
        .await
        .unwrap();

    let count: i64 = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE external_id = 'pr-2'",
                [],
                |r| r.get(0),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();
    assert_eq!(count, 1, "solo repo-sub-epic task must survive");
}

/// Inserting a task with a duplicate external_id into the same role sub-epic
/// must be rejected. Since v76, a plain second INSERT (no ON CONFLICT
/// clause) at the same (epic_id, external_id) is rejected by the
/// `tasks_epic_external_id` unique index (v38) rather than by the subtree
/// trigger itself — the trigger now excludes same-epic matches so it no
/// longer false-positives on ON CONFLICT DO UPDATE upserts — but the net
/// effect asserted here (insert is still rejected) is unchanged.
#[tokio::test]
async fn v72_trigger_blocks_duplicate_insert_into_role_sub_epic() {
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id, external_id)
             VALUES (10, 'PR #1', '', '/r', 'backlog', 'main', 2, 'pr-1');",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();

    let result = db
        .db_call(|conn| {
            conn.execute(
                "INSERT INTO tasks (title, description, repo_path, status, base_branch, epic_id, external_id)
                 VALUES ('PR #1 dup', '', '/r', 'backlog', 'main', 2, 'pr-1')",
                [],
            )
            .map_err(anyhow::Error::from)
        })
        .await;

    assert!(
        result.is_err(),
        "trigger must reject duplicate external_id in same role sub-epic"
    );
}

/// Inserting a task with a duplicate external_id into a repo-group sub-epic
/// whose parent role sub-epic already has that external_id must be rejected.
#[tokio::test]
async fn v72_trigger_blocks_duplicate_insert_into_repo_group_sub_epic() {
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (3, 'myrepo', '', 'backlog', 'none', 'repo-group', 2);
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id, external_id)
             VALUES (10, 'PR #1', '', '/r', 'backlog', 'main', 2, 'pr-1');",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();

    let result = db
        .db_call(|conn| {
            conn.execute(
                "INSERT INTO tasks (title, description, repo_path, status, base_branch, epic_id, external_id)
                 VALUES ('PR #1 dup', '', '/r', 'backlog', 'main', 3, 'pr-1')",
                [],
            )
            .map_err(anyhow::Error::from)
        })
        .await;

    assert!(
        result.is_err(),
        "trigger must reject insert into repo-group sub-epic when role sub-epic already has same external_id"
    );
}

/// Moving a task into a role sub-epic subtree that already holds the same
/// external_id must be rejected.
#[tokio::test]
async fn v72_trigger_blocks_move_that_creates_duplicate() {
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (3, 'Team Reviews', '', 'backlog', 'team-reviews', 'manual', 1);
             -- pr-1 exists in My Reviews
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id, external_id)
             VALUES (10, 'PR #1', '', '/r', 'backlog', 'main', 2, 'pr-1');
             -- pr-1 also in Team Reviews (this is the pre-existing duplicate we are moving)
             INSERT INTO tasks (id, title, description, repo_path, status, base_branch, epic_id, external_id)
             VALUES (11, 'PR #1 team', '', '/r', 'backlog', 'main', 3, 'pr-1');",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();

    // Try to move task 11 from Team Reviews to My Reviews — should fail
    // because My Reviews already has pr-1.
    let result = db
        .db_call(|conn| {
            conn.execute("UPDATE tasks SET epic_id = 2 WHERE id = 11", [])
                .map_err(anyhow::Error::from)
        })
        .await;

    assert!(
        result.is_err(),
        "trigger must reject move that would create duplicate external_id in target subtree"
    );
}

/// Tasks with NULL external_id are never subject to the trigger.
#[tokio::test]
async fn v72_trigger_allows_manual_tasks_with_null_external_id() {
    let db = in_memory_db().await;
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();

    // Two manual tasks with NULL external_id should coexist fine.
    let result = db
        .db_call(|conn| {
            conn.execute_batch(
                "INSERT INTO tasks (title, description, repo_path, status, base_branch, epic_id)
                 VALUES ('Manual 1', '', '/r', 'backlog', 'main', 2);
                 INSERT INTO tasks (title, description, repo_path, status, base_branch, epic_id)
                 VALUES ('Manual 2', '', '/r', 'backlog', 'main', 2);",
            )
            .map_err(anyhow::Error::from)
        })
        .await;

    assert!(
        result.is_ok(),
        "manual tasks with NULL external_id must not trigger the constraint"
    );
}
