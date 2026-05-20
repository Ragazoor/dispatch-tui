use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::models::{EpicId, TaskId, TaskStatus};

use super::super::{Database, EpicPatch};
use super::{row_to_epic, row_to_task, TASK_COLUMNS};

#[async_trait::async_trait]
impl super::super::EpicCrud for Database {
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        parent_epic_id: Option<EpicId>,
        project_id: crate::models::ProjectId,
    ) -> Result<crate::models::Epic> {
        let title = title.to_string();
        let description = description.to_string();
        let repo_path = repo_path.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO epics (title, description, repo_path, parent_epic_id, project_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    title,
                    description,
                    repo_path,
                    parent_epic_id.map(|e| e.0),
                    project_id.0
                ],
            )
            .context("Failed to insert epic")?;
            let id = EpicId(conn.last_insert_rowid());
            get_epic_row(conn, id)?
                .ok_or_else(|| anyhow::anyhow!("Epic {id} vanished after insert"))
        })
        .await
    }

    async fn get_epic(&self, id: EpicId) -> Result<Option<crate::models::Epic>> {
        self.db_call(move |conn| get_epic_row(conn, id)).await
    }

    async fn list_epics(&self) -> Result<Vec<crate::models::Epic>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
                     parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id, group_by_repo \
                     FROM epics ORDER BY COALESCE(sort_order, id) ASC, id ASC",
                )
                .context("Failed to prepare list_epics")?;
            let epics = stmt
                .query_map([], row_to_epic)
                .context("Failed to query epics")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect epics")?;
            Ok(epics)
        })
        .await
    }

    async fn list_root_epics(&self) -> Result<Vec<crate::models::Epic>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
                     parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id, group_by_repo \
                     FROM epics WHERE parent_epic_id IS NULL ORDER BY COALESCE(sort_order, id) ASC, id ASC",
                )
                .context("Failed to prepare list_root_epics")?;
            let epics = stmt
                .query_map([], row_to_epic)
                .context("Failed to query root epics")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect root epics")?;
            Ok(epics)
        })
        .await
    }

    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<crate::models::Epic>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
                     parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id, group_by_repo \
                     FROM epics WHERE parent_epic_id = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC",
                )
                .context("Failed to prepare list_sub_epics")?;
            let epics = stmt
                .query_map(params![parent_id.0], row_to_epic)
                .context("Failed to query sub-epics")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect sub-epics")?;
            Ok(epics)
        })
        .await
    }

    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        // Materialise the patch into owned (sets, values) before crossing the
        // db_call boundary — `&EpicPatch<'_>` cannot be moved into a 'static
        // closure.
        let mut sets: Vec<&'static str> = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql + Send>> = Vec::new();

        if let Some(t) = patch.title {
            sets.push("title = ?");
            values.push(Box::new(t.to_string()));
        }
        if let Some(d) = patch.description {
            sets.push("description = ?");
            values.push(Box::new(d.to_string()));
        }
        if let Some(s) = patch.status {
            sets.push("status = ?");
            values.push(Box::new(s.as_str().to_string()));
        }
        if let Some(p) = patch.plan_path {
            sets.push("plan_path = ?");
            values.push(Box::new(p.map(|s| s.to_string())));
        }
        if let Some(so) = patch.sort_order {
            sets.push("sort_order = ?");
            values.push(Box::new(so));
        }
        if let Some(rp) = patch.repo_path {
            sets.push("repo_path = ?");
            values.push(Box::new(rp.to_string()));
        }
        if let Some(ad) = patch.auto_dispatch {
            sets.push("auto_dispatch = ?");
            values.push(Box::new(ad));
        }
        if let Some(gbr) = patch.group_by_repo {
            sets.push("group_by_repo = ?");
            values.push(Box::new(gbr));
        }
        if let Some(fc) = patch.feed_command {
            sets.push("feed_command = ?");
            values.push(Box::new(fc.map(|s| s.to_string())));
        }
        if let Some(fi) = patch.feed_interval_secs {
            sets.push("feed_interval_secs = ?");
            values.push(Box::new(fi));
        }
        if let Some(pid) = patch.project_id {
            sets.push("project_id = ?");
            values.push(Box::new(pid.0));
        }
        if let Some(peid) = patch.parent_epic_id {
            sets.push("parent_epic_id = ?");
            values.push(Box::new(peid.map(|e| e.0)));
        }

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
        self.db_call(move |conn| {
            conn.execute_batch("BEGIN IMMEDIATE")
                .context("Failed to begin transaction")?;
            let result = delete_epic_recursive(conn, id);
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
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        })
        .await
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
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

    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<crate::models::Task>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(
                    &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE epic_id = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC"),
                )
                .context("Failed to prepare list_tasks_for_epic")?;
            let tasks = stmt
                .query_map(params![epic_id.0], row_to_task)
                .context("Failed to query tasks for epic")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect tasks for epic")?;
            Ok(tasks)
        })
        .await
    }

    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<crate::models::Task>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {TASK_COLUMNS} FROM tasks WHERE epic_id IS NOT NULL ORDER BY epic_id ASC, COALESCE(sort_order, id) ASC, id ASC"
                ))
                .context("Failed to prepare list_all_tasks_with_epic_id")?;
            let tasks = stmt
                .query_map([], row_to_task)
                .context("Failed to query tasks with epic_id")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect tasks with epic_id")?;
            Ok(tasks)
        })
        .await
    }

    async fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()> {
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
        "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
         parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id, group_by_repo \
         FROM epics WHERE id = ?1",
        params![id.0],
        row_to_epic,
    )
    .optional()
    .context("Failed to get epic")
}

