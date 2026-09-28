//! Database migrations.
//!
//! ## Convention
//!
//! [`MIGRATIONS`] is the single ordered registry: a list of
//! `(version, fn)` pairs that the migration runner applies in array order.
//! That order is the **source of truth** — the migration functions below it
//! are defined in arbitrary order and grouped only by where they happened to
//! land over time.
//!
//! Rules for editing:
//! - **Append only.** Add new migrations at the end with the next sequential
//!   version number. Never reorder, renumber, or delete an existing entry —
//!   each has run against production databases and the version number is
//!   recorded there.
//! - The `// ── era ──` comments inside `MIGRATIONS` group entries by rough
//!   era/concern for **readability only**. They carry no runtime meaning;
//!   moving an entry across a group boundary would change schema history.
//!
//! ## Squashing policy
//!
//! We do **not** squash migrations. Every entry in `MIGRATIONS` has run in
//! production at some point; squashing would diverge the schema history from
//! the audit trail. Running all N migrations on a fresh install is negligible
//! (sub-millisecond for SQLite). If the number grows beyond ~100, revisit.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::queries::HOST_ID_KEY;
use crate::models::{SubStatus, TaskStatus};

pub(super) type Migration = (i64, fn(&Connection) -> Result<()>);

pub(super) fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        params![table, column],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

pub(super) const MIGRATIONS: &[Migration] = &[
    // ── Foundations (v1–v9): initial task/epic/settings schema ──
    (1, migrate_v1_add_plan_column),
    (2, migrate_v2_drop_notes_table),
    (3, migrate_v3_create_epics_table),
    (4, migrate_v4_add_needs_input_drop_epic_plan),
    (5, migrate_v5_create_settings_table),
    (6, migrate_v6_rename_ready_to_backlog),
    (7, migrate_v7_add_pr_columns),
    (8, migrate_v8_add_epic_plan),
    (9, migrate_v9_add_sort_order),
    // ── Usage, filter presets, tags & sub-statuses (v10–v17) ──
    (10, migrate_v10_create_task_usage_table),
    (11, migrate_v11_create_filter_presets_table),
    (12, migrate_v12_drop_pr_number),
    (13, migrate_v13_add_tag),
    (14, migrate_v14_create_review_prs_table),
    (15, migrate_v15_add_sub_status),
    (16, migrate_v16_add_status_check_constraint),
    (17, migrate_v17_add_conflict_sub_status),
    // ── Path normalization, PR/agent tracking & epic status (v18–v31) ──
    (18, migrate_v18_expand_tilde_paths),
    (19, migrate_v19_add_review_pr_columns),
    (20, migrate_v20_epic_status_enum),
    (21, migrate_v21_create_my_prs_table),
    (22, migrate_v22_add_filter_preset_mode),
    (23, migrate_v23_create_bot_prs_table),
    (24, migrate_v24_create_security_alerts_table),
    (25, migrate_v25_rename_plan_to_plan_path),
    (26, migrate_v26_add_agent_columns),
    (27, migrate_v27_add_agent_status),
    (28, migrate_v28_add_my_prs_agent_status),
    (29, migrate_v29_json_filter_presets),
    (30, migrate_v30_allow_conflict_for_review),
    (31, migrate_v31_re_expand_tilde_paths),
    // ── Branches, dispatch, epic nesting, feeds & projects (v32–v39) ──
    (32, migrate_v32_add_base_branch),
    (33, migrate_v33_add_auto_dispatch),
    (34, migrate_v34_add_parent_epic_id),
    (35, migrate_v35_add_self_ref_check),
    (36, migrate_v36_tips_state),         // superseded by v84
    (37, migrate_v37_pr_workflow_states), // pr_workflow_states created here — superseded by v51 drop
    (38, migrate_v38_feed_epic_columns),
    (39, migrate_v39_add_projects),
    // ── Knowledge base / learnings (v40–v51) ──
    (40, migrate_v40_create_learnings),
    (41, migrate_v41_drop_cost_usd),
    (42, migrate_v42_drop_epic_tag),
    (43, migrate_v43_proposed_to_approved),
    (44, migrate_v44_episodic_to_convention),
    (45, migrate_v45_add_task_labels),
    (46, migrate_v46_learning_source_task_set_null),
    (47, migrate_v47_task_usage_restore_cascade),
    (48, migrate_v48_learning_validation),
    (49, migrate_v49_rename_confirmed_to_upvote),
    (50, migrate_v50_add_hook_timestamps),
    (51, migrate_v51_drop_pr_workflow_states), // drops pr_workflow_states created in v37
    // ── Wrap-up, embeddings, schema cleanup & typed URLs (v52–v64) ──
    (52, migrate_v52_add_verify_command_to_repo_paths),
    (53, migrate_v53_add_wrap_up_mode),
    (54, migrate_v54_add_group_by_repo),
    (55, migrate_v55_add_learning_embedding),
    (56, migrate_v56_drop_task_usage),
    (57, migrate_v57_enforce_epic_project_consistency),
    (58, migrate_v58_reset_intermediate_epic_statuses),
    (59, migrate_v59_create_usage_events),
    (60, migrate_v60_drop_projects),
    (61, migrate_v61_drop_epic_repo_path),
    (62, migrate_v62_drop_unused_verdicts),
    (63, migrate_v63_add_task_indexes),
    (64, migrate_v64_typed_url),
    (65, migrate_v65_add_epic_feed_role),
    (66, migrate_v66_add_pr_learnings_gate),
    (67, migrate_v67_create_todos),
    (68, migrate_v68_add_todo_links),
    (69, migrate_v69_add_epic_origin),
    (70, migrate_v70_add_todo_parent_id),
    (71, migrate_v71_dedup_role_subtree_tasks),
    (72, migrate_v72_add_feed_task_subtree_unique_triggers),
    (73, migrate_v73_needs_review_to_approved),
    (74, migrate_v74_drop_learning_verdicts),
    (75, migrate_v75_create_repo_base_branches),
    (
        76,
        migrate_v76_fix_feed_task_subtree_insert_trigger_self_conflict,
    ),
    (77, migrate_v77_add_auto_run_plan),
    (78, migrate_v78_create_task_watchers),
    (79, migrate_v79_backfill_done_sort_order),
    (80, migrate_v80_drop_agent_status),
    (81, migrate_v81_create_task_subagents),
    (82, migrate_v82_resolve_stranded_pending_stops),
    (83, migrate_v83_add_stop_pending_at),
    (84, migrate_v84_drop_tips_state), // drops table created in v36
    (85, migrate_v85_create_task_shells),
    (86, migrate_v86_allow_stale_shell),
    (87, migrate_v87_add_peer_message_columns),
    (88, migrate_v88_add_scheduling_fields),
    (89, migrate_v89_allow_pr_closed_for_review),
    (90, migrate_v90_drop_scheduling_fields),
    (91, migrate_v91_clamp_feed_intervals),
    (92, migrate_v92_add_phoenix),
    (93, migrate_v93_fix_root_epic_feed_role_uniqueness),
    (94, migrate_v94_add_feed_append_only),
    (95, migrate_v95_drop_main_session_dir),
    (96, migrate_v96_allow_pr_unreachable_for_review),
    (97, migrate_v97_add_task_host),
    (98, migrate_v98_create_subscriptions),
    (99, migrate_v99_add_completed_at),
    (100, migrate_v100_add_todo_owner),
    (101, migrate_v101_drop_shell_tracking),
    (102, migrate_v102_drop_todos),
    (103, migrate_v103_drop_legacy_pr_tables), // drops tables created in v14/v21/v23/v24
    (104, migrate_v104_drop_filter_presets),
    // ── Archive → permanent delete (v105–v106, task #4971) ──
    (105, migrate_v105_create_retired_feed_items),
    (106, migrate_v106_archived_status_migration),
];

/// The schema version a fresh database ends up at after all migrations run.
/// Derived from [`MIGRATIONS`] so adding a migration bumps this — and every
/// test that asserts against it — without touching a scattered literal.
#[cfg(test)]
pub(super) const LATEST_SCHEMA_VERSION: i64 = MIGRATIONS[MIGRATIONS.len() - 1].0;

/// Replace the single `pr_url` column with a typed URL: `url` + `url_type`.
/// Backfill classifies existing URLs the same way the old render-time
/// heuristic did. `security_alert` has NO legacy rows — it only arrives via
/// new code paths (and, later, feed-declared types per task #1808).
/// Uses DROP COLUMN (SQLite 3.35+, bundled) so all other columns, indexes
/// (tasks_epic_external_id, idx_tasks_status, idx_tasks_epic_id) and triggers
/// survive untouched — no table rebuild.
pub(super) fn migrate_v64_typed_url(conn: &Connection) -> Result<()> {
    // Guard for idempotency / partial runs.
    if !column_exists(conn, "tasks", "url") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN url TEXT")
            .context("v64: add url column")?;
    }
    if !column_exists(conn, "tasks", "url_type") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN url_type TEXT")
            .context("v64: add url_type column")?;
    }
    // Some historical pre-v64 schemas (and idempotent re-runs) may not have a
    // `pr_url` column at all; only backfill and drop when it is present.
    if column_exists(conn, "tasks", "pr_url") {
        // The WHERE clause covers exactly the two backfill cases: rows with a
        // legacy `pr_url`, and any partially-migrated row whose `url` is set but
        // lacks a `url_type`. Scoping it this way avoids leaving an inconsistent
        // (url set, url_type NULL) state behind after a mid-UPDATE partial run.
        conn.execute_batch(
            "UPDATE tasks SET
                 url = pr_url,
                 url_type = CASE
                     WHEN pr_url IS NULL           THEN NULL
                     -- LIKE '%/pull/%' / '%/issues/%' do not strip ?query/#fragment (unlike
                     -- UrlType::infer); GitHub URLs never place /pull/ or /issues/ after ? or #,
                     -- so the classification matches in practice.
                     WHEN pr_url LIKE '%/pull/%'   THEN 'pr'
                     WHEN pr_url LIKE '%/issues/%' THEN 'issue'
                     ELSE 'other'
                 END
             WHERE pr_url IS NOT NULL
                OR (url IS NOT NULL AND url_type IS NULL)",
        )
        .context("v64: backfill url/url_type from pr_url")?;
        conn.execute_batch("ALTER TABLE tasks DROP COLUMN pr_url")
            .context("v64: drop pr_url column")?;
    }
    Ok(())
}

pub(super) fn migrate_v62_drop_unused_verdicts(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "learning_verdicts") {
        return Ok(());
    }
    conn.execute_batch("DELETE FROM learning_verdicts WHERE verdict = 'unused'")
        .context("Failed to delete legacy 'unused' learning verdicts")
}

fn migrate_v53_add_wrap_up_mode(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE tasks ADD COLUMN wrap_up_mode TEXT")
        .context("Failed to add wrap_up_mode column")
}

fn migrate_v77_add_auto_run_plan(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE tasks ADD COLUMN auto_run_plan BOOLEAN NOT NULL DEFAULT 0")
        .context("Failed to add auto_run_plan column to tasks")
}

/// A phoenix task recreates itself on completion: entering Done spawns a fresh
/// Backlog copy and the flag moves to it (`PhoenixRespawn` in
/// `docs/specs/tasks.allium`). `NOT NULL DEFAULT 0` backfills every existing
/// row to "not recurring", which is what they all are.
///
/// `ADD COLUMN` — unlike `DROP COLUMN`, it does not make SQLite re-resolve the
/// table's triggers, so the `tasks` feed-subtree triggers are untouched here.
/// See `migrate_v90_drop_scheduling_fields` for the hazard that applies the
/// other way round.
pub(super) fn migrate_v92_add_phoenix(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE tasks ADD COLUMN phoenix BOOLEAN NOT NULL DEFAULT 0")
        .context("Failed to add phoenix column to tasks")
}

/// v65's `idx_epics_parent_feed_role` indexes raw `parent_epic_id`, but SQLite
/// treats every `NULL` in a unique index as distinct from every other `NULL` —
/// so it never actually deduplicated root-level managed epics (those with no
/// parent, e.g. `reviews-parent`/`cve`), only sub-epics. Rebuild it over
/// `COALESCE(parent_epic_id, -1)` so root epics collide on `feed_role` too;
/// -1 is never a real epic id (autoincrement rowids start at 1).
///
/// A database that already has duplicate root-level managed epics (only
/// reachable by having lost the exact startup race this fixes, before it was
/// fixed) would fail `CREATE UNIQUE INDEX`. Rather than delete or merge a
/// user's epics automatically, skip index creation and warn — the existing
/// duplicates are a pre-existing condition this migration does not make
/// worse, and can be resolved manually.
fn migrate_v93_fix_root_epic_feed_role_uniqueness(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "epics") || !column_exists(conn, "epics", "parent_epic_id") {
        return Ok(());
    }
    conn.execute_batch("DROP INDEX IF EXISTS idx_epics_parent_feed_role;")
        .context("Failed to drop old feed_role unique index (migration v93)")?;
    let has_duplicates: bool = conn
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM epics
                 WHERE feed_role <> 'none'
                 GROUP BY COALESCE(parent_epic_id, -1), feed_role
                 HAVING COUNT(*) > 1
             )",
            [],
            |r| r.get(0),
        )
        .context("Failed to check for duplicate managed epics (migration v93)")?;
    if has_duplicates {
        tracing::warn!(
            "migration v93: found duplicate managed-role epics (same parent + feed_role); \
             leaving idx_epics_parent_feed_role unbuilt rather than deleting data. \
             Resolve manually by reassigning or clearing feed_role on the duplicates."
        );
        return Ok(());
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_epics_parent_feed_role
             ON epics(COALESCE(parent_epic_id, -1), feed_role)
             WHERE feed_role <> 'none';",
    )
    .context("Failed to add fixed feed_role unique index (migration v93)")
}

fn migrate_v1_add_plan_column(conn: &Connection) -> Result<()> {
    let _ = conn.execute_batch("ALTER TABLE tasks ADD COLUMN plan TEXT");
    Ok(())
}

fn migrate_v2_drop_notes_table(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS notes")
        .context("Failed to drop notes table")
}

fn migrate_v3_create_epics_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS epics (
            id          INTEGER PRIMARY KEY,
            title       TEXT NOT NULL,
            description TEXT NOT NULL,
            plan        TEXT NOT NULL DEFAULT '',
            repo_path   TEXT NOT NULL,
            done        INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .context("Failed to create epics table")?;

    let _ = conn.execute_batch("ALTER TABLE tasks ADD COLUMN epic_id INTEGER REFERENCES epics(id)");

    Ok(())
}

fn migrate_v4_add_needs_input_drop_epic_plan(conn: &Connection) -> Result<()> {
    let _ =
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN needs_input INTEGER NOT NULL DEFAULT 0");

    // SQLite doesn't support DROP COLUMN before 3.35.0; recreate the table.
    // FK checks and transaction boundaries around this DDL are the caller's
    // responsibility (see `apply_pending_migrations` in `src/db/mod.rs`) —
    // `PRAGMA foreign_keys` is a no-op mid-transaction, so it can't be
    // toggled from inside a migration body that runs inside one.
    conn.execute_batch(
        "CREATE TABLE epics_new (
            id          INTEGER PRIMARY KEY,
            title       TEXT NOT NULL,
            description TEXT NOT NULL,
            repo_path   TEXT NOT NULL,
            done        INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        INSERT INTO epics_new (id, title, description, repo_path, done, created_at, updated_at)
            SELECT id, title, description, repo_path, done, created_at, updated_at FROM epics;
        DROP TABLE epics;
        ALTER TABLE epics_new RENAME TO epics;",
    )
    .context("Failed to migrate epics (drop plan column)")
}

fn migrate_v5_create_settings_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
    )
    .context("Failed to create settings table")
}

fn migrate_v6_rename_ready_to_backlog(conn: &Connection) -> Result<()> {
    conn.execute_batch("UPDATE tasks SET status = 'backlog' WHERE status = 'ready'")
        .context("Failed to migrate ready tasks to backlog")
}

fn migrate_v7_add_pr_columns(conn: &Connection) -> Result<()> {
    let _ = conn.execute_batch("ALTER TABLE tasks ADD COLUMN pr_url TEXT");
    let _ = conn.execute_batch("ALTER TABLE tasks ADD COLUMN pr_number INTEGER");
    Ok(())
}

