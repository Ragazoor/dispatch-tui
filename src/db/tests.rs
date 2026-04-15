use super::*;

fn in_memory_db() -> Database {
    Database::open_in_memory().unwrap()
}

/// Helper: create_task + get_task in one step (replaces removed trait method).
fn create_task_returning(
    db: &Database,
    title: &str,
    description: &str,
    repo_path: &str,
    plan: Option<&str>,
    status: TaskStatus,
) -> anyhow::Result<Task> {
    let id = db.create_task(title, description, repo_path, plan, status, "main")?;
    db.get_task(id)?
        .ok_or_else(|| anyhow::anyhow!("Task {id} vanished after insert"))
}

#[test]
fn create_and_get() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "My Task",
            "A description",
            "/repo/path",
            None,
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();
    let task = db.get_task(id).unwrap().expect("task should exist");
    assert_eq!(task.id, id);
    assert_eq!(task.title, "My Task");
    assert_eq!(task.description, "A description");
    assert_eq!(task.repo_path, "/repo/path");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[test]
fn list_all() {
    let db = in_memory_db();
    db.create_task("Task A", "desc", "/a", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.create_task("Task B", "desc", "/b", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.create_task("Task C", "desc", "/c", None, TaskStatus::Backlog, "main")
        .unwrap();
    let tasks = db.list_all().unwrap();
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].title, "Task A");
    assert_eq!(tasks[1].title, "Task B");
    assert_eq!(tasks[2].title, "Task C");
}

#[test]
fn list_by_status() {
    let db = in_memory_db();
    let id1 = db
        .create_task("Task A", "desc", "/a", None, TaskStatus::Backlog, "main")
        .unwrap();
    let id2 = db
        .create_task("Task B", "desc", "/b", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.create_task("Task C", "desc", "/c", None, TaskStatus::Backlog, "main")
        .unwrap();

    db.patch_task(id1, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();
    db.patch_task(id2, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    let running = db.list_by_status(TaskStatus::Running).unwrap();
    assert_eq!(running.len(), 2);

    let backlog = db.list_by_status(TaskStatus::Backlog).unwrap();
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0].title, "Task C");
}

#[test]
fn get_nonexistent() {
    let db = in_memory_db();
    let result = db.get_task(TaskId(9999)).unwrap();
    assert!(result.is_none());
}

#[test]
fn create_task_with_plan() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "Planned Task",
            "desc",
            "/repo",
            Some("docs/plan.md"),
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.plan_path.as_deref(), Some("docs/plan.md"));
}

#[test]
fn create_task_without_plan() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "Simple Task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.plan_path.is_none());
}

#[test]
fn find_task_by_plan_returns_match() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "Planned",
            "desc",
            "/repo",
            Some("/plans/my-plan.md"),
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();

    let found = db.find_task_by_plan("/plans/my-plan.md").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, id);
}

#[test]
fn find_task_by_plan_returns_none_when_no_match() {
    let db = in_memory_db();
    db.create_task(
        "Other",
        "desc",
        "/repo",
        Some("/plans/other.md"),
        TaskStatus::Backlog,
        "main",
    )
    .unwrap();

    let found = db.find_task_by_plan("/plans/nonexistent.md").unwrap();
    assert!(found.is_none());
}

#[test]
fn find_task_by_plan_ignores_tasks_without_plan() {
    let db = in_memory_db();
    db.create_task(
        "No Plan",
        "desc",
        "/repo",
        None,
        TaskStatus::Backlog,
        "main",
    )
    .unwrap();

    let found = db.find_task_by_plan("/plans/any.md").unwrap();
    assert!(found.is_none());
}

#[test]
fn get_setting_bool_returns_none_when_absent() {
    let db = Database::open_in_memory().unwrap();
    assert_eq!(db.get_setting_bool("notifications_enabled").unwrap(), None);
}

#[test]
fn set_and_get_setting_bool_roundtrips() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_bool("notifications_enabled", true).unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").unwrap(),
        Some(true)
    );

    db.set_setting_bool("notifications_enabled", false).unwrap();
    assert_eq!(
        db.get_setting_bool("notifications_enabled").unwrap(),
        Some(false)
    );
}

#[test]
fn get_setting_string_returns_none_when_absent() {
    let db = Database::open_in_memory().unwrap();
    assert_eq!(db.get_setting_string("repo_filter").unwrap(), None);
}

#[test]
fn set_and_get_setting_string() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_string("repo_filter", "/repo1\n/repo2")
        .unwrap();
    assert_eq!(
        db.get_setting_string("repo_filter").unwrap(),
        Some("/repo1\n/repo2".to_string())
    );
}

#[test]
fn set_setting_string_upserts() {
    let db = Database::open_in_memory().unwrap();
    db.set_setting_string("repo_filter", "old").unwrap();
    db.set_setting_string("repo_filter", "new").unwrap();
    assert_eq!(
        db.get_setting_string("repo_filter").unwrap(),
        Some("new".to_string())
    );
}

#[test]
fn fresh_db_has_latest_schema_version() {
    let db = in_memory_db();
    let conn = db.conn.lock().unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 33);
}

#[test]
fn legacy_db_migrates_to_latest_version() {
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
    Database::init_schema(&conn).unwrap();

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
    assert_eq!(version, 33);
}

#[test]
fn migration_25_renames_plan_to_plan_path() {
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
    Database::init_schema(&conn).unwrap();

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
    assert_eq!(version, 33);
}

#[test]
fn migrate_v26_adds_agent_columns() {
    let db = in_memory_db();
    let conn = db.conn.lock().unwrap();

    // Verify columns exist by inserting data with them
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

    let (tw, wt): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT tmux_window, worktree FROM review_prs WHERE repo = 'acme/app' AND number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(tw.as_deref(), Some("dispatch:review-1"));
    assert_eq!(wt.as_deref(), Some("/tmp/wt"));

    // Verify security_alerts too
    conn.execute(
        "INSERT INTO security_alerts (repo, number, kind, severity, title,
         url, created_at, state, description, tmux_window, worktree)
         VALUES ('acme/app', 1, 'dependabot', 'high', 'Alert',
         'https://example.com', '2024-01-01T00:00:00Z', 'open', 'desc',
         'dispatch:fix-1', '/tmp/wt4')",
        [],
    )
    .unwrap();

    let (tw, wt): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT tmux_window, worktree FROM security_alerts WHERE repo = 'acme/app'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(tw.as_deref(), Some("dispatch:fix-1"));
    assert_eq!(wt.as_deref(), Some("/tmp/wt4"));
}

#[test]
fn migration_6_converts_ready_to_backlog() {
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

    Database::init_schema(&conn).unwrap();

    let status: String = conn
        .query_row("SELECT status FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "backlog");

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 33);
}

