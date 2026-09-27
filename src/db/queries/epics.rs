use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use crate::set_field;

use crate::models::{completed_at_for_status_transition, EpicId, TaskId, TaskStatus};

use super::super::{Database, EpicPatch};
use super::{collect_decodable, row_to_epic, row_to_task, EPIC_COLUMNS, TASK_COLUMNS};

#[async_trait::async_trait]
impl super::super::EpicRead for Database {
    async fn get_epic(&self, id: EpicId) -> Result<Option<crate::models::Epic>> {
        if let Some(reader) = self.shared_reader() {
            return reader.get_epic(id).await;
        }
        self.db_call_read(move |conn| get_epic_row(conn, id)).await
    }

    async fn list_epics(&self) -> Result<Vec<crate::models::Epic>> {
        if let Some(reader) = self.shared_reader() {
            return reader.list_epics().await;
        }
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare_cached(&format!(
                    "SELECT {EPIC_COLUMNS} FROM epics ORDER BY COALESCE(sort_order, id) ASC, id ASC"
                ))
                .context("Failed to prepare list_epics")?;
            let rows = stmt
                .query_map([], row_to_epic)
                .context("Failed to query epics")?;
            let epics = collect_decodable(rows, "epics").context("Failed to collect epics")?;
            Ok(epics)
        })
        .await
    }

    async fn list_root_epics(&self) -> Result<Vec<crate::models::Epic>> {
        if let Some(reader) = self.shared_reader() {
            return reader.list_epics_with_parent(None).await;
        }
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare_cached(&format!(
                    "SELECT {EPIC_COLUMNS} FROM epics WHERE parent_epic_id IS NULL \
                     ORDER BY COALESCE(sort_order, id) ASC, id ASC"
                ))
                .context("Failed to prepare list_root_epics")?;
            let rows = stmt
                .query_map([], row_to_epic)
                .context("Failed to query root epics")?;
            let epics = collect_decodable(rows, "epics").context("Failed to collect root epics")?;
            Ok(epics)
        })
        .await
    }

    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<crate::models::Epic>> {
        if let Some(reader) = self.shared_reader() {
            return reader.list_epics_with_parent(Some(parent_id)).await;
        }
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare_cached(&format!(
                    "SELECT {EPIC_COLUMNS} FROM epics WHERE parent_epic_id = ?1 \
                     ORDER BY COALESCE(sort_order, id) ASC, id ASC"
                ))
                .context("Failed to prepare list_sub_epics")?;
            let rows = stmt
                .query_map(params![parent_id.0], row_to_epic)
                .context("Failed to query sub-epics")?;
            let epics = collect_decodable(rows, "epics").context("Failed to collect sub-epics")?;
            Ok(epics)
        })
        .await
    }

    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<crate::models::Task>> {
        if let Some(reader) = self.shared_reader() {
            return reader.list_tasks_for_epic(epic_id).await;
        }
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE epic_id = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC"),
                )
                .context("Failed to prepare list_tasks_for_epic")?;
            let rows = stmt
                .query_map(params![epic_id.0], row_to_task)
                .context("Failed to query tasks for epic")?;
            let tasks =
                collect_decodable(rows, "tasks").context("Failed to collect tasks for epic")?;
            Ok(tasks)
        })
        .await
    }

    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<crate::models::Task>> {
        if let Some(reader) = self.shared_reader() {
            return reader.list_all_tasks_with_epic_id().await;
        }
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare_cached(&format!(
                    "SELECT {TASK_COLUMNS} FROM tasks WHERE epic_id IS NOT NULL ORDER BY epic_id ASC, COALESCE(sort_order, id) ASC, id ASC"
                ))
                .context("Failed to prepare list_all_tasks_with_epic_id")?;
            let rows = stmt
                .query_map([], row_to_task)
                .context("Failed to query tasks with epic_id")?;
            let tasks =
                collect_decodable(rows, "tasks").context("Failed to collect tasks with epic_id")?;
            Ok(tasks)
        })
        .await
    }
}