fn migrate_v8_add_epic_plan(conn: &Connection) -> Result<()> {
    let _ = conn.execute_batch("ALTER TABLE epics ADD COLUMN plan TEXT");
    Ok(())
}

fn migrate_v9_add_sort_order(conn: &Connection) -> Result<()> {
    let _ = conn.execute_batch("ALTER TABLE tasks ADD COLUMN sort_order INTEGER");
    let _ = conn.execute_batch("ALTER TABLE epics ADD COLUMN sort_order INTEGER");
    Ok(())
}

fn migrate_v10_create_task_usage_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_usage (
            task_id            INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            cost_usd           REAL    NOT NULL DEFAULT 0.0,
            input_tokens       INTEGER NOT NULL DEFAULT 0,
            output_tokens      INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            updated_at         TEXT    NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .context("Failed to create task_usage table")
}

fn migrate_v11_create_filter_presets_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS filter_presets (
            name       TEXT PRIMARY KEY,
            repo_paths TEXT NOT NULL
        )",
    )
    .context("Failed to create filter_presets table")
}

fn migrate_v12_drop_pr_number(conn: &Connection) -> Result<()> {
    // DROP COLUMN requires SQLite 3.35.0+; bundled libsqlite3-sys satisfies this.
    // Ignore error for fresh DBs where the column was never added.
    let _ = conn.execute_batch("ALTER TABLE tasks DROP COLUMN pr_number");
    Ok(())
}

fn migrate_v13_add_tag(conn: &Connection) -> Result<()> {
    let _ = conn.execute_batch("ALTER TABLE tasks ADD COLUMN tag TEXT");
    Ok(())
}

pub(super) fn migrate_v14_create_review_prs_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS review_prs (
            repo            TEXT    NOT NULL,
            number          INTEGER NOT NULL,
            title           TEXT    NOT NULL,
            author          TEXT    NOT NULL,
            url             TEXT    NOT NULL,
            is_draft        INTEGER NOT NULL,
            created_at      TEXT    NOT NULL,
            updated_at      TEXT    NOT NULL,
            additions       INTEGER NOT NULL,
            deletions       INTEGER NOT NULL,
            review_decision TEXT    NOT NULL,
            labels          TEXT    NOT NULL,
            body            TEXT    NOT NULL DEFAULT '',
            head_ref        TEXT    NOT NULL DEFAULT '',
            ci_status       TEXT    NOT NULL DEFAULT 'none',
            reviewers       TEXT    NOT NULL DEFAULT '[]',
            PRIMARY KEY (repo, number)
        )",
    )
    .context("Failed to create review_prs table")
}

fn migrate_v15_add_sub_status(conn: &Connection) -> Result<()> {
    let _ =
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN sub_status TEXT NOT NULL DEFAULT 'none'");
    let _ = conn.execute_batch("UPDATE tasks SET sub_status = 'needs_input' WHERE needs_input = 1");
    let _ = conn.execute_batch(
        "UPDATE tasks SET sub_status = 'active' WHERE status = 'running' AND sub_status = 'none'",
    );
    let _ = conn.execute_batch(
        "UPDATE tasks SET sub_status = 'awaiting_review' WHERE status = 'review' AND sub_status = 'none'",
    );
    conn.execute_batch(
        "CREATE TABLE tasks_new (
            id          INTEGER PRIMARY KEY,
            title       TEXT NOT NULL,
            description TEXT NOT NULL,
            repo_path   TEXT NOT NULL,
            status      TEXT NOT NULL DEFAULT 'backlog',
            worktree    TEXT,
            tmux_window TEXT,
            plan        TEXT,
            epic_id     INTEGER REFERENCES epics(id),
            sub_status  TEXT NOT NULL DEFAULT 'none',
            pr_url      TEXT,
            tag         TEXT,
            sort_order  INTEGER,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        INSERT INTO tasks_new SELECT id, title, description, repo_path, status, worktree, tmux_window, plan, epic_id, sub_status, pr_url, tag, sort_order, created_at, updated_at FROM tasks;
        DROP TABLE tasks;
        ALTER TABLE tasks_new RENAME TO tasks;",
    )
    .context("Failed to rebuild tasks table for sub_status migration")
}

fn migrate_v16_add_status_check_constraint(conn: &Connection) -> Result<()> {
    // Clean up invalid (status, sub_status) pairs so the CHECK constraint can be added.
    // Before this migration, (review, needs_input) rows could exist from old hook behavior.
    let _ = conn.execute_batch(
        "-- Legacy (review, needs_input) from old HookNotification hook → awaiting_review
         UPDATE tasks SET sub_status = 'awaiting_review'
         WHERE status = 'review' AND sub_status = 'needs_input';

         -- Any other invalid running pairs → active
         UPDATE tasks SET sub_status = 'active'
         WHERE status = 'running'
           AND sub_status NOT IN ('active', 'needs_input', 'stale', 'crashed');

         -- Any other invalid review pairs → awaiting_review
         UPDATE tasks SET sub_status = 'awaiting_review'
         WHERE status = 'review'
           AND sub_status NOT IN ('awaiting_review', 'changes_requested', 'approved');

         -- Any other invalid terminal-status pairs → none
         UPDATE tasks SET sub_status = 'none'
         WHERE status IN ('backlog', 'done', 'archived') AND sub_status != 'none';",
    );

    conn.execute_batch(
        "CREATE TABLE tasks_new (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER REFERENCES epics(id),
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
         INSERT INTO tasks_new
             SELECT id, title, description, repo_path, status, worktree, tmux_window, plan,
                    epic_id, sub_status, pr_url, tag, sort_order, created_at, updated_at
             FROM tasks;
         DROP TABLE tasks;
         ALTER TABLE tasks_new RENAME TO tasks;",
    )
    .context("Failed to rebuild tasks table with CHECK constraint")
}

fn migrate_v17_add_conflict_sub_status(conn: &Connection) -> Result<()> {
    // Add 'conflict' as a valid running sub_status. Rebuild table to update the CHECK constraint.
    conn.execute_batch(
        "CREATE TABLE tasks_new (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan        TEXT,
             epic_id     INTEGER REFERENCES epics(id),
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
         INSERT INTO tasks_new
             SELECT id, title, description, repo_path, status, worktree, tmux_window, plan,
                    epic_id, sub_status, pr_url, tag, sort_order, created_at, updated_at
             FROM tasks;
         DROP TABLE tasks;
         ALTER TABLE tasks_new RENAME TO tasks;",
    )
    .context("Failed to rebuild tasks table for migration 17 (add conflict sub_status)")
}

fn migrate_v18_expand_tilde_paths(conn: &Connection) -> Result<()> {
    // Expand ~/... to $HOME/... in all repo_path columns.
    // This prevents filter mismatches between tilde and absolute forms.
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        let prefix = format!("{home}/");
        conn.execute(
            "UPDATE tasks SET repo_path = ?1 || substr(repo_path, 3) WHERE repo_path LIKE '~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in tasks.repo_path")?;
        conn.execute(
            "UPDATE epics SET repo_path = ?1 || substr(repo_path, 3) WHERE repo_path LIKE '~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in epics.repo_path")?;
        conn.execute(
            "UPDATE repo_paths SET path = ?1 || substr(path, 3) WHERE path LIKE '~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in repo_paths.path")?;
        conn.execute(
            "UPDATE filter_presets SET repo_paths = replace(repo_paths, '~/', ?1) WHERE repo_paths LIKE '%~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in filter_presets.repo_paths")?;
        conn.execute(
            "UPDATE settings SET value = replace(value, '~/', ?1) WHERE key = 'repo_filter' AND value LIKE '%~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in settings.repo_filter")?;
    }
    Ok(())
}

fn migrate_v19_add_review_pr_columns(conn: &Connection) -> Result<()> {
    // Fresh DBs already have these from the CREATE TABLE in migration 14,
    // so ignore "duplicate column" errors.
    let _ = conn.execute_batch("ALTER TABLE review_prs ADD COLUMN body TEXT NOT NULL DEFAULT ''");
    let _ =
        conn.execute_batch("ALTER TABLE review_prs ADD COLUMN head_ref TEXT NOT NULL DEFAULT ''");
    let _ = conn
        .execute_batch("ALTER TABLE review_prs ADD COLUMN ci_status TEXT NOT NULL DEFAULT 'none'");
    let _ = conn
        .execute_batch("ALTER TABLE review_prs ADD COLUMN reviewers TEXT NOT NULL DEFAULT '[]'");
    Ok(())
}

fn migrate_v20_epic_status_enum(conn: &Connection) -> Result<()> {
    // Replace epic `done` boolean with `status` enum.
    conn.execute_batch(
        "CREATE TABLE epics_new (
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
         INSERT INTO epics_new (id, title, description, repo_path, status, plan, sort_order, created_at, updated_at)
             SELECT id, title, description, repo_path,
                    CASE WHEN done = 1 THEN 'done' ELSE 'backlog' END,
                    plan, sort_order, created_at, updated_at
             FROM epics;
         DROP TABLE epics;
         ALTER TABLE epics_new RENAME TO epics;",
    )
    .context("Failed to rebuild epics table for migration 20 (status enum)")?;

    // Derive status for non-done epics from their subtasks
    let epics: Vec<(i64, String)> = conn
        .prepare("SELECT id, status FROM epics WHERE status != 'done'")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for (epic_id, _) in epics {
        let statuses: Vec<String> = conn
            .prepare("SELECT status FROM tasks WHERE epic_id = ?1 AND status != 'archived'")?
            .query_map(params![epic_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let new_status = if statuses.is_empty() {
            "backlog"
        } else if statuses.iter().all(|s| s == "done") {
            "done"
        } else if statuses.iter().all(|s| s == "done" || s == "review") {
            "review"
        } else if statuses.iter().any(|s| s == "running") {
            "running"
        } else {
            "backlog"
        };
        conn.execute(
            "UPDATE epics SET status = ?1 WHERE id = ?2",
            params![new_status, epic_id],
        )?;
    }

    Ok(())
}

pub(super) fn migrate_v21_create_my_prs_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS my_prs (
            repo            TEXT    NOT NULL,
            number          INTEGER NOT NULL,
            title           TEXT    NOT NULL,
            author          TEXT    NOT NULL,
            url             TEXT    NOT NULL,
            is_draft        INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL,
            updated_at      TEXT    NOT NULL,
            additions       INTEGER NOT NULL DEFAULT 0,
            deletions       INTEGER NOT NULL DEFAULT 0,
            review_decision TEXT    NOT NULL DEFAULT 'ReviewRequired',
            labels          TEXT    NOT NULL DEFAULT '[]',
            body            TEXT    NOT NULL DEFAULT '',
            head_ref        TEXT    NOT NULL DEFAULT '',
            ci_status       TEXT    NOT NULL DEFAULT 'None',
            reviewers       TEXT    NOT NULL DEFAULT '[]',
            PRIMARY KEY (repo, number)
        )",
    )
    .context("Failed to create my_prs table")
}

fn migrate_v22_add_filter_preset_mode(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE filter_presets ADD COLUMN mode TEXT NOT NULL DEFAULT 'include'")
        .context("Failed to add mode column to filter_presets")
}

pub(super) fn migrate_v23_create_bot_prs_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bot_prs (
            repo            TEXT    NOT NULL,
            number          INTEGER NOT NULL,
            title           TEXT    NOT NULL,
            author          TEXT    NOT NULL,
            url             TEXT    NOT NULL,
            is_draft        INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL,
            updated_at      TEXT    NOT NULL,
            additions       INTEGER NOT NULL DEFAULT 0,
            deletions       INTEGER NOT NULL DEFAULT 0,
            review_decision TEXT    NOT NULL DEFAULT 'ReviewRequired',
            labels          TEXT    NOT NULL DEFAULT '[]',
            body            TEXT    NOT NULL DEFAULT '',
            head_ref        TEXT    NOT NULL DEFAULT '',
            ci_status       TEXT    NOT NULL DEFAULT 'None',
            reviewers       TEXT    NOT NULL DEFAULT '[]',
            PRIMARY KEY (repo, number)
        )",
    )
    .context("Failed to create bot_prs table")
}

fn migrate_v25_rename_plan_to_plan_path(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE tasks RENAME COLUMN plan TO plan_path")
        .context("Failed to rename tasks.plan to plan_path")?;
    conn.execute_batch("ALTER TABLE epics RENAME COLUMN plan TO plan_path")
        .context("Failed to rename epics.plan to plan_path")?;
    Ok(())
}

pub(super) fn migrate_v26_add_agent_columns(conn: &Connection) -> Result<()> {
    for table in &["review_prs", "my_prs", "bot_prs", "security_alerts"] {
        if let Err(e) =
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN tmux_window TEXT"))
        {
            tracing::debug!("ALTER {table} ADD tmux_window (may already exist): {e}");
        }
        if let Err(e) = conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN worktree TEXT"))
        {
            tracing::debug!("ALTER {table} ADD worktree (may already exist): {e}");
        }
    }
    Ok(())
}

fn migrate_v27_add_agent_status(conn: &Connection) -> Result<()> {
    for table in &["review_prs", "bot_prs", "security_alerts"] {
        if let Err(e) =
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN agent_status TEXT"))
        {
            tracing::debug!("ALTER {table} ADD agent_status (may already exist): {e}");
        }
    }
    Ok(())
}

fn migrate_v30_allow_conflict_for_review(conn: &Connection) -> Result<()> {
    // Allow 'conflict' sub_status for review tasks (rebase conflicts during wrap_up/finish).
    conn.execute_batch(
        "CREATE TABLE tasks_new (
             id          INTEGER PRIMARY KEY,
             title       TEXT NOT NULL,
             description TEXT NOT NULL,
             repo_path   TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'backlog',
             worktree    TEXT,
             tmux_window TEXT,
             plan_path   TEXT,
             epic_id     INTEGER REFERENCES epics(id),
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
         INSERT INTO tasks_new
             SELECT id, title, description, repo_path, status, worktree, tmux_window, plan_path,
                    epic_id, sub_status, pr_url, tag, sort_order, created_at, updated_at
             FROM tasks;
         DROP TABLE tasks;
         ALTER TABLE tasks_new RENAME TO tasks;",
    )
    .context("Failed to rebuild tasks table for migration 30 (allow conflict for review)")
}

pub(super) fn migrate_v24_create_security_alerts_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS security_alerts (
            repo              TEXT    NOT NULL,
            number            INTEGER NOT NULL,
            kind              TEXT    NOT NULL,
            severity          TEXT    NOT NULL,
            title             TEXT    NOT NULL,
            package           TEXT,
            vulnerable_range  TEXT,
            fixed_version     TEXT,
            cvss_score        REAL,
            url               TEXT    NOT NULL,
            created_at        TEXT    NOT NULL,
            state             TEXT    NOT NULL,
            description       TEXT    NOT NULL DEFAULT '',
            PRIMARY KEY (repo, number, kind)
        )",
    )
    .context("Failed to create security_alerts table")
}

fn migrate_v28_add_my_prs_agent_status(conn: &Connection) -> Result<()> {
    // v27 missed my_prs when adding agent_status. Fix the gap.
    if let Err(e) = conn.execute_batch("ALTER TABLE my_prs ADD COLUMN agent_status TEXT") {
        tracing::debug!("ALTER my_prs ADD agent_status (may already exist): {e}");
    }
    Ok(())
}