#[test]
fn save_and_list_repo_paths() {
    let db = in_memory_db();
    assert!(db.list_repo_paths().unwrap().is_empty());
    db.save_repo_path("/home/user/project").unwrap();
    db.save_repo_path("/home/user/other").unwrap();
    let paths = db.list_repo_paths().unwrap();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"/home/user/project".to_string()));
    assert!(paths.contains(&"/home/user/other".to_string()));
}

#[test]
fn save_repo_path_deduplicates() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/project").unwrap();
    db.save_repo_path("/home/user/project").unwrap();
    assert_eq!(db.list_repo_paths().unwrap().len(), 1);
}

#[test]
fn list_repo_paths_empty_by_default() {
    let db = in_memory_db();
    assert!(db.list_repo_paths().unwrap().is_empty());
}

#[test]
fn list_repo_paths_returns_all_beyond_nine() {
    let db = in_memory_db();
    for i in 0..15 {
        db.save_repo_path(&format!("/home/user/project{i}")).unwrap();
    }
    let paths = db.list_repo_paths().unwrap();
    assert_eq!(paths.len(), 15, "all 15 paths should be returned, not just 9");
}

#[test]
fn create_task_returning_returns_full_task() {
    let db = in_memory_db();
    let task =
        create_task_returning(&db, "Title", "Desc", "/repo", None, TaskStatus::Backlog).unwrap();
    assert_eq!(task.title, "Title");
    assert_eq!(task.description, "Desc");
    assert_eq!(task.repo_path, "/repo");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
    assert!(task.plan_path.is_none());
}

#[test]
fn create_task_returning_with_plan() {
    let db = in_memory_db();
    let task =
        create_task_returning(&db, "T", "D", "/r", Some("plan.md"), TaskStatus::Backlog).unwrap();
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[test]
fn patch_task_applies_all_fields() {
    let db = in_memory_db();
    let id = db
        .create_task("title", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let patch = TaskPatch::new()
        .status(TaskStatus::Running)
        .plan_path(Some("plan.md"))
        .title("new title");
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.title, "new title");
    assert_eq!(task.description, "desc"); // unchanged
}

#[test]
fn patch_task_none_fields_unchanged() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "title",
            "desc",
            "/repo",
            Some("plan.md"),
            TaskStatus::Running,
            "main",
        )
        .unwrap();
    let patch = TaskPatch::new();
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.title, "title");
    assert_eq!(task.plan_path.as_deref(), Some("plan.md"));
    assert_eq!(task.status, TaskStatus::Running);
}

#[test]
fn patch_task_sets_tag() {
    let db = in_memory_db();
    let id = db
        .create_task("title", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(Some(TaskTag::Bug)))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.tag, Some(TaskTag::Bug));
}

#[test]
fn patch_task_clears_tag() {
    let db = in_memory_db();
    let id = db
        .create_task("title", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(Some(TaskTag::Feature)))
        .unwrap();
    db.patch_task(id, &TaskPatch::new().tag(None)).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.tag.is_none());
}

