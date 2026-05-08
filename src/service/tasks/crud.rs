//! `TaskService` — CRUD and lifecycle operations for tasks.
//!
//! Pure parameter shapes live in `params.rs`; the patch builder lives in
//! `validators.rs`. Methods that read or mutate DB state are kept here
//! because they need `&self`.

use std::sync::Arc;

use crate::db::{self, CreateTaskRequest, TaskPatch};
use crate::models::{
    EpicId, SubStatus, Task, TaskId, TaskStatus, UsageReport, DEFAULT_BASE_BRANCH,
};
use crate::service::ServiceError;

use super::params::{ClaimTaskParams, CreateTaskParams, ListTasksFilter, UpdateTaskParams};
use super::validators::build_task_patch;
use crate::service::FieldUpdate;

/// Result of [`TaskService::update_task`]. Carries the updated task id plus
/// presentation-relevant transition flags so MCP handlers can format their
/// response without re-reading the DB.
#[derive(Debug, Clone)]
pub struct UpdateTaskResult {
    pub task_id: TaskId,
    /// `true` when the same call set a non-empty `pr_url` on a task that
    /// previously had none AND moved its status to Review.
    pub was_pr_finalisation: bool,
}

pub struct TaskService {
    pub db: Arc<dyn db::TaskAndEpicStore>,
}

impl TaskService {
    pub fn new(db: Arc<dyn db::TaskAndEpicStore>) -> Self {
        Self { db }
    }

    /// Updates a task from an MCP agent or external tool call.
    ///
    /// **Caller:** MCP handlers (`src/mcp/handlers/tasks.rs`).
    ///
    /// **Restrictions:** Cannot transition status to `Done` or `Archived` — those
    /// transitions are reserved for human operators via the CLI. Supports the full
    /// `UpdateTaskParams` builder (title, description, repo_path, pr_url, worktree,
    /// tmux_window, base_branch, sub_status, epic_id, sort_order, tag, status).
    ///
    /// Use [`cli_update_task`](Self::cli_update_task) instead when writing CLI
    /// subcommands that need to complete or archive tasks.
    pub fn update_task(&self, params: UpdateTaskParams) -> Result<UpdateTaskResult, ServiceError> {
        if !params.has_any_field() {
            return Err(ServiceError::Validation(
                "At least one field must be provided".into(),
            ));
        }

        let task_id = params.task_id;
        let expanded_repo_path = params.repo_path.as_deref().map(crate::models::expand_tilde);
        let validated_sub_status = self.validate_sub_status(task_id, &params)?;
        let patch = build_task_patch(&params, expanded_repo_path.as_deref(), validated_sub_status);

        // Snapshot the task before the patch so we can detect the
        // null-pr_url → set transition without an extra round-trip later.
        let prior = self.db.get_task(task_id).ok().flatten();
        let was_pr_finalisation = params.status == Some(TaskStatus::Review)
            && matches!(
                params.pr_url.as_ref(),
                Some(FieldUpdate::Set(s)) if !s.is_empty()
            )
            && prior.as_ref().is_some_and(|t| t.pr_url.is_none());

        self.db
            .patch_task(task_id, &patch)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        if let Some(new_epic_id) = params.epic_id {
            let old_epic_id = prior.as_ref().and_then(|t| t.epic_id);
            self.db
                .set_task_epic_id(task_id, Some(new_epic_id))
                .map_err(|e| ServiceError::Internal(format!("Failed to link task to epic: {e}")))?;
            if let Some(old) = old_epic_id {
                self.recalculate_epic(old);
            }
            self.recalculate_epic(new_epic_id);
        }

        if params.status.is_some() {
            self.recalculate_epic_for_task(task_id);
        }

        Ok(UpdateTaskResult {
            task_id,
            was_pr_finalisation,
        })
    }

    /// Validate `params.sub_status` against the task's effective (current or
    /// requested) status. Returns the sub_status to write, if any.
    ///
    /// Intentional TOCTOU: we read the current status here to validate the
    /// sub_status, then write via patch_task in `update_task` afterwards. A
    /// concurrent update between the two is theoretically possible but benign
    /// in practice — Dispatch is a single-process tokio runtime with
    /// cooperative scheduling, so no two MCP handlers run truly concurrently
    /// on the same task. SQLite serialises writes regardless.
    fn validate_sub_status(
        &self,
        task_id: TaskId,
        params: &UpdateTaskParams,
    ) -> Result<Option<SubStatus>, ServiceError> {
        let Some(ss) = params.sub_status else {
            return Ok(None);
        };
        let effective_status = params
            .status
            .or_else(|| self.db.get_task(task_id).ok().flatten().map(|t| t.status));
        if let Some(eff) = effective_status {
            if !ss.is_valid_for(eff) {
                return Err(ServiceError::Validation(format!(
                    "sub_status '{}' is not valid for status '{}'",
                    ss.as_str(),
                    eff.as_str()
                )));
            }
        }
        Ok(Some(ss))
    }