fn migrate_v31_re_expand_tilde_paths(conn: &Connection) -> Result<()> {
    // Re-expand ~/... to $HOME/... in all path columns.
    // Migration v18 did this once, but paths saved between v18 and the
    // expand_tilde-on-write fix (commit fd26d80) may still contain tildes.
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        let prefix = format!("{home}/");

        // Simple text columns: tasks.repo_path, epics.repo_path, repo_paths.path
        conn.execute(
            "UPDATE tasks SET repo_path = ?1 || substr(repo_path, 3) WHERE repo_path LIKE '~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in tasks.repo_path")?;
        conn.execute(
            "UPDATE epics SET repo_path = ?1 || substr(repo_path, 3) WHERE repo_path LIKE '~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in epics.repo_path")?;
        conn.execute(
            "UPDATE repo_paths SET path = ?1 || substr(path, 3) WHERE path LIKE '~/%'",
            params![prefix],
        )
        .context("Failed to expand ~ in repo_paths.path")?;

        // JSON array columns (post v29): filter_presets.repo_paths, settings.repo_filter
        let presets: Vec<(String, String)> = conn
            .prepare("SELECT name, repo_paths FROM filter_presets WHERE repo_paths LIKE '%~/%'")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for (name, raw) in presets {
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(&raw) {
                let expanded: Vec<String> = paths
                    .into_iter()
                    .map(|p| {
                        if let Some(rest) = p.strip_prefix("~/") {
                            format!("{prefix}{rest}")
                        } else {
                            p
                        }
                    })
                    .collect();
                let json = serde_json::to_string(&expanded).unwrap_or(raw);
                conn.execute(
                    "UPDATE filter_presets SET repo_paths = ?1 WHERE name = ?2",
                    params![json, name],
                )?;
            }
        }

        let filter: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'repo_filter' AND value LIKE '%~/%'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(raw) = filter {
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(&raw) {
                let expanded: Vec<String> = paths
                    .into_iter()
                    .map(|p| {
                        if let Some(rest) = p.strip_prefix("~/") {
                            format!("{prefix}{rest}")
                        } else {
                            p
                        }
                    })
                    .collect();
                let json = serde_json::to_string(&expanded).unwrap_or(raw);
                conn.execute(
                    "UPDATE settings SET value = ?1 WHERE key = 'repo_filter'",
                    params![json],
                )?;
            }
        }
    }
    Ok(())
}

fn migrate_v32_add_base_branch(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE tasks ADD COLUMN base_branch TEXT NOT NULL DEFAULT 'main';")
        .context("Failed to add base_branch column to tasks")
}

fn migrate_v29_json_filter_presets(conn: &Connection) -> Result<()> {
    // Convert filter_presets.repo_paths from newline-delimited to JSON arrays.
    let rows: Vec<(String, String)> = conn
        .prepare("SELECT name, repo_paths FROM filter_presets")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for (name, raw) in rows {
        // Skip values that are already valid JSON arrays
        if raw.starts_with('[') {
            continue;
        }
        let paths: Vec<&str> = raw.split('\n').filter(|s| !s.is_empty()).collect();
        let json = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "UPDATE filter_presets SET repo_paths = ?1 WHERE name = ?2",
            params![json, name],
        )?;
    }

    // Convert settings.repo_filter from newline-delimited to JSON array.
    let filter: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'repo_filter'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(raw) = filter {
        if !raw.starts_with('[') {
            let paths: Vec<&str> = raw.split('\n').filter(|s| !s.is_empty()).collect();
            let json = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_string());
            conn.execute(
                "UPDATE settings SET value = ?1 WHERE key = 'repo_filter'",
                params![json],
            )?;
        }
    }

    Ok(())
}

pub(super) fn migrate_v33_add_auto_dispatch(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE epics ADD COLUMN auto_dispatch BOOLEAN NOT NULL DEFAULT 1;")
        .context("Failed to add auto_dispatch column to epics")
}

fn migrate_v34_add_parent_epic_id(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE epics ADD COLUMN parent_epic_id INTEGER REFERENCES epics(id);")
        .context("Failed to add parent_epic_id column to epics")
}

fn migrate_v35_add_self_ref_check(conn: &Connection) -> Result<()> {
    // SQLite does not support ADD CONSTRAINT, so we rebuild the epics table to
    // add CHECK (parent_epic_id != id), which prevents a row from being its
    // own parent. This is defense-in-depth alongside the visited-set guard in
    // recalculate_epic_status_inner.
    //
    // Column order matches the post-v34 layout produced by ALTER TABLE additions:
    //   id, title, description, repo_path, status, plan_path, sort_order,
    //   created_at, updated_at, auto_dispatch, parent_epic_id
    conn.execute_batch(
        "CREATE TABLE epics_new (
             id             INTEGER PRIMARY KEY,
             title          TEXT NOT NULL,
             description    TEXT NOT NULL,
             repo_path      TEXT NOT NULL,
             status         TEXT NOT NULL DEFAULT 'backlog',
             plan_path      TEXT,
             sort_order     INTEGER,
             created_at     TEXT NOT NULL DEFAULT (datetime('now')),
             updated_at     TEXT NOT NULL DEFAULT (datetime('now')),
             auto_dispatch  BOOLEAN NOT NULL DEFAULT 1,
             parent_epic_id INTEGER REFERENCES epics_new(id),
             CHECK (parent_epic_id != id)
         );
         INSERT INTO epics_new (
             id, title, description, repo_path, status, plan_path, sort_order,
             created_at, updated_at, auto_dispatch, parent_epic_id
         )
         SELECT
             id, title, description, repo_path, status, plan_path, sort_order,
             created_at, updated_at, auto_dispatch, parent_epic_id
         FROM epics;
         DROP TABLE epics;
         ALTER TABLE epics_new RENAME TO epics;",
    )
    .context("Failed to rebuild epics table for migration v35 (self-ref CHECK)")
}

fn migrate_v37_pr_workflow_states(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE pr_workflow_states (
            repo       TEXT    NOT NULL,
            number     INTEGER NOT NULL,
            kind       TEXT    NOT NULL,
            state      TEXT    NOT NULL DEFAULT 'backlog',
            sub_state  TEXT,
            updated_at TEXT    NOT NULL,
            PRIMARY KEY (repo, number, kind)
        );",
    )
    .context("Failed to create pr_workflow_states table (migration v37)")
}

fn migrate_v38_feed_epic_columns(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE epics ADD COLUMN feed_command TEXT;
         ALTER TABLE epics ADD COLUMN feed_interval_secs INTEGER;
         ALTER TABLE tasks ADD COLUMN external_id TEXT;
         CREATE UNIQUE INDEX IF NOT EXISTS tasks_epic_external_id
             ON tasks (epic_id, external_id)
             WHERE external_id IS NOT NULL;",
    )
    .context("Failed to add feed columns (migration v38)")
}

fn migrate_v39_add_projects(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            is_default INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO projects (name, sort_order, is_default) VALUES ('Default', 0, 1);
        ALTER TABLE tasks ADD COLUMN project_id INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE epics ADD COLUMN project_id INTEGER NOT NULL DEFAULT 1;
        CREATE INDEX idx_tasks_project_id ON tasks(project_id);
        CREATE INDEX idx_epics_project_id ON epics(project_id);",
    )
    .context("Failed to add projects table (migration v39)")
}

pub(super) fn migrate_v36_tips_state(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tips_state (
            id         INTEGER PRIMARY KEY DEFAULT 1,
            seen_up_to INTEGER NOT NULL DEFAULT 0,
            show_mode  TEXT    NOT NULL DEFAULT 'always',
            CHECK (id = 1)
        );
        INSERT OR IGNORE INTO tips_state (id, seen_up_to, show_mode)
        VALUES (1, 0, 'always');",
    )
    .context("Failed to create tips_state table (migration v36)")
}

fn migrate_v40_create_learnings(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS learnings (
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
        CREATE INDEX IF NOT EXISTS idx_learnings_status ON learnings(status);",
    )
    .context("Failed to create learnings table (migration v40)")
}

fn migrate_v41_drop_cost_usd(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "task_usage", "cost_usd") {
        return Ok(());
    }
    conn.execute_batch(
        "CREATE TABLE task_usage_new (
            task_id            INTEGER NOT NULL PRIMARY KEY REFERENCES tasks(id),
            input_tokens       INTEGER NOT NULL DEFAULT 0,
            output_tokens      INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            updated_at         TEXT NOT NULL DEFAULT ''
        );
        INSERT INTO task_usage_new
            SELECT task_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, updated_at
            FROM task_usage;
        DROP TABLE task_usage;
        ALTER TABLE task_usage_new RENAME TO task_usage;",
    )
    .context("Failed to drop cost_usd from task_usage (migration v41)")
}

pub(super) fn migrate_v73_needs_review_to_approved(conn: &Connection) -> Result<()> {
    // The needs_review learning status was removed (human-approval gate dropped).
    // Convert any surviving rows back to 'approved' so LearningStatus::parse,
    // which now hard-errors on the unknown 'needs_review' string (storage-boundary
    // policy in core.allium), never encounters one.
    //
    // Skip gracefully if the learnings table doesn't exist (minimal-schema
    // migration tests that never ran v40).
    if !table_exists(conn, "learnings") {
        return Ok(());
    }
    let promoted = conn
        .execute(
            "UPDATE learnings SET status = 'approved', updated_at = datetime('now') WHERE status = 'needs_review'",
            [],
        )
        .context("Failed to convert needs_review learnings to approved (migration v73)")?;
    if promoted > 0 {
        tracing::info!("Migration v73: promoted {promoted} needs_review learning(s) to approved");
    }
    Ok(())
}

pub(super) fn migrate_v74_drop_learning_verdicts(conn: &Connection) -> Result<()> {
    // Learning verdicts are no longer persisted: rate_learning applies only the
    // in-flight score effect (helped → +1, wrong → -1). Drop the vestigial
    // learning_verdicts table (and its index) that apply_verdicts_tx used to
    // write to. Guarded so minimal-schema migration tests that never created it
    // still pass. See docs/specs/learnings.allium (Retrievals & Verdicts).
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_lv_learning;
         DROP TABLE IF EXISTS learning_verdicts;",
    )
    .context("Failed to drop learning_verdicts table (migration v74)")?;
    Ok(())
}

/// Create the `repo_base_branches` table: per-repo base_branch history (see
/// docs/specs/dispatch.allium: rule RecordBaseBranch, surface BaseBranchPicker;
/// docs/specs/core.allium: entity SavedRepoBranch). Identity is the
/// `(repo_path, branch)` pair, enforced by a UNIQUE constraint so the
/// production upsert can `ON CONFLICT(repo_path, branch)`.
pub(super) fn migrate_v75_create_repo_base_branches(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS repo_base_branches (
            id        INTEGER PRIMARY KEY,
            repo_path TEXT NOT NULL,
            branch    TEXT NOT NULL,
            last_used TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(repo_path, branch)
        );",
    )
    .context("Failed to create repo_base_branches table (migration v75)")?;
    Ok(())
}

/// Creates `task_watchers`: one-shot subscriptions where `watcher_task_id`
/// wants to be notified when `target_task_id` reaches `Done`/`Archived`, or
/// is deleted before finishing. See `docs/specs/task-watchers.allium`.
pub(super) fn migrate_v78_create_task_watchers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_watchers (
            id              INTEGER PRIMARY KEY,
            watcher_task_id INTEGER NOT NULL,
            target_task_id  INTEGER NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(watcher_task_id, target_task_id)
        );",
    )
    .context("v78: failed to create task_watchers table")?;
    Ok(())
}

/// Backfills `sort_order` for tasks and epics already sitting in `Done`
/// status, using their existing `updated_at` as an approximation of
/// completion time (no real completion timestamp exists for historical
/// data — see the Done-column completion-order design doc). Only fills in
/// `NULL` values; never overwrites an already-set `sort_order` (e.g. one
/// set by a prior manual reorder). This backfill is deliberately
/// seconds-scale, matching `updated_at`'s storage precision, while live
/// transitions at the time used millisecond precision — see the design doc
/// for why mixing scales there is correct, not a bug.
///
/// **Superseded by [`migrate_v99_add_completed_at`]**, which moves every rank
/// this wrote out of `sort_order` and into `completed_at`. Kept because a
/// database that has not reached v99 yet still runs this one on the way.
pub(super) fn migrate_v79_backfill_done_sort_order(conn: &Connection) -> Result<()> {
    let tasks_updated = conn
        .execute(
            "UPDATE tasks SET sort_order = -CAST(strftime('%s', updated_at) AS INTEGER)
             WHERE status = 'done' AND sort_order IS NULL",
            [],
        )
        .context("Failed to backfill sort_order for Done tasks (migration v79)")?;
    if tasks_updated > 0 {
        tracing::info!("Migration v79: backfilled sort_order for {tasks_updated} Done task(s)");
    }

    let epics_updated = conn
        .execute(
            "UPDATE epics SET sort_order = -CAST(strftime('%s', updated_at) AS INTEGER)
             WHERE status = 'done' AND sort_order IS NULL",
            [],
        )
        .context("Failed to backfill sort_order for Done epics (migration v79)")?;
    if epics_updated > 0 {
        tracing::info!("Migration v79: backfilled sort_order for {epics_updated} Done epic(s)");
    }

    Ok(())
}

/// Give Done its own completion timestamp instead of overloading `sort_order`.
///
/// Adds a nullable `completed_at` TEXT column to `tasks` and `epics`, then
/// moves the completion-recency rank that v79 (and every live transition since)
/// wrote into `sort_order` across to it.
///
/// The rank was the *negated* Unix time in milliseconds, so a done row whose
/// `sort_order` is negative is carrying one: `completed_at` is
/// `-sort_order` milliseconds since the epoch, and the `sort_order` is then
/// cleared back to NULL so the field means manual/feed ordering and nothing
/// else. A *positive* `sort_order` on a done row is real manual or feed
/// ordering — the very case the Done column could not tell apart, and the
/// reason this split exists — so it is left exactly as it is.
///
/// Millisecond precision is preserved on the way across: the rank was written
/// with it, and `format_datetime_millis` reads it back. v79's own backfill was
/// seconds-scale (`-strftime('%s', updated_at)`), which divides evenly by 1000
/// and lands on a whole second here, exactly as it should.
///
/// A second pass then dates every done row the first one left undated, from
/// `updated_at` — the same approximation v79 used, and for the same reason: no
/// real completion timestamp exists for historical data. Without it the
/// residual set would not be empty, and it would be exactly the rows this
/// split exists to rescue: a done task whose rank a feed's positive
/// `sort_order` had overwritten. Such a card would render at the bottom of
/// Done for good and be permanently refused a manual reorder, because there is
/// nothing left to swap and no route back short of re-entering done. A
/// seconds-scale approximation is a worse answer than the truth and a far
/// better one than that.
pub(super) fn migrate_v99_add_completed_at(conn: &Connection) -> Result<()> {
    for table in ["tasks", "epics"] {
        if !column_exists(conn, table, "completed_at") {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN completed_at TEXT"))
                .with_context(|| format!("v99: add completed_at column to {table}"))?;
        }
        // strftime('%f') yields "SS.SSS", so the format string below produces
        // the same "YYYY-MM-DD HH:MM:SS.sss" shape `format_datetime_millis`
        // writes and `parse_datetime` reads.
        let moved = conn
            .execute(
                &format!(
                    "UPDATE {table} SET
                         completed_at = strftime('%Y-%m-%d %H:%M:%f', -sort_order / 1000.0, 'unixepoch'),
                         sort_order = NULL
                     WHERE status = 'done' AND sort_order IS NOT NULL AND sort_order < 0"
                ),
                [],
            )
            .with_context(|| {
                format!("v99: move the completion rank from {table}.sort_order to completed_at")
            })?;
        if moved > 0 {
            tracing::info!("Migration v99: moved {moved} completion rank(s) in {table}");
        }

        let dated = conn
            .execute(
                &format!(
                    "UPDATE {table} SET completed_at = updated_at
                     WHERE status = 'done' AND completed_at IS NULL"
                ),
                [],
            )
            .with_context(|| {
                format!("v99: date the remaining done {table} rows from updated_at")
            })?;
        if dated > 0 {
            tracing::info!("Migration v99: dated {dated} undated done row(s) in {table}");
        }
    }
    Ok(())
}

/// Drop the orphaned `agent_status` column added by v27 (`review_prs`,
/// `bot_prs`, `security_alerts`) and v28 (`my_prs`). It was residue of the
/// removed `ReviewAgentStatus` feature — no production code has read or
/// written it since. v27/v28 stay untouched: this registry is append-only
/// (see the module header) and both have already run against real databases.
///
/// Tolerant of a missing table as well as a missing column: which of these
/// tables a given database has depends on how far its migration history got,
/// and `column_exists` on an absent table simply returns false. `tasks` is in
/// the list for the same reason — no registered migration adds `agent_status`
/// there, but the pre-v42 schema replica in `src/db/tests/migrations.rs` shows
/// early hand-built databases carried it, and the guard makes covering that a
/// free no-op everywhere else.
pub(super) fn migrate_v80_drop_agent_status(conn: &Connection) -> Result<()> {
    for table in &[
        "review_prs",
        "bot_prs",
        "security_alerts",
        "my_prs",
        "tasks",
    ] {
        if column_exists(conn, table, "agent_status") {
            conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN agent_status"))
                .with_context(|| {
                    format!("Failed to drop agent_status from {table} (migration v80)")
                })?;
        }
    }
    Ok(())
}

