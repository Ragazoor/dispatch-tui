use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use rusqlite::{params, OptionalExtension};

use crate::models::{
    EpicId, FeedItem, ProjectId, SubStatus, TaskId, TaskStatus, TaskUsage, UsageReport,
};

use super::super::{CreateTaskRequest, Database, TaskPatch};
use super::{row_to_task, write_json_string_vec, TASK_COLUMNS};

/// Owned mirror of [`CreateTaskRequest`] suitable for moving into a `db_call`
/// closure (drops the lifetime).
#[derive(Debug)]
struct OwnedCreateTaskRequest {
    title: String,
    description: String,
    repo_path: String,
    plan: Option<String>,
    status: TaskStatus,
    base_branch: String,
    epic_id: Option<EpicId>,
    sort_order: Option<i64>,
    tag: Option<crate::models::TaskTag>,
    project_id: ProjectId,
}

impl<'a> From<CreateTaskRequest<'a>> for OwnedCreateTaskRequest {
    fn from(r: CreateTaskRequest<'a>) -> Self {
        Self {
            title: r.title.to_string(),
            description: r.description.to_string(),
            repo_path: r.repo_path.to_string(),
            plan: r.plan.map(|s| s.to_string()),
            status: r.status,
            base_branch: r.base_branch.to_string(),
            epic_id: r.epic_id,
            sort_order: r.sort_order,
            tag: r.tag,
            project_id: r.project_id,
        }
    }
}

/// Owned mirror of [`TaskPatch`] suitable for moving into a `db_call` closure.
#[derive(Debug, Default)]
struct OwnedTaskPatch {
    status: Option<TaskStatus>,
    plan_path: Option<Option<String>>,
    title: Option<String>,
    description: Option<String>,
    repo_path: Option<String>,
    worktree: Option<Option<String>>,
    tmux_window: Option<Option<String>>,
    sub_status: Option<SubStatus>,
    pr_url: Option<Option<String>>,
    tag: Option<Option<crate::models::TaskTag>>,
    sort_order: Option<Option<i64>>,
    base_branch: Option<String>,
    external_id: Option<Option<String>>,
    project_id: Option<ProjectId>,
    labels: Option<Vec<String>>,
    last_pre_tool_use_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    last_notification_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

impl OwnedTaskPatch {
    fn has_changes(&self) -> bool {
        self.status.is_some()
            || self.plan_path.is_some()
            || self.title.is_some()
            || self.description.is_some()
            || self.repo_path.is_some()
            || self.worktree.is_some()
            || self.tmux_window.is_some()
            || self.sub_status.is_some()
            || self.pr_url.is_some()
            || self.tag.is_some()
            || self.sort_order.is_some()
            || self.base_branch.is_some()
            || self.external_id.is_some()
            || self.project_id.is_some()
            || self.labels.is_some()
            || self.last_pre_tool_use_at.is_some()
            || self.last_notification_at.is_some()
    }
}

impl<'a> From<&TaskPatch<'a>> for OwnedTaskPatch {
    fn from(p: &TaskPatch<'a>) -> Self {
        Self {
            status: p.status,
            plan_path: p.plan_path.map(|o| o.map(|s| s.to_string())),
            title: p.title.map(|s| s.to_string()),
            description: p.description.map(|s| s.to_string()),
            repo_path: p.repo_path.map(|s| s.to_string()),
            worktree: p.worktree.map(|o| o.map(|s| s.to_string())),
            tmux_window: p.tmux_window.map(|o| o.map(|s| s.to_string())),
            sub_status: p.sub_status,
            pr_url: p.pr_url.map(|o| o.map(|s| s.to_string())),
            tag: p.tag,
            sort_order: p.sort_order,
            base_branch: p.base_branch.map(|s| s.to_string()),
            external_id: p.external_id.map(|o| o.map(|s| s.to_string())),
            project_id: p.project_id,
            labels: p.labels.map(|s| s.to_vec()),
            last_pre_tool_use_at: p.last_pre_tool_use_at,
            last_notification_at: p.last_notification_at,
        }
    }
}

#[async_trait::async_trait]
impl super::super::TaskCrud for Database {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId> {
        let req = OwnedCreateTaskRequest::from(req);
        self.db_call(move |conn| {
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
        })
        .await
    }