#[test]
fn has_other_tasks_with_worktree_returns_false_when_no_others() {
    let db = in_memory_db();
    let id = db
        .create_task("Task A", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(
        id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    assert!(!db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id)
        .unwrap());
}

#[test]
fn has_other_tasks_with_worktree_returns_true_when_shared() {
    let db = in_memory_db();
    let id1 = db
        .create_task("Task A", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let id2 = db
        .create_task("Task B", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(
        id1,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();
    db.patch_task(
        id2,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    assert!(db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id1)
        .unwrap());
    assert!(db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id2)
        .unwrap());
}

#[test]
fn has_other_tasks_with_worktree_ignores_done_tasks() {
    let db = in_memory_db();
    let id1 = db
        .create_task("Task A", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let id2 = db
        .create_task("Task B", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(
        id1,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();
    db.patch_task(
        id2,
        &TaskPatch::new()
            .status(TaskStatus::Done)
            .worktree(Some("/repo/.worktrees/1-task-a"))
            .tmux_window(Some("task-1")),
    )
    .unwrap();

    assert!(!db
        .has_other_tasks_with_worktree("/repo/.worktrees/1-task-a", id1)
        .unwrap());
}

#[test]
fn patch_task_clears_plan() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "title",
            "desc",
            "/repo",
            Some("plan.md"),
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();
    let patch = TaskPatch::new().plan_path(None);
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.plan_path.is_none());
}

#[test]
fn patch_task_sets_dispatch_fields() {
    let db = in_memory_db();
    let id = db
        .create_task("title", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let patch = TaskPatch::new()
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some("session:1-my-task"));
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-my-task"));
    assert_eq!(task.tmux_window.as_deref(), Some("session:1-my-task"));
}

#[test]
fn patch_task_clears_dispatch_fields() {
    let db = in_memory_db();
    let id = db
        .create_task("title", "desc", "/repo", None, TaskStatus::Running, "main")
        .unwrap();
    // Set dispatch fields first
    let patch = TaskPatch::new()
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some("session:1-my-task"));
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.worktree.is_some());
    assert!(task.tmux_window.is_some());

    // Clear them
    let patch = TaskPatch::new().worktree(None).tmux_window(None);
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[test]
fn patch_task_status_and_dispatch_together() {
    let db = in_memory_db();
    let id = db
        .create_task("title", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let patch = TaskPatch::new()
        .status(TaskStatus::Running)
        .worktree(Some("/repo/.worktrees/1-my-task"))
        .tmux_window(Some("session:1-my-task"));
    db.patch_task(id, &patch).unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/1-my-task"));
    assert_eq!(task.tmux_window.as_deref(), Some("session:1-my-task"));
}

#[test]
fn task_patch_status_does_not_set_sub_status() {
    // status() no longer auto-sets sub_status; patch_task handles the default
    let patch = TaskPatch::new().status(TaskStatus::Review);
    assert_eq!(patch.status, Some(TaskStatus::Review));
    assert_eq!(patch.sub_status, None);
}

#[test]
fn task_patch_status_and_sub_status_independent() {
    // Order of builder calls doesn't matter — both fields are set independently
    let patch_a = TaskPatch::new()
        .status(TaskStatus::Running)
        .sub_status(SubStatus::NeedsInput);
    let patch_b = TaskPatch::new()
        .sub_status(SubStatus::NeedsInput)
        .status(TaskStatus::Running);
    assert_eq!(patch_a.status, Some(TaskStatus::Running));
    assert_eq!(patch_a.sub_status, Some(SubStatus::NeedsInput));
    assert_eq!(patch_b.status, Some(TaskStatus::Running));
    assert_eq!(patch_b.sub_status, Some(SubStatus::NeedsInput));
}

#[test]
fn patch_task_status_change_resets_sub_status_in_db() {
    // End-to-end: after a status-only patch, sub_status in DB reflects the new default
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Running, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .unwrap();

    db.patch_task(id, &TaskPatch::default().status(TaskStatus::Review))
        .unwrap();

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
}

#[test]
fn update_status_if_matching() {
    let db = in_memory_db();
    let id = db
        .create_task("Task", "desc", "/repo", None, TaskStatus::Running, "main")
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(updated, "should update when current status matches");

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
}

#[test]
fn update_status_if_not_matching() {
    let db = in_memory_db();
    let id = db
        .create_task("Task", "desc", "/repo", None, TaskStatus::Done, "main")
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(
        !updated,
        "should not update when current status doesn't match"
    );

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done, "status should be unchanged");
}

#[test]
fn update_status_if_nonexistent() {
    let db = in_memory_db();
    let updated = db
        .update_status_if(TaskId(9999), TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(!updated, "should return false for nonexistent task");
}

// --- Epic CRUD ---

#[test]
fn create_and_get_epic() {
    let db = in_memory_db();
    let epic = db
        .create_epic("Auth Rewrite", "Rewrite auth", "/repo")
        .unwrap();
    assert_eq!(epic.title, "Auth Rewrite");
    assert_eq!(epic.description, "Rewrite auth");
    assert_eq!(epic.repo_path, "/repo");
    assert_eq!(epic.status, TaskStatus::Backlog);

    let fetched = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(fetched.id, epic.id);
    assert_eq!(fetched.title, "Auth Rewrite");
}

#[test]
fn list_epics() {
    let db = in_memory_db();
    db.create_epic("Epic A", "desc", "/a").unwrap();
    db.create_epic("Epic B", "desc", "/b").unwrap();
    let epics = db.list_epics().unwrap();
    assert_eq!(epics.len(), 2);
}

#[test]
fn get_epic_nonexistent() {
    let db = in_memory_db();
    assert!(db.get_epic(EpicId(999)).unwrap().is_none());
}

#[test]
fn delete_epic_cascades_subtasks() {
    let db = in_memory_db();
    let epic = db.create_epic("Epic", "desc", "/repo").unwrap();
    db.create_task("Sub 1", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let sub_id = db
        .create_task("Sub 2", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();

    // Link sub 2 to epic
    db.set_task_epic_id(sub_id, Some(epic.id)).unwrap();

    db.delete_epic(epic.id).unwrap();

    // Epic should be gone
    assert!(db.get_epic(epic.id).unwrap().is_none());
    // Sub 2 (linked to epic) should be deleted
    assert!(db.get_task(sub_id).unwrap().is_none());
    // Sub 1 (not linked) should still exist
    assert_eq!(db.list_all().unwrap().len(), 1);
}

#[test]
fn epic_has_status_field() {
    let db = Database::open_in_memory().unwrap();
    let epic = db.create_epic("Test", "Desc", "/repo").unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[test]
fn patch_epic_status() {
    let db = Database::open_in_memory().unwrap();
    let epic = db.create_epic("Test", "Desc", "/repo").unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn patch_epic_title() {
    let db = in_memory_db();
    let epic = db.create_epic("Old Title", "desc", "/repo").unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().title("New Title"))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.title, "New Title");
    assert_eq!(updated.description, "desc"); // unchanged
}

#[test]
fn task_epic_id_roundtrip() {
    let db = in_memory_db();
    let epic = db.create_epic("Epic", "desc", "/repo").unwrap();
    let task_id = db
        .create_task("Task", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();

    db.set_task_epic_id(task_id, Some(epic.id)).unwrap();
    let task = db.get_task(task_id).unwrap().unwrap();
    assert_eq!(task.epic_id, Some(epic.id));

    db.set_task_epic_id(task_id, None).unwrap();
    let task = db.get_task(task_id).unwrap().unwrap();
    assert!(task.epic_id.is_none());
}

#[test]
fn list_tasks_for_epic() {
    let db = in_memory_db();
    let epic = db.create_epic("Epic", "desc", "/repo").unwrap();
    let id1 = db
        .create_task("Sub A", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let _id2 = db
        .create_task(
            "Standalone",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();

    db.set_task_epic_id(id1, Some(epic.id)).unwrap();

    let subtasks = db.list_tasks_for_epic(epic.id).unwrap();
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0].title, "Sub A");
}

#[test]
fn task_roundtrip_with_pr_fields() {
    let db = in_memory_db();
    let id = db
        .create_task(
            "PR task",
            "desc",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();

    db.patch_task(
        id,
        &TaskPatch::new().pr_url(Some("https://github.com/org/repo/pull/42")),
    )
    .unwrap();

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/42")
    );
}

#[test]
fn task_pr_fields_default_to_none() {
    let db = in_memory_db();
    let id = db
        .create_task("No PR", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert!(task.pr_url.is_none());
}

#[test]
fn patch_task_sets_pr_url() {
    let db = in_memory_db();
    let id = db
        .create_task("t", "d", "/r", None, TaskStatus::Backlog, "main")
        .unwrap();

    db.patch_task(
        id,
        &TaskPatch::new().pr_url(Some("https://example.com/pull/1")),
    )
    .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.pr_url.as_deref(), Some("https://example.com/pull/1"));
}

#[test]
fn patch_epic_plan() {
    let db = in_memory_db();
    let epic = db.create_epic("Epic", "desc", "/repo").unwrap();
    assert!(epic.plan_path.is_none());

    db.patch_epic(epic.id, &EpicPatch::new().plan_path(Some("docs/plan.md")))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.plan_path.as_deref(), Some("docs/plan.md"));
}

#[test]
fn patch_epic_clear_plan() {
    let db = in_memory_db();
    let epic = db.create_epic("Epic", "desc", "/repo").unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().plan_path(Some("docs/plan.md")))
        .unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().plan_path(None))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert!(updated.plan_path.is_none());
}

#[test]
fn patch_epic_repo_path() {
    let db = in_memory_db();
    let epic = db.create_epic("Epic", "desc", "/old").unwrap();

    db.patch_epic(epic.id, &EpicPatch::new().repo_path("/new"))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.repo_path, "/new");
    assert_eq!(updated.title, "Epic"); // unchanged
}

#[test]
fn patch_task_sets_sort_order() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(Some(500)))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sort_order, Some(500));
}

#[test]
fn patch_task_clears_sort_order() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(Some(100)))
        .unwrap();
    db.patch_task(id, &TaskPatch::new().sort_order(None))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sort_order, None);
}

#[test]
fn report_usage_first_insert() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("T", "D", "/r", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.report_usage(
        id,
        &UsageReport {
            cost_usd: 0.42,
            input_tokens: 10_000,
            output_tokens: 2_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        },
    )
    .unwrap();
    let all = db.get_all_usage().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].task_id, id);
    assert!((all[0].cost_usd - 0.42).abs() < 1e-9);
    assert_eq!(all[0].input_tokens, 10_000);
    assert_eq!(all[0].output_tokens, 2_000);
    assert_eq!(all[0].cache_read_tokens, 0);
    assert_eq!(all[0].cache_write_tokens, 0);
}