/// Recursively deletes sub-epics and their tasks, then deletes the epic row.
/// Returns the number of rows deleted for the root epic (0 = not found).
/// Caller must hold the connection lock and manage the transaction.
fn delete_epic_recursive(conn: &rusqlite::Connection, id: EpicId) -> Result<usize> {
    // Find direct children — collect fully before dropping the statement
    let mut stmt = conn
        .prepare("SELECT id FROM epics WHERE parent_epic_id = ?1")
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
fn recalculate_epic_status_inner(
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

    // Active task statuses for this epic
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE epic_id = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC"
        ))
        .context("Failed to prepare list_tasks_for_epic (recalc)")?;
    let task_statuses: Vec<TaskStatus> = stmt
        .query_map(params![epic_id.0], row_to_task)
        .context("Failed to query tasks for epic (recalc)")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect tasks for epic (recalc)")?
        .into_iter()
        .filter(|t| t.status != TaskStatus::Archived)
        .map(|t| t.status)
        .collect();
    drop(stmt);

    // Active sub-epic statuses
    let mut stmt = conn
        .prepare(
            "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
             parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id, group_by_repo \
             FROM epics WHERE parent_epic_id = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC",
        )
        .context("Failed to prepare list_sub_epics (recalc)")?;
    let sub_epic_statuses: Vec<TaskStatus> = stmt
        .query_map(params![epic_id.0], row_to_epic)
        .context("Failed to query sub-epics (recalc)")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect sub-epics (recalc)")?
        .into_iter()
        .filter(|e| e.status != TaskStatus::Archived)
        .map(|e| e.status)
        .collect();
    drop(stmt);

    let all_statuses: Vec<TaskStatus> =
        task_statuses.into_iter().chain(sub_epic_statuses).collect();

    let derived = if all_statuses.is_empty() {
        TaskStatus::Backlog
    } else if all_statuses.iter().all(|s| *s == TaskStatus::Done) {
        TaskStatus::Done
    } else if all_statuses.contains(&TaskStatus::Review) {
        TaskStatus::Review
    } else if all_statuses.contains(&TaskStatus::Running) {
        TaskStatus::Running
    } else {
        TaskStatus::Backlog
    };

    if derived != epic.status {
        let rows = conn
            .execute(
                "UPDATE epics SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![derived.as_str(), epic_id.0],
            )
            .context("Failed to update epic status (recalc)")?;
        if rows == 0 {
            anyhow::bail!("Epic {epic_id} not found");
        }
    }

    if let Some(parent_id) = epic.parent_epic_id {
        recalculate_epic_status_inner(conn, parent_id, visited)?;
    }

    Ok(())
}