    /// Recalculate the given epic, logging any database error.
    fn recalculate_epic(&self, epic_id: EpicId) {
        if let Err(err) = self.db.recalculate_epic_status(epic_id) {
            tracing::warn!(
                "failed to recalculate epic status for epic {}: {err}",
                epic_id.0
            );
        }
    }

    /// Recalculate the parent epic of the given task, if it has one.
    fn recalculate_epic_for_task(&self, task_id: TaskId) {
        if let Ok(Some(task)) = self.db.get_task(task_id) {
            if let Some(epic_id) = task.epic_id {
                self.recalculate_epic(epic_id);
            }
        }
    }

    /// Updates a task status from a CLI subcommand (human operator path).
    ///
    /// **Caller:** `src/main.rs` CLI subcommands (`dispatch update`, etc.).
    ///
    /// **Differences from [`update_task`](Self::update_task):**
    /// - Can transition to any status including `Done` and `Archived`.
    /// - Supports conditional update: `only_if` skips the write if the current
    ///   status doesn't match, returning `Ok(false)` instead of an error.
    /// - Accepts only status + sub_status — not the full field builder.
    ///
    /// Use `update_task` for agent/MCP call sites that must not complete tasks.
    pub fn cli_update_task(
        &self,
        task_id: TaskId,
        new_status: TaskStatus,
        only_if: Option<TaskStatus>,
        sub_status: Option<SubStatus>,
    ) -> Result<bool, ServiceError> {
        let updated = if let Some(expected) = only_if {
            let changed = self
                .db
                .update_status_if(task_id, new_status, expected)
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
            if changed {
                if let Some(ss) = sub_status {
                    self.db
                        .patch_task(task_id, &crate::db::TaskPatch::new().sub_status(ss))
                        .map_err(|e| ServiceError::Internal(e.to_string()))?;
                }
            }
            changed
        } else {
            let mut patch = crate::db::TaskPatch::new().status(new_status);
            if let Some(ss) = sub_status {
                patch = patch.sub_status(ss);
            }
            self.db
                .patch_task(task_id, &patch)
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
            true
        };

        if updated {
            self.recalculate_epic_for_task(task_id);
        }

        Ok(updated)
    }

    pub fn create_task(&self, params: CreateTaskParams) -> Result<TaskId, ServiceError> {
        Ok(self.create_task_returning(params)?.id)
    }

    /// Create a task and return the full Task object (used by TUI).
    pub fn create_task_returning(&self, params: CreateTaskParams) -> Result<Task, ServiceError> {
        let repo_path = crate::models::expand_tilde(&params.repo_path);

        let plan = params.plan_path.as_deref().map(|p| {
            std::fs::canonicalize(p)
                .map(|abs| abs.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string())
        });

        let base_branch = params.base_branch.as_deref().unwrap_or(DEFAULT_BASE_BRANCH);
        let task_id = self
            .db
            .create_task(CreateTaskRequest {
                title: &params.title,
                description: &params.description,
                repo_path: &repo_path,
                plan: plan.as_deref(),
                status: TaskStatus::Backlog,
                base_branch,
                epic_id: params.epic_id,
                sort_order: params.sort_order,
                tag: params.tag,
                project_id: params.project_id,
            })
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        self.get_task(task_id)
    }

