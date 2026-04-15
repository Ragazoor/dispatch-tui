use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, OptionalExtension};

use crate::models::{
    CiStatus, Epic, EpicId, ReviewAgentStatus, ReviewDecision, ReviewPr, Reviewer, SubStatus, Task,
    TaskId, TaskStatus, TaskTag, TaskUsage, UsageReport,
};

use super::{Database, EpicPatch, TaskPatch};

/// Column list shared by all task SELECT queries. Pair with `row_to_task`.
const TASK_COLUMNS: &str = "id, title, description, repo_path, status, worktree, tmux_window, \
     plan_path, epic_id, sub_status, pr_url, tag, sort_order, base_branch, created_at, updated_at";

impl super::TaskCrud for Database {
    fn create_task(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        plan: Option<&str>,
        status: TaskStatus,
        base_branch: &str,
    ) -> Result<TaskId> {
        let conn = self.conn()?;
        let sub_status = SubStatus::default_for(status);
        conn.execute(
            "INSERT INTO tasks (title, description, repo_path, plan_path, status, sub_status, base_branch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![title, description, repo_path, plan, status.as_str(), sub_status.as_str(), base_branch],
        )
        .context("Failed to insert task")?;
        Ok(TaskId(conn.last_insert_rowid()))
    }

    fn get_task(&self, id: TaskId) -> Result<Option<Task>> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
            params![id.0],
            row_to_task,
        )
        .optional()
        .context("Failed to get task")
    }

    fn list_all(&self) -> Result<Vec<Task>> {
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

    fn list_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
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

    fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>> {
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
}

// ---------------------------------------------------------------------------
// SettingsStore
// ---------------------------------------------------------------------------

impl super::SettingsStore for Database {
    fn list_repo_paths(&self) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT path FROM repo_paths ORDER BY last_used DESC")
            .context("Failed to prepare list_repo_paths")?;
        let paths = stmt
            .query_map([], |row| row.get(0))
            .context("Failed to query repo_paths")?
            .collect::<rusqlite::Result<Vec<String>>>()
            .context("Failed to collect repo_paths")?;
        Ok(paths)
    }

    fn save_repo_path(&self, path: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO repo_paths (path) VALUES (?1)
             ON CONFLICT(path) DO UPDATE SET last_used = datetime('now')",
            params![path],
        )
        .context("Failed to save repo_path")?;
        Ok(())
    }

    fn delete_repo_path(&self, path: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM repo_paths WHERE path = ?1", params![path])
            .context("Failed to delete repo_path")?;
        // Clean up filter presets that reference this path
        let presets: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT name, repo_paths FROM filter_presets")
                .context("Failed to prepare preset query")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to list presets for cleanup")?;
            rows
        };
        for (name, json) in presets {
            let paths: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
            let filtered: Vec<String> = paths.into_iter().filter(|p| p != path).collect();
            if filtered.is_empty() {
                conn.execute("DELETE FROM filter_presets WHERE name = ?1", params![name])?;
            } else {
                let updated = serde_json::to_string(&filtered)
                    .context("Failed to serialize filtered repo_paths")?;
                conn.execute(
                    "UPDATE filter_presets SET repo_paths = ?1 WHERE name = ?2",
                    params![updated, name],
                )?;
            }
        }
        Ok(())
    }

    fn get_setting_bool(&self, key: &str) -> Result<Option<bool>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| {
                let v: String = row.get(0)?;
                Ok(v == "1")
            },
        )
        .optional()
        .context("Failed to get setting")
    }

    fn set_setting_bool(&self, key: &str, value: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, if value { "1" } else { "0" }],
        )?;
        Ok(())
    }

    fn get_setting_string(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to get setting")
    }

    fn set_setting_string(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    fn seed_github_query_defaults(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let defaults: &[(&str, &str)] = &[
            (
                "github_queries_review",
                "is:pr is:open review-requested:@me -is:draft -author:app/dependabot -author:app/renovate archived:false\n\
                 is:pr is:open reviewed-by:@me -author:@me -is:draft -author:app/dependabot -author:app/renovate archived:false\n\
                 is:pr is:open commenter:@me -author:@me -is:draft -author:app/dependabot -author:app/renovate archived:false",
            ),
            (
                "github_queries_my_prs",
                "is:pr is:open author:@me -is:draft archived:false",
            ),
            (
                "github_queries_bot",
                "# Uncomment and edit one or more lines below to configure the Dependabot board.\n\
                 # Each uncommented line becomes a separate GitHub search query.\n\
                 #\n\
                 # Dependabot PRs in a specific repo:\n\
                 # is:pr is:open author:app/dependabot repo:myorg/myrepo -is:draft archived:false\n\
                 #\n\
                 # Dependabot PRs across an org:\n\
                 # is:pr is:open author:app/dependabot org:myorg -is:draft archived:false\n\
                 #\n\
                 # Renovate PRs across an org:\n\
                 # is:pr is:open author:app/renovate org:myorg -is:draft archived:false\n\
                 #\n\
                 # Renovate PRs in a specific repo:\n\
                 # is:pr is:open author:app/renovate repo:myorg/myrepo -is:draft archived:false",
            ),
        ];
        for (key, value) in defaults {
            conn.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
        }
        Ok(())
    }

    fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO task_usage
                 (task_id, cost_usd, input_tokens, output_tokens,
                  cache_read_tokens, cache_write_tokens, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             ON CONFLICT(task_id) DO UPDATE SET
                 cost_usd           = cost_usd           + excluded.cost_usd,
                 input_tokens       = input_tokens       + excluded.input_tokens,
                 output_tokens      = output_tokens      + excluded.output_tokens,
                 cache_read_tokens  = cache_read_tokens  + excluded.cache_read_tokens,
                 cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                 updated_at         = excluded.updated_at",
            params![
                task_id.0,
                usage.cost_usd,
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
                "SELECT task_id, cost_usd, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, updated_at
             FROM task_usage",
            )
            .context("Failed to prepare get_all_usage")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .context("Failed to query task_usage")?;
        let mut out = Vec::new();
        for row in rows {
            let (task_id, cost_usd, input, output, cr, cw, updated_at_str) =
                row.context("Failed to read usage row")?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at_str, "%Y-%m-%d %H:%M:%S")
                .with_context(|| format!("Invalid updated_at in task_usage: {updated_at_str:?}"))?
                .and_utc();
            out.push(TaskUsage {
                task_id: TaskId(task_id),
                cost_usd,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cr,
                cache_write_tokens: cw,
                updated_at,
            });
        }
        Ok(out)
    }

    fn save_filter_preset(&self, name: &str, repo_paths: &[String], mode: &str) -> Result<()> {
        let conn = self.conn()?;
        let json = serde_json::to_string(repo_paths).context("Failed to serialize repo_paths")?;
        conn.execute(
            "INSERT INTO filter_presets (name, repo_paths, mode) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET repo_paths = ?2, mode = ?3",
            params![name, json, mode],
        )?;
        Ok(())
    }

    fn delete_filter_preset(&self, name: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM filter_presets WHERE name = ?1", params![name])?;
        Ok(())
    }

    fn list_filter_presets(&self) -> Result<Vec<(String, Vec<String>, String)>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT name, repo_paths, mode FROM filter_presets ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let raw: Vec<(String, String, String)> = rows
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to list filter presets")?;
        Ok(raw
            .into_iter()
            .map(|(name, json, mode)| {
                let paths: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                (name, paths, mode)
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// EpicCrud
// ---------------------------------------------------------------------------

impl super::EpicCrud for Database {
    fn create_epic(&self, title: &str, description: &str, repo_path: &str) -> Result<Epic> {
        let id = {
            let conn = self.conn()?;
            conn.execute(
                "INSERT INTO epics (title, description, repo_path) VALUES (?1, ?2, ?3)",
                params![title, description, repo_path],
            )
            .context("Failed to insert epic")?;
            EpicId(conn.last_insert_rowid())
        }; // MutexGuard dropped here — avoids deadlock when get_epic() re-locks
        self.get_epic(id)?
            .ok_or_else(|| anyhow::anyhow!("Epic {id} vanished after insert"))
    }

    fn get_epic(&self, id: EpicId) -> Result<Option<Epic>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, created_at, updated_at
             FROM epics WHERE id = ?1",
            params![id.0],
            row_to_epic,
        )
        .optional()
        .context("Failed to get epic")
    }

    fn list_epics(&self) -> Result<Vec<Epic>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, description, repo_path, status, plan_path, sort_order, auto_dispatch, created_at, updated_at
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
        conn.execute("DELETE FROM tasks WHERE epic_id = ?1", params![id.0])
            .context("Failed to delete epic subtasks")?;
        let rows = conn
            .execute("DELETE FROM epics WHERE id = ?1", params![id.0])
            .context("Failed to delete epic")?;
        if rows == 0 {
            anyhow::bail!("Epic {} not found", id);
        }
        Ok(())
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

    fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<Task>> {
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

    fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()> {
        let epic = match self.get_epic(epic_id)? {
            Some(e) => e,
            None => return Ok(()),
        };

        let subtasks = self.list_tasks_for_epic(epic_id)?;
        let statuses: Vec<TaskStatus> = subtasks
            .iter()
            .filter(|t| t.status != TaskStatus::Archived)
            .map(|t| t.status)
            .collect();

        let derived = if statuses.is_empty() {
            TaskStatus::Backlog
        } else if statuses.iter().all(|s| *s == TaskStatus::Done) {
            TaskStatus::Done
        } else if statuses.contains(&TaskStatus::Review) {
            TaskStatus::Review
        } else if statuses.contains(&TaskStatus::Running) {
            TaskStatus::Running
        } else {
            TaskStatus::Backlog
        };

        if derived != epic.status {
            self.patch_epic(epic_id, &EpicPatch::new().status(derived))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PrStore
// ---------------------------------------------------------------------------

impl super::PrStore for Database {
    fn save_prs(&self, kind: super::PrKind, prs: &[ReviewPr]) -> Result<()> {
        let conn = self.conn()?;
        save_prs_to_table(&conn, kind.table_name(), prs)
    }

    fn load_prs(&self, kind: super::PrKind) -> Result<Vec<ReviewPr>> {
        let conn = self.conn()?;
        load_prs_from_table(&conn, kind.table_name())
    }

    fn set_pr_agent(
        &self,
        kind: super::PrKind,
        repo: &str,
        number: i64,
        tmux_window: &str,
        worktree: &str,
    ) -> Result<()> {
        let table = kind.table_name();
        let conn = self.conn()?;
        conn.execute(
            &format!("UPDATE {table} SET tmux_window = ?1, worktree = ?2, agent_status = 'reviewing' WHERE repo = ?3 AND number = ?4"),
            params![tmux_window, worktree, repo, number],
        )?;
        Ok(())
    }

    fn update_agent_status(&self, repo: &str, number: i64, status: Option<&str>) -> Result<String> {
        let conn = self.conn()?;
        for table in &["review_prs", "bot_prs"] {
            let affected = conn.execute(
                &format!("UPDATE {table} SET agent_status = ?1 WHERE repo = ?2 AND number = ?3 AND tmux_window IS NOT NULL"),
                params![status, repo, number],
            )?;
            if affected > 0 {
                return Ok(table.to_string());
            }
        }
        let affected = conn.execute(
            "UPDATE security_alerts SET agent_status = ?1 WHERE repo = ?2 AND number = ?3 AND tmux_window IS NOT NULL",
            params![status, repo, number],
        )?;
        if affected > 0 {
            return Ok("security_alerts".to_string());
        }
        anyhow::bail!("No active agent found for {repo}#{number}");
    }
}

// ---------------------------------------------------------------------------
// AlertStore
// ---------------------------------------------------------------------------

impl super::AlertStore for Database {
    fn save_security_alerts(&self, alerts: &[crate::models::SecurityAlert]) -> Result<()> {
        let conn = self.conn()?;
        save_security_alerts_impl(&conn, alerts)
    }

    fn load_security_alerts(&self) -> Result<Vec<crate::models::SecurityAlert>> {
        let conn = self.conn()?;
        load_security_alerts_impl(&conn)
    }

    fn set_alert_agent(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
        tmux_window: &str,
        worktree: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE security_alerts SET tmux_window = ?1, worktree = ?2, agent_status = 'reviewing' WHERE repo = ?3 AND number = ?4 AND kind = ?5",
            params![tmux_window, worktree, repo, number, kind.as_db_str()],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shared PR save/load helpers
// ---------------------------------------------------------------------------

fn save_prs_to_table(conn: &rusqlite::Connection, table: &str, prs: &[ReviewPr]) -> Result<()> {
    assert!(
        matches!(table, "review_prs" | "my_prs" | "bot_prs"),
        "invalid PR table: {table}"
    );
    let tx = conn.unchecked_transaction()?;

    // Upsert all PRs — ON CONFLICT preserves tmux_window and worktree
    {
        let mut stmt = tx.prepare(&format!(
            "INSERT INTO {table} (repo, number, title, author, url, is_draft,
             created_at, updated_at, additions, deletions, review_decision, labels,
             body, head_ref, ci_status, reviewers)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(repo, number) DO UPDATE SET
             title = excluded.title, author = excluded.author, url = excluded.url,
             is_draft = excluded.is_draft, created_at = excluded.created_at,
             updated_at = excluded.updated_at, additions = excluded.additions,
             deletions = excluded.deletions, review_decision = excluded.review_decision,
             labels = excluded.labels, body = excluded.body, head_ref = excluded.head_ref,
             ci_status = excluded.ci_status, reviewers = excluded.reviewers"
        ))?;
        for pr in prs {
            let labels_json =
                serde_json::to_string(&pr.labels).context("Failed to serialize labels")?;
            let reviewers_json = serde_json::to_string(
                &pr.reviewers
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "login": r.login,
                            "decision": r.decision.map(|d| d.as_db_str())
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
            stmt.execute(params![
                pr.repo,
                pr.number,
                pr.title,
                pr.author,
                pr.url,
                pr.is_draft,
                pr.created_at.to_rfc3339(),
                pr.updated_at.to_rfc3339(),
                pr.additions,
                pr.deletions,
                pr.review_decision.as_db_str(),
                labels_json,
                pr.body,
                pr.head_ref,
                pr.ci_status.as_db_str(),
                reviewers_json,
            ])?;
        }
    }

    // Delete stale rows not in the fresh set
    if prs.is_empty() {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    } else {
        let placeholders: Vec<String> = (0..prs.len())
            .map(|i| format!("(?{}, ?{})", i * 2 + 1, i * 2 + 2))
            .collect();
        let sql = format!(
            "DELETE FROM {table} WHERE (repo, number) NOT IN (VALUES {})",
            placeholders.join(", ")
        );
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = prs
            .iter()
            .flat_map(|pr| {
                vec![
                    Box::new(pr.repo.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(pr.number) as Box<dyn rusqlite::types::ToSql>,
                ]
            })
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        tx.execute(&sql, param_refs.as_slice())?;
    }

    tx.commit()?;
    Ok(())
}

fn load_prs_from_table(conn: &rusqlite::Connection, table: &str) -> Result<Vec<ReviewPr>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT repo, number, title, author, url, is_draft,
                created_at, updated_at, additions, deletions,
                review_decision, labels, body, head_ref, ci_status, reviewers,
                tmux_window, worktree, agent_status
         FROM {table}"
    ))?;
    let rows = stmt.query_map([], |row| {
        let repo: String = row.get(0)?;
        let number: i64 = row.get(1)?;
        let title: String = row.get(2)?;
        let author: String = row.get(3)?;
        let url: String = row.get(4)?;
        let is_draft: bool = row.get(5)?;
        let created_at_str: String = row.get(6)?;
        let updated_at_str: String = row.get(7)?;
        let additions: i64 = row.get(8)?;
        let deletions: i64 = row.get(9)?;
        let decision_str: String = row.get(10)?;
        let labels_json: String = row.get(11)?;
        let body: String = row.get(12)?;
        let head_ref: String = row.get(13)?;
        let ci_status_str: String = row.get(14)?;
        let reviewers_json: String = row.get(15)?;
        let tmux_window: Option<String> = row.get(16)?;
        let worktree: Option<String> = row.get(17)?;
        let agent_status_str: Option<String> = row.get(18)?;
        Ok((
            repo,
            number,
            title,
            author,
            url,
            is_draft,
            created_at_str,
            updated_at_str,
            additions,
            deletions,
            decision_str,
            labels_json,
            body,
            head_ref,
            ci_status_str,
            reviewers_json,
            tmux_window,
            worktree,
            agent_status_str,
        ))
    })?;

    let mut prs = Vec::new();
    for row in rows {
        let (
            repo,
            number,
            title,
            author,
            url,
            is_draft,
            created_at_str,
            updated_at_str,
            additions,
            deletions,
            decision_str,
            labels_json,
            body,
            head_ref,
            ci_status_str,
            reviewers_json,
            tmux_window,
            worktree,
            agent_status_str,
        ) = row?;

        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let review_decision =
            ReviewDecision::from_db_str(&decision_str).unwrap_or(ReviewDecision::ReviewRequired);
        let labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();
        let ci_status = CiStatus::from_db_str(&ci_status_str);
        let reviewers: Vec<Reviewer> =
            serde_json::from_str::<Vec<serde_json::Value>>(&reviewers_json)
                .unwrap_or_default()
                .iter()
                .map(|v| Reviewer {
                    login: v["login"].as_str().unwrap_or("").to_string(),
                    decision: v["decision"].as_str().and_then(ReviewDecision::from_db_str),
                })
                .collect();

        prs.push(ReviewPr {
            number,
            title,
            author,
            repo,
            url,
            is_draft,
            created_at,
            updated_at,
            additions,
            deletions,
            review_decision,
            labels,
            body,
            head_ref,
            ci_status,
            reviewers,
            tmux_window,
            worktree,
            agent_status: agent_status_str
                .as_deref()
                .and_then(ReviewAgentStatus::from_db_str),
        });
    }
    Ok(prs)
}

// ---------------------------------------------------------------------------
// Security alert save/load helpers
// ---------------------------------------------------------------------------

fn save_security_alerts_impl(
    conn: &rusqlite::Connection,
    alerts: &[crate::models::SecurityAlert],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut stmt = tx.prepare(
            "INSERT INTO security_alerts (repo, number, kind, severity, title, package,
             vulnerable_range, fixed_version, cvss_score, url, created_at, state, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(repo, number, kind) DO UPDATE SET
             severity = excluded.severity, title = excluded.title,
             package = excluded.package, vulnerable_range = excluded.vulnerable_range,
             fixed_version = excluded.fixed_version, cvss_score = excluded.cvss_score,
             url = excluded.url, created_at = excluded.created_at,
             state = excluded.state, description = excluded.description",
        )?;
        for a in alerts {
            stmt.execute(params![
                a.repo,
                a.number,
                a.kind.as_db_str(),
                a.severity.as_db_str(),
                a.title,
                a.package,
                a.vulnerable_range,
                a.fixed_version,
                a.cvss_score,
                a.url,
                a.created_at.to_rfc3339(),
                a.state,
                a.description,
            ])?;
        }
    }

    // Delete stale rows
    if alerts.is_empty() {
        tx.execute("DELETE FROM security_alerts", [])?;
    } else {
        let placeholders: Vec<String> = (0..alerts.len())
            .map(|i| format!("(?{}, ?{}, ?{})", i * 3 + 1, i * 3 + 2, i * 3 + 3))
            .collect();
        let sql = format!(
            "DELETE FROM security_alerts WHERE (repo, number, kind) NOT IN (VALUES {})",
            placeholders.join(", ")
        );
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = alerts
            .iter()
            .flat_map(|a| {
                vec![
                    Box::new(a.repo.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(a.number) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(a.kind.as_db_str()) as Box<dyn rusqlite::types::ToSql>,
                ]
            })
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        tx.execute(&sql, param_refs.as_slice())?;
    }

    tx.commit()?;
    Ok(())
}

fn load_security_alerts_impl(
    conn: &rusqlite::Connection,
) -> Result<Vec<crate::models::SecurityAlert>> {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let mut stmt = conn.prepare(
        "SELECT repo, number, kind, severity, title, package,
                vulnerable_range, fixed_version, cvss_score, url,
                created_at, state, description, tmux_window, worktree, agent_status
         FROM security_alerts",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<f64>>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<String>>(15)?,
        ))
    })?;

    let mut alerts = Vec::new();
    for row in rows {
        let (
            repo,
            number,
            kind_str,
            severity_str,
            title,
            package,
            vulnerable_range,
            fixed_version,
            cvss_score,
            url,
            created_at_str,
            state,
            description,
            tmux_window,
            worktree,
            agent_status_str,
        ) = row?;

        let kind = AlertKind::from_db_str(&kind_str).unwrap_or(AlertKind::Dependabot);
        let severity = AlertSeverity::from_db_str(&severity_str).unwrap_or(AlertSeverity::Medium);
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        alerts.push(SecurityAlert {
            number,
            repo,
            severity,
            kind,
            title,
            package,
            vulnerable_range,
            fixed_version,
            cvss_score,
            url,
            created_at,
            state,
            description,
            tmux_window,
            worktree,
            agent_status: agent_status_str
                .as_deref()
                .and_then(ReviewAgentStatus::from_db_str),
        });
    }
    Ok(alerts)
}

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status_str: String = row.get("status")?;
    let status = TaskStatus::parse(&status_str).unwrap_or_else(|| {
        tracing::warn!(raw = %status_str, "unrecognised task status, defaulting to Backlog");
        TaskStatus::Backlog
    });

    let created_str: String = row.get("created_at")?;
    let updated_str: String = row.get("updated_at")?;

    Ok(Task {
        id: TaskId(row.get("id")?),
        title: row.get("title")?,
        description: row.get("description")?,
        repo_path: row.get("repo_path")?,
        status,
        worktree: row.get("worktree")?,
        tmux_window: row.get("tmux_window")?,
        plan_path: row.get("plan_path")?,
        epic_id: row
            .get::<_, Option<i64>>("epic_id")
            .unwrap_or(None)
            .map(EpicId),
        sub_status: row
            .get::<_, String>("sub_status")
            .ok()
            .and_then(|s| SubStatus::parse(&s))
            .unwrap_or(SubStatus::None),
        pr_url: row.get::<_, Option<String>>("pr_url").unwrap_or(None),
        tag: row
            .get::<_, Option<String>>("tag")
            .unwrap_or(None)
            .as_deref()
            .and_then(TaskTag::parse),
        sort_order: row.get::<_, Option<i64>>("sort_order").unwrap_or(None),
        base_branch: row
            .get::<_, Option<String>>("base_branch")
            .unwrap_or(None)
            .unwrap_or_else(|| "main".to_string()),
        created_at: parse_datetime(&created_str),
        updated_at: parse_datetime(&updated_str),
    })
}

fn row_to_epic(row: &rusqlite::Row<'_>) -> rusqlite::Result<Epic> {
    let created_str: String = row.get("created_at")?;
    let updated_str: String = row.get("updated_at")?;
    let status_str: String = row.get("status")?;

    Ok(Epic {
        id: EpicId(row.get("id")?),
        title: row.get("title")?,
        description: row.get("description")?,
        repo_path: row.get("repo_path")?,
        status: TaskStatus::parse(&status_str).unwrap_or(TaskStatus::Backlog),
        plan_path: row.get("plan_path")?,
        sort_order: row.get::<_, Option<i64>>("sort_order").unwrap_or(None),
        auto_dispatch: row.get::<_, bool>("auto_dispatch").unwrap_or(true),
        created_at: parse_datetime(&created_str),
        updated_at: parse_datetime(&updated_str),
    })
}

/// Parse SQLite `datetime('now')` output: "YYYY-MM-DD HH:MM:SS"
fn parse_datetime(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|ndt| Utc.from_utc_datetime(&ndt))
        .unwrap_or_else(Utc::now)
}