#[test]
fn report_usage_accumulates() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("T", "D", "/r", None, TaskStatus::Backlog, "main")
        .unwrap();
    db.report_usage(
        id,
        &UsageReport {
            cost_usd: 0.10,
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cache_write_tokens: 50,
        },
    )
    .unwrap();
    db.report_usage(
        id,
        &UsageReport {
            cost_usd: 0.05,
            input_tokens: 500,
            output_tokens: 250,
            cache_read_tokens: 50,
            cache_write_tokens: 25,
        },
    )
    .unwrap();
    let all = db.get_all_usage().unwrap();
    assert_eq!(all.len(), 1);
    let u = &all[0];
    assert!((u.cost_usd - 0.15).abs() < 1e-9);
    assert_eq!(u.input_tokens, 1_500);
    assert_eq!(u.output_tokens, 750);
    assert_eq!(u.cache_read_tokens, 150);
    assert_eq!(u.cache_write_tokens, 75);
}

#[test]
fn get_all_usage_empty() {
    let db = Database::open_in_memory().unwrap();
    assert!(db.get_all_usage().unwrap().is_empty());
}

#[test]
fn filter_presets_save_and_list() {
    let db = Database::open_in_memory().unwrap();
    db.save_filter_preset(
        "frontend",
        &["/repo-a".to_string(), "/repo-b".to_string()],
        "include",
    )
    .unwrap();
    db.save_filter_preset("backend", &["/repo-c".to_string()], "exclude")
        .unwrap();

    let presets = db.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 2);
    assert_eq!(presets[0].0, "backend"); // sorted by name
    assert_eq!(presets[0].2, "exclude");
    assert_eq!(presets[1].0, "frontend");
    assert_eq!(
        presets[1].1,
        vec!["/repo-a".to_string(), "/repo-b".to_string()]
    );
    assert_eq!(presets[1].2, "include");
}

#[test]
fn filter_presets_overwrite_and_delete() {
    let db = Database::open_in_memory().unwrap();
    db.save_filter_preset("frontend", &["/repo-a".to_string()], "include")
        .unwrap();
    db.save_filter_preset(
        "frontend",
        &["/repo-x".to_string(), "/repo-y".to_string()],
        "exclude",
    )
    .unwrap();

    let presets = db.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(
        presets[0].1,
        vec!["/repo-x".to_string(), "/repo-y".to_string()]
    );
    assert_eq!(presets[0].2, "exclude");

    db.delete_filter_preset("frontend").unwrap();
    let presets = db.list_filter_presets().unwrap();
    assert!(presets.is_empty());
}

#[test]
fn save_and_load_review_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    // Initially empty
    let prs = db.load_prs(super::PrKind::Review).unwrap();
    assert!(prs.is_empty());

    // Save some PRs
    let pr1 = ReviewPr {
        number: 42,
        title: "Fix bug".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec!["bug".to_string()],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    let pr2 = ReviewPr {
        number: 99,
        title: "Add feature".to_string(),
        author: "bob".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/99".to_string(),
        is_draft: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 200,
        deletions: 80,
        review_decision: ReviewDecision::Approved,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };

    db.save_prs(super::PrKind::Review, &[pr1, pr2]).unwrap();

    let loaded = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(loaded.len(), 2);

    let p1 = loaded.iter().find(|p| p.number == 42).unwrap();
    assert_eq!(p1.title, "Fix bug");
    assert_eq!(p1.author, "alice");
    assert_eq!(p1.repo, "acme/app");
    assert!(!p1.is_draft);
    assert_eq!(p1.additions, 10);
    assert_eq!(p1.review_decision, ReviewDecision::ReviewRequired);
    assert_eq!(p1.labels, vec!["bug".to_string()]);

    let p2 = loaded.iter().find(|p| p.number == 99).unwrap();
    assert_eq!(p2.review_decision, ReviewDecision::Approved);
    assert!(p2.is_draft);
    assert!(p2.labels.is_empty());
}

#[test]
fn save_review_prs_replaces_all() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr, Reviewer};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let pr1 = ReviewPr {
        number: 1,
        title: "Old PR".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/1".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: "Initial body".to_string(),
        head_ref: "feature/old-branch".to_string(),
        ci_status: CiStatus::Pending,
        reviewers: vec![Reviewer {
            login: "carol".to_string(),
            decision: None,
        }],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[pr1]).unwrap();
    assert_eq!(db.load_prs(super::PrKind::Review).unwrap().len(), 1);

    // Verify new fields round-trip on the first save
    let loaded_first = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(loaded_first[0].body, "Initial body");
    assert_eq!(loaded_first[0].head_ref, "feature/old-branch");
    assert_eq!(loaded_first[0].ci_status, CiStatus::Pending);
    assert_eq!(loaded_first[0].reviewers.len(), 1);
    assert_eq!(loaded_first[0].reviewers[0].login, "carol");
    assert_eq!(loaded_first[0].reviewers[0].decision, None);

    // Save new set — old ones should be gone
    let pr2 = ReviewPr {
        number: 2,
        title: "New PR".to_string(),
        author: "bob".to_string(),
        repo: "acme/other".to_string(),
        url: "https://github.com/acme/other/pull/2".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 5,
        deletions: 3,
        review_decision: ReviewDecision::ChangesRequested,
        labels: vec!["urgent".to_string()],
        body: "Needs more work".to_string(),
        head_ref: "fix/new-branch".to_string(),
        ci_status: CiStatus::Failure,
        reviewers: vec![Reviewer {
            login: "dave".to_string(),
            decision: Some(ReviewDecision::ChangesRequested),
        }],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[pr2]).unwrap();

    let loaded = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 2);
    assert_eq!(loaded[0].repo, "acme/other");
    assert_eq!(loaded[0].body, "Needs more work");
    assert_eq!(loaded[0].head_ref, "fix/new-branch");
    assert_eq!(loaded[0].ci_status, CiStatus::Failure);
    assert_eq!(loaded[0].reviewers.len(), 1);
    assert_eq!(loaded[0].reviewers[0].login, "dave");
    assert_eq!(
        loaded[0].reviewers[0].decision,
        Some(ReviewDecision::ChangesRequested)
    );
}