/// Creates `task_subagents` (one row per live subagent, keyed by
/// `(task_id, agent_id)`) and adds `tasks.live_subagents` /
/// `tasks.stop_pending`. See `SubagentEntry` in `docs/specs/core.allium` and
/// the `live_subagents` / `stop_pending` fields in `docs/specs/agent-health.allium`.
pub(super) fn migrate_v81_create_task_subagents(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_subagents (
             task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
             agent_id   TEXT    NOT NULL,
             session_id TEXT    NOT NULL,
             started_at TEXT    NOT NULL,
             PRIMARY KEY (task_id, agent_id)
         );
         CREATE INDEX IF NOT EXISTS idx_task_subagents_task
             ON task_subagents(task_id);",
    )
    .context("Failed to create task_subagents (migration v81)")?;

    if !column_exists(conn, "tasks", "live_subagents") {
        conn.execute_batch(
            "ALTER TABLE tasks ADD COLUMN live_subagents INTEGER NOT NULL DEFAULT 0",
        )
        .context("Failed to add tasks.live_subagents (migration v81)")?;
    }
    if !column_exists(conn, "tasks", "stop_pending") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN stop_pending INTEGER NOT NULL DEFAULT 0")
            .context("Failed to add tasks.stop_pending (migration v81)")?;
    }
    Ok(())
}

/// Resolve tasks left stranded by the pre-conditional-write `Stop` handling.
///
/// Before the `Stop` hook and the subagent-drain became single conditional
/// writes, each read the task, decided, then wrote. Across two `dispatch`
/// processes those could interleave so `stop_pending` was set *after* the count
/// had already reached zero, leaving `Running` + `stop_pending` +
/// `live_subagents = 0`: nothing left to drain it, `Stop` does not re-fire, and
/// `PreToolUse` never touches status. A tick reconciler used to sweep those up
/// every 2s; it has been retired because the state is now unreachable.
///
/// Unreachable going forward, but not retroactive — a database written by the
/// older code can still hold such a row, and with the reconciler gone nothing
/// else would ever resolve it. This applies the withheld flip once.
///
/// The statement below spells out the same predicate and SET list as
/// `apply_pending_stop_if_drained` (`src/db/queries/subagents.rs`) but must
/// **not** be refactored to share `STOP_FLIP_SET` with it. A migration is a
/// frozen historical record — see the "we do not squash migrations" note in
/// this module's header — so binding it to a shared constant would let a later
/// edit retroactively change what v82 did on databases that already ran it.
/// Duplication is the correct choice here.
///
/// Epic status is intentionally not recalculated here: it only changes when
/// *every* child is Done (`recalculate_epic_status_inner`), so moving a task
/// from Running to Review cannot alter it.
pub(super) fn migrate_v82_resolve_stranded_pending_stops(conn: &Connection) -> Result<()> {
    // This is a data fix-up, not a schema change, so it is safe to skip when
    // the columns it touches are not all present. The migration tests build
    // synthetic partial `tasks` tables from various eras — v81 adds
    // `stop_pending`/`live_subagents` to whatever table exists, which can leave
    // those present while the hook timestamps are not.
    for column in [
        "status",
        "sub_status",
        "stop_pending",
        "live_subagents",
        "last_pre_tool_use_at",
        "last_notification_at",
    ] {
        if !column_exists(conn, "tasks", column) {
            return Ok(());
        }
    }
    let resolved = conn
        .execute(
            "UPDATE tasks \
             SET status = ?1, sub_status = ?2, \
                 last_pre_tool_use_at = NULL, last_notification_at = NULL, \
                 stop_pending = 0, updated_at = datetime('now') \
             WHERE status = ?3 AND stop_pending = 1 AND live_subagents = 0",
            rusqlite::params![
                TaskStatus::Review.as_str(),
                SubStatus::default_for(TaskStatus::Review).as_str(),
                TaskStatus::Running.as_str(),
            ],
        )
        .context("Failed to resolve stranded pending stops (migration v82)")?;
    if resolved > 0 {
        tracing::info!(
            count = resolved,
            "migration v82: applied deferred stops that had no subagent left to drain them"
        );
    }
    Ok(())
}

/// Adds `tasks.stop_pending_at`: when the `Stop` that `stop_pending` defers
/// actually fired.
///
/// `record_user_prompt_submit` (`src/db/queries/tasks.rs`) needs it to tell a
/// Stop deferred by the previous turn — which a human resuming voids — from one
/// deferred by the turn the arriving prompt itself started, whose write can land
/// first because every hook is a separate process. Comparing *event* times is
/// what makes that sound; anything derived from write order inherits the race.
///
/// Additive only, with no backfill. A row already carrying `stop_pending` gets a
/// null, which reads as "the Stop fired before any prompt" — exactly the
/// unconditional clear those rows were written under, so they keep their old
/// behaviour. See the `stop_pending_at` field in `docs/specs/core.allium` and
/// `HookUserPromptSubmit` in `docs/specs/agent-health.allium`.
pub(super) fn migrate_v83_add_stop_pending_at(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "tasks", "stop_pending_at") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN stop_pending_at TEXT")
            .context("Failed to add tasks.stop_pending_at (migration v83)")?;
    }
    Ok(())
}

/// Drops the singleton `tips_state` table created by v36. The startup tips
/// popup was removed outright, so nothing reads or writes the watermark or the
/// show mode any more.
///
/// v36 itself is deliberately left in place: it has run against production
/// databases, and a database stamped below 36 still replays it on the way here.
/// The pair creating-then-dropping across the replay is the normal cost of the
/// append-only policy in this module's header — the alternative, editing v36,
/// would rewrite recorded schema history.
pub(super) fn migrate_v84_drop_tips_state(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS tips_state")
        .context("Failed to drop tips_state (migration v84)")
}

/// Tracks live backgrounded shells (Bash tool with `run_in_background: true`)
/// per task, mirroring `task_subagents`/`live_subagents` (migration v81). See
/// docs/superpowers/specs/2026-08-15-shell-visibility-design.md.
pub(super) fn migrate_v85_create_task_shells(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_shells (
             task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
             shell_id   TEXT    NOT NULL,
             session_id TEXT    NOT NULL,
             started_at TEXT    NOT NULL,
             PRIMARY KEY (task_id, shell_id)
         );
         CREATE INDEX IF NOT EXISTS idx_task_shells_task
             ON task_shells(task_id);",
    )
    .context("Failed to create task_shells (migration v85)")?;

    if !column_exists(conn, "tasks", "live_shells") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN live_shells INTEGER NOT NULL DEFAULT 0")
            .context("Failed to add tasks.live_shells (migration v85)")?;
    }
    if !column_exists(conn, "tasks", "oldest_live_shell_started_at") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN oldest_live_shell_started_at TEXT")
            .context("Failed to add tasks.oldest_live_shell_started_at (migration v85)")?;
    }
    Ok(())
}

/// Rebuilds the `tasks` table with a new `(status, sub_status)` CHECK
/// constraint clause, preserving every column (types, defaults, the
/// `epic_id` FK) and every index/trigger. SQLite cannot alter an existing
/// CHECK constraint in place, so widening it for a newly-valid sub_status
/// (`migrate_v30_allow_conflict_for_review`, `migrate_v86_allow_stale_shell`,
/// `migrate_v89_allow_pr_closed_for_review`, ...) means rebuilding the whole
/// table. Shared by all such migrations so the rebuild mechanics — most
/// importantly the runtime column/index/trigger introspection below — live
/// in exactly one place.
///
/// The column list, types, defaults, and the indexes/triggers to recreate
/// are all discovered at runtime via `pragma_table_info`/`sqlite_master`,
/// rather than hardcoded. A hardcoded full column list broke several
/// migration tests that deliberately replay only a partial migration
/// history against a synthetic seed schema (e.g. `migration_v38_feed_epic_columns`,
/// `migration_v52_adds_verify_command_to_repo_paths`) — those tables never
/// had `worktree`/`tag`/`url`/etc. added, since those columns' migrations
/// predate the synthetic seed's starting point. Reading the actual current
/// columns makes this correct against whatever subset of the schema
/// actually exists, real or synthetic.
///
/// `check_clause` is the full `CHECK (...)` body (including the `CHECK (`
/// and closing `)`); `label` names the calling migration for error context
/// (e.g. `"v89"`).
fn rebuild_tasks_table_with_check(
    conn: &Connection,
    check_clause: &str,
    label: &str,
) -> Result<()> {
    rebuild_table_with_check(
        conn,
        "tasks",
        Some(("epic_id", "epics")),
        check_clause,
        label,
    )
}

/// Shared rebuild mechanics for [`rebuild_tasks_table_with_check`] and
/// [`rebuild_epics_table_with_check`]: introspects `table_name`'s current
/// columns/indexes/triggers via `pragma_table_info`/`sqlite_master` (rather
/// than a hardcoded column list — see the rationale on the callers' own doc
/// comments) and rebuilds it with `check_clause` as its only `CHECK`.
/// `fk_column`, when present, is `(column_name, referenced_table)` for the
/// one self- or cross-referencing FK the table carries; `pragma_table_info`
/// doesn't report FK constraints, so it's reattached by name rather than
/// introspected generically. `check_clause` is the full `CHECK (...)` body
/// (including the `CHECK (` and closing `)`); `label` names the calling
/// migration for error context (e.g. `"v89"`).
fn rebuild_table_with_check(
    conn: &Connection,
    table_name: &str,
    fk_column: Option<(&str, &str)>,
    check_clause: &str,
    label: &str,
) -> Result<()> {
    struct ColumnDef {
        name: String,
        decl_type: String,
        notnull: bool,
        dflt_value: Option<String>,
        pk: bool,
    }

    let mut columns = Vec::new();
    {
        let mut stmt = conn.prepare(&format!(
            "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info('{table_name}')"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok(ColumnDef {
                name: r.get(0)?,
                decl_type: r.get(1)?,
                notnull: r.get::<_, i64>(2)? != 0,
                dflt_value: r.get(3)?,
                pk: r.get::<_, i64>(4)? != 0,
            })
        })?;
        for row in rows {
            columns.push(row?);
        }
    }

    let column_defs: Vec<String> = columns
        .iter()
        .map(|c| {
            let mut def = format!("{} {}", c.name, c.decl_type);
            if c.pk {
                def.push_str(" PRIMARY KEY");
            } else if c.notnull {
                def.push_str(" NOT NULL");
            }
            if let Some(dflt) = &c.dflt_value {
                // Always parenthesized: `pragma_table_info` strips the outer
                // parens SQLite requires around a non-constant default
                // expression (e.g. `datetime('now')`), so re-adding them
                // unconditionally is the only way to get both literals and
                // expressions to splice back in as valid syntax.
                def.push_str(&format!(" DEFAULT ({dflt})"));
            }
            if let Some((fk_col, fk_table)) = fk_column {
                if c.name == fk_col {
                    def.push_str(&format!(" REFERENCES {fk_table}(id)"));
                }
            }
            def
        })
        .collect();
    let column_list = columns
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    // Existing indexes/triggers on the table, captured verbatim so they can
    // be replayed after the rebuild — `DROP TABLE` implicitly drops
    // everything attached to it, and a synthetic/partial schema may not have
    // all of today's indexes/triggers, so replay only what actually existed.
    let mut extra_sql = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT sql FROM sqlite_master \
             WHERE tbl_name = ?1 AND type IN ('index', 'trigger') AND sql IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![table_name], |r| r.get::<_, String>(0))?;
        for row in rows {
            extra_sql.push(row?);
        }
    }

    let new_table = format!("{table_name}_new");
    conn.execute_batch(&format!(
        "CREATE TABLE {new_table} (\n    {},\n    {}\n);",
        column_defs.join(",\n    "),
        check_clause
    ))
    .with_context(|| format!("Failed to create {new_table} (migration {label})"))?;

    conn.execute_batch(&format!(
        "INSERT INTO {new_table} ({column_list}) SELECT {column_list} FROM {table_name};"
    ))
    .with_context(|| {
        format!("Failed to copy {table_name} rows into {new_table} (migration {label})")
    })?;

    conn.execute_batch(&format!(
        "DROP TABLE {table_name}; ALTER TABLE {new_table} RENAME TO {table_name};"
    ))
    .with_context(|| format!("Failed to swap {new_table} into {table_name} (migration {label})"))?;

    for sql in &extra_sql {
        conn.execute_batch(sql).with_context(|| {
            format!("Failed to recreate a {table_name} index/trigger (migration {label}): {sql}")
        })?;
    }
    Ok(())
}

/// `epics`' twin of [`rebuild_tasks_table_with_check`]: introspects the
/// table's current columns/indexes/triggers and rebuilds it with
/// `check_clause` as its only `CHECK`. `epics` has never had a status-value
/// `CHECK` (unlike `tasks`, which has enforced `(status, sub_status)` since
/// v16) — `migrate_v106_archived_status_migration` is what adds one for the
/// first time, so `check_clause` is expected to fully replace whatever
/// constraints the table had (in practice just the v35 self-ref `CHECK
/// (parent_epic_id != id)`, which callers must include in `check_clause`
/// themselves if they want it kept).
fn rebuild_epics_table_with_check(
    conn: &Connection,
    check_clause: &str,
    label: &str,
) -> Result<()> {
    rebuild_table_with_check(
        conn,
        "epics",
        Some(("parent_epic_id", "epics")),
        check_clause,
        label,
    )
}

/// Adds `'stale_shell'` to the tasks table's `(status, sub_status)` CHECK
/// constraint's `running` branch, so that sub-status could actually be
/// persisted. See `rebuild_tasks_table_with_check` for the rebuild
/// mechanics. #4965 later removed the enum variant this let through; the
/// string stays legal in the CHECK for old rows, but nothing writes it
/// anymore.
pub(super) fn migrate_v86_allow_stale_shell(conn: &Connection) -> Result<()> {
    rebuild_tasks_table_with_check(
        conn,
        "CHECK (\n        \
             (status = 'backlog'  AND sub_status = 'none') OR\n        \
             (status = 'running'  AND sub_status IN ('active','needs_input','stale','stale_shell','crashed','conflict')) OR\n        \
             (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved','conflict')) OR\n        \
             (status = 'done'     AND sub_status = 'none') OR\n        \
             (status = 'archived' AND sub_status = 'none')\n    )",
        "v86",
    )
}

/// Adds `'pr_closed'` to the tasks table's `(status, sub_status)` CHECK
/// constraint's `review` branch, so `SubStatus::PrClosed` can actually be
/// persisted (task #4382: a closed-without-merge PR now flags the task with
/// `pr_closed` instead of moving it to `done`). See
/// `rebuild_tasks_table_with_check` for the rebuild mechanics.
pub(super) fn migrate_v89_allow_pr_closed_for_review(conn: &Connection) -> Result<()> {
    rebuild_tasks_table_with_check(
        conn,
        "CHECK (\n        \
             (status = 'backlog'  AND sub_status = 'none') OR\n        \
             (status = 'running'  AND sub_status IN ('active','needs_input','stale','stale_shell','crashed','conflict')) OR\n        \
             (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved','conflict','pr_closed')) OR\n        \
             (status = 'done'     AND sub_status = 'none') OR\n        \
             (status = 'archived' AND sub_status = 'none')\n    )",
        "v89",
    )
}