    pub fn delete_task(&self, task_id: TaskId) -> Result<(), ServiceError> {
        // Verify task exists
        self.get_task(task_id)?;

        self.db
            .delete_task(task_id)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub fn get_task(&self, task_id: TaskId) -> Result<Task, ServiceError> {
        match self.db.get_task(task_id) {
            Ok(Some(task)) => Ok(task),
            Ok(None) => Err(ServiceError::NotFound(format!(
                "Task {} not found",
                task_id.0
            ))),
            Err(e) => Err(ServiceError::Internal(format!("Database error: {e}"))),
        }
    }

    pub fn list_tasks(&self, filter: ListTasksFilter) -> Result<Vec<Task>, ServiceError> {
        let tasks = self
            .db
            .list_all()
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        let filtered: Vec<_> = tasks
            .into_iter()
            .filter(|t| match &filter.statuses {
                Some(statuses) => statuses.contains(&t.status),
                None => t.status != TaskStatus::Archived,
            })
            .filter(|t| match filter.epic_id {
                Some(eid) => t.epic_id == Some(eid),
                None => true,
            })
            .filter(|t| match filter.project_id {
                Some(pid) => t.project_id == pid,
                None => true,
            })
            .filter(|t| match &filter.repo_paths {
                Some(paths) => paths.iter().any(|p| p == &t.repo_path),
                None => true,
            })
            .filter(|t| match filter.exclude_task_id {
                Some(excluded) => t.id != excluded,
                None => true,
            })
            .collect();

        Ok(filtered)
    }

    pub fn claim_task(&self, params: ClaimTaskParams) -> Result<Task, ServiceError> {
        let task = self.get_task(params.task_id)?;

        if task.status != TaskStatus::Backlog {
            return Err(ServiceError::Validation(format!(
                "Task {} is already {}",
                params.task_id,
                task.status.as_str()
            )));
        }

        // Same-repo check: derive repo from worktree by stripping /.worktrees/<anything>
        let repo_from_worktree = params
            .worktree
            .find("/.worktrees/")
            .map(|idx| &params.worktree[..idx])
            .unwrap_or(&params.worktree);

        if repo_from_worktree != task.repo_path {
            return Err(ServiceError::Validation(format!(
                "Repo mismatch: task belongs to {}, your worktree is in {}",
                task.repo_path, repo_from_worktree
            )));
        }

        self.db
            .patch_task(
                params.task_id,
                &TaskPatch::new()
                    .status(TaskStatus::Running)
                    .worktree(Some(&params.worktree))
                    .tmux_window(Some(&params.tmux_window)),
            )
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        Ok(task)
    }

    pub fn validate_wrap_up(&self, task_id: TaskId) -> Result<Task, ServiceError> {
        let task = self.get_task(task_id)?;

        if !crate::dispatch::is_wrappable(&task) {
            return Err(ServiceError::Validation(format!(
                "Task {} cannot be wrapped up. Requires Running or Review status with a worktree.",
                task_id.0
            )));
        }

        Ok(task)
    }

    pub fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<(), ServiceError> {
        // Verify task exists
        self.get_task(task_id)?;

        self.db
            .report_usage(task_id, usage)
            .map_err(|e| ServiceError::Internal(format!("Failed to record usage: {e}")))
    }

    pub fn validate_send_message(
        &self,
        from_task_id: TaskId,
        to_task_id: TaskId,
    ) -> Result<(Task, Task), ServiceError> {
        let from_task = match self.db.get_task(from_task_id) {
            Ok(Some(t)) => t,
            Ok(None) => {
                return Err(ServiceError::NotFound(format!(
                    "sender task {} not found",
                    from_task_id.0
                )));
            }
            Err(e) => {
                return Err(ServiceError::Internal(format!(
                    "failed to look up sender: {e}"
                )));
            }
        };

        let to_task = match self.db.get_task(to_task_id) {
            Ok(Some(t)) => t,
            Ok(None) => {
                return Err(ServiceError::NotFound(format!(
                    "target task {} not found",
                    to_task_id.0
                )));
            }
            Err(e) => {
                return Err(ServiceError::Internal(format!(
                    "failed to look up target: {e}"
                )));
            }
        };

        if to_task.worktree.is_none() {
            return Err(ServiceError::Validation(format!(
                "target task {} has no worktree (not running)",
                to_task_id.0
            )));
        }

        if to_task.tmux_window.is_none() {
            return Err(ServiceError::Validation(format!(
                "target task {} has no tmux window (not running)",
                to_task_id.0
            )));
        }

        Ok((from_task, to_task))
    }

    /// Find the next backlog task for an epic, sorted by sort_order then id.
    /// Returns `Ok(None)` if no backlog tasks remain.
    pub fn next_backlog_task(&self, epic_id: EpicId) -> Result<Option<Task>, ServiceError> {
        // Verify the epic exists
        match self.db.get_epic(epic_id) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(ServiceError::NotFound(format!(
                    "Epic {} not found",
                    epic_id.0
                )))
            }
            Err(e) => return Err(ServiceError::Internal(format!("database error: {e}"))),
        }

        let tasks = self
            .db
            .list_tasks_for_epic(epic_id)
            .map_err(|e| ServiceError::Internal(format!("failed to list epic tasks: {e}")))?;

        let mut backlog: Vec<Task> = tasks
            .into_iter()
            .filter(|t| t.status == TaskStatus::Backlog)
            .collect();
        backlog.sort_by_key(|t| (t.sort_order.unwrap_or(t.id.0), t.id.0));

        Ok(backlog.into_iter().next())
    }
}