#[test]
fn save_review_prs_preserves_agent_fields() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    // Insert a PR and manually set agent fields
    let pr = ReviewPr {
        number: 42,
        title: "Initial".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: "feature-branch".to_string(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[pr]).unwrap();

    // Simulate agent dispatch by setting agent fields directly
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE review_prs SET tmux_window = 'dispatch:review-42', worktree = '/tmp/wt'
             WHERE repo = 'acme/app' AND number = 42",
            [],
        )
        .unwrap();
    }

    // Now save a refreshed version of the same PR (as if GitHub API returned it)
    let refreshed_pr = ReviewPr {
        number: 42,
        title: "Updated title".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 15,
        deletions: 8,
        review_decision: ReviewDecision::Approved,
        labels: vec![],
        body: String::new(),
        head_ref: "feature-branch".to_string(),
        ci_status: CiStatus::Success,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[refreshed_pr]).unwrap();

    // Agent fields should be preserved, GitHub fields should be updated
    let loaded = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "Updated title");
    assert_eq!(loaded[0].review_decision, ReviewDecision::Approved);
    assert_eq!(loaded[0].tmux_window.as_deref(), Some("dispatch:review-42"));
    assert_eq!(loaded[0].worktree.as_deref(), Some("/tmp/wt"));
}

#[test]
fn save_review_prs_removes_stale_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let make_pr = |number: i64, repo: &str| ReviewPr {
        number,
        title: format!("PR {number}"),
        author: "alice".to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };

    // Save two PRs
    db.save_prs(
        super::PrKind::Review,
        &[make_pr(1, "acme/app"), make_pr(2, "acme/other")],
    )
    .unwrap();
    assert_eq!(db.load_prs(super::PrKind::Review).unwrap().len(), 2);

    // Refresh with only one — the other should be removed
    db.save_prs(super::PrKind::Review, &[make_pr(1, "acme/app")])
        .unwrap();
    let loaded = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 1);
}

#[test]
fn task_sub_status_persists() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("Test", "desc", "/repo", None, TaskStatus::Running, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Stale);
}

#[test]
fn task_sub_status_defaults_to_none() {
    let db = Database::open_in_memory().unwrap();
    let id = db
        .create_task("Test", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn migration_13_converts_needs_input() {
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
             cost_usd           REAL    NOT NULL DEFAULT 0.0,
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
    Database::init_schema(&conn).unwrap();

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 33);

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

#[test]
fn create_task_sets_default_sub_status_for_running() {
    // create_task with status=Running must produce sub_status=active, not 'none'
    let db = in_memory_db();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Running, "main")
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
}

#[test]
fn create_task_sets_default_sub_status_for_backlog() {
    let db = in_memory_db();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Backlog, "main")
        .unwrap();
    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn update_status_if_resets_sub_status_to_default() {
    let db = in_memory_db();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Running, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Stale))
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Running)
        .unwrap();
    assert!(updated, "should have updated");

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview); // default for review
}

#[test]
fn update_status_if_leaves_sub_status_unchanged_when_condition_fails() {
    let db = in_memory_db();
    let id = db
        .create_task("T", "d", "/r", None, TaskStatus::Running, "main")
        .unwrap();
    db.patch_task(id, &TaskPatch::default().sub_status(SubStatus::Active))
        .unwrap();

    let updated = db
        .update_status_if(id, TaskStatus::Review, TaskStatus::Backlog)
        .unwrap();
    assert!(!updated, "condition was wrong, should not have updated");

    let task = db.get_task(id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::Active); // unchanged
}

#[test]
fn check_constraint_rejects_review_with_active_substatus() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status) \
         VALUES ('T', 'D', '/r', 'backlog', 'none')",
        [],
    )
    .unwrap();
    let result = conn.execute(
        "UPDATE tasks SET status = 'review', sub_status = 'active' WHERE id = 1",
        [],
    );
    assert!(
        result.is_err(),
        "CHECK constraint must reject (review, active)"
    );
}

#[test]
fn check_constraint_accepts_review_with_awaiting_review() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO tasks (title, description, repo_path, status, sub_status) \
         VALUES ('T', 'D', '/r', 'backlog', 'none')",
        [],
    )
    .unwrap();
    let result = conn.execute(
        "UPDATE tasks SET status = 'review', sub_status = 'awaiting_review' WHERE id = 1",
        [],
    );
    assert!(result.is_ok(), "valid pair should be accepted");
}

#[test]
fn migration_16_cleans_invalid_review_needs_input() {
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
             cost_usd REAL NOT NULL DEFAULT 0.0,
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
    Database::init_schema(&conn).unwrap();

    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 33);

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

#[test]
fn recalculate_epic_status_advances_to_running() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_moves_backward_from_review_to_running() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_moves_backward_from_review_to_backlog() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();

    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[test]
fn recalculate_epic_status_moves_backward_when_review_subtask_completes() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();

    // Manually set epic to Review (simulating a subtask that was in review and then moved to done)
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Review))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    // Should drop back to Running since no subtask is in review but one is running
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn recalculate_epic_status_all_done() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);
}

#[test]
fn recalculate_epic_status_all_review_or_done() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Review))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[test]
fn recalculate_epic_status_review_beats_running() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t3 = create_task_returning(&db, "T3", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t3.id, Some(epic.id)).unwrap();
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Review))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Review))
        .unwrap();
    db.patch_task(t3.id, &TaskPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[test]
fn cli_update_conditional_sets_epic_to_review() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db.create_epic("E", "", "/repo").unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.recalculate_epic_status(epic.id).unwrap();

    // Simulate hook: dispatch update <id> review --only-if running
    let svc = TaskService::new(db.clone());
    let updated = svc
        .cli_update_task(task.id, TaskStatus::Review, Some(TaskStatus::Running), None)
        .unwrap();
    assert!(updated);

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Review);
}

#[test]
fn cli_update_unconditional_sets_epic_to_running() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db.create_epic("E", "", "/repo").unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();

    // Simulate: dispatch update <id> running (no --only-if)
    let svc = TaskService::new(db.clone());
    let updated = svc
        .cli_update_task(task.id, TaskStatus::Running, None, None)
        .unwrap();
    assert!(updated);

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn cli_update_epic_drops_back_when_review_task_done() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db.create_epic("E", "", "/repo").unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Review).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();
    db.recalculate_epic_status(epic.id).unwrap();
    assert_eq!(
        db.get_epic(epic.id).unwrap().unwrap().status,
        TaskStatus::Review
    );

    // t2 moves to done — epic should drop to Running (t1 still running)
    let svc = TaskService::new(db.clone());
    svc.cli_update_task(t2.id, TaskStatus::Done, Some(TaskStatus::Review), None)
        .unwrap();

    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);
}

#[test]
fn cli_update_with_substatus_keeps_running_and_recalculates_epic() {
    use crate::service::TaskService;

    let db = std::sync::Arc::new(in_memory_db());
    let epic = db.create_epic("E", "", "/repo").unwrap();
    let task = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Running).unwrap();
    db.set_task_epic_id(task.id, Some(epic.id)).unwrap();
    db.recalculate_epic_status(epic.id).unwrap();

    // Hook sets needs_input while staying running:
    // dispatch update <id> running --only-if running --sub-status needs_input
    let svc = TaskService::new(db.clone());
    svc.cli_update_task(
        task.id,
        TaskStatus::Running,
        Some(TaskStatus::Running),
        Some(SubStatus::NeedsInput),
    )
    .unwrap();

    // Epic should still be Running
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Running);

    // Task sub_status should be NeedsInput
    let task = db.get_task(task.id).unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
}