/// Adds `'pr_unreachable'` to the tasks table's `(status, sub_status)` CHECK
/// constraint's `review` branch, so `SubStatus::PrUnreachable` can actually be
/// persisted. Mirrors `migrate_v89_allow_pr_closed_for_review` exactly; see
/// `rebuild_tasks_table_with_check` for the rebuild mechanics.
///
/// The value is set when PR polling gives up on a task after repeated
/// permanent failures, so the board can say the PR is unreadable instead of
/// showing a review decision dispatch has not been able to refresh
/// (pr-workflow.allium: PrPollGaveUp).
pub(super) fn migrate_v96_allow_pr_unreachable_for_review(conn: &Connection) -> Result<()> {
    rebuild_tasks_table_with_check(
        conn,
        "CHECK (\n        \
             (status = 'backlog'  AND sub_status = 'none') OR\n        \
             (status = 'running'  AND sub_status IN ('active','needs_input','stale','stale_shell','crashed','conflict')) OR\n        \
             (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved','conflict','pr_closed','pr_unreachable')) OR\n        \
             (status = 'done'     AND sub_status = 'none') OR\n        \
             (status = 'archived' AND sub_status = 'none')\n    )",
        "v96",
    )
}

pub(super) fn migrate_v42_drop_epic_tag(conn: &Connection) -> Result<()> {
    // Some migration tests build a pre-v13 schema with no `tag` column.
    if !column_exists(conn, "tasks", "tag") {
        return Ok(());
    }
    let cleared = conn
        .execute("UPDATE tasks SET tag = NULL WHERE tag = 'epic'", [])
        .context("Failed to drop epic tag from tasks (migration v42)")?;
    if cleared > 0 {
        tracing::info!("Migration v42: cleared `epic` tag on {cleared} task(s)");
    }
    Ok(())
}

pub(super) fn migrate_v43_proposed_to_approved(conn: &Connection) -> Result<()> {
    // Change the default status for new learnings from 'proposed' to 'approved'
    // and promote all existing 'proposed' learnings to 'approved'.
    //
    // If the learnings table doesn't exist (e.g. in tests that build minimal
    // schemas without running v40), skip gracefully — there's nothing to migrate.
    if !table_exists(conn, "learnings") {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TABLE learnings_new (
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
        INSERT INTO learnings_new
            SELECT id, kind, summary, detail, scope, scope_ref, tags,
                CASE WHEN status = 'proposed' THEN 'approved' ELSE status END,
                source_task_id, confirmed_count, last_confirmed_at, created_at, updated_at
            FROM learnings;
        DROP TABLE learnings;
        ALTER TABLE learnings_new RENAME TO learnings;
        CREATE INDEX IF NOT EXISTS idx_learnings_scope ON learnings(scope, scope_ref);
        CREATE INDEX IF NOT EXISTS idx_learnings_status ON learnings(status);",
    )
    .context("Failed to migrate learnings to default 'approved' status (migration v43)")
}

pub(super) fn migrate_v44_episodic_to_convention(conn: &Connection) -> Result<()> {
    conn.execute_batch("UPDATE learnings SET kind = 'convention' WHERE kind = 'episodic'")
        .context("Failed to migrate episodic learnings to convention (migration v44)")
}

pub(super) fn migrate_v45_add_task_labels(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "tasks", "labels") {
        conn.execute_batch("ALTER TABLE tasks ADD COLUMN labels TEXT NOT NULL DEFAULT '[]'")
            .context("Failed to add labels column to tasks (migration v45)")?;
    }
    Ok(())
}

pub(super) fn migrate_v46_learning_source_task_set_null(conn: &Connection) -> Result<()> {
    // learnings.source_task_id originally referenced tasks(id) without an
    // ON DELETE action. That blocked feed-task purges (UpsertFeedTasks)
    // whenever a feed-derived task had been the source of any learning.
    // Rebuild the table with ON DELETE SET NULL so a learning outlives its
    // source task as orphaned provenance rather than blocking the delete.

    if !table_exists(conn, "learnings") {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TABLE learnings_new (
            id                INTEGER PRIMARY KEY,
            kind              TEXT    NOT NULL,
            summary           TEXT    NOT NULL,
            detail            TEXT,
            scope             TEXT    NOT NULL,
            scope_ref         TEXT,
            tags              TEXT    NOT NULL DEFAULT '[]',
            status            TEXT    NOT NULL DEFAULT 'approved',
            source_task_id    INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
            confirmed_count   INTEGER NOT NULL DEFAULT 0,
            last_confirmed_at TEXT,
            created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
            updated_at        TEXT    NOT NULL DEFAULT (datetime('now')),
            CHECK (
                (scope = 'user' AND scope_ref IS NULL)
                OR (scope != 'user' AND scope_ref IS NOT NULL)
            )
        );
        INSERT INTO learnings_new
            SELECT id, kind, summary, detail, scope, scope_ref, tags, status,
                source_task_id, confirmed_count, last_confirmed_at, created_at, updated_at
            FROM learnings;
        DROP TABLE learnings;
        ALTER TABLE learnings_new RENAME TO learnings;
        CREATE INDEX IF NOT EXISTS idx_learnings_scope ON learnings(scope, scope_ref);
        CREATE INDEX IF NOT EXISTS idx_learnings_status ON learnings(status);",
    )
    .context("Failed to rebuild learnings with ON DELETE SET NULL (migration v46)")
}

pub(super) fn migrate_v47_task_usage_restore_cascade(conn: &Connection) -> Result<()> {
    // Migration v41 recreated task_usage to drop cost_usd but accidentally
    // dropped the ON DELETE CASCADE clause that was on the original FK in
    // v10. That broke feed-task purges (UpsertFeedTasks) for any task that
    // had usage reported. Recreate the table with the cascade restored.

    if !table_exists(conn, "task_usage") {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TABLE task_usage_new (
            task_id            INTEGER NOT NULL PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
            input_tokens       INTEGER NOT NULL DEFAULT 0,
            output_tokens      INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            updated_at         TEXT NOT NULL DEFAULT ''
        );
        INSERT INTO task_usage_new
            SELECT task_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, updated_at
            FROM task_usage;
        DROP TABLE task_usage;
        ALTER TABLE task_usage_new RENAME TO task_usage;",
    )
    .context("Failed to restore ON DELETE CASCADE on task_usage (migration v47)")
}

fn migrate_v48_learning_validation(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE learning_retrievals (
            id           INTEGER PRIMARY KEY,
            task_id      INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            learning_id  INTEGER NOT NULL REFERENCES learnings(id) ON DELETE CASCADE,
            source       TEXT    NOT NULL,
            retrieved_at TEXT    NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX idx_lr_task ON learning_retrievals(task_id);
         CREATE INDEX idx_lr_learning ON learning_retrievals(learning_id);

         CREATE TABLE learning_verdicts (
            id           INTEGER PRIMARY KEY,
            task_id      INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            learning_id  INTEGER NOT NULL REFERENCES learnings(id) ON DELETE CASCADE,
            verdict      TEXT    NOT NULL,
            recorded_at  TEXT    NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX idx_lv_learning ON learning_verdicts(learning_id);",
    )
    .context("Failed to create learning validation tables (migration v48)")
}

pub(super) fn migrate_v49_rename_confirmed_to_upvote(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE learnings RENAME COLUMN confirmed_count TO upvote_count;
         ALTER TABLE learnings RENAME COLUMN last_confirmed_at TO last_upvoted_at;",
    )
    .context("Failed to rename confirmed_* columns to upvote_* on learnings (migration v49)")
}

fn migrate_v50_add_hook_timestamps(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "tasks", "last_pre_tool_use_at") {
        conn.execute("ALTER TABLE tasks ADD COLUMN last_pre_tool_use_at TEXT", [])
            .context("migration v50: add last_pre_tool_use_at")?;
    }
    if !column_exists(conn, "tasks", "last_notification_at") {
        conn.execute("ALTER TABLE tasks ADD COLUMN last_notification_at TEXT", [])
            .context("migration v50: add last_notification_at")?;
    }
    Ok(())
}

/// Timestamps stamped by the `dispatch hook <id> peer-message` CLI subcommand
/// (task #4098) when it observes a native `SendMessage` tool call via the
/// Claude Code hook pipeline: `last_peer_message_sent_at` on the sending
/// task's own row, `last_peer_message_received_at` on the resolved target's
/// row. The TUI's tick-driven task diff (mirroring how `last_notification_at`
/// already drives `needs_input` detection) turns a changed value into a
/// flash on that task's card. See
/// `docs/superpowers/specs/2026-08-15-send-message-native-relay-design.md`.
fn migrate_v87_add_peer_message_columns(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "tasks", "last_peer_message_sent_at") {
        conn.execute(
            "ALTER TABLE tasks ADD COLUMN last_peer_message_sent_at TEXT",
            [],
        )
        .context("migration v87: add last_peer_message_sent_at")?;
    }
    if !column_exists(conn, "tasks", "last_peer_message_received_at") {
        conn.execute(
            "ALTER TABLE tasks ADD COLUMN last_peer_message_received_at TEXT",
            [],
        )
        .context("migration v87: add last_peer_message_received_at")?;
    }
    Ok(())
}

/// The generic scheduling primitive (task #4203): `schedule_interval_secs`,
/// `pinned_branch`, `last_processed_sha` and `last_scheduled_check_at`, all
/// nullable and null-by-default.
///
/// **Historical.** The feature was reverted in task #4407 and v90 drops all
/// four columns again — nothing in the codebase reads them, and the design
/// document this once cited is gone. It stays because the migration chain is
/// append-only: a database that has never migrated still walks through here on
/// its way to v90.
pub(super) fn migrate_v88_add_scheduling_fields(conn: &Connection) -> Result<()> {
    for (column, ty) in [
        ("schedule_interval_secs", "INTEGER"),
        ("pinned_branch", "TEXT"),
        ("last_processed_sha", "TEXT"),
        ("last_scheduled_check_at", "TEXT"),
    ] {
        if !column_exists(conn, "tasks", column) {
            conn.execute(&format!("ALTER TABLE tasks ADD COLUMN {column} {ty}"), [])
                .with_context(|| format!("migration v88: add {column}"))?;
        }
    }
    // Partial index for the scheduler's every-few-seconds poll. Dropped again
    // by v90 along with the columns; see this function's doc comment.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_scheduled ON tasks(status) \
         WHERE schedule_interval_secs IS NOT NULL",
    )
    .context("migration v88: add idx_tasks_scheduled")?;
    Ok(())
}

/// Undo v88: drop the four scheduling columns and the partial index over them.
///
/// The scheduling primitive (`SchedulerRunner`, `pinned_branch` worktrees, the
/// pipeline dispatch mode) was reverted wholesale in task #4407 — nothing reads
/// these columns any more, and a later scheduling abstraction will be designed
/// from scratch rather than grown from them.
///
/// v88 itself is left in place and untouched: the chain is append-only, so a
/// database that has never migrated must still walk through v88's ALTERs before
/// arriving here. That costs one pass over a table SQLite rewrites anyway.
///
/// The index is dropped BEFORE the columns, and that order is load-bearing:
/// SQLite refuses `DROP COLUMN` on a column any index mentions, and
/// `idx_tasks_scheduled` is partial on `schedule_interval_secs`.
///
/// v72's two feed-subtree triggers are left alone, and that is deliberate
/// rather than an oversight. `DROP COLUMN` makes SQLite re-resolve EVERY
/// trigger on the table, not only those naming the dropped column — and a
/// trigger body is never resolved when it is *created*, so a `tasks` missing a
/// column some trigger names carries a latent error that surfaces here, as a
/// failed migration on startup. Both trigger bodies name only columns that
/// survive this migration, so they re-resolve cleanly and come through
/// untouched; `migration_v90_leaves_the_feed_subtree_triggers_intact` pins
/// that. Dropping and recreating them would be both unnecessary and wrong —
/// it costs the uniqueness invariant for the width of the migration.
///
/// Every step is guarded so the migration is idempotent and safe on a database
/// that somehow predates v88's columns.
pub(super) fn migrate_v90_drop_scheduling_fields(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP INDEX IF EXISTS idx_tasks_scheduled")
        .context("migration v90: drop idx_tasks_scheduled")?;
    for column in [
        "schedule_interval_secs",
        "pinned_branch",
        "last_processed_sha",
        "last_scheduled_check_at",
    ] {
        if column_exists(conn, "tasks", column) {
            conn.execute(&format!("ALTER TABLE tasks DROP COLUMN {column}"), [])
                .with_context(|| format!("migration v90: drop {column}"))?;
        }
    }
    Ok(())
}

fn migrate_v51_drop_pr_workflow_states(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS pr_workflow_states;")
        .context("Failed to drop pr_workflow_states table (migration v51)")
}

pub(super) fn migrate_v52_add_verify_command_to_repo_paths(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "repo_paths", "verify_command") {
        conn.execute_batch("ALTER TABLE repo_paths ADD COLUMN verify_command TEXT")
            .context("v52: add verify_command column")?;
    }
    Ok(())
}

/// Add the `feed_role` column to `epics` plus a partial unique index so a
/// feed parent can hold at most one sub-epic per non-`none` role. The column
/// stores the kebab-case `FeedRole` string (defaults to `'none'`). The index
/// is partial (`WHERE feed_role <> 'none'`) so ordinary epics — all sharing
/// `'none'` — are not constrained.
fn migrate_v65_add_epic_feed_role(conn: &Connection) -> Result<()> {
    // Some migration tests build minimal schemas without an epics table.
    if !table_exists(conn, "epics") {
        return Ok(());
    }
    if !column_exists(conn, "epics", "feed_role") {
        conn.execute_batch("ALTER TABLE epics ADD COLUMN feed_role TEXT NOT NULL DEFAULT 'none';")
            .context("Failed to add feed_role column to epics (migration v65)")?;
    }
    // The index references parent_epic_id; some migration tests build minimal
    // epics schemas without it, so guard on the column existing (learning #110).
    if column_exists(conn, "epics", "parent_epic_id") {
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_epics_parent_feed_role
                 ON epics(parent_epic_id, feed_role)
                 WHERE feed_role <> 'none';",
        )
        .context("Failed to add feed_role unique index (migration v65)")?;
    }
    Ok(())
}

/// `feed_append_only` marks an epic whose feed emits EVENTS rather than
/// mirroring upstream state, so its sync never removes (feeds.allium:
/// AppendOnlyFeed). Defaults to 0: every existing epic keeps mirroring.
fn migrate_v94_add_feed_append_only(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "epics", "feed_append_only") {
        conn.execute_batch(
            "ALTER TABLE epics ADD COLUMN feed_append_only BOOLEAN NOT NULL DEFAULT 0;",
        )
        .context("Failed to add feed_append_only column to epics")?;
    }
    Ok(())
}

fn migrate_v54_add_group_by_repo(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "epics", "group_by_repo") {
        conn.execute_batch(
            "ALTER TABLE epics ADD COLUMN group_by_repo BOOLEAN NOT NULL DEFAULT 0;",
        )
        .context("Failed to add group_by_repo column to epics")?;
    }
    Ok(())
}

pub(super) fn migrate_v55_add_learning_embedding(conn: &Connection) -> Result<()> {
    // Skip if learnings table doesn't exist (e.g. in tests with minimal schemas)
    if !table_exists(conn, "learnings") {
        return Ok(());
    }

    conn.execute_batch("ALTER TABLE learnings ADD COLUMN embedding BLOB;")
        .context("Failed to add embedding column to learnings")
}

fn migrate_v56_drop_task_usage(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS task_usage;")
        .context("Failed to drop task_usage table (migration v56)")
}