#[async_trait::async_trait]
impl super::super::EpicCrud for Database {
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<crate::models::Epic> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        if let Some(writer) = self.shared_writer() {
            return writer.create_epic(title, description, parent_epic_id).await;
        }
        let title = title.to_string();
        let description = description.to_string();
        self.db_call(move |conn| {
            // auto_dispatch is set explicitly to 0 (false): new epics do not
            // auto-dispatch by default — it is an opt-in toggled with `U` in
            // the epic view. Specifying it here makes the default independent
            // of the column's historical `DEFAULT 1`.
            conn.execute(
                "INSERT INTO epics (title, description, parent_epic_id, auto_dispatch) \
                 VALUES (?1, ?2, ?3, 0)",
                params![title, description, parent_epic_id.map(|e| e.0),],
            )
            .context("Failed to insert epic")?;
            let id = EpicId(conn.last_insert_rowid());
            get_epic_row(conn, id)?
                .ok_or_else(|| anyhow::anyhow!("Epic {id} vanished after insert"))
        })
        .await
    }

    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId> {
        if let Some(writer) = self.shared_writer() {
            return writer.create_repo_group_sub_epic(parent_id, title).await;
        }
        let title = title.to_string();
        self.db_call(move |conn| {
            // Reuse an existing RepoGroup sub-epic of this (parent, title),
            // regardless of status; unarchive it if needed.
            if let Some(id) = conn
                .query_row(
                    "SELECT id FROM epics \
                     WHERE parent_epic_id = ?1 AND title = ?2 AND origin = 'repo-group'",
                    params![parent_id.0, title],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .context("lookup repo-group sub-epic")?
            {
                conn.execute(
                    "UPDATE epics SET status = 'backlog', updated_at = datetime('now') \
                     WHERE id = ?1 AND status = 'archived'",
                    params![id],
                )
                .context("unarchive repo-group sub-epic")?;
                return Ok(EpicId(id));
            }

            // Create it. auto_dispatch=0, group_by_repo=0, origin='repo-group'.
            match conn.execute(
                "INSERT INTO epics (title, description, parent_epic_id, auto_dispatch, group_by_repo, origin) \
                 VALUES (?1, '', ?2, 0, 0, 'repo-group')",
                params![title, parent_id.0],
            ) {
                Ok(_) => Ok(EpicId(conn.last_insert_rowid())),
                // Lost a race: another writer inserted the same (parent,title).
                // The partial unique index rejected us — re-select and return it.
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    let id = conn
                        .query_row(
                            "SELECT id FROM epics \
                             WHERE parent_epic_id = ?1 AND title = ?2 AND origin = 'repo-group'",
                            params![parent_id.0, title],
                            |r| r.get::<_, i64>(0),
                        )
                        .context("re-select after unique violation")?;
                    Ok(EpicId(id))
                }
                Err(e) => Err(anyhow::Error::from(e).context("insert repo-group sub-epic")),
            }
        })
        .await
    }

    async fn create_managed_role_epic(
        &self,
        title: &str,
        parent_epic_id: Option<EpicId>,
        role: crate::models::FeedRole,
        feed_command: Option<&str>,
        feed_interval_secs: Option<i64>,
    ) -> Result<EpicId> {
        if let Some(writer) = self.shared_writer() {
            return writer
                .create_managed_role_epic(
                    title,
                    parent_epic_id,
                    role,
                    feed_command,
                    feed_interval_secs,
                )
                .await;
        }
        let title = title.to_string();
        let role_str = role.as_str();
        let feed_command = feed_command.map(|c| c.to_string());
        self.db_call(move |conn| {
            match conn.execute(
                "INSERT INTO epics \
                     (title, description, parent_epic_id, auto_dispatch, feed_role, feed_command, feed_interval_secs) \
                 VALUES (?1, '', ?2, 0, ?3, ?4, ?5)",
                params![
                    title,
                    parent_epic_id.map(|e| e.0),
                    role_str,
                    feed_command,
                    feed_interval_secs,
                ],
            ) {
                Ok(_) => Ok(EpicId(conn.last_insert_rowid())),
                // Lost a race: another writer inserted the same (parent,role)
                // first. The partial unique index rejected us — re-select and
                // return the winner's id, a true no-op.
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    let id = conn
                        .query_row(
                            "SELECT id FROM epics \
                             WHERE parent_epic_id IS ?1 AND feed_role = ?2",
                            params![parent_epic_id.map(|e| e.0), role_str],
                            |r| r.get::<_, i64>(0),
                        )
                        .context("re-select after unique violation")?;
                    Ok(EpicId(id))
                }
                Err(e) => Err(anyhow::Error::from(e).context("insert managed-role epic")),
            }
        })
        .await
    }

    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        if let Some(writer) = self.shared_writer() {
            return writer.patch_epic(id, patch).await;
        }
        // Materialise the patch into owned (sets, values) before crossing the
        // db_call boundary — `&EpicPatch<'_>` cannot be moved into a 'static
        // closure.
        let mut sets: Vec<&'static str> = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql + Send>> = Vec::new();

        set_field!(sets, values, patch.title.map(str::to_string), "title");
        set_field!(
            sets,
            values,
            patch.description.map(str::to_string),
            "description"
        );
        set_field!(
            sets,
            values,
            patch.status.map(|s| s.as_str().to_string()),
            "status"
        );
        set_field!(
            sets,
            values,
            patch.plan_path.map(|opt| opt.map(str::to_string)),
            "plan_path"
        );
        set_field!(sets, values, patch.sort_order, "sort_order");
        // Millisecond precision: the Done column orders on this field, and
        // whole seconds tie too often for a bulk close (see
        // `completed_at_for_status_transition`).
        set_field!(
            sets,
            values,
            patch
                .completed_at
                .map(|opt| opt.map(super::format_datetime_millis)),
            "completed_at"
        );
        set_field!(sets, values, patch.auto_dispatch, "auto_dispatch");
        set_field!(sets, values, patch.group_by_repo, "group_by_repo");
        set_field!(sets, values, patch.feed_append_only, "feed_append_only");
        set_field!(
            sets,
            values,
            patch.feed_role.map(|r| r.as_str().to_string()),
            "feed_role"
        );
        set_field!(
            sets,
            values,
            patch.origin.map(|o| o.as_str().to_string()),
            "origin"
        );
        set_field!(
            sets,
            values,
            patch.feed_command.map(|opt| opt.map(str::to_string)),
            "feed_command"
        );
        set_field!(sets, values, patch.feed_interval_secs, "feed_interval_secs");
        set_field!(
            sets,
            values,
            patch.parent_epic_id.map(|opt| opt.map(|e| e.0)),
            "parent_epic_id"
        );

        sets.push("updated_at = datetime('now')");
        values.push(Box::new(id.0));

        let sql = format!("UPDATE epics SET {} WHERE id = ?", sets.join(", "));

        self.db_call(move |conn| {
            let refs: Vec<&dyn rusqlite::types::ToSql> = values
                .iter()
                .map(|v| v.as_ref() as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = conn
                .execute(&sql, refs.as_slice())
                .context("Failed to patch epic")?;
            if rows == 0 {
                anyhow::bail!("Epic {id} not found");
            }
            Ok(())
        })
        .await
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        if let Some(writer) = self.shared_writer() {
            return writer.delete_epic(id).await;
        }
        self.db_call(move |conn| {
            conn.execute_batch("BEGIN IMMEDIATE")
                .context("Failed to begin transaction")?;
            let result = retire_feed_tasks_before_epic_delete(conn, id)
                .and_then(|_| delete_epic_recursive(conn, id));
            match result {
                Ok(rows) => {
                    conn.execute_batch("COMMIT")
                        .context("Failed to commit delete_epic transaction")?;
                    if rows == 0 {
                        anyhow::bail!("Epic {} not found", id);
                    }
                    Ok(())
                }
                Err(e) => {
                    conn.execute_batch("ROLLBACK").ok(); // ignore rollback error; preserves and returns the original error
                    Err(e)
                }
            }
        })
        .await
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        if let Some(writer) = self.shared_writer() {
            return writer.set_task_epic_id(task_id, epic_id).await;
        }
        self.db_call(move |conn| {
            let rows = conn
                .execute(
                    "UPDATE tasks SET epic_id = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![epic_id.map(|e| e.0), task_id.0],
                )
                .context("Failed to set task epic_id")?;
            if rows == 0 {
                anyhow::bail!("Task {} not found", task_id);
            }
            Ok(())
        })
        .await
    }

    async fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()> {
        // ROUTED, and on a store this is the ONLY place it can run: the
        // derivation needs every child, and no board subscribes to all of them
        // (`epics.allium: EpicStatusRecalculation`).
        if let Some(writer) = self.shared_writer() {
            return writer.recalculate_epic_status(epic_id).await;
        }
        // Run the entire recursive walk inside a single db_call closure so the
        // recursion stays sync on the dedicated tokio_rusqlite thread. This
        // avoids the need for Box<dyn Future> to recurse through async fn.
        self.db_call(move |conn| {
            let mut visited = HashSet::new();
            recalculate_epic_status_inner(conn, epic_id, &mut visited)
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// shared sync helpers (run inside db_call closures or hold the sync mutex)
// ---------------------------------------------------------------------------

fn get_epic_row(conn: &rusqlite::Connection, id: EpicId) -> Result<Option<crate::models::Epic>> {
    conn.query_row(
        &format!("SELECT {EPIC_COLUMNS} FROM epics WHERE id = ?1"),
        params![id.0],
        row_to_epic,
    )
    .optional()
    .context("Failed to get epic")
}

/// epics.allium: `DeleteEpic`'s retirement clause. For every feed task
/// anywhere in `id`'s about-to-be-deleted subtree, write a
/// `retired_feed_items` row keyed on its `nearest_feed_epic` — UNLESS that
/// epic is itself part of the doomed subtree, in which case there is nothing
/// surviving to retire under (the feed epic's own delete is a reset, and its
/// existing records go with it via the table's `ON DELETE CASCADE`). Must run
/// BEFORE `delete_epic_recursive` in the same transaction: reading the
/// subtree's tasks/epics after they are gone would see nothing.
fn retire_feed_tasks_before_epic_delete(conn: &rusqlite::Connection, id: EpicId) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE doomed(id) AS (\
                 SELECT ?1 \
                 UNION \
                 SELECT e.id FROM epics e JOIN doomed d ON e.parent_epic_id = d.id\
             ) \
             SELECT id FROM doomed",
        )
        .context("Failed to prepare doomed-epic scan for delete_epic retirement")?;
    let doomed: HashSet<i64> = stmt
        .query_map(params![id.0], |r| r.get::<_, i64>(0))
        .context("Failed to scan doomed epics")?
        .collect::<rusqlite::Result<_>>()
        .context("Failed to collect doomed epics")?;
    drop(stmt);

    let doomed_list = doomed
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT t.epic_id, t.external_id FROM tasks t \
             WHERE t.epic_id IN ({doomed_list}) AND t.external_id IS NOT NULL"
        ))
        .context("Failed to prepare doomed feed task scan for delete_epic retirement")?;
    let tasks: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .context("Failed to scan doomed feed tasks")?
        .collect::<rusqlite::Result<_>>()
        .context("Failed to collect doomed feed tasks")?;
    drop(stmt);

    for (epic_id, external_id) in tasks {
        let feed_epic_id: Option<i64> = conn
            .query_row(
                "WITH RECURSIVE chain(id, feed_command, parent_epic_id) AS (\
                     SELECT id, feed_command, parent_epic_id FROM epics WHERE id = ?1 \
                     UNION \
                     SELECT e.id, e.feed_command, e.parent_epic_id \
                     FROM epics e JOIN chain c ON e.id = c.parent_epic_id\
                 ) \
                 SELECT id FROM chain WHERE feed_command IS NOT NULL LIMIT 1",
                params![epic_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .context("Failed to resolve nearest_feed_epic for delete_epic retirement")?;
        let Some(feed_epic_id) = feed_epic_id else {
            continue;
        };
        // The feed epic itself is being deleted: nothing survives to retire
        // under. Its existing records go with it via ON DELETE CASCADE.
        if doomed.contains(&feed_epic_id) {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO retired_feed_items (feed_epic_id, external_id) \
             VALUES (?1, ?2)",
            params![feed_epic_id, external_id],
        )
        .context("Failed to write retired_feed_item on delete_epic")?;
    }

    Ok(())
}

/// Recursively deletes sub-epics and their tasks, then deletes the epic row.
/// Returns the number of rows deleted for the root epic (0 = not found).
/// Caller must hold the connection lock and manage the transaction.
fn delete_epic_recursive(conn: &rusqlite::Connection, id: EpicId) -> Result<usize> {
    // Find direct children — collect fully before dropping the statement
    let mut stmt = conn
        .prepare_cached("SELECT id FROM epics WHERE parent_epic_id = ?1")
        .context("Failed to prepare child epic query")?;
    let child_ids: Vec<EpicId> = stmt
        .query_map(params![id.0], |row| row.get::<_, i64>(0))
        .context("Failed to query child epics")?
        .map(|r| r.map(EpicId))
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to collect child epic ids")?;
    drop(stmt);
    for child_id in child_ids {
        delete_epic_recursive(conn, child_id)?;
    }
    // task-watchers.allium: a deleted epic's subtasks are row removals exactly
    // like DeleteTask, so they owe the same watch-row cleanup — a
    // pre-existing gap (task #4971's design doc), fixed in passing. Collected
    // before the DELETE below removes the rows this scopes on.
    let task_ids: Vec<i64> = {
        let mut stmt = conn
            .prepare_cached("SELECT id FROM tasks WHERE epic_id = ?1")
            .context("Failed to prepare epic subtask id query")?;
        let ids = stmt
            .query_map(params![id.0], |row| row.get::<_, i64>(0))
            .context("Failed to query epic subtask ids")?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to collect epic subtask ids")?;
        ids
    };
    for task_id in &task_ids {
        conn.execute(
            "DELETE FROM task_watchers WHERE target_task_id = ?1",
            params![task_id],
        )
        .context("Failed to delete watches of a deleted epic's subtask target")?;
        conn.execute(
            "DELETE FROM task_watchers WHERE watcher_task_id = ?1",
            params![task_id],
        )
        .context("Failed to delete watches by a deleted epic's subtask watcher")?;
    }
    conn.execute("DELETE FROM tasks WHERE epic_id = ?1", params![id.0])
        .context("Failed to delete epic subtasks")?;
    conn.execute("DELETE FROM epics WHERE id = ?1", params![id.0])
        .context("Failed to delete epic")
}

/// Inner recursive helper for `EpicCrud::recalculate_epic_status`. Runs sync
/// against a single `&rusqlite::Connection` (the dedicated tokio_rusqlite
/// thread) so we can use plain recursion instead of async recursion.
///
/// Threads a visited set to detect and break parent cycles, preventing
/// infinite recursion when `parent_epic_id` forms a cycle in the DB.
pub(in crate::db) fn recalculate_epic_status_inner(
    conn: &rusqlite::Connection,
    epic_id: EpicId,
    visited: &mut HashSet<EpicId>,
) -> Result<()> {
    if !visited.insert(epic_id) {
        return Ok(());
    }

    let epic = match get_epic_row(conn, epic_id)? {
        Some(e) => e,
        None => return Ok(()),
    };

    // Active task statuses for this epic — project only the status column
    let mut stmt = conn
        .prepare_cached("SELECT status FROM tasks WHERE epic_id = ?1 AND status != 'archived'")
        .context("Failed to prepare task status query (recalc)")?;
    let task_statuses: Vec<TaskStatus> = stmt
        .query_map(params![epic_id.0], |row| row.get::<_, String>(0))
        .context("Failed to query task statuses (recalc)")?
        .map(|r| {
            r.map_err(anyhow::Error::from).and_then(|s| {
                TaskStatus::parse(&s)
                    .ok_or_else(|| anyhow::anyhow!("unknown task status {s:?} in recalc"))
            })
        })
        .collect::<Result<Vec<_>>>()
        .context("Failed to collect task statuses (recalc)")?;
    drop(stmt);

    // Active sub-epic statuses — project only the status column
    let mut stmt = conn
        .prepare_cached(
            "SELECT status FROM epics WHERE parent_epic_id = ?1 AND status != 'archived'",
        )
        .context("Failed to prepare sub-epic status query (recalc)")?;
    let sub_epic_statuses: Vec<TaskStatus> = stmt
        .query_map(params![epic_id.0], |row| row.get::<_, String>(0))
        .context("Failed to query sub-epic statuses (recalc)")?
        .map(|r| {
            r.map_err(anyhow::Error::from).and_then(|s| {
                TaskStatus::parse(&s)
                    .ok_or_else(|| anyhow::anyhow!("unknown epic status {s:?} in recalc"))
            })
        })
        .collect::<Result<Vec<_>>>()
        .context("Failed to collect sub-epic statuses (recalc)")?;
    drop(stmt);

    let all_statuses: Vec<TaskStatus> =
        task_statuses.into_iter().chain(sub_epic_statuses).collect();

    let target = if epic.status == TaskStatus::Archived {
        // Archived is terminal for this recalculation: an archived epic must
        // never be flipped back to `Done` just because a newly-attached
        // child happens to be all-done.
        epic.status
    } else if all_statuses.is_empty() {
        epic.status
    } else if all_statuses.iter().all(|s| *s == TaskStatus::Done) {
        TaskStatus::Done
    } else if epic.status == TaskStatus::Done {
        // regression: done epic has active non-done children
        TaskStatus::Backlog
    } else {
        epic.status
    };

    if target != epic.status {
        let now = Utc::now();
        let rows = match completed_at_for_status_transition(epic.status, target, now) {
            Some(completed_at) => conn
                .execute(
                    "UPDATE epics SET status = ?1, completed_at = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![
                        target.as_str(),
                        super::format_datetime_millis(completed_at),
                        epic_id.0
                    ],
                )
                .context("Failed to update epic status (recalc)")?,
            None => conn
                .execute(
                    "UPDATE epics SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![target.as_str(), epic_id.0],
                )
                .context("Failed to update epic status (recalc)")?,
        };
        if rows == 0 {
            anyhow::bail!("Epic {epic_id} not found");
        }
    }

    if let Some(parent_id) = epic.parent_epic_id {
        recalculate_epic_status_inner(conn, parent_id, visited)?;
    }

    Ok(())
}