#[test]
fn security_alerts_round_trip() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let db = in_memory_db();
    let now = chrono::Utc::now();

    let alerts = vec![
        SecurityAlert {
            number: 1,
            repo: "acme/app".to_string(),
            severity: AlertSeverity::Critical,
            kind: AlertKind::Dependabot,
            title: "CVE-2024-1234".to_string(),
            package: Some("lodash".to_string()),
            vulnerable_range: Some("< 4.17.21".to_string()),
            fixed_version: Some("4.17.21".to_string()),
            cvss_score: Some(9.8),
            url: "https://github.com/acme/app/security/dependabot/1".to_string(),
            created_at: now,
            state: "open".to_string(),
            description: "Prototype pollution".to_string(),
            tmux_window: None,
            worktree: None,
            agent_status: None,
        },
        SecurityAlert {
            number: 2,
            repo: "acme/app".to_string(),
            severity: AlertSeverity::Low,
            kind: AlertKind::CodeScanning,
            title: "SQL injection".to_string(),
            package: None,
            vulnerable_range: None,
            fixed_version: None,
            cvss_score: None,
            url: "https://github.com/acme/app/security/code-scanning/2".to_string(),
            created_at: now,
            state: "open".to_string(),
            description: "Potential SQL injection".to_string(),
            tmux_window: None,
            worktree: None,
            agent_status: None,
        },
    ];

    db.save_security_alerts(&alerts).unwrap();
    let loaded = db.load_security_alerts().unwrap();

    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].number, 1);
    assert_eq!(loaded[0].repo, "acme/app");
    assert_eq!(loaded[0].severity, AlertSeverity::Critical);
    assert_eq!(loaded[0].kind, AlertKind::Dependabot);
    assert_eq!(loaded[0].package.as_deref(), Some("lodash"));
    assert_eq!(loaded[0].cvss_score, Some(9.8));
    assert_eq!(loaded[0].description, "Prototype pollution");

    assert_eq!(loaded[1].number, 2);
    assert_eq!(loaded[1].severity, AlertSeverity::Low);
    assert_eq!(loaded[1].kind, AlertKind::CodeScanning);
    assert!(loaded[1].package.is_none());
    assert!(loaded[1].cvss_score.is_none());
}

#[test]
fn security_alerts_save_replaces_previous() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let db = in_memory_db();
    let now = chrono::Utc::now();

    let alerts1 = vec![SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "Old alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/1".to_string(),
        created_at: now,
        state: "open".to_string(),
        description: "".to_string(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    }];
    db.save_security_alerts(&alerts1).unwrap();
    assert_eq!(db.load_security_alerts().unwrap().len(), 1);

    let alerts2 = vec![SecurityAlert {
        number: 10,
        repo: "acme/new".to_string(),
        severity: AlertSeverity::Medium,
        kind: AlertKind::CodeScanning,
        title: "New alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/10".to_string(),
        created_at: now,
        state: "open".to_string(),
        description: "".to_string(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    }];
    db.save_security_alerts(&alerts2).unwrap();
    let loaded = db.load_security_alerts().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "New alert");
}

#[test]
fn save_security_alerts_preserves_agent_fields() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let alert = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-1234".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.21".to_string()),
        cvss_score: Some(7.5),
        url: "https://github.com/acme/app/security/dependabot/1".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_security_alerts(&[alert]).unwrap();

    // Simulate agent dispatch
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE security_alerts SET tmux_window = 'dispatch:fix-1', worktree = '/tmp/wt'
             WHERE repo = 'acme/app' AND number = 1 AND kind = 'dependabot'",
            [],
        )
        .unwrap();
    }

    // Refresh with updated alert data
    let refreshed = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-1234 (updated)".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.22".to_string()),
        cvss_score: Some(7.5),
        url: "https://github.com/acme/app/security/dependabot/1".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_security_alerts(&[refreshed]).unwrap();

    let loaded = db.load_security_alerts().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "CVE-2024-1234 (updated)");
    assert_eq!(loaded[0].fixed_version.as_deref(), Some("4.17.22"));
    assert_eq!(loaded[0].tmux_window.as_deref(), Some("dispatch:fix-1"));
    assert_eq!(loaded[0].worktree.as_deref(), Some("/tmp/wt"));
}

#[test]
fn seed_github_query_defaults_sets_all_three() {
    let db = in_memory_db();
    db.seed_github_query_defaults().unwrap();

    let review = db
        .get_setting_string("github_queries_review")
        .unwrap()
        .expect("review queries should be set");
    assert!(review.contains("review-requested:@me"));

    let my_prs = db
        .get_setting_string("github_queries_my_prs")
        .unwrap()
        .expect("my_prs queries should be set");
    assert!(my_prs.contains("author:@me"));

    let bot = db
        .get_setting_string("github_queries_bot")
        .unwrap()
        .expect("bot queries should be set");
    assert!(bot.contains("app/dependabot"));
    assert!(bot.contains("app/renovate"));
}

#[test]
fn seed_github_query_defaults_does_not_overwrite_user_edits() {
    let db = in_memory_db();
    db.seed_github_query_defaults().unwrap();

    // User edits the review queries
    db.set_setting_string("github_queries_review", "my custom query")
        .unwrap();

    // Re-seed should not overwrite
    db.seed_github_query_defaults().unwrap();

    let review = db
        .get_setting_string("github_queries_review")
        .unwrap()
        .unwrap();
    assert_eq!(review, "my custom query");
}

#[test]
fn delete_repo_path_removes_entry() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/project").unwrap();
    db.save_repo_path("/home/user/other").unwrap();
    assert_eq!(db.list_repo_paths().unwrap().len(), 2);
    db.delete_repo_path("/home/user/project").unwrap();
    let paths = db.list_repo_paths().unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], "/home/user/other");
}

#[test]
fn delete_repo_path_nonexistent_is_ok() {
    let db = in_memory_db();
    db.delete_repo_path("/does/not/exist").unwrap();
}

#[test]
fn delete_repo_path_cleans_presets() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/a").unwrap();
    db.save_repo_path("/home/user/b").unwrap();
    db.save_filter_preset(
        "my_preset",
        &["/home/user/a".to_string(), "/home/user/b".to_string()],
        "include",
    )
    .unwrap();
    db.delete_repo_path("/home/user/a").unwrap();
    let presets = db.list_filter_presets().unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].0, "my_preset");
    assert_eq!(presets[0].1, vec!["/home/user/b".to_string()]);
}

