use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use rusqlite::{params, OptionalExtension};

use crate::models::{EpicId, FeedItem, ProjectId, SubStatus, TaskId, TaskStatus, TaskUsage, UsageReport};

use super::super::{CreateTaskRequest, Database, TaskPatch};
use super::{row_to_task, TASK_COLUMNS};

impl super::super::TaskCrud for Database {
    fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        let conn = self.conn()?;
        let sub_status = SubStatus::default_for(req.status);
        conn.execute(
            "INSERT INTO tasks \
             (title, description, repo_path, plan_path, status, sub_status, base_branch, \
              epic_id, sort_order, tag, project_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                req.title,
                req.description,
                req.repo_path,
                req.plan,
                req.status.as_str(),
                sub_status.as_str(),
                req.base_branch,
                req.epic_id.map(|e| e.0),
                req.sort_order,
                req.tag.map(|t| t.as_str()),
                req.project_id.0,
            ],
        )
        .context("Failed to insert task")?;
        Ok(TaskId(conn.last_insert_rowid()))
    }

    fn get_task(&self, id: TaskId) -> Result<Option<crate::models::Task>> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
            params![id.0],
            row_to_task,
        )
        .optional()
        .context("Failed to get task")
    }

    fn list_all(&self) -> Result<Vec<crate::models::Task>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM tasks ORDER BY COALESCE(sort_order, id) ASC, id ASC"
            ))
            .context("Failed to prepare list_all")?;
        let tasks = stmt
            .query_map([], row_to_task)
            .context("Failed to query tasks")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect tasks")?;
        Ok(tasks)
    }

    fn list_by_status(&self, status: TaskStatus) -> Result<Vec<crate::models::Task>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE status = ?1 ORDER BY COALESCE(sort_order, id) ASC, id ASC"),
            )
            .context("Failed to prepare list_by_status")?;
        let tasks = stmt
            .query_map(params![status.as_str()], row_to_task)
            .context("Failed to query tasks by status")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect tasks by status")?;
        Ok(tasks)
    }

    fn update_status_if(
        &self,
        id: TaskId,
        new_status: TaskStatus,
        expected: TaskStatus,
    ) -> Result<bool> {
        let conn = self.conn()?;
        let default_sub = SubStatus::default_for(new_status);
        let rows = conn
            .execute(
                "UPDATE tasks SET status = ?1, sub_status = ?2, updated_at = datetime('now') WHERE id = ?3 AND status = ?4",
                params![new_status.as_str(), default_sub.as_str(), id.0, expected.as_str()],
            )
            .context("Failed to conditional-update status")?;
        Ok(rows > 0)
    }

    fn delete_task(&self, id: TaskId) -> Result<()> {
        let conn = self.conn()?;
        let rows = conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![id.0])
            .context("Failed to delete task")?;
        if rows == 0 {
            anyhow::bail!("Task {} not found", id);
        }
        Ok(())
    }

    fn find_task_by_plan(&self, plan: &str) -> Result<Option<crate::models::Task>> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE plan_path = ?1"),
            params![plan],
            row_to_task,
        )
        .optional()
        .context("Failed to find task by plan")
    }

    fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        if matches!((patch.status, patch.sub_status), (Some(s), Some(ss)) if !ss.is_valid_for(s)) {
            anyhow::bail!(
                "invalid (status, sub_status) pair in patch: {:?}/{:?}",
                patch.status,
                patch.sub_status
            );
        }
        let conn = self.conn()?;
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(s) = patch.status {
            sets.push("status = ?");
            values.push(Box::new(s.as_str().to_string()));
        }
        let effective_sub_status = patch
            .sub_status
            .or_else(|| patch.status.map(SubStatus::default_for));
        if let Some(t) = patch.title {
            sets.push("title = ?");
            values.push(Box::new(t.to_string()));
        }
        if let Some(d) = patch.description {
            sets.push("description = ?");
            values.push(Box::new(d.to_string()));
        }
        if let Some(r) = patch.repo_path {
            sets.push("repo_path = ?");
            values.push(Box::new(r.to_string()));
        }
        if let Some(p) = patch.plan_path {
            sets.push("plan_path = ?");
            values.push(Box::new(p.map(|s| s.to_string())));
        }
        if let Some(w) = patch.worktree {
            sets.push("worktree = ?");
            values.push(Box::new(w.map(|s| s.to_string())));
        }
        if let Some(t) = patch.tmux_window {
            sets.push("tmux_window = ?");
            values.push(Box::new(t.map(|s| s.to_string())));
        }
        if let Some(ss) = effective_sub_status {
            sets.push("sub_status = ?");
            values.push(Box::new(ss.as_str().to_string()));
        }
        if let Some(url) = &patch.pr_url {
            sets.push("pr_url = ?");
            values.push(Box::new(url.map(|s| s.to_string())));
        }
        if let Some(tag) = &patch.tag {
            sets.push("tag = ?");
            values.push(Box::new(tag.map(|t| t.as_str().to_string())));
        }
        if let Some(so) = patch.sort_order {
            sets.push("sort_order = ?");
            values.push(Box::new(so));
        }
        if let Some(bb) = patch.base_branch {
            sets.push("base_branch = ?");
            values.push(Box::new(bb.to_string()));
        }
        if let Some(eid) = patch.external_id {
            sets.push("external_id = ?");
            values.push(Box::new(eid.map(|s| s.to_string())));
        }
        if let Some(pid) = patch.project_id {
            sets.push("project_id = ?");
            values.push(Box::new(pid.0));
        }

        sets.push("updated_at = datetime('now')");
        values.push(Box::new(id.0));

        let sql = format!("UPDATE tasks SET {} WHERE id = ?", sets.join(", "));
        let refs: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let rows = conn
            .execute(&sql, refs.as_slice())
            .context("Failed to patch task")?;
        if rows == 0 {
            anyhow::bail!("Task {id} not found");
        }
        Ok(())
    }

    fn has_other_tasks_with_worktree(&self, worktree: &str, exclude_id: TaskId) -> Result<bool> {
        let conn = self.conn()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE worktree = ?1 AND id != ?2 AND status != 'done'",
                params![worktree, exclude_id.0],
                |row| row.get(0),
            )
            .context("Failed to check shared worktree")?;
        Ok(count > 0)
    }

    fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO task_usage
                 (task_id, input_tokens, output_tokens,
                  cache_read_tokens, cache_write_tokens, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
             ON CONFLICT(task_id) DO UPDATE SET
                 input_tokens       = input_tokens       + excluded.input_tokens,
                 output_tokens      = output_tokens      + excluded.output_tokens,
                 cache_read_tokens  = cache_read_tokens  + excluded.cache_read_tokens,
                 cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                 updated_at         = excluded.updated_at",
            params![
                task_id.0,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens
            ],
        )
        .context("Failed to upsert task_usage")?;
        Ok(())
    }

    fn get_all_usage(&self) -> Result<Vec<TaskUsage>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT task_id, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, updated_at
             FROM task_usage",
            )
            .context("Failed to prepare get_all_usage")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .context("Failed to query task_usage")?;
        let mut out = Vec::new();
        for row in rows {
            let (task_id, input, output, cr, cw, updated_at_str) =
                row.context("Failed to read usage row")?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at_str, "%Y-%m-%d %H:%M:%S")
                .with_context(|| format!("Invalid updated_at in task_usage: {updated_at_str:?}"))?
                .and_utc();
            out.push(TaskUsage {
                task_id: TaskId(task_id),
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cr,
                cache_write_tokens: cw,
                updated_at,
            });
        }
        Ok(out)
    }

    fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<()> {
        let conn = self.conn()?;
        let project_id: ProjectId = conn
            .query_row(
                "SELECT project_id FROM epics WHERE id = ?1",
                params![epic_id.0],
                |row| row.get::<_, i64>(0).map(ProjectId),
            )
            .with_context(|| format!("Epic {} not found for upsert_feed_tasks", epic_id))?;

        let tx = conn.unchecked_transaction()?;

        for ((item, repo_path), base_branch) in items
            .iter()
            .zip(repo_paths.iter())
            .zip(base_branches.iter())
        {
            let sub_status = SubStatus::default_for(item.status).as_str().to_string();
            tx.execute(
                "INSERT INTO tasks
                     (title, description, repo_path, status, sub_status, base_branch,
                      epic_id, external_id, project_id, tag)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(epic_id, external_id) WHERE external_id IS NOT NULL
                 DO UPDATE SET
                     title       = excluded.title,
                     description = excluded.description,
                     tag         = excluded.tag,
                     updated_at  = datetime('now')",
                params![
                    item.title,
                    item.description,
                    repo_path,
                    item.status.as_str(),
                    sub_status,
                    base_branch,
                    epic_id.0,
                    item.external_id,
                    project_id.0,
                    item.tag.as_str(),
                ],
            )
            .with_context(|| format!("Failed to upsert feed task '{}'", item.external_id))?;
        }

        let keep_ids =
            serde_json::to_string(&items.iter().map(|i| &i.external_id).collect::<Vec<_>>())
                .expect("Vec<String> serialization is infallible");
        tx.execute(
            "DELETE FROM tasks
             WHERE epic_id = ?1
               AND external_id IS NOT NULL
               AND external_id NOT IN (SELECT value FROM json_each(?2))",
            params![epic_id.0, keep_ids],
        )
        .context("Failed to delete stale feed tasks")?;

        tx.commit()?;
        Ok(())
    }
}
