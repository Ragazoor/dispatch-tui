use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::models::{EpicId, TaskId, TaskStatus};

use super::super::{Database, EpicPatch};
use super::{row_to_epic, row_to_task, TASK_COLUMNS};

impl super::super::EpicCrud for Database {
    fn create_epic(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        parent_epic_id: Option<EpicId>,
        project_id: crate::models::ProjectId,
    ) -> Result<crate::models::Epic> {
        let id =
            {
                let conn = self.conn()?;
                conn.execute(
                "INSERT INTO epics (title, description, repo_path, parent_epic_id, project_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![title, description, repo_path, parent_epic_id.map(|e| e.0), project_id.0],
            )
            .context("Failed to insert epic")?;
                EpicId(conn.last_insert_rowid())
            }; // MutexGuard dropped here — avoids deadlock when get_epic() re-locks
        self.get_epic(id)?
            .ok_or_else(|| anyhow::anyhow!("Epic {id} vanished after insert"))
    }

    fn get_epic(&self, id: EpicId) -> Result<Option<crate::models::Epic>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
             parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id \
             FROM epics WHERE id = ?1",
            params![id.0],
            row_to_epic,
        )
        .optional()
        .context("Failed to get epic")
    }

    fn list_epics(&self) -> Result<Vec<crate::models::Epic>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
                 parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id \
                 FROM epics ORDER BY COALESCE(sort_order, id) ASC, id ASC",
            )
            .context("Failed to prepare list_epics")?;
        let epics = stmt
            .query_map([], row_to_epic)
            .context("Failed to query epics")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect epics")?;
        Ok(epics)
    }

    fn list_root_epics(&self) -> Result<Vec<crate::models::Epic>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
                 parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id \
                 FROM epics WHERE parent_epic_id IS NULL ORDER BY COALESCE(sort_order, id) ASC, id ASC",
            )
            .context("Failed to prepare list_root_epics")?;
        let epics = stmt
            .query_map([], row_to_epic)
            .context("Failed to query root epics")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect root epics")?;
        Ok(epics)
    }

    fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<crate::models::Epic>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, \
                 parent_epic_id, feed_command, feed_interval_secs, created_at, updated_at, project_id \
                 FROM epics WHERE parent_epic_id = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC",
            )
            .context("Failed to prepare list_sub_epics")?;
        let epics = stmt
            .query_map(params![parent_id.0], row_to_epic)
            .context("Failed to query sub-epics")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect sub-epics")?;
        Ok(epics)
    }

    fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        let conn = self.conn()?;
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

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

        sets.push("updated_at = datetime('now')");
        values.push(Box::new(id.0));

        let sql = format!("UPDATE epics SET {} WHERE id = ?", sets.join(", "));
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows = conn
            .execute(&sql, refs.as_slice())
            .context("Failed to patch epic")?;
        if rows == 0 {
            anyhow::bail!("Epic {id} not found");
        }
        Ok(())
    }

    fn delete_epic(&self, id: EpicId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .context("Failed to begin transaction")?;
        let result = delete_epic_recursive(&conn, id);
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
    }

    fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        let conn = self.conn()?;
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
    }

    fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<crate::models::Task>> {
        let conn = self.conn()?;
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
    }

    fn list_all_tasks_with_epic_id(&self) -> Result<Vec<crate::models::Task>> {
        let conn = self.conn()?;
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
    }

    fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()> {
        let mut visited = HashSet::new();
        recalculate_epic_status_inner(self, epic_id, &mut visited)
    }
}

// ---------------------------------------------------------------------------
// recalculate_epic_status helper
// ---------------------------------------------------------------------------

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

/// Inner recursive helper for `EpicCrud::recalculate_epic_status`.
/// Threads a visited set to detect and break parent cycles, preventing
/// infinite recursion when `parent_epic_id` forms a cycle in the DB.
fn recalculate_epic_status_inner(
    db: &Database,
    epic_id: EpicId,
    visited: &mut HashSet<EpicId>,
) -> Result<()> {
    use super::super::EpicCrud;

    if !visited.insert(epic_id) {
        // Already processed this epic in the current chain — cycle detected.
        return Ok(());
    }

    let epic = match db.get_epic(epic_id)? {
        Some(e) => e,
        None => return Ok(()),
    };

    // Collect statuses from active tasks
    let task_statuses: Vec<TaskStatus> = db
        .list_tasks_for_epic(epic_id)?
        .into_iter()
        .filter(|t| t.status != TaskStatus::Archived)
        .map(|t| t.status)
        .collect();

    // Collect statuses from active sub-epics
    let sub_epic_statuses: Vec<TaskStatus> = db
        .list_sub_epics(epic_id)?
        .into_iter()
        .filter(|e| e.status != TaskStatus::Archived)
        .map(|e| e.status)
        .collect();

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
        db.patch_epic(epic_id, &EpicPatch::new().status(derived))?;
    }

    // Propagate upward to the parent epic if one exists
    if let Some(parent_id) = epic.parent_epic_id {
        recalculate_epic_status_inner(db, parent_id, visited)?;
    }

    Ok(())
}