#[test]
fn delete_repo_path_removes_empty_preset() {
    let db = in_memory_db();
    db.save_repo_path("/home/user/solo").unwrap();
    db.save_filter_preset("solo_preset", &["/home/user/solo".to_string()], "include")
        .unwrap();
    db.delete_repo_path("/home/user/solo").unwrap();
    let presets = db.list_filter_presets().unwrap();
    assert!(presets.is_empty());
}

// ---------------------------------------------------------------------------
// Migration-specific tests — verify data preservation through table rebuilds
// ---------------------------------------------------------------------------

#[test]
fn migration_v4_preserves_epic_data_after_table_rebuild() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn migration_v15_converts_needs_input_to_sub_status() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn migration_v16_cleans_invalid_status_pairs() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn migration_v18_expands_tilde_paths() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn migration_v20_converts_done_boolean_to_status_enum() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn set_pr_agent_updates_fields() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let pr = ReviewPr {
        number: 42,
        title: "Test".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[pr]).unwrap();

    db.set_pr_agent(
        super::PrKind::Review,
        "acme/app",
        42,
        "dispatch:review-42",
        "/tmp/wt",
    )
    .unwrap();

    let loaded = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(loaded[0].tmux_window.as_deref(), Some("dispatch:review-42"));
    assert_eq!(loaded[0].worktree.as_deref(), Some("/tmp/wt"));
    assert_eq!(
        loaded[0].agent_status,
        Some(crate::models::ReviewAgentStatus::Reviewing)
    );
}

#[test]
fn set_alert_agent_updates_fields() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let alert = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: String::new(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_security_alerts(&[alert]).unwrap();

    db.set_alert_agent(
        "acme/app",
        1,
        AlertKind::Dependabot,
        "dispatch:fix-1",
        "/tmp/wt",
    )
    .unwrap();

    let loaded = db.load_security_alerts().unwrap();
    assert_eq!(loaded[0].tmux_window.as_deref(), Some("dispatch:fix-1"));
    assert_eq!(loaded[0].worktree.as_deref(), Some("/tmp/wt"));
    assert_eq!(
        loaded[0].agent_status,
        Some(crate::models::ReviewAgentStatus::Reviewing)
    );
}

#[test]
fn update_agent_status_finds_review_pr() {
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 42,
        title: "Test".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[pr]).unwrap();
    db.set_pr_agent(
        super::PrKind::Review,
        "acme/app",
        42,
        "dispatch:review-42",
        "/tmp/wt",
    )
    .unwrap();

    let table = db
        .update_agent_status("acme/app", 42, Some("findings_ready"))
        .unwrap();
    assert_eq!(table, "review_prs");

    let loaded = db.load_prs(super::PrKind::Review).unwrap();
    assert_eq!(
        loaded[0].agent_status,
        Some(ReviewAgentStatus::FindingsReady)
    );
}

#[test]
fn update_agent_status_finds_bot_pr() {
    use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 10,
        title: "Bump dep".to_string(),
        author: "dependabot".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/10".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 1,
        deletions: 1,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Bot, &[pr]).unwrap();
    db.set_pr_agent(
        super::PrKind::Bot,
        "acme/app",
        10,
        "dispatch:review-10",
        "/tmp/wt",
    )
    .unwrap();

    let table = db
        .update_agent_status("acme/app", 10, Some("idle"))
        .unwrap();
    assert_eq!(table, "bot_prs");

    let loaded = db.load_prs(super::PrKind::Bot).unwrap();
    assert_eq!(loaded[0].agent_status, Some(ReviewAgentStatus::Idle));
}

#[test]
fn update_agent_status_finds_security_alert() {
    use crate::models::{AlertKind, AlertSeverity, ReviewAgentStatus, SecurityAlert};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let alert = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: String::new(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_security_alerts(&[alert]).unwrap();
    db.set_alert_agent(
        "acme/app",
        1,
        AlertKind::Dependabot,
        "dispatch:fix-1",
        "/tmp/wt",
    )
    .unwrap();

    let table = db
        .update_agent_status("acme/app", 1, Some("findings_ready"))
        .unwrap();
    assert_eq!(table, "security_alerts");

    let loaded = db.load_security_alerts().unwrap();
    assert_eq!(
        loaded[0].agent_status,
        Some(ReviewAgentStatus::FindingsReady)
    );
}

#[test]
fn update_agent_status_errors_when_no_match() {
    let db = Database::open_in_memory().unwrap();
    let result = db.update_agent_status("acme/unknown", 999, Some("idle"));
    assert!(result.is_err());
}

#[test]
fn update_agent_status_skips_pr_without_tmux() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    let pr = ReviewPr {
        number: 42,
        title: "Test".to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/42".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Review, &[pr]).unwrap();

    // PR has no tmux_window, so update should fail
    let result = db.update_agent_status("acme/app", 42, Some("findings_ready"));
    assert!(result.is_err());
}

#[test]
fn migration_v17_adds_conflict_sub_status() {
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

    Database::init_schema(&conn).unwrap();

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

// ---------------------------------------------------------------------------
// Query coverage: delete_task
// ---------------------------------------------------------------------------

#[test]
fn delete_task_removes_task() {
    let db = in_memory_db();
    let id = db
        .create_task("Doomed", "desc", "/repo", None, TaskStatus::Backlog, "main")
        .unwrap();
    assert!(db.get_task(id).unwrap().is_some());

    db.delete_task(id).unwrap();
    assert!(db.get_task(id).unwrap().is_none());
}

#[test]
fn delete_task_nonexistent_errors() {
    let db = in_memory_db();
    let result = db.delete_task(TaskId(9999));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Query coverage: my_prs / bot_prs round-trip
// ---------------------------------------------------------------------------

#[test]
fn save_and_load_my_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    assert!(db.load_prs(super::PrKind::My).unwrap().is_empty());

    let pr = ReviewPr {
        number: 7,
        title: "My feature".to_string(),
        author: "me".to_string(),
        repo: "acme/app".to_string(),
        url: "https://github.com/acme/app/pull/7".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 42,
        deletions: 10,
        review_decision: ReviewDecision::Approved,
        labels: vec!["feature".to_string()],
        body: "Add new feature".to_string(),
        head_ref: "feature/my-branch".to_string(),
        ci_status: CiStatus::Success,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::My, &[pr]).unwrap();

    let loaded = db.load_prs(super::PrKind::My).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 7);
    assert_eq!(loaded[0].title, "My feature");
    assert_eq!(loaded[0].author, "me");
    assert_eq!(loaded[0].review_decision, ReviewDecision::Approved);
    assert_eq!(loaded[0].labels, vec!["feature".to_string()]);
    assert_eq!(loaded[0].body, "Add new feature");
    assert_eq!(loaded[0].ci_status, CiStatus::Success);
}

#[test]
fn save_and_load_bot_prs() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();
    assert!(db.load_prs(super::PrKind::Bot).unwrap().is_empty());

    let pr = ReviewPr {
        number: 55,
        title: "Bump lodash".to_string(),
        author: "dependabot[bot]".to_string(),
        repo: "acme/lib".to_string(),
        url: "https://github.com/acme/lib/pull/55".to_string(),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 3,
        deletions: 3,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec!["dependencies".to_string()],
        body: "Bumps lodash".to_string(),
        head_ref: "dependabot/npm_and_yarn/lodash-4.17.21".to_string(),
        ci_status: CiStatus::Pending,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };
    db.save_prs(super::PrKind::Bot, &[pr]).unwrap();

    let loaded = db.load_prs(super::PrKind::Bot).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].number, 55);
    assert_eq!(loaded[0].title, "Bump lodash");
    assert_eq!(loaded[0].author, "dependabot[bot]");
    assert_eq!(loaded[0].ci_status, CiStatus::Pending);
}