    async fn get_task(&self, id: TaskId) -> Result<Option<crate::models::Task>> {
        self.db_call(move |conn| {
            conn.query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
                params![id.0],
                row_to_task,
            )
            .optional()
            .context("Failed to get task")
        })
        .await
    }

    async fn list_all(&self) -> Result<Vec<crate::models::Task>> {
        self.db_call(move |conn| {
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
        })
        .await
    }

    async fn list_by_status(&self, status: TaskStatus) -> Result<Vec<crate::models::Task>> {
        self.db_call(move |conn| {
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
        })
        .await
    }

    async fn update_status_if(
        &self,
        id: TaskId,
        new_status: TaskStatus,
        expected: TaskStatus,
    ) -> Result<bool> {
        self.db_call(move |conn| {
            let default_sub = SubStatus::default_for(new_status);
            let rows = conn
                .execute(
                    "UPDATE tasks SET status = ?1, sub_status = ?2, updated_at = datetime('now') WHERE id = ?3 AND status = ?4",
                    params![new_status.as_str(), default_sub.as_str(), id.0, expected.as_str()],
                )
                .context("Failed to conditional-update status")?;
            Ok(rows > 0)
        })
        .await
    }

    async fn delete_task(&self, id: TaskId) -> Result<()> {
        self.db_call(move |conn| {
            let rows = conn
                .execute("DELETE FROM tasks WHERE id = ?1", params![id.0])
                .context("Failed to delete task")?;
            if rows == 0 {
                anyhow::bail!("Task {} not found", id);
            }
            Ok(())
        })
        .await
    }

    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<crate::models::Task>> {
        let plan = plan.to_string();
        self.db_call(move |conn| {
            conn.query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE plan_path = ?1"),
                params![plan],
                row_to_task,
            )
            .optional()
            .context("Failed to find task by plan")
        })
        .await
    }

    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()> {
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
        let labels_json = match patch.labels {
            Some(labels) => Some(write_json_string_vec(labels)?),
            None => None,
        };
        let patch = OwnedTaskPatch::from(patch);
        self.db_call(move |conn| {
            if !patch.has_changes() {
                return Ok(());
            }
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
                values.push(Box::new(t));
            }
            if let Some(d) = patch.description {
                sets.push("description = ?");
                values.push(Box::new(d));
            }
            if let Some(r) = patch.repo_path {
                sets.push("repo_path = ?");
                values.push(Box::new(r));
            }
            if let Some(p) = patch.plan_path {
                sets.push("plan_path = ?");
                values.push(Box::new(p));
            }
            if let Some(w) = patch.worktree {
                sets.push("worktree = ?");
                values.push(Box::new(w));
            }
            if let Some(t) = patch.tmux_window {
                sets.push("tmux_window = ?");
                values.push(Box::new(t));
            }
            if let Some(ss) = effective_sub_status {
                sets.push("sub_status = ?");
                values.push(Box::new(ss.as_str().to_string()));
            }
            if let Some(url) = patch.pr_url {
                sets.push("pr_url = ?");
                values.push(Box::new(url));
            }
            if let Some(tag) = patch.tag {
                sets.push("tag = ?");
                values.push(Box::new(tag.map(|t| t.as_str().to_string())));
            }
            if let Some(so) = patch.sort_order {
                sets.push("sort_order = ?");
                values.push(Box::new(so));
            }
            if let Some(bb) = patch.base_branch {
                sets.push("base_branch = ?");
                values.push(Box::new(bb));
            }
            if let Some(eid) = patch.external_id {
                sets.push("external_id = ?");
                values.push(Box::new(eid));
            }
            if let Some(pid) = patch.project_id {
                sets.push("project_id = ?");
                values.push(Box::new(pid.0));
            }
            if let Some(json) = labels_json {
                sets.push("labels = ?");
                values.push(Box::new(json));
            }
            if let Some(t) = patch.last_pre_tool_use_at {
                sets.push("last_pre_tool_use_at = ?");
                values.push(Box::new(t.map(super::format_datetime)));
            }
            if let Some(t) = patch.last_notification_at {
                sets.push("last_notification_at = ?");
                values.push(Box::new(t.map(super::format_datetime)));
            }

            sets.push("updated_at = datetime('now')");
            values.push(Box::new(id.0));

            let sql = format!("UPDATE tasks SET {} WHERE id = ?", sets.join(", "));
            let refs: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            let rows = conn
                .execute(&sql, refs.as_slice())
                .context("Failed to patch task")?;
            if rows == 0 {
                anyhow::bail!("Task {id} not found");
            }
            Ok(())
        })
        .await
    }

    async fn has_other_tasks_with_worktree(
        &self,
        worktree: &str,
        exclude_id: TaskId,
    ) -> Result<bool> {
        let worktree = worktree.to_string();
        self.db_call(move |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tasks WHERE worktree = ?1 AND id != ?2 AND status != 'done'",
                    params![worktree, exclude_id.0],
                    |row| row.get(0),
                )
                .context("Failed to check shared worktree")?;
            Ok(count > 0)
        })
        .await
    }

    async fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<()> {
        let usage = usage.clone();
        self.db_call(move |conn| {
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
        })
        .await
    }

    async fn get_all_usage(&self) -> Result<Vec<TaskUsage>> {
        self.db_call(move |conn| {
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
                let updated_at =
                    NaiveDateTime::parse_from_str(&updated_at_str, "%Y-%m-%d %H:%M:%S")
                        .with_context(|| {
                            format!("Invalid updated_at in task_usage: {updated_at_str:?}")
                        })?
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
        })
        .await
    }

    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<()> {
        let items = items.to_vec();
        let repo_paths = repo_paths.to_vec();
        let base_branches = base_branches.to_vec();
        // Pre-serialize labels (write_json_string_vec is sync but returns Result).
        let labels_jsons: Vec<String> = items
            .iter()
            .map(|i| write_json_string_vec(&i.labels))
            .collect::<Result<Vec<_>>>()?;
        let keep_ids =
            serde_json::to_string(&items.iter().map(|i| &i.external_id).collect::<Vec<_>>())
                .context("failed to serialize external_ids for feed task cleanup")?;
        self.db_call(move |conn| {
            let project_id: ProjectId = conn
                .query_row(
                    "SELECT project_id FROM epics WHERE id = ?1",
                    params![epic_id.0],
                    |row| row.get::<_, i64>(0).map(ProjectId),
                )
                .with_context(|| format!("Epic {} not found for upsert_feed_tasks", epic_id))?;

            let tx = conn.unchecked_transaction()?;

            for (((item, repo_path), base_branch), labels_json) in items
                .iter()
                .zip(repo_paths.iter())
                .zip(base_branches.iter())
                .zip(labels_jsons.iter())
            {
                let sub_status = SubStatus::default_for(item.status).as_str().to_string();
                tx.execute(
                    "INSERT INTO tasks
                         (title, description, repo_path, status, sub_status, base_branch,
                          epic_id, external_id, project_id, tag, labels, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(epic_id, external_id) WHERE external_id IS NOT NULL
                     DO UPDATE SET
                         title       = excluded.title,
                         description = excluded.description,
                         tag         = excluded.tag,
                         labels      = excluded.labels,
                         sort_order  = excluded.sort_order,
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
                        labels_json,
                        item.sort_order,
                    ],
                )
                .with_context(|| format!("Failed to upsert feed task '{}'", item.external_id))?;
            }

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
        })
        .await
    }
}