pub(super) fn migrate_v57_enforce_epic_project_consistency(conn: &Connection) -> Result<()> {
    let epics_has_parent = column_exists(conn, "epics", "parent_epic_id");
    let epics_has_project = column_exists(conn, "epics", "project_id");
    let tasks_has_epic = column_exists(conn, "tasks", "epic_id");
    let tasks_has_project = column_exists(conn, "tasks", "project_id");

    if epics_has_parent && epics_has_project {
        loop {
            let fixed = conn
                .execute(
                    "UPDATE epics
                     SET project_id = (SELECT p.project_id FROM epics p WHERE p.id = epics.parent_epic_id)
                     WHERE parent_epic_id IS NOT NULL
                       AND project_id != (SELECT p.project_id FROM epics p WHERE p.id = epics.parent_epic_id)",
                    [],
                )
                .context("Failed to fix sub-epic project_id violations")?;
            if fixed == 0 {
                break;
            }
        }
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS enforce_sub_epic_project_insert
             BEFORE INSERT ON epics
             WHEN NEW.parent_epic_id IS NOT NULL
             BEGIN
               SELECT RAISE(ABORT, 'sub-epic project_id must match parent epic project_id')
               WHERE (SELECT project_id FROM epics WHERE id = NEW.parent_epic_id) != NEW.project_id;
             END;

             CREATE TRIGGER IF NOT EXISTS enforce_sub_epic_project_update
             BEFORE UPDATE ON epics
             WHEN NEW.parent_epic_id IS NOT NULL
             BEGIN
               SELECT RAISE(ABORT, 'sub-epic project_id must match parent epic project_id')
               WHERE (SELECT project_id FROM epics WHERE id = NEW.parent_epic_id) != NEW.project_id;
             END;",
        )
        .context("Failed to create sub-epic project consistency triggers")?;
    }

    if tasks_has_epic && tasks_has_project && epics_has_project {
        conn.execute(
            "UPDATE tasks
             SET project_id = (SELECT e.project_id FROM epics e WHERE e.id = tasks.epic_id)
             WHERE epic_id IS NOT NULL
               AND project_id != (SELECT e.project_id FROM epics e WHERE e.id = tasks.epic_id)",
            [],
        )
        .context("Failed to fix task project_id violations")?;
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS enforce_task_epic_project_insert
             BEFORE INSERT ON tasks
             WHEN NEW.epic_id IS NOT NULL
             BEGIN
               SELECT RAISE(ABORT, 'task project_id must match epic project_id')
               WHERE (SELECT project_id FROM epics WHERE id = NEW.epic_id) != NEW.project_id;
             END;

             CREATE TRIGGER IF NOT EXISTS enforce_task_epic_project_update
             BEFORE UPDATE ON tasks
             WHEN NEW.epic_id IS NOT NULL
             BEGIN
               SELECT RAISE(ABORT, 'task project_id must match epic project_id')
               WHERE (SELECT project_id FROM epics WHERE id = NEW.epic_id) != NEW.project_id;
             END;",
        )
        .context("Failed to create task epic project consistency triggers")?;
    }

    Ok(())
}

pub(super) fn migrate_v58_reset_intermediate_epic_statuses(conn: &Connection) -> Result<()> {
    // Guard for migration tests that run against minimal schemas without `epics.status`.
    if column_exists(conn, "epics", "status") {
        conn.execute(
            "UPDATE epics SET status = 'backlog', updated_at = datetime('now') WHERE status IN ('running', 'review')",
            [],
        )
        .context("Failed to reset intermediate epic statuses to backlog (migration v58)")?;
    }
    Ok(())
}

fn migrate_v59_create_usage_events(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            recorded_at TEXT    NOT NULL DEFAULT (datetime('now')),
            category    TEXT    NOT NULL,
            action      TEXT    NOT NULL,
            detail      TEXT,
            actor       TEXT    NOT NULL
        )",
    )
    .context("Failed to create usage_events table")
}

pub(super) fn migrate_v60_drop_projects(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS enforce_sub_epic_project_insert;
         DROP TRIGGER IF EXISTS enforce_sub_epic_project_update;
         DROP TRIGGER IF EXISTS enforce_task_epic_project_insert;
         DROP TRIGGER IF EXISTS enforce_task_epic_project_update;
         DROP INDEX IF EXISTS idx_tasks_project_id;
         DROP INDEX IF EXISTS idx_epics_project_id;",
    )
    .context("Failed to drop project triggers and indexes (migration v60)")?;
    if column_exists(conn, "tasks", "project_id") {
        conn.execute_batch("ALTER TABLE tasks DROP COLUMN project_id")
            .context("Failed to drop project_id from tasks (migration v60)")?;
    }
    if column_exists(conn, "epics", "project_id") {
        conn.execute_batch("ALTER TABLE epics DROP COLUMN project_id")
            .context("Failed to drop project_id from epics (migration v60)")?;
    }
    conn.execute_batch("DROP TABLE IF EXISTS projects")
        .context("Failed to drop projects table (migration v60)")?;
    // Promote the first project-namespaced repo_filter settings key (e.g.
    // "repo_filter:1") to the new unnamespaced key. INSERT OR IGNORE ensures we
    // don't overwrite a key that was already written unnamespaced.
    if table_exists(conn, "settings") {
        conn.execute_batch(
            "INSERT OR IGNORE INTO settings (key, value)
                 SELECT 'repo_filter', value FROM settings
                 WHERE key LIKE 'repo_filter:%' ORDER BY key LIMIT 1;
             DELETE FROM settings WHERE key LIKE 'repo_filter:%';
             INSERT OR IGNORE INTO settings (key, value)
                 SELECT 'repo_filter_mode', value FROM settings
                 WHERE key LIKE 'repo_filter_mode:%' ORDER BY key LIMIT 1;
             DELETE FROM settings WHERE key LIKE 'repo_filter_mode:%';",
        )
        .context("Failed to migrate repo_filter settings keys (migration v60)")?;
    }
    // Drop project-scoped learnings — LearningScope::Project no longer exists
    // and any rows with scope='project' would cause deserialization errors.
    if table_exists(conn, "learnings") {
        conn.execute_batch("DELETE FROM learnings WHERE scope = 'project'")
            .context("Failed to delete project-scoped learnings (migration v60)")?;
    }
    Ok(())
}

fn migrate_v63_add_task_indexes(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "tasks") {
        return Ok(());
    }
    if column_exists(conn, "tasks", "status") {
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);")
            .context("Failed to add idx_tasks_status (migration v63)")?;
    }
    if column_exists(conn, "tasks", "epic_id") {
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_tasks_epic_id ON tasks(epic_id);")
            .context("Failed to add idx_tasks_epic_id (migration v63)")?;
    }
    Ok(())
}

fn migrate_v61_drop_epic_repo_path(conn: &Connection) -> Result<()> {
    if column_exists(conn, "epics", "repo_path") {
        conn.execute_batch("ALTER TABLE epics DROP COLUMN repo_path")
            .context("Failed to drop repo_path from epics (migration v61)")?;
    }
    Ok(())
}

fn migrate_v66_add_pr_learnings_gate(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "tasks", "pr_learnings_gate_shown_at") {
        conn.execute(
            "ALTER TABLE tasks ADD COLUMN pr_learnings_gate_shown_at TEXT",
            [],
        )
        .context("migration v66: add pr_learnings_gate_shown_at")?;
    }
    Ok(())
}

pub(super) fn migrate_v67_create_todos(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "todos") {
        conn.execute_batch(
            "CREATE TABLE todos (
                id          INTEGER PRIMARY KEY,
                title       TEXT NOT NULL,
                done        INTEGER NOT NULL DEFAULT 0,
                sort_order  INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .context("Failed to create todos table (migration v67)")?;
    }
    Ok(())
}

pub(super) fn migrate_v70_add_todo_parent_id(conn: &Connection) -> Result<()> {
    if !column_exists(conn, "todos", "parent_id") {
        conn.execute_batch(
            "ALTER TABLE todos ADD COLUMN parent_id INTEGER REFERENCES todos(id) ON DELETE SET NULL;",
        )
        .context("v69: add parent_id to todos")?;
    }
    Ok(())
}

pub(super) fn migrate_v68_add_todo_links(conn: &Connection) -> Result<()> {
    // The "at most one of task_id/epic_id is non-null" invariant is enforced
    // at the service layer (via TodoLink enum exhaustive match) rather than at
    // the schema level. SQLite ALTER TABLE ADD COLUMN does not support table-level
    // CHECK constraints; adding one would require a full table rebuild in a future
    // migration if deemed necessary.
    if !column_exists(conn, "todos", "task_id") {
        conn.execute_batch(
            "ALTER TABLE todos ADD COLUMN task_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL;
             ALTER TABLE todos ADD COLUMN epic_id INTEGER REFERENCES epics(id) ON DELETE SET NULL;",
        )
        .context("v68: add task_id and epic_id to todos")?;
    }
    Ok(())
}

fn migrate_v69_add_epic_origin(conn: &Connection) -> Result<()> {
    // Minimal-schema migration tests may lack an epics table.
    if !table_exists(conn, "epics") {
        return Ok(());
    }
    if !column_exists(conn, "epics", "origin") {
        conn.execute_batch("ALTER TABLE epics ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual';")
            .context("Failed to add origin column to epics (migration v69)")?;
    }
    // Guard on title + parent_epic_id (minimal schemas may omit parent_epic_id).
    if column_exists(conn, "epics", "parent_epic_id") {
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_epics_parent_origin_repo_group
                 ON epics(parent_epic_id, title)
                 WHERE origin = 'repo-group';",
        )
        .context("Failed to add origin unique index (migration v69)")?;
    }
    Ok(())
}

/// Back up the database ahead of the v71 dedup migration below, via
/// `VACUUM INTO` (atomic and consistent). Skipped for in-memory databases
/// (empty path string from `PRAGMA database_list`) and a no-op if the backup
/// file already exists.
///
/// Called separately from — and *before* — `apply_pending_migrations`'s
/// migration transaction (`src/db/mod.rs`): SQLite refuses to `VACUUM`
/// inside an open transaction, so this can't live inside
/// `migrate_v71_dedup_role_subtree_tasks` itself once that runs
/// transactionally.
pub(super) fn migrate_v71_create_backup(conn: &Connection) -> Result<()> {
    let db_path: String = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get(0),
        )
        .context("v71: failed to read database path")?;

    if !db_path.is_empty() {
        let backup_path = format!("{}.bak.v71", db_path);
        if !std::path::Path::new(&backup_path).exists() {
            let escaped = backup_path.replace('\'', "''");
            conn.execute_batch(&format!("VACUUM INTO '{}'", escaped))
                .context("v71: failed to create database backup")?;
        }
    }
    Ok(())
}

/// Remove duplicate feed tasks from role sub-epic subtrees. A "duplicate" is
/// a task in a repo-group sub-epic whose `external_id` also appears directly
/// on the parent role sub-epic. The copy on the role sub-epic is kept; the
/// repo-group copy is deleted.
///
/// The pre-migration backup (`migrate_v71_create_backup`) must run before
/// this, outside any transaction — see that function's doc comment.
pub(super) fn migrate_v71_dedup_role_subtree_tasks(conn: &Connection) -> Result<()> {
    // Remove repo-group-sub-epic task copies that are shadowed by a copy
    // on the parent role sub-epic (matched by external_id).
    // Guard: skip if required columns are absent (minimal-schema migration tests).
    let has_origin = column_exists(conn, "epics", "origin");
    let has_feed_role = column_exists(conn, "epics", "feed_role");
    let has_parent_epic_id = column_exists(conn, "epics", "parent_epic_id");
    let has_external_id = column_exists(conn, "tasks", "external_id");
    if has_origin && has_feed_role && has_parent_epic_id && has_external_id {
        conn.execute_batch(
            "DELETE FROM tasks WHERE id IN (
                SELECT t.id
                FROM tasks t
                JOIN epics e      ON e.id = t.epic_id
                JOIN epics parent ON parent.id = e.parent_epic_id
                WHERE e.origin = 'repo-group'
                  AND parent.feed_role NOT IN ('none', 'reviews-parent')
                  AND EXISTS (
                    SELECT 1 FROM tasks t2
                    WHERE t2.epic_id = parent.id
                      AND t2.external_id = t.external_id
                  )
            )",
        )
        .context("v71: failed to remove duplicate role sub-epic tasks")?;
    }

    Ok(())
}

/// Add BEFORE INSERT and BEFORE UPDATE OF epic_id triggers on `tasks` that
/// enforce: within any role sub-epic's subtree (role sub-epic + its direct
/// repo-group children), no two tasks share the same `external_id`.
///
/// "Role sub-epic" is identified by `feed_role NOT IN ('none', 'reviews-parent')`.
/// "Repo-group sub-epic" is identified by `origin = 'repo-group'` with a
/// parent whose `feed_role NOT IN ('none', 'reviews-parent')`.
///
/// NULL external_ids (manually-created tasks) are excluded from the check.
pub(super) fn migrate_v72_add_feed_task_subtree_unique_triggers(conn: &Connection) -> Result<()> {
    // BEFORE INSERT trigger.
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS enforce_feed_task_subtree_unique_insert
         BEFORE INSERT ON tasks
         FOR EACH ROW
         WHEN NEW.external_id IS NOT NULL
         BEGIN
             SELECT RAISE(ABORT, 'duplicate external_id in role sub-epic subtree')
             WHERE EXISTS (
                 SELECT 1 FROM tasks t
                 JOIN epics target ON target.id = NEW.epic_id
                 LEFT JOIN epics tpar ON tpar.id = target.parent_epic_id
                 JOIN epics te ON te.id = t.epic_id
                 WHERE t.external_id = NEW.external_id
                   AND (
                     -- Target is a role sub-epic: check itself and all repo-group children.
                     ( target.feed_role NOT IN ('none', 'reviews-parent')
                       AND (te.id = target.id
                            OR te.parent_epic_id = target.id) )
                     OR
                     -- Target is a repo-group sub-epic: check the parent role sub-epic and all its children.
                     ( target.origin = 'repo-group'
                       AND tpar.feed_role NOT IN ('none', 'reviews-parent')
                       AND (te.id = tpar.id
                            OR te.parent_epic_id = tpar.id) )
                   )
             );
         END;",
    )
    .context("v72: failed to create insert trigger")?;

    // BEFORE UPDATE OF epic_id trigger (excludes the row being moved via OLD.id).
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS enforce_feed_task_subtree_unique_update
         BEFORE UPDATE OF epic_id ON tasks
         FOR EACH ROW
         WHEN NEW.external_id IS NOT NULL AND NEW.epic_id != OLD.epic_id
         BEGIN
             SELECT RAISE(ABORT, 'duplicate external_id in role sub-epic subtree')
             WHERE EXISTS (
                 SELECT 1 FROM tasks t
                 JOIN epics target ON target.id = NEW.epic_id
                 LEFT JOIN epics tpar ON tpar.id = target.parent_epic_id
                 JOIN epics te ON te.id = t.epic_id
                 WHERE t.external_id = NEW.external_id
                   AND t.id != OLD.id
                   AND (
                     ( target.feed_role NOT IN ('none', 'reviews-parent')
                       AND (te.id = target.id
                            OR te.parent_epic_id = target.id) )
                     OR
                     ( target.origin = 'repo-group'
                       AND tpar.feed_role NOT IN ('none', 'reviews-parent')
                       AND (te.id = tpar.id
                            OR te.parent_epic_id = tpar.id) )
                   )
             );
         END;",
    )
    .context("v72: failed to create update trigger")?;

    Ok(())
}

/// Fix `enforce_feed_task_subtree_unique_insert` (v72): SQLite fires BEFORE
/// INSERT triggers even for `INSERT ... ON CONFLICT(epic_id, external_id) DO
/// UPDATE`, before the conflict is resolved. The v72 trigger's EXISTS check
/// had no exclusion for the row that ON CONFLICT is about to reconcile with
/// (unlike the sibling update trigger's `t.id != OLD.id`), so re-upserting
/// ANY already-tracked task — the normal path on every feed poll — matched
/// its own prior row and aborted the whole `upsert_feed_tasks` transaction,
/// silently dropping any other new task batched in the same call. Adding
/// `t.epic_id != NEW.epic_id` excludes exactly that self-row: the partial
/// unique index `tasks_epic_external_id` (v38) already guarantees at most
/// one row exists at (epic_id, external_id), so this can only ever exclude
/// the row ON CONFLICT would update — never a second, distinct same-epic
/// row, and never a genuine cross-epic duplicate elsewhere in the subtree.
pub(super) fn migrate_v76_fix_feed_task_subtree_insert_trigger_self_conflict(
    conn: &Connection,
) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS enforce_feed_task_subtree_unique_insert;
         CREATE TRIGGER enforce_feed_task_subtree_unique_insert
         BEFORE INSERT ON tasks
         FOR EACH ROW
         WHEN NEW.external_id IS NOT NULL
         BEGIN
             SELECT RAISE(ABORT, 'duplicate external_id in role sub-epic subtree')
             WHERE EXISTS (
                 SELECT 1 FROM tasks t
                 JOIN epics target ON target.id = NEW.epic_id
                 LEFT JOIN epics tpar ON tpar.id = target.parent_epic_id
                 JOIN epics te ON te.id = t.epic_id
                 WHERE t.external_id = NEW.external_id
                   AND t.epic_id != NEW.epic_id
                   AND (
                     -- Target is a role sub-epic: check itself and all repo-group children.
                     ( target.feed_role NOT IN ('none', 'reviews-parent')
                       AND (te.id = target.id
                            OR te.parent_epic_id = target.id) )
                     OR
                     -- Target is a repo-group sub-epic: check the parent role sub-epic and all its children.
                     ( target.origin = 'repo-group'
                       AND tpar.feed_role NOT IN ('none', 'reviews-parent')
                       AND (te.id = tpar.id
                            OR te.parent_epic_id = tpar.id) )
                   )
             );
         END;",
    )
    .context("v76: failed to recreate insert trigger without self-conflict")?;

    Ok(())
}