#[test]
fn my_prs_and_review_prs_are_independent() {
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    let db = Database::open_in_memory().unwrap();

    let make_pr = |number: i64, title: &str| ReviewPr {
        number,
        title: title.to_string(),
        author: "alice".to_string(),
        repo: "acme/app".to_string(),
        url: format!("https://github.com/acme/app/pull/{number}"),
        is_draft: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        additions: 0,
        deletions: 0,
        review_decision: ReviewDecision::ReviewRequired,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    };

    db.save_prs(super::PrKind::My, &[make_pr(1, "My PR")])
        .unwrap();
    db.save_prs(super::PrKind::Review, &[make_pr(2, "Review PR")])
        .unwrap();
    db.save_prs(super::PrKind::Bot, &[make_pr(3, "Bot PR")])
        .unwrap();

    assert_eq!(db.load_prs(super::PrKind::My).unwrap().len(), 1);
    assert_eq!(db.load_prs(super::PrKind::Review).unwrap().len(), 1);
    assert_eq!(db.load_prs(super::PrKind::Bot).unwrap().len(), 1);

    // Saving empty to one table doesn't affect others
    db.save_prs(super::PrKind::My, &[]).unwrap();
    assert!(db.load_prs(super::PrKind::My).unwrap().is_empty());
    assert_eq!(db.load_prs(super::PrKind::Review).unwrap().len(), 1);
    assert_eq!(db.load_prs(super::PrKind::Bot).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Query coverage: patch_epic edge cases
// ---------------------------------------------------------------------------

#[test]
fn patch_epic_nonexistent_errors() {
    let db = in_memory_db();
    let result = db.patch_epic(EpicId(9999), &EpicPatch::new().title("x"));
    assert!(result.is_err());
}

#[test]
fn patch_epic_no_changes_is_noop() {
    let db = in_memory_db();
    let epic = db.create_epic("Title", "desc", "/repo").unwrap();
    // Empty patch — has_changes() is false, so this should succeed without touching DB
    db.patch_epic(epic.id, &EpicPatch::new()).unwrap();
    let fetched = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(fetched.title, "Title");
}

#[test]
fn patch_epic_sort_order() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "desc", "/repo").unwrap();
    assert!(epic.sort_order.is_none());

    db.patch_epic(epic.id, &EpicPatch::new().sort_order(Some(42)))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(updated.sort_order, Some(42));

    // Clear sort_order
    db.patch_epic(epic.id, &EpicPatch::new().sort_order(None))
        .unwrap();
    let cleared = db.get_epic(epic.id).unwrap().unwrap();
    assert!(cleared.sort_order.is_none());
}

#[test]
fn delete_epic_nonexistent_errors() {
    let db = in_memory_db();
    let result = db.delete_epic(EpicId(9999));
    assert!(result.is_err());
}

#[test]
fn recalculate_epic_status_ignores_archived_subtasks() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();

    let t1 = create_task_returning(&db, "T1", "", "/repo", None, TaskStatus::Backlog).unwrap();
    let t2 = create_task_returning(&db, "T2", "", "/repo", None, TaskStatus::Backlog).unwrap();
    db.set_task_epic_id(t1.id, Some(epic.id)).unwrap();
    db.set_task_epic_id(t2.id, Some(epic.id)).unwrap();

    // t1 done, t2 archived — only non-archived counted, so all done → Done
    db.patch_task(t1.id, &TaskPatch::new().status(TaskStatus::Done))
        .unwrap();
    db.patch_task(t2.id, &TaskPatch::new().status(TaskStatus::Archived))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Done);
}

#[test]
fn recalculate_epic_status_no_subtasks_stays_backlog() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "", "/repo").unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().status(TaskStatus::Running))
        .unwrap();

    db.recalculate_epic_status(epic.id).unwrap();
    let epic = db.get_epic(epic.id).unwrap().unwrap();
    assert_eq!(epic.status, TaskStatus::Backlog);
}

#[test]
fn recalculate_epic_status_nonexistent_is_noop() {
    let db = in_memory_db();
    // Should not error for nonexistent epic
    db.recalculate_epic_status(EpicId(9999)).unwrap();
}

#[test]
fn migration_v29_converts_newline_presets_to_json() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn migration_v29_skips_already_json_presets() {
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

    Database::init_schema(&conn).unwrap();

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

#[test]
fn migration_31_re_expands_tilde_paths() {
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

    Database::init_schema(&conn).unwrap();

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
    assert_eq!(version, 33);
}

#[test]
fn migrate_v32_adds_base_branch_column() {
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
    Database::init_schema(&conn).unwrap();

    // Existing task should have base_branch defaulted to 'main'
    let base_branch: String = conn
        .query_row("SELECT base_branch FROM tasks WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(base_branch, "main");

    // init_schema runs all pending migrations (v32 and v33), so final version is 33
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 33);
}

#[test]
fn migration_v33_adds_auto_dispatch_to_epics() {
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

    migrations::migrate_v33_add_auto_dispatch(&conn).unwrap();

    let auto_dispatch: i64 = conn
        .query_row(
            "SELECT auto_dispatch FROM epics WHERE title = 'Test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(auto_dispatch, 1);
}

#[test]
fn patch_epic_auto_dispatch_persists() {
    let db = in_memory_db();
    let epic = db.create_epic("E", "desc", "/repo").unwrap();
    assert!(epic.auto_dispatch); // default true

    db.patch_epic(epic.id, &EpicPatch::new().auto_dispatch(false))
        .unwrap();
    let updated = db.get_epic(epic.id).unwrap().unwrap();
    assert!(!updated.auto_dispatch);

    db.patch_epic(epic.id, &EpicPatch::new().auto_dispatch(true))
        .unwrap();
    let re_enabled = db.get_epic(epic.id).unwrap().unwrap();
    assert!(re_enabled.auto_dispatch);
}