/// Clamp every stored feed cadence below `min_feed_interval` up to it.
///
/// A one-time data fix for rows written before the floor existed, when the
/// integer write paths had no lower bound at all: a `0` made the feed runner
/// respawn the command on every poll tick, and a negative wrapped into an
/// effectively infinite cadence that silenced the feed. See "Interval literals"
/// in `docs/specs/core.allium`.
///
/// Clamped UP rather than nulled so the value stays explicit — a later change
/// to `default_feed_interval` must not silently retarget an epic whose cadence
/// somebody once chose. `NULL` already means "inherit the default" and is left
/// alone.
///
/// Two homes, because the cadence has two: the per-epic column, and the two
/// managed-feed settings rows that provisioning copies onto managed epics.
/// Missing tables/columns are tolerated so the migration is safe on any
/// schema history, and it is idempotent — a second run finds nothing left
/// below the floor.
///
/// This does not relieve any write path of validating. It cleans what already
/// exists; `crate::service::validate_feed_interval` is what stops new rows.
pub(super) fn migrate_v91_clamp_feed_intervals(conn: &Connection) -> Result<()> {
    let floor = crate::models::MIN_FEED_INTERVAL_SECS;

    if column_exists(conn, "epics", "feed_interval_secs") {
        let clamped = conn
            .execute(
                "UPDATE epics SET feed_interval_secs = ?1
                 WHERE feed_interval_secs IS NOT NULL AND feed_interval_secs < ?1",
                params![floor],
            )
            .context("Failed to clamp sub-minimum epics.feed_interval_secs (migration v91)")?;
        if clamped > 0 {
            tracing::info!(
                "Migration v91: clamped feed_interval_secs up to {floor}s on {clamped} epic(s)"
            );
        }
    }

    if table_exists(conn, "settings") {
        // Stored as TEXT like every other settings value, so the comparison is
        // cast rather than done on the string — '9' > '60' lexically.
        let clamped = conn
            .execute(
                "UPDATE settings SET value = ?1
                 WHERE key IN ('reviews_feed_interval_secs', 'cve_feed_interval_secs')
                   AND CAST(value AS INTEGER) < ?2",
                params![floor.to_string(), floor],
            )
            .context("Failed to clamp sub-minimum managed-feed intervals (migration v91)")?;
        if clamped > 0 {
            tracing::info!(
                "Migration v91: clamped {clamped} managed-feed interval setting(s) up to {floor}s"
            );
        }
    }

    Ok(())
}

/// Delete the orphaned `main_session.dir` setting row.
///
/// The main session — a task-independent "dispatch-main" tmux window opened
/// with `:` — was removed, and with it the only reader and writer of this key.
/// Databases that ran the feature still carry the row, where it is now dead
/// data that no code path can reach. Nothing else about the feature was
/// persisted: the window identity was always re-derived from a live tmux check.
///
/// A missing settings table is tolerated so the migration is safe on any schema
/// history, and it is idempotent — a second run finds nothing left to delete.
pub(super) fn migrate_v95_drop_main_session_dir(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "settings") {
        return Ok(());
    }

    let deleted = conn
        .execute("DELETE FROM settings WHERE key = 'main_session.dir'", [])
        .context("Failed to delete the main_session.dir setting (migration v95)")?;
    if deleted > 0 {
        tracing::info!("Migration v95: dropped the orphaned main_session.dir setting");
    }

    Ok(())
}

/// Add the nullable `host` column to `tasks`, then backfill it for every
/// existing row that already holds a worktree.
///
/// Foundations for task #4812's distributed-dispatch design
/// (`docs/superpowers/specs/2026-09-13-distributed-dispatch-design.md`). See
/// `core/Task.host` and the `HostTracksWorktree` invariant in
/// `docs/specs/core.allium`.
///
/// The `ADD COLUMN` alone is not enough: `HostTracksWorktree` requires
/// `worktree != null implies host != null`, and a fresh nullable column reads
/// `NULL` on every existing row, including the ones that already hold a
/// worktree (running/review/done tasks, and a backlog task whose worktree
/// `MoveTaskBackward` preserved). Without a backfill those rows violate the
/// invariant from the moment this migration runs, until each is next
/// dispatched.
///
/// The backfill is safe by construction: host tracking did not exist before
/// this migration, so every worktree on disk was necessarily provisioned by
/// THIS machine — there was only ever one machine in the picture. So the
/// backfill mints (or reads back) this install's Host id inline, the same
/// `ON CONFLICT DO NOTHING`-then-read shape as
/// `SettingsStore::ensure_host_identity`, rather than waiting for the runtime
/// to call that at its next startup — a gap a slow first boot would otherwise
/// leave open. Minting here is not a second mint: it is the same idempotent
/// operation, just run earlier.
///
/// Only the id is minted — no `host_label` is seeded here, and none is
/// invented for an install that has none. `host_label` is a key in the
/// `settings` key/value table rather than a SQL column, so whether a machine
/// is named is a fact about application logic, not about the schema: an
/// install that has completed a prior run of dispatch already holds a real
/// label row and must not be re-prompted or renamed, and one that does not is
/// exactly the "unnamed" state `docs/specs/startup.allium`'s
/// `PromptForHostLabelWhenUnnamed` gate exists to handle at startup. Asking is
/// the startup gate's job, never a migration's.
pub(super) fn migrate_v97_add_task_host(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE tasks ADD COLUMN host TEXT")
        .context("Failed to add host column to tasks")?;

    if !table_exists(conn, "settings") {
        return Ok(());
    }

    // NOTHING TO BACKFILL, NOTHING TO MINT. A database with no worktree-holding
    // task — every brand-new one — needs no host id from this migration, and
    // minting one anyway is not merely wasted: the schema TEMPLATE that
    // in-memory databases are cloned from is built by replaying this chain
    // once, and the backup API copies rows, so an id minted here is baked into
    // the template and inherited by every clone. Two databases standing for two
    // machines then agree they are the same machine, every locality gate passes
    // where it should refuse, and a test written to prove separation proves the
    // opposite while passing.
    //
    // Minting belongs to `ensure_host_identity`, which is per-install and
    // idempotent and runs at startup. This migration mints only when it has
    // rows whose host it must answer for — which is exactly when the answer is
    // knowable, because host tracking did not exist before it and every
    // worktree on disk was necessarily provisioned by THIS machine.
    let needs_backfill: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE worktree IS NOT NULL)",
            [],
            |row| row.get(0),
        )
        .context("Failed to check for worktree-holding tasks (migration v97)")?;
    if !needs_backfill {
        return Ok(());
    }

    let generated_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO NOTHING",
        params![HOST_ID_KEY, generated_id],
    )
    .context("Failed to mint host id during migration v97 backfill")?;
    let host_id: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![HOST_ID_KEY],
            |row| row.get(0),
        )
        .context("Failed to read host id during migration v97 backfill")?;

    let backfilled = conn
        .execute(
            "UPDATE tasks SET host = ?1 WHERE worktree IS NOT NULL AND host IS NULL",
            params![host_id],
        )
        .context("Failed to backfill host for existing worktree-holding tasks (migration v97)")?;
    if backfilled > 0 {
        tracing::info!(
            "Migration v97: backfilled host on {backfilled} pre-existing worktree-holding task(s)"
        );
    }

    Ok(())
}

/// The `subscriptions` table: one person's standing interest in one epic.
///
/// `core.allium: Subscription`, and `docs/specs/sync.allium`'s SubscribeToEpic
/// / UnsubscribeFromEpic. Empty until somebody follows an epic, and empty
/// forever on an install that never reaches a shared store — subscribing
/// requires an identity, and an install with no store has none.
///
/// **Column order matches the SpacetimeDB module's, and must keep matching.**
/// `src/spacetime/tests/module_schema.rs` compares the two positionally,
/// because the shared store can append a column and cannot insert one: a table
/// whose columns merely match as a set is already unmigratable.
///
/// `id` is the derived `<subscriber>/<epic_id>` pair rather than a generated
/// number, which is what makes re-subscribing an overwrite instead of a
/// duplicate (`core.allium: SubscriptionIsUniquePerSubscriberAndEpic`) and what
/// keeps this table out of the id-sequence burn entirely.
///
/// No foreign key to `epics`. A subscription is one person's interest, and the
/// epics a person follows are not all on this machine — a colleague's epic is
/// exactly the case subscribing exists for, and a constraint here would refuse
/// the only rows worth having.
fn migrate_v98_create_subscriptions(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS subscriptions (
             id         TEXT PRIMARY KEY,
             epic_id    INTEGER NOT NULL,
             subscriber TEXT NOT NULL
         )",
    )
    .context("Failed to create subscriptions table")?;
    Ok(())
}

/// v100: `todos` gains `owner` — the person whose checklist this is.
///
/// A todo has no epic to belong to and nothing to share it through, so before
/// this column the only query a shared store could answer for todos was "all of
/// them". `docs/specs/sync.allium`'s subscription selects `WHERE owner = <me>`,
/// and that query needs a column to select on.
///
/// **Backfilled to this install's stored identity, where there is one.** Every
/// todo in an existing database was written by the person running it — there
/// was only ever one person in the picture, exactly as there was only ever one
/// machine in v97's — so stamping them is recovering a fact rather than
/// inventing one. An install that has never connected has no identity to stamp
/// with, and its todos stay unowned until the seed backfills them; that is the
/// same lasting, supported state `core/Host.owner` describes.
///
/// Appended, like every column here, because the shared store can append one
/// and cannot insert one.
fn migrate_v100_add_todo_owner(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE todos ADD COLUMN owner TEXT")
        .context("Failed to add owner column to todos")?;

    if !table_exists(conn, "settings") {
        return Ok(());
    }

    let identity: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [crate::db::USER_IDENTITY_KEY],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to read the stored user identity (migration v100)")?;

    let Some(identity) = identity.filter(|id| !id.trim().is_empty()) else {
        return Ok(());
    };

    let stamped = conn
        .execute(
            "UPDATE todos SET owner = ?1 WHERE owner IS NULL",
            [&identity],
        )
        .context("Failed to backfill todos.owner (migration v100)")?;
    if stamped > 0 {
        tracing::info!(
            "Migration v100: stamped {stamped} pre-existing todo(s) with the local owner"
        );
    }
    Ok(())
}

/// v101: drop `task_shells` and `tasks.live_shells`/
/// `tasks.oldest_live_shell_started_at` (task #4187's shell-visibility
/// feature).
///
/// #4965 found the tracking was built on an assumption that does not hold:
/// Claude Code fires a hook for a backgrounded shell only when the agent
/// explicitly polls it (BashOutput/TaskOutput) or kills it (KillBash/
/// TaskStop), never when the shell exits on its own. A task whose agent never
/// checks back in keeps an inflated count forever, with no reliable signal
/// available to fix it — confirmed live on task #4913, where the recorded
/// count was 5 while every one of those shells had already exited. Dropped
/// rather than patched again; see docs/specs/agent-health.allium.
pub(super) fn migrate_v101_drop_shell_tracking(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS task_shells")
        .context("Failed to drop task_shells table (migration v101)")?;
    if column_exists(conn, "tasks", "live_shells") {
        conn.execute_batch("ALTER TABLE tasks DROP COLUMN live_shells")
            .context("Failed to drop tasks.live_shells (migration v101)")?;
    }
    if column_exists(conn, "tasks", "oldest_live_shell_started_at") {
        conn.execute_batch("ALTER TABLE tasks DROP COLUMN oldest_live_shell_started_at")
            .context("Failed to drop tasks.oldest_live_shell_started_at (migration v101)")?;
    }
    Ok(())
}

/// v102: drop `todos` (the TODO overlay, v67 onward).
///
/// #4970 removed the TODO subsystem after keybinding telemetry showed it
/// was barely used. The rows go with the table; the user chose removal
/// without an export. See docs/specs/spacetime-seed.allium for why the
/// shared store keeps its own `todos` table as dead schema instead.
pub(super) fn migrate_v102_drop_todos(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS todos")
        .context("Failed to drop todos table (migration v102)")?;
    Ok(())
}

/// v103: drop `my_prs`, `review_prs`, `bot_prs` and `security_alerts`.
///
/// They held the Review and Security boards, which feed epics subsumed (see
/// docs/specs/feeds.allium). Nothing outside this module has read or written
/// them since, and the shared store never had them, so there is nothing to
/// seed from them either. No trigger names these tables, so the drop cannot
/// break another table's trigger resolution.
///
/// v14, v21, v23, v24 and every later step touching them stay in place for
/// the same reason v36 does beside v84: a database stamped below them still
/// replays them on its way here.
pub(super) fn migrate_v103_drop_legacy_pr_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS my_prs;
         DROP TABLE IF EXISTS review_prs;
         DROP TABLE IF EXISTS bot_prs;
         DROP TABLE IF EXISTS security_alerts;",
    )
    .context("Failed to drop the legacy PR tables (migration v103)")
}

/// v104: drop `filter_presets` (saved repo-filter presets, v11 onward).
///
/// #4972 removed filter presets after keybinding telemetry recorded one
/// cancelled save and no loads or deletes. The rows go with the table. The
/// shared store keeps its own `filter_presets` table as dead schema, because
/// SpacetimeDB refuses to drop a table that still holds rows.
pub(super) fn migrate_v104_drop_filter_presets(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS filter_presets")
        .context("Failed to drop filter_presets table (migration v104)")?;
    Ok(())
}

/// v105: creates `retired_feed_items` — see `docs/specs/core.allium`'s
/// `RetiredFeedItem` entity. One row per (feed_epic_id, external_id) a human
/// deleted from a feed's subtree, so the feed's next cycle does not put the
/// same item straight back. `ON DELETE CASCADE` is how `DeleteEpic`
/// (`docs/specs/epics.allium`) drops a feed epic's records when the epic
/// itself is deleted — deleting a feed epic is a reset, not a prune.
pub(super) fn migrate_v105_create_retired_feed_items(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS retired_feed_items (
            id           INTEGER PRIMARY KEY,
            feed_epic_id INTEGER NOT NULL REFERENCES epics(id) ON DELETE CASCADE,
            external_id  TEXT NOT NULL,
            retired_at   TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(feed_epic_id, external_id)
        );
        CREATE INDEX IF NOT EXISTS idx_retired_feed_items_feed_epic
            ON retired_feed_items(feed_epic_id);",
    )
    .context("v105: failed to create retired_feed_items table")
}

/// v106: `ArchivedStatusMigration` (`docs/specs/epics.allium`). Runs the five
/// phases in order — the order is load-bearing, see the spec's guidance:
/// retirement records are written before any row they describe is deleted,
/// tasks are settled before epics, and the managed-feed opt-out is recorded
/// before the managed epic that expressed it disappears.
///
/// Uses raw `'archived'`/`'done'` string literals rather than
/// `TaskStatus`/an enum variant: by the time this migration ships, the Rust
/// enum no longer has an `Archived` case (task #4971's consequence list) —
/// this migration is the one place the old status is still named, reading it
/// as a black-box column value the way `migrate_v42_drop_epic_tag` and
/// others already read since-removed string values.
pub(super) fn migrate_v106_archived_status_migration(conn: &Connection) -> Result<()> {
    // Guard for migration tests that build minimal schemas without these
    // tables/columns.
    if !table_exists(conn, "tasks") || !table_exists(conn, "epics") {
        return Ok(());
    }

    // Phase 1: retire every archived feed task, keyed on the nearest ancestor
    // epic (itself or an ancestor) that carries a feed_command — the same
    // fully-recursive core/Epic.nearest_feed_epic walk DeleteTask,
    // upsert_feed_tasks_inner and DeleteEpic use at runtime (not a one-level
    // shortcut: a task can sit arbitrarily deep under its feed epic, e.g. a
    // user-nested epic under a repo/role sub-epic). `UNION` (not `UNION ALL`)
    // in both recursive CTEs so a `parent_epic_id` cycle — never legal
    // through the write paths that check for one, but not something this
    // migration should hang on if it ever occurs — terminates by row
    // dedup rather than looping forever.
    //
    // Guarded: a migration test's synthetic pre-v38 schema has neither
    // `tasks.external_id` nor `epics.feed_command` (feeds did not exist
    // yet), and a real database at that vintage has no archived feed task to
    // retire either.
    if column_exists(conn, "tasks", "external_id")
        && column_exists(conn, "epics", "feed_command")
        && column_exists(conn, "epics", "parent_epic_id")
    {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.external_id,
                        (WITH RECURSIVE chain(id, feed_command, parent_epic_id) AS (
                             SELECT id, feed_command, parent_epic_id FROM epics WHERE id = t.epic_id
                             UNION
                             SELECT e.id, e.feed_command, e.parent_epic_id
                             FROM epics e JOIN chain c ON e.id = c.parent_epic_id
                         )
                         SELECT id FROM chain WHERE feed_command IS NOT NULL LIMIT 1)
                            AS feed_epic_id
                 FROM tasks t
                 WHERE t.status = 'archived' AND t.external_id IS NOT NULL",
            )
            .context("v106: failed to prepare archived feed task scan")?;
        let rows: Vec<(i64, String, Option<i64>)> = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get(2)?))
            })
            .context("v106: failed to scan archived feed tasks")?
            .collect::<rusqlite::Result<_>>()
            .context("v106: failed to collect archived feed tasks")?;
        drop(stmt);

        for (_task_id, external_id, feed_epic_id) in rows {
            let Some(feed_epic_id) = feed_epic_id else {
                continue;
            };
            conn.execute(
                "INSERT OR IGNORE INTO retired_feed_items (feed_epic_id, external_id) \
                 VALUES (?1, ?2)",
                params![feed_epic_id, external_id],
            )
            .context("v106: failed to backfill retired_feed_items")?;
        }
    }

    // Phase 2: an archived task still holding a worktree had a teardown that
    // failed; move it to done so the user can delete it and retry teardown
    // from the board. Every other archived task is deleted outright.
    // Guarded the same way as phase 1: a synthetic pre-worktree-column schema
    // has nothing to preserve here.
    if column_exists(conn, "tasks", "worktree") {
        conn.execute(
            "UPDATE tasks SET status = 'done', sub_status = 'none', updated_at = datetime('now') \
             WHERE status = 'archived' AND worktree IS NOT NULL",
            [],
        )
        .context("v106: failed to settle worktree-holding archived tasks to done")?;
    }
    // The migration runner brackets EVERY migration with `PRAGMA foreign_keys
    // = OFF` (src/db/mod.rs::apply_pending_migrations, for the ALTER-TABLE
    // table-rebuild idiom other migrations need), so none of these tables'
    // `ON DELETE CASCADE` (or, for task_watchers, its total absence of a
    // declared FK) fires for the DELETE below. Purged explicitly instead —
    // the same rows DeleteTask's real runtime path relies on cascade/its own
    // cleanup calls for.
    if table_exists(conn, "task_subagents") {
        conn.execute(
            "DELETE FROM task_subagents WHERE task_id IN (SELECT id FROM tasks WHERE status = 'archived')",
            [],
        )
        .context("v105: failed to purge task_subagents of archived tasks")?;
    }
    // `learnings.source_task_id` carries `ON DELETE SET NULL` (migration v47),
    // which foreign_keys=OFF also disables here — detached explicitly, same
    // reason and same guard as the other three purges below. Without this, a
    // learning whose source was archived keeps a source_task_id pointing at a
    // row this same migration is about to delete, and a later snapshot/seed
    // round-trip would copy that dangling id into the store.
    if table_exists(conn, "learnings") {
        conn.execute(
            "UPDATE learnings SET source_task_id = NULL \
             WHERE source_task_id IN (SELECT id FROM tasks WHERE status = 'archived')",
            [],
        )
        .context("v105: failed to detach learnings of archived tasks")?;
    }
    if table_exists(conn, "learning_retrievals") {
        conn.execute(
            "DELETE FROM learning_retrievals WHERE task_id IN (SELECT id FROM tasks WHERE status = 'archived')",
            [],
        )
        .context("v105: failed to purge learning_retrievals of archived tasks")?;
    }
    if table_exists(conn, "task_watchers") {
        conn.execute(
            "DELETE FROM task_watchers WHERE target_task_id IN (SELECT id FROM tasks WHERE status = 'archived') \
                                           OR watcher_task_id IN (SELECT id FROM tasks WHERE status = 'archived')",
            [],
        )
        .context("v105: failed to purge task_watchers of archived tasks")?;
    }
    conn.execute("DELETE FROM tasks WHERE status = 'archived'", [])
        .context("v106: failed to delete archived tasks")?;

    // Phase 3: an archived managed ROOT epic (feed_role IN
    // ('reviews_parent', 'cve')) was the old opt-out for its managed feed.
    // Clear the matching settings so ProvisionManagedEpics does not
    // re-provision what the user switched off. Guarded: fresh/minimal test
    // schemas may not have `feed_role` or `settings`.
    if column_exists(conn, "epics", "feed_role") && table_exists(conn, "settings") {
        let has_archived_reviews_root: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM epics WHERE status = 'archived' AND feed_role = 'reviews-parent'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if has_archived_reviews_root {
            conn.execute_batch(
                "DELETE FROM settings WHERE key IN ('reviews_feed_command', 'reviews_feed_interval_secs');",
            )
            .context("v106: failed to clear reviews feed config")?;
        }
        let has_archived_cve_root: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM epics WHERE status = 'archived' AND feed_role = 'cve'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if has_archived_cve_root {
            conn.execute_batch(
                "DELETE FROM settings WHERE key IN ('cve_feed_command', 'cve_feed_interval_secs');",
            )
            .context("v106: failed to clear cve feed config")?;
        }
    }

    // Phase 4: archived epics are deleted — together with their whole
    // subtree, per epics.allium's DeleteEpic (`doomed_epics = epic.
    // subtree_epics + epic`), which is exactly what an empty `subtree_tasks`
    // makes this migration's delete equivalent to — when no task remains
    // anywhere in that subtree (after phase 2); otherwise the archived epic
    // alone is moved to done and recalculated.
    //
    // Outer loop processed with the deepest still-ARCHIVED epics first (an
    // archived epic with no archived child left), which matches the
    // recursive "subtree" reading of the spec and keeps every delete
    // FK-legal: `epics.parent_epic_id` has no `ON DELETE CASCADE`, so
    // deleting a row still referenced by a child would fail under `PRAGMA
    // foreign_keys=ON`. A leaf-first order means an archived child is always
    // resolved (deleted or settled to done) before its archived parent is
    // considered.
    //
    // `recalculate_epic_status_inner` is deliberately NOT called inside this
    // loop, even for an epic just settled to done — only queued in
    // `settled_ids` below. It reads the row of every epic it touches
    // (`get_epic_row`) and of every direct child/sibling epic along the way,
    // and `row_to_epic` parses `status` through `TaskStatus::parse`, which no
    // longer accepts `'archived'` (task #4971 dropped the variant). Calling
    // it mid-loop reaches exactly the rows this loop has not gotten to yet —
    // an unprocessed ARCHIVED ancestor (walking up from a just-settled child)
    // or an unprocessed ARCHIVED sibling (reading a live parent's other
    // children) — and the migration fails outright, so the board never
    // opens. Recalculating only after this loop has fully drained (no
    // 'archived' epic anywhere left unresolved) is what makes every row a
    // recalculation reads guaranteed to parse.
    let mut settled_ids: Vec<i64> = Vec::new();
    loop {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM epics WHERE status = 'archived' \
                 AND id NOT IN (\
                     SELECT DISTINCT parent_epic_id FROM epics \
                     WHERE parent_epic_id IS NOT NULL AND status = 'archived'\
                 )",
            )
            .context("v106: failed to prepare leaf archived epic scan")?;
        let leaf_ids: Vec<i64> = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .context("v106: failed to scan leaf archived epics")?
            .collect::<rusqlite::Result<_>>()
            .context("v106: failed to collect leaf archived epics")?;
        drop(stmt);
        if leaf_ids.is_empty() {
            break;
        }
        for id in leaf_ids {
            // "Anywhere in the subtree" — every task under this epic or any
            // descendant epic, at any depth (regardless of that descendant's
            // own status), not just this epic's own direct tasks.
            let has_subtree_task: bool = conn
                .query_row(
                    "WITH RECURSIVE subtree(id) AS (\
                         SELECT ?1 \
                         UNION \
                         SELECT e.id FROM epics e JOIN subtree s ON e.parent_epic_id = s.id\
                     ) \
                     SELECT COUNT(*) FROM tasks WHERE epic_id IN (SELECT id FROM subtree)",
                    params![id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if has_subtree_task {
                conn.execute(
                    "UPDATE epics SET status = 'done', updated_at = datetime('now') WHERE id = ?1",
                    params![id],
                )
                .context("v106: failed to settle archived epic to done")?;
                settled_ids.push(id);
            } else {
                // The subtree holds no task anywhere, at any depth, so
                // DeleteEpic's `requires` (every subtree task done) holds
                // vacuously and the whole subtree — this epic AND every
                // descendant, whatever THEIR status, e.g. a sub-epic that
                // itself was never archived — is doomed with it, exactly as
                // epics.allium's `doomed_epics = epic.subtree_epics + epic`
                // reads. A leftover non-archived child is not a defensive
                // edge case to route around: it goes with its emptied
                // parent's subtree, same as a real DeleteEpic call would
                // take it.
                delete_empty_epic_subtree(conn, id)?;
            }
        }
    }

    // Now that every 'archived' epic anywhere has been resolved (deleted or
    // settled to done), it is safe to recalculate: every row any of these
    // calls reads — the settled epic's own, its ancestors', its siblings' —
    // parses cleanly. Each settled epic's own recalculation walks up through
    // its ancestors on its own (`recalculate_epic_status_inner`'s recursive
    // parent step), so queuing just the settled ids here, not their
    // ancestors too, is enough.
    for id in settled_ids {
        let mut visited = std::collections::HashSet::new();
        super::queries::epics::recalculate_epic_status_inner(
            conn,
            crate::models::EpicId(id),
            &mut visited,
        )
        .context("v106: failed to recalculate settled epic status")?;
    }

    // Phase 5: rebuild the status CHECK constraints without `archived`, on
    // BOTH tables, so no later write can bring the value back. `tasks` has
    // enforced its `(status, sub_status)` CHECK since v16; `epics` gets a
    // status CHECK for the first time here — it never had one before, so
    // this also re-adds the v35 self-ref CHECK the rebuild would otherwise
    // drop.
    rebuild_tasks_table_with_check(
        conn,
        "CHECK (\n        \
             (status = 'backlog'  AND sub_status = 'none') OR\n        \
             (status = 'running'  AND sub_status IN ('active','needs_input','stale','stale_shell','crashed','conflict')) OR\n        \
             (status = 'review'   AND sub_status IN ('awaiting_review','changes_requested','approved','conflict','pr_closed','pr_unreachable')) OR\n        \
             (status = 'done'     AND sub_status = 'none')\n    )",
        "v106",
    )?;

    // v72/v76's two `tasks` triggers (`enforce_feed_task_subtree_unique_insert`/
    // `_update`) reference `epics` in their bodies. `ALTER TABLE ... RENAME TO`
    // re-resolves every trigger that mentions the renamed table, on whichever
    // table it is actually defined — that is what makes
    // `rebuild_tasks_table_with_check` (just above) safe to call on its own
    // schema history, and what makes rebuilding `epics` WITHOUT this drop
    // unsafe: mid-rebuild, between `DROP TABLE epics` and the rename back,
    // re-resolving these two tasks-side triggers fails with "no such table:
    // epics". Drop them first (capturing their exact SQL, not a hand-written
    // copy, so this cannot drift from whatever v72/v76 last left behind) and
    // recreate them once `epics` exists again under its real name.
    let subtree_trigger_sql: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT sql FROM sqlite_master \
             WHERE type = 'trigger' \
               AND name IN ('enforce_feed_task_subtree_unique_insert', \
                            'enforce_feed_task_subtree_unique_update') \
               AND sql IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()
            .context("v106: failed to capture feed-task-subtree trigger SQL")?
    };
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS enforce_feed_task_subtree_unique_insert;
         DROP TRIGGER IF EXISTS enforce_feed_task_subtree_unique_update;",
    )
    .context("v106: failed to drop feed-task-subtree triggers before rebuilding epics")?;

    rebuild_epics_table_with_check(
        conn,
        "CHECK (parent_epic_id != id),\n    \
         CHECK (status IN ('backlog', 'running', 'review', 'done'))",
        "v106",
    )?;

    for sql in &subtree_trigger_sql {
        conn.execute_batch(sql).with_context(|| {
            format!("v106: failed to recreate a feed-task-subtree trigger: {sql}")
        })?;
    }
    Ok(())
}

/// v106 phase 4's delete side: `root_id`'s subtree holds no task anywhere, so
/// `root_id` and every descendant epic — regardless of THAT descendant's own
/// status, e.g. a sub-epic that was never archived — are doomed together
/// (epics.allium's `DeleteEpic`: `doomed_epics = epic.subtree_epics + epic`).
///
/// Deletes deepest-first: each pass removes whichever subtree members
/// currently have no child epic left, so every `DELETE` stays FK-legal
/// (`epics.parent_epic_id` has no `ON DELETE CASCADE`). Bails out — leaving
/// whatever remains — rather than looping forever if a genuine
/// `parent_epic_id` cycle ever left no leaf to remove; no write path is
/// meant to create one.
fn delete_empty_epic_subtree(conn: &Connection, root_id: i64) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE subtree(id) AS (\
                 SELECT ?1 \
                 UNION \
                 SELECT e.id FROM epics e JOIN subtree s ON e.parent_epic_id = s.id\
             ) \
             SELECT id FROM subtree",
        )
        .context("v106: failed to prepare empty epic subtree scan")?;
    let mut remaining: std::collections::HashSet<i64> = stmt
        .query_map(params![root_id], |r| r.get::<_, i64>(0))
        .context("v106: failed to scan empty epic subtree")?
        .collect::<rusqlite::Result<_>>()
        .context("v106: failed to collect empty epic subtree")?;
    drop(stmt);

    while !remaining.is_empty() {
        let mut leaves = Vec::new();
        for &id in &remaining {
            let has_child: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM epics WHERE parent_epic_id = ?1",
                    params![id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if !has_child {
                leaves.push(id);
            }
        }
        if leaves.is_empty() {
            break;
        }
        for id in leaves {
            // `retired_feed_items.feed_epic_id` is declared `ON DELETE
            // CASCADE`, but the migration runner's `PRAGMA foreign_keys =
            // OFF` means that cascade never fires here — including for a
            // record phase 1, above, wrote under this very epic (an
            // archived, now-emptied feed epic is exactly the "nothing
            // survives to retire under" case core.allium's RetiredFeedItem
            // lifetime describes). Deleted explicitly instead, for every
            // doomed id, not just the subtree root — a nested feed sub-epic
            // has its own retirement records keyed to ITS id.
            if table_exists(conn, "retired_feed_items") {
                conn.execute(
                    "DELETE FROM retired_feed_items WHERE feed_epic_id = ?1",
                    params![id],
                )
                .context("v106: failed to purge retired_feed_items of a doomed epic")?;
            }
            conn.execute("DELETE FROM epics WHERE id = ?1", params![id])
                .context("v106: failed to delete an emptied epic subtree member")?;
            remaining.remove(&id);
        }
    }
    Ok(())
}
