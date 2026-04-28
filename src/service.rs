use std::sync::Arc;

use crate::db::{self, EpicPatch, TaskPatch};
use crate::models::{
    Epic, EpicId, ProjectId, SubStatus, Task, TaskId, TaskStatus, TaskTag, UsageReport,
    DEFAULT_BASE_BRANCH,
};

// ---------------------------------------------------------------------------
// Service error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ServiceError {
    /// Client-provided data is invalid (bad status, missing fields, etc.)
    Validation(String),
    /// Entity not found
    NotFound(String),
    /// Database or internal error
    Internal(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Validation(msg) => write!(f, "{msg}"),
            ServiceError::NotFound(msg) => write!(f, "{msg}"),
            ServiceError::Internal(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ServiceError {}

// ---------------------------------------------------------------------------
// FieldUpdate — explicit set-or-clear for nullable string fields
// ---------------------------------------------------------------------------

/// Replaces the `Option<String>` + empty-string sentinel pattern.
/// `Set(value)` sets the field, `Clear` sets it to NULL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldUpdate {
    Set(String),
    Clear,
}

// ---------------------------------------------------------------------------
// UpdateTaskParams — transport-agnostic input for update_task
// ---------------------------------------------------------------------------

pub struct UpdateTaskParams {
    pub task_id: i64,
    pub status: Option<TaskStatus>,
    pub plan_path: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub repo_path: Option<String>,
    pub sort_order: Option<i64>,
    pub pr_url: Option<FieldUpdate>,
    pub tag: Option<TaskTag>,
    pub sub_status: Option<SubStatus>,
    pub epic_id: Option<i64>,
    pub worktree: Option<FieldUpdate>,
    pub tmux_window: Option<FieldUpdate>,
    pub base_branch: Option<String>,
    pub project_id: Option<ProjectId>,
}

impl UpdateTaskParams {
    fn has_any_field(&self) -> bool {
        !self.updated_field_names().is_empty()
    }

    pub fn updated_field_names(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if self.status.is_some() {
            names.push("status");
        }
        if self.plan_path.is_some() {
            names.push("plan_path");
        }
        if self.title.is_some() {
            names.push("title");
        }
        if self.description.is_some() {
            names.push("description");
        }
        if self.repo_path.is_some() {
            names.push("repo_path");
        }
        if self.sort_order.is_some() {
            names.push("sort_order");
        }
        if self.pr_url.is_some() {
            names.push("pr_url");
        }
        if self.tag.is_some() {
            names.push("tag");
        }
        if self.sub_status.is_some() {
            names.push("sub_status");
        }
        if self.epic_id.is_some() {
            names.push("epic_id");
        }
        if self.worktree.is_some() {
            names.push("worktree");
        }
        if self.tmux_window.is_some() {
            names.push("tmux_window");
        }
        if self.base_branch.is_some() {
            names.push("base_branch");
        }
        if self.project_id.is_some() {
            names.push("project_id");
        }
        names
    }

    /// Create params with all optional fields unset (no-op except task_id).
    pub fn for_task(task_id: i64) -> Self {
        Self {
            task_id,
            status: None,
            plan_path: None,
            title: None,
            description: None,
            repo_path: None,
            sort_order: None,
            pr_url: None,
            tag: None,
            sub_status: None,
            epic_id: None,
            worktree: None,
            tmux_window: None,
            base_branch: None,
            project_id: None,
        }
    }

    pub fn status(mut self, status: TaskStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn plan_path(mut self, plan_path: Option<String>) -> Self {
        self.plan_path = plan_path;
        self
    }

    pub fn title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    pub fn description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    pub fn repo_path(mut self, repo_path: String) -> Self {
        self.repo_path = Some(repo_path);
        self
    }

    pub fn sort_order(mut self, sort_order: i64) -> Self {
        self.sort_order = Some(sort_order);
        self
    }

    pub fn pr_url(mut self, pr_url: FieldUpdate) -> Self {
        self.pr_url = Some(pr_url);
        self
    }

    pub fn tag(mut self, tag: Option<TaskTag>) -> Self {
        self.tag = tag;
        self
    }

    pub fn sub_status(mut self, sub_status: SubStatus) -> Self {
        self.sub_status = Some(sub_status);
        self
    }

    pub fn epic_id(mut self, epic_id: i64) -> Self {
        self.epic_id = Some(epic_id);
        self
    }

    pub fn worktree(mut self, worktree: FieldUpdate) -> Self {
        self.worktree = Some(worktree);
        self
    }

    pub fn tmux_window(mut self, tmux_window: FieldUpdate) -> Self {
        self.tmux_window = Some(tmux_window);
        self
    }

    pub fn base_branch(mut self, base_branch: Option<String>) -> Self {
        self.base_branch = base_branch;
        self
    }

    pub fn project_id(mut self, project_id: ProjectId) -> Self {
        self.project_id = Some(project_id);
        self
    }
}

// ---------------------------------------------------------------------------
// CreateTaskParams
// ---------------------------------------------------------------------------

pub struct CreateTaskParams {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub plan_path: Option<String>,
    pub epic_id: Option<i64>,
    pub sort_order: Option<i64>,
    pub tag: Option<TaskTag>,
    pub base_branch: Option<String>,
    pub project_id: ProjectId,
}

// ---------------------------------------------------------------------------
// ClaimTaskParams
// ---------------------------------------------------------------------------

pub struct ClaimTaskParams {
    pub task_id: i64,
    pub worktree: String,
    pub tmux_window: String,
}

// ---------------------------------------------------------------------------
// ListTasksFilter
// ---------------------------------------------------------------------------

pub struct ListTasksFilter {
    pub statuses: Option<Vec<TaskStatus>>,
    pub epic_id: Option<EpicId>,
}

// ---------------------------------------------------------------------------
// TaskService
// ---------------------------------------------------------------------------

/// Build a `TaskPatch` from `UpdateTaskParams`. The expanded repo path and
/// the (already-validated) sub_status are passed in separately because they
/// require either tilde-expansion or a database-bound check before being
/// committed to the patch.
fn build_task_patch<'a>(
    params: &'a UpdateTaskParams,
    expanded_repo_path: Option<&'a str>,
    sub_status: Option<SubStatus>,
) -> TaskPatch<'a> {
    let mut patch = TaskPatch::new();
    if let Some(s) = params.status {
        patch = patch.status(s);
    }
    if let Some(p) = params.plan_path.as_deref() {
        patch = patch.plan_path(Some(p));
    }
    if let Some(t) = params.title.as_deref() {
        patch = patch.title(t);
    }
    if let Some(d) = params.description.as_deref() {
        patch = patch.description(d);
    }
    if let Some(r) = expanded_repo_path {
        patch = patch.repo_path(r);
    }
    if let Some(so) = params.sort_order {
        patch = patch.sort_order(Some(so));
    }
    if let Some(update) = params.pr_url.as_ref() {
        patch = match update {
            FieldUpdate::Set(url) => patch.pr_url(Some(url.as_str())),
            FieldUpdate::Clear => patch.pr_url(None),
        };
    }
    if let Some(tag) = params.tag {
        patch = patch.tag(Some(tag));
    }
    if let Some(update) = params.worktree.as_ref() {
        patch = match update {
            FieldUpdate::Set(wt) => patch.worktree(Some(wt.as_str())),
            FieldUpdate::Clear => patch.worktree(None),
        };
    }
    if let Some(update) = params.tmux_window.as_ref() {
        patch = match update {
            FieldUpdate::Set(tw) => patch.tmux_window(Some(tw.as_str())),
            FieldUpdate::Clear => patch.tmux_window(None),
        };
    }
    if let Some(bb) = params.base_branch.as_deref() {
        patch = patch.base_branch(bb);
    }
    if let Some(ss) = sub_status {
        patch = patch.sub_status(ss);
    }
    if let Some(pid) = params.project_id {
        patch = patch.project_id(pid);
    }
    patch
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
    pub fn update_task(&self, params: UpdateTaskParams) -> Result<TaskId, ServiceError> {
        if !params.has_any_field() {
            return Err(ServiceError::Validation(
                "At least one field must be provided".into(),
            ));
        }

        let task_id = TaskId(params.task_id);
        let expanded_repo_path = params.repo_path.as_deref().map(crate::models::expand_tilde);
        let validated_sub_status = self.validate_sub_status(task_id, &params)?;
        let patch = build_task_patch(&params, expanded_repo_path.as_deref(), validated_sub_status);

        self.db
            .patch_task(task_id, &patch)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        if let Some(new_epic_id) = params.epic_id {
            let old_epic_id = self
                .db
                .get_task(task_id)
                .ok()
                .flatten()
                .and_then(|t| t.epic_id);
            self.db
                .set_task_epic_id(task_id, Some(EpicId(new_epic_id)))
                .map_err(|e| ServiceError::Internal(format!("Failed to link task to epic: {e}")))?;
            if let Some(old) = old_epic_id {
                self.recalculate_epic(old);
            }
            self.recalculate_epic(EpicId(new_epic_id));
        }

        if params.status.is_some() {
            self.recalculate_epic_for_task(task_id);
        }

        Ok(task_id)
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
            .create_task(
                &params.title,
                &params.description,
                &repo_path,
                plan.as_deref(),
                TaskStatus::Backlog,
                base_branch,
                params.epic_id.map(EpicId),
                params.sort_order,
                params.tag,
                params.project_id,
            )
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        self.get_task(task_id.0)
    }

    pub fn delete_task(&self, task_id: i64) -> Result<(), ServiceError> {
        // Verify task exists
        self.get_task(task_id)?;

        self.db
            .delete_task(TaskId(task_id))
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub fn get_task(&self, task_id: i64) -> Result<Task, ServiceError> {
        match self.db.get_task(TaskId(task_id)) {
            Ok(Some(task)) => Ok(task),
            Ok(None) => Err(ServiceError::NotFound(format!(
                "Task {} not found",
                task_id
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
                TaskId(params.task_id),
                &TaskPatch::new()
                    .status(TaskStatus::Running)
                    .worktree(Some(&params.worktree))
                    .tmux_window(Some(&params.tmux_window)),
            )
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        Ok(task)
    }

    pub fn validate_wrap_up(&self, task_id: i64) -> Result<Task, ServiceError> {
        let task = self.get_task(task_id)?;

        if !crate::dispatch::is_wrappable(&task) {
            return Err(ServiceError::Validation(format!(
                "Task {} cannot be wrapped up. Requires Running or Review status with a worktree.",
                task_id
            )));
        }

        Ok(task)
    }

    pub fn report_usage(&self, task_id: i64, usage: &UsageReport) -> Result<(), ServiceError> {
        // Verify task exists
        self.get_task(task_id)?;

        self.db
            .report_usage(TaskId(task_id), usage)
            .map_err(|e| ServiceError::Internal(format!("Failed to record usage: {e}")))
    }

    pub fn validate_send_message(
        &self,
        from_task_id: i64,
        to_task_id: i64,
    ) -> Result<(Task, Task), ServiceError> {
        let from_task = match self.db.get_task(TaskId(from_task_id)) {
            Ok(Some(t)) => t,
            Ok(None) => {
                return Err(ServiceError::NotFound(format!(
                    "sender task {} not found",
                    from_task_id
                )));
            }
            Err(e) => {
                return Err(ServiceError::Internal(format!(
                    "failed to look up sender: {e}"
                )));
            }
        };

        let to_task = match self.db.get_task(TaskId(to_task_id)) {
            Ok(Some(t)) => t,
            Ok(None) => {
                return Err(ServiceError::NotFound(format!(
                    "target task {} not found",
                    to_task_id
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
                to_task_id
            )));
        }

        if to_task.tmux_window.is_none() {
            return Err(ServiceError::Validation(format!(
                "target task {} has no tmux window (not running)",
                to_task_id
            )));
        }

        Ok((from_task, to_task))
    }

    /// Find the next backlog task for an epic, sorted by sort_order then id.
    /// Returns `Ok(None)` if no backlog tasks remain.
    pub fn next_backlog_task(&self, epic_id: i64) -> Result<Option<Task>, ServiceError> {
        // Verify the epic exists
        match self.db.get_epic(EpicId(epic_id)) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(ServiceError::NotFound(format!(
                    "Epic {} not found",
                    epic_id
                )))
            }
            Err(e) => return Err(ServiceError::Internal(format!("database error: {e}"))),
        }

        let tasks = self
            .db
            .list_tasks_for_epic(EpicId(epic_id))
            .map_err(|e| ServiceError::Internal(format!("failed to list epic tasks: {e}")))?;

        let mut backlog: Vec<Task> = tasks
            .into_iter()
            .filter(|t| t.status == TaskStatus::Backlog)
            .collect();
        backlog.sort_by_key(|t| (t.sort_order.unwrap_or(t.id.0), t.id.0));

        Ok(backlog.into_iter().next())
    }
}

// ---------------------------------------------------------------------------
// UpdateEpicParams
// ---------------------------------------------------------------------------

pub struct UpdateEpicParams {
    pub epic_id: i64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    pub repo_path: Option<String>,
    pub auto_dispatch: Option<bool>,
    pub feed_command: Option<FieldUpdate>,
    pub feed_interval_secs: Option<i64>,
    pub project_id: Option<ProjectId>,
}

impl UpdateEpicParams {
    fn has_any_field(&self) -> bool {
        !self.updated_field_names().is_empty()
    }

    pub fn updated_field_names(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if self.title.is_some() {
            names.push("title");
        }
        if self.description.is_some() {
            names.push("description");
        }
        if self.status.is_some() {
            names.push("status");
        }
        if self.plan_path.is_some() {
            names.push("plan_path");
        }
        if self.sort_order.is_some() {
            names.push("sort_order");
        }
        if self.repo_path.is_some() {
            names.push("repo_path");
        }
        if self.auto_dispatch.is_some() {
            names.push("auto_dispatch");
        }
        if self.feed_command.is_some() {
            names.push("feed_command");
        }
        if self.feed_interval_secs.is_some() {
            names.push("feed_interval_secs");
        }
        if self.project_id.is_some() {
            names.push("project_id");
        }
        names
    }
}

// ---------------------------------------------------------------------------
// CreateEpicParams
// ---------------------------------------------------------------------------

pub struct CreateEpicParams {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub sort_order: Option<i64>,
    pub parent_epic_id: Option<EpicId>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
    pub project_id: ProjectId,
}

// ---------------------------------------------------------------------------
// EpicService
// ---------------------------------------------------------------------------

pub struct EpicService {
    pub db: Arc<dyn db::EpicCrud>,
}

impl EpicService {
    pub fn new(db: Arc<dyn db::EpicCrud>) -> Self {
        Self { db }
    }

    pub fn create_epic(&self, params: CreateEpicParams) -> Result<Epic, ServiceError> {
        let epic = self
            .db
            .create_epic(
                &params.title,
                &params.description,
                &params.repo_path,
                params.parent_epic_id,
                params.project_id,
            )
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        let mut patch = EpicPatch::new();
        let mut has_extra = false;
        if let Some(so) = params.sort_order {
            patch = patch.sort_order(Some(so));
            has_extra = true;
        }
        if let Some(ref fc) = params.feed_command {
            patch = patch.feed_command(Some(fc.as_str()));
            has_extra = true;
        }
        if let Some(fi) = params.feed_interval_secs {
            patch = patch.feed_interval_secs(Some(fi));
            has_extra = true;
        }
        if has_extra {
            let _ = self.db.patch_epic(epic.id, &patch);
        }

        Ok(epic)
    }

    pub fn get_epic(&self, epic_id: i64) -> Result<Epic, ServiceError> {
        match self.db.get_epic(EpicId(epic_id)) {
            Ok(Some(epic)) => Ok(epic),
            Ok(None) => Err(ServiceError::NotFound(format!(
                "Epic {} not found",
                epic_id
            ))),
            Err(e) => Err(ServiceError::Internal(format!("Database error: {e}"))),
        }
    }

    pub fn get_epic_with_subtasks(&self, epic_id: i64) -> Result<(Epic, Vec<Task>), ServiceError> {
        let epic = self.get_epic(epic_id)?;
        let subtasks = self.db.list_tasks_for_epic(epic.id).unwrap_or_default();
        Ok((epic, subtasks))
    }

    pub fn list_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        self.db
            .list_epics()
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub fn list_root_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        self.db
            .list_root_epics()
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>, ServiceError> {
        self.db
            .list_sub_epics(parent_id)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub fn list_epics_with_progress(&self) -> Result<Vec<(Epic, usize, usize)>, ServiceError> {
        let epics = self.list_epics()?;
        let all_subtasks = self
            .db
            .list_all_tasks_with_epic_id()
            .map_err(|e| ServiceError::Internal(format!("Failed to list tasks with epic: {e}")))?;

        // Group tasks by epic_id in Rust — avoids N+1 queries
        let mut tasks_by_epic: std::collections::HashMap<i64, Vec<&Task>> =
            std::collections::HashMap::new();
        for task in &all_subtasks {
            if let Some(eid) = task.epic_id {
                tasks_by_epic.entry(eid.0).or_default().push(task);
            }
        }

        let result = epics
            .into_iter()
            .filter(|e| e.status != TaskStatus::Archived)
            .map(|e| {
                let subtasks = tasks_by_epic
                    .get(&e.id.0)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let done = subtasks
                    .iter()
                    .filter(|t| t.status == TaskStatus::Done)
                    .count();
                let total = subtasks.len();
                (e, done, total)
            })
            .collect();
        Ok(result)
    }

    pub fn update_epic(&self, params: UpdateEpicParams) -> Result<EpicId, ServiceError> {
        if !params.has_any_field() {
            return Err(ServiceError::Validation(
                "At least one field must be provided".into(),
            ));
        }

        let repo_path = params.repo_path.as_deref().map(crate::models::expand_tilde);
        let mut patch = EpicPatch::new();
        if let Some(ref t) = params.title {
            patch = patch.title(t);
        }
        if let Some(ref d) = params.description {
            patch = patch.description(d);
        }
        if let Some(status) = params.status {
            patch = patch.status(status);
        }
        if let Some(ref p) = params.plan_path {
            patch = patch.plan_path(Some(p.as_str()));
        }
        if let Some(so) = params.sort_order {
            patch = patch.sort_order(Some(so));
        }
        if let Some(ref rp) = repo_path {
            patch = patch.repo_path(rp);
        }
        if let Some(ad) = params.auto_dispatch {
            patch = patch.auto_dispatch(ad);
        }
        if let Some(ref fc) = params.feed_command {
            patch = patch.feed_command(match fc {
                FieldUpdate::Set(s) => Some(s.as_str()),
                FieldUpdate::Clear => None,
            });
        }
        if let Some(fi) = params.feed_interval_secs {
            patch = patch.feed_interval_secs(Some(fi));
        }
        if let Some(pid) = params.project_id {
            patch = patch.project_id(pid);
        }

        let epic_id = EpicId(params.epic_id);
        self.db
            .patch_epic(epic_id, &patch)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        Ok(epic_id)
    }

    pub fn delete_epic(&self, epic_id: i64) -> Result<(), ServiceError> {
        // Verify epic exists
        self.get_epic(epic_id)?;

        self.db
            .delete_epic(EpicId(epic_id))
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }
}

// ---------------------------------------------------------------------------
// CreateLearningParams / UpdateLearningParams
// ---------------------------------------------------------------------------

pub struct CreateLearningParams {
    pub kind: crate::models::LearningKind,
    pub summary: String,
    pub detail: Option<String>,
    pub scope: crate::models::LearningScope,
    pub scope_ref: Option<String>,
    pub tags: Vec<String>,
    pub source_task_id: Option<TaskId>,
}

pub struct UpdateLearningParams {
    pub id: crate::models::LearningId,
    pub summary: Option<String>,
    /// `None` = don't change; `Some(None)` = clear; `Some(Some(v))` = set.
    pub detail: Option<Option<String>>,
    pub kind: Option<crate::models::LearningKind>,
    pub tags: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// LearningService
// ---------------------------------------------------------------------------

pub struct LearningService {
    pub db: Arc<dyn db::LearningStore>,
}

impl LearningService {
    pub fn new(db: Arc<dyn db::LearningStore>) -> Self {
        Self { db }
    }

    pub fn create_learning(
        &self,
        params: CreateLearningParams,
    ) -> Result<crate::models::LearningId, ServiceError> {
        if params.summary.trim().is_empty() {
            return Err(ServiceError::Validation("summary must not be empty".into()));
        }
        match params.scope {
            crate::models::LearningScope::User => {
                if params.scope_ref.is_some() {
                    return Err(ServiceError::Validation(
                        "scope_ref must be null for user-scoped learnings".into(),
                    ));
                }
            }
            _ => {
                if params.scope_ref.is_none() {
                    return Err(ServiceError::Validation(
                        "scope_ref is required for non-user-scoped learnings".into(),
                    ));
                }
            }
        }
        self.db
            .create_learning(
                params.kind,
                &params.summary,
                params.detail.as_deref(),
                params.scope,
                params.scope_ref.as_deref(),
                &params.tags,
                params.source_task_id,
            )
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn get_learning(
        &self,
        id: crate::models::LearningId,
    ) -> Result<crate::models::Learning, ServiceError> {
        self.db
            .get_learning(id)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))?
            .ok_or_else(|| ServiceError::NotFound(format!("learning {id} not found")))
    }

    pub fn list_learnings(
        &self,
        filter: db::LearningFilter,
    ) -> Result<Vec<crate::models::Learning>, ServiceError> {
        self.db
            .list_learnings(filter)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn approve_learning(&self, id: crate::models::LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if learning.status != crate::models::LearningStatus::Proposed {
            return Err(ServiceError::Validation(format!(
                "can only approve a proposed learning (current status: {})",
                learning.status
            )));
        }
        self.db
            .patch_learning(
                id,
                &db::LearningPatch::new().status(crate::models::LearningStatus::Approved),
            )
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn reject_learning(&self, id: crate::models::LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if learning.status.is_terminal() {
            return Err(ServiceError::Validation(format!(
                "cannot reject a {} learning",
                learning.status
            )));
        }
        self.db
            .patch_learning(
                id,
                &db::LearningPatch::new().status(crate::models::LearningStatus::Rejected),
            )
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn archive_learning(&self, id: crate::models::LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if learning.status != crate::models::LearningStatus::Approved {
            return Err(ServiceError::Validation(format!(
                "can only archive an approved learning (current status: {})",
                learning.status
            )));
        }
        self.db
            .patch_learning(
                id,
                &db::LearningPatch::new().status(crate::models::LearningStatus::Archived),
            )
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn update_learning(&self, params: UpdateLearningParams) -> Result<(), ServiceError> {
        let learning = self.get_learning(params.id)?;
        if learning.status != crate::models::LearningStatus::Proposed
            && learning.status != crate::models::LearningStatus::Approved
        {
            return Err(ServiceError::Validation(format!(
                "can only edit proposed or approved learnings (current status: {})",
                learning.status
            )));
        }
        if let Some(ref s) = params.summary {
            if s.trim().is_empty() {
                return Err(ServiceError::Validation("summary must not be empty".into()));
            }
        }
        let mut patch = db::LearningPatch::new();
        if let Some(ref s) = params.summary {
            patch = patch.summary(s.as_str());
        }
        if let Some(ref d) = params.detail {
            patch = patch.detail(d.as_deref());
        }
        if let Some(k) = params.kind {
            patch = patch.kind(k);
        }
        if let Some(ref t) = params.tags {
            patch = patch.tags(t.as_slice());
        }
        self.db
            .patch_learning(params.id, &patch)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn confirm_learning(&self, id: crate::models::LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if learning.status != crate::models::LearningStatus::Approved {
            return Err(ServiceError::Validation(format!(
                "can only confirm an approved learning (current status: {})",
                learning.status
            )));
        }
        self.db
            .confirm_learning(id)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, ProjectCrud, TaskCrud};

    fn test_db() -> Arc<dyn db::TaskStore> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn task_svc(db: &Arc<dyn db::TaskStore>) -> TaskService {
        let d: Arc<dyn db::TaskAndEpicStore> = db.clone();
        TaskService::new(d)
    }

    fn epic_svc(db: &Arc<dyn db::TaskStore>) -> EpicService {
        let d: Arc<dyn db::EpicCrud> = db.clone();
        EpicService::new(d)
    }

    // -- TaskService ----------------------------------------------------------

    #[test]
    fn create_and_get_task() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "Test".into(),
                description: "desc".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.title, "Test");
        assert_eq!(task.status, TaskStatus::Backlog);
    }

    #[test]
    fn create_task_with_tag() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "Bug fix".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: Some(5),
                tag: Some(TaskTag::Bug),
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.tag, Some(TaskTag::Bug));
        assert_eq!(task.sort_order, Some(5));
    }

    #[test]
    fn create_task_with_sort_order() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "Sorted".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: Some(42),
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.sort_order, Some(42));
    }

    #[test]
    fn update_task_status() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Running))
            .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.status, TaskStatus::Running);
    }

    // Note: Done/Archived restriction moved to MCP handler layer.
    // The service now allows any status transition (TUI needs it).

    #[test]
    fn update_task_no_fields_returns_error() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let err = svc
            .update_task(UpdateTaskParams::for_task(id.0))
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn update_task_params_builder_compiles() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Running))
            .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.status, TaskStatus::Running);
    }

    #[test]
    fn update_task_invalid_substatus_for_status() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // active is not valid for backlog
        let err = svc
            .update_task(UpdateTaskParams::for_task(id.0).sub_status(SubStatus::Active))
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn claim_task_success() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let task = svc
            .claim_task(ClaimTaskParams {
                task_id: id.0,
                worktree: "/repo/.worktrees/feature".into(),
                tmux_window: "win1".into(),
            })
            .unwrap();
        assert_eq!(task.title, "T");

        // Verify it was actually updated
        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/feature"));
    }

    #[test]
    fn claim_task_wrong_repo() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo-a".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let err = svc
            .claim_task(ClaimTaskParams {
                task_id: id.0,
                worktree: "/repo-b/.worktrees/feature".into(),
                tmux_window: "win1".into(),
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn claim_task_not_backlog() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // Move to running first
        svc.update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Running))
            .unwrap();

        let err = svc
            .claim_task(ClaimTaskParams {
                task_id: id.0,
                worktree: "/repo/.worktrees/feature".into(),
                tmux_window: "win1".into(),
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn list_tasks_with_filter() {
        let db = test_db();
        let svc = task_svc(&db);

        svc.create_task(CreateTaskParams {
            title: "T1".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: 1,
        })
        .unwrap();

        let tasks = svc
            .list_tasks(ListTasksFilter {
                statuses: Some(vec![TaskStatus::Backlog]),
                epic_id: None,
            })
            .unwrap();
        assert_eq!(tasks.len(), 1);

        let tasks = svc
            .list_tasks(ListTasksFilter {
                statuses: Some(vec![TaskStatus::Running]),
                epic_id: None,
            })
            .unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn get_task_not_found() {
        let db = test_db();
        let svc = task_svc(&db);
        let err = svc.get_task(999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    #[test]
    fn report_usage_for_nonexistent_task() {
        let db = test_db();
        let svc = task_svc(&db);
        let err = svc
            .report_usage(
                999,
                &UsageReport {
                    cost_usd: 1.0,
                    input_tokens: 100,
                    output_tokens: 50,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                },
            )
            .unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    #[test]
    fn update_task_with_epic_linkage() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "Epic".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let id = task_svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .update_task(UpdateTaskParams::for_task(id.0).epic_id(epic.id.0))
            .unwrap();

        let task = task_svc.get_task(id.0).unwrap();
        assert_eq!(task.epic_id, Some(epic.id));
    }

    #[test]
    fn update_task_status_recalculates_parent_epic() {
        // Status-change branch of recalculate_epic_for_task: an epic that
        // contains a single task should follow the task's status.
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let id = task_svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Running))
            .unwrap();

        let refreshed = epic_svc.get_epic(epic.id.0).unwrap();
        assert_eq!(refreshed.status, TaskStatus::Running);
    }

    #[test]
    fn update_task_relink_recalculates_old_and_new_epic() {
        // Linkage-change branch of recalculate_epic_for_task: moving a Running
        // task between two epics should leave the old epic empty (Backlog) and
        // the new epic Running.
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic_a = epic_svc
            .create_epic(CreateEpicParams {
                title: "A".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();
        let epic_b = epic_svc
            .create_epic(CreateEpicParams {
                title: "B".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let id = task_svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic_a.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();
        task_svc
            .update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Running))
            .unwrap();

        // Sanity: epic A is now Running.
        assert_eq!(
            epic_svc.get_epic(epic_a.id.0).unwrap().status,
            TaskStatus::Running
        );

        task_svc
            .update_task(UpdateTaskParams::for_task(id.0).epic_id(epic_b.id.0))
            .unwrap();

        assert_eq!(
            epic_svc.get_epic(epic_a.id.0).unwrap().status,
            TaskStatus::Backlog
        );
        assert_eq!(
            epic_svc.get_epic(epic_b.id.0).unwrap().status,
            TaskStatus::Running
        );
    }

    // -- EpicService ----------------------------------------------------------

    #[test]
    fn create_and_get_epic() {
        let db = test_db();
        let svc = epic_svc(&db);

        let epic = svc
            .create_epic(CreateEpicParams {
                title: "Epic 1".into(),
                description: "desc".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let fetched = svc.get_epic(epic.id.0).unwrap();
        assert_eq!(fetched.title, "Epic 1");
    }

    #[test]
    fn get_epic_not_found() {
        let db = test_db();
        let svc = epic_svc(&db);
        let err = svc.get_epic(999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    #[test]
    fn update_epic_status() {
        let db = test_db();
        let svc = epic_svc(&db);

        let epic = svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_epic(UpdateEpicParams {
            epic_id: epic.id.0,
            title: None,
            description: None,
            status: Some(TaskStatus::Running),
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        })
        .unwrap();

        let updated = svc.get_epic(epic.id.0).unwrap();
        assert_eq!(updated.status, TaskStatus::Running);
    }

    #[test]
    fn update_epic_no_fields_returns_error() {
        let db = test_db();
        let svc = epic_svc(&db);

        let epic = svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let err = svc
            .update_epic(UpdateEpicParams {
                epic_id: epic.id.0,
                title: None,
                description: None,
                status: None,
                plan_path: None,
                sort_order: None,
                repo_path: None,
                auto_dispatch: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn update_epic_auto_dispatch_persists() {
        let db = test_db();
        let svc = epic_svc(&db);

        let epic = svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        assert!(db.get_epic(epic.id).unwrap().unwrap().auto_dispatch);

        svc.update_epic(UpdateEpicParams {
            epic_id: epic.id.0,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: Some(false),
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        })
        .unwrap();

        assert!(!db.get_epic(epic.id).unwrap().unwrap().auto_dispatch);
    }

    #[test]
    fn list_epics_with_progress() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .create_task(CreateTaskParams {
                title: "Sub1".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let list = epic_svc.list_epics_with_progress().unwrap();
        assert_eq!(list.len(), 1);
        let (_, done, total) = &list[0];
        assert_eq!(*done, 0);
        assert_eq!(*total, 1);
    }

    #[test]
    fn list_epics_with_progress_multiple_epics() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let e1 = epic_svc
            .create_epic(CreateEpicParams {
                title: "E1".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();
        let e2 = epic_svc
            .create_epic(CreateEpicParams {
                title: "E2".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        // 2 tasks in E1
        let t1 = task_svc
            .create_task(CreateTaskParams {
                title: "T1".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(e1.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();
        task_svc
            .create_task(CreateTaskParams {
                title: "T2".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(e1.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();
        // 1 task in E2
        task_svc
            .create_task(CreateTaskParams {
                title: "T3".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(e2.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // Mark T1 as done
        task_svc
            .update_task(UpdateTaskParams::for_task(t1.0).status(TaskStatus::Done))
            .unwrap();

        let list = epic_svc.list_epics_with_progress().unwrap();
        assert_eq!(list.len(), 2);
        let e1_progress = list.iter().find(|(e, _, _)| e.id == e1.id).unwrap();
        assert_eq!(e1_progress.1, 1); // 1 done
        assert_eq!(e1_progress.2, 2); // 2 total
        let e2_progress = list.iter().find(|(e, _, _)| e.id == e2.id).unwrap();
        assert_eq!(e2_progress.1, 0);
        assert_eq!(e2_progress.2, 1);
    }

    #[test]
    fn update_task_status_recalculates_epic() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let task_id = task_svc
            .create_task(CreateTaskParams {
                title: "Sub".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .update_task(UpdateTaskParams::for_task(task_id.0).status(TaskStatus::Done))
            .unwrap();

        let updated_epic = epic_svc.get_epic(epic.id.0).unwrap();
        assert_eq!(updated_epic.status, TaskStatus::Done);
    }

    #[test]
    fn get_epic_with_subtasks() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .create_task(CreateTaskParams {
                title: "Sub".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let (e, subtasks) = epic_svc.get_epic_with_subtasks(epic.id.0).unwrap();
        assert_eq!(e.title, "E");
        assert_eq!(subtasks.len(), 1);
    }

    // -- next_backlog_task -----------------------------------------------------

    #[test]
    fn next_backlog_task_returns_first_by_sort_order() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .create_task(CreateTaskParams {
                title: "Second".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: Some(20),
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        task_svc
            .create_task(CreateTaskParams {
                title: "First".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: Some(10),
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let next = task_svc.next_backlog_task(epic.id.0).unwrap();
        assert_eq!(next.unwrap().title, "First");
    }

    #[test]
    fn next_backlog_task_skips_non_backlog() {
        let db = test_db();
        let task_svc = task_svc(&db);
        let epic_svc = epic_svc(&db);

        let epic = epic_svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let id = task_svc
            .create_task(CreateTaskParams {
                title: "Running".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: Some(1),
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // Move to running
        task_svc
            .update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Running))
            .unwrap();

        let next = task_svc.next_backlog_task(epic.id.0).unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn next_backlog_task_epic_not_found() {
        let db = test_db();
        let svc = task_svc(&db);
        let err = svc.next_backlog_task(999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    // -- create_task_returning ---------------------------------------------------

    #[test]
    fn create_task_returning_gives_full_task() {
        let db = test_db();
        let svc = task_svc(&db);

        let task = svc
            .create_task_returning(CreateTaskParams {
                title: "Full task".into(),
                description: "desc".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: Some(TaskTag::Feature),
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        assert_eq!(task.title, "Full task");
        assert_eq!(task.description, "desc");
        assert_eq!(task.tag, Some(TaskTag::Feature));
        assert_eq!(task.status, TaskStatus::Backlog);
    }

    #[test]
    fn create_task_returning_with_epic() {
        let db = test_db();
        let tsvc = task_svc(&db);
        let esvc = epic_svc(&db);

        let epic = esvc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let task = tsvc
            .create_task_returning(CreateTaskParams {
                title: "Sub".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        assert_eq!(task.epic_id, Some(epic.id));
    }

    #[test]
    fn create_task_returning_sets_all_optional_fields_atomically() {
        let db = test_db();
        let tsvc = task_svc(&db);
        let esvc = epic_svc(&db);

        let epic = esvc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let task = tsvc
            .create_task_returning(CreateTaskParams {
                title: "Atomic".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: Some(3),
                tag: Some(TaskTag::Feature),
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        assert_eq!(task.epic_id, Some(epic.id));
        assert_eq!(task.sort_order, Some(3));
        assert_eq!(task.tag, Some(TaskTag::Feature));
    }

    // -- delete_task -------------------------------------------------------------

    #[test]
    fn delete_task_removes_it() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.delete_task(id.0).unwrap();

        let err = svc.get_task(id.0).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    #[test]
    fn delete_task_not_found() {
        let db = test_db();
        let svc = task_svc(&db);
        let err = svc.delete_task(999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    // -- update_task with worktree/tmux_window -----------------------------------

    #[test]
    fn update_task_sets_worktree_and_tmux_window() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_task(
            UpdateTaskParams::for_task(id.0)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
                .tmux_window(FieldUpdate::Set("task-1".into())),
        )
        .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/feat"));
        assert_eq!(task.tmux_window.as_deref(), Some("task-1"));
    }

    #[test]
    fn update_task_clears_worktree() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // Set worktree
        svc.update_task(
            UpdateTaskParams::for_task(id.0)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into()))
                .tmux_window(FieldUpdate::Set("task-1".into())),
        )
        .unwrap();

        // Clear worktree via FieldUpdate::Clear
        svc.update_task(
            UpdateTaskParams::for_task(id.0)
                .status(TaskStatus::Done)
                .worktree(FieldUpdate::Clear)
                .tmux_window(FieldUpdate::Clear),
        )
        .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert!(task.worktree.is_none());
        assert!(task.tmux_window.is_none());
    }

    // -- update_task allows done/archived (MCP restriction moved to handler) -----

    #[test]
    fn update_task_allows_done_status() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Done))
            .unwrap();

        let task = svc.get_task(id.0).unwrap();
        assert_eq!(task.status, TaskStatus::Done);
    }

    // -- delete_epic -------------------------------------------------------------

    #[test]
    fn delete_epic_removes_it() {
        let db = test_db();
        let svc = epic_svc(&db);

        let epic = svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        svc.delete_epic(epic.id.0).unwrap();

        let err = svc.get_epic(epic.id.0).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    #[test]
    fn delete_epic_not_found() {
        let db = test_db();
        let svc = epic_svc(&db);
        let err = svc.delete_epic(999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    // --- FieldUpdate ---

    #[test]
    fn field_update_set_has_value() {
        let fu: FieldUpdate = FieldUpdate::Set("hello".to_string());
        assert!(matches!(fu, FieldUpdate::Set(ref s) if s == "hello"));
    }

    #[test]
    fn field_update_clear_is_clear() {
        let fu: FieldUpdate = FieldUpdate::Clear;
        assert!(matches!(fu, FieldUpdate::Clear));
    }

    #[test]
    fn update_task_worktree_set_persists() {
        let db = test_db();
        let svc = task_svc(&db);
        let id = svc
            .create_task(CreateTaskParams {
                title: "t".into(),
                description: "d".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();
        svc.update_task(
            UpdateTaskParams::for_task(id.0)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/wt".to_string()))
                .tmux_window(FieldUpdate::Set("win".to_string())),
        )
        .unwrap();
        let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
        assert_eq!(task.worktree.as_deref(), Some("/wt"));
        assert_eq!(task.tmux_window.as_deref(), Some("win"));
    }

    #[test]
    fn update_task_worktree_clear_sets_null() {
        let db = test_db();
        let svc = task_svc(&db);
        let id = svc
            .create_task(CreateTaskParams {
                title: "t".into(),
                description: "d".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();
        // First set a value
        svc.update_task(
            UpdateTaskParams::for_task(id.0)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/wt".to_string()))
                .tmux_window(FieldUpdate::Set("win".to_string())),
        )
        .unwrap();
        // Then clear it
        svc.update_task(
            UpdateTaskParams::for_task(id.0)
                .worktree(FieldUpdate::Clear)
                .tmux_window(FieldUpdate::Clear),
        )
        .unwrap();
        let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
        assert_eq!(task.worktree, None);
        assert_eq!(task.tmux_window, None);
    }

    #[test]
    fn update_task_pr_url_set_and_clear() {
        let db = test_db();
        let svc = task_svc(&db);
        let id = svc
            .create_task(CreateTaskParams {
                title: "t".into(),
                description: "d".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();
        // Set PR URL
        svc.update_task(UpdateTaskParams::for_task(id.0).pr_url(FieldUpdate::Set(
            "https://github.com/org/repo/pull/1".to_string(),
        )))
        .unwrap();
        let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
        assert_eq!(
            task.pr_url.as_deref(),
            Some("https://github.com/org/repo/pull/1")
        );
        // Clear PR URL
        svc.update_task(UpdateTaskParams::for_task(id.0).pr_url(FieldUpdate::Clear))
            .unwrap();
        let task = db.get_task(TaskId(id.0)).unwrap().unwrap();
        assert_eq!(task.pr_url, None);
    }

    #[test]
    fn list_tasks_filters_by_epic_id() {
        let db = test_db();
        let svc = task_svc(&db);
        let esvc = epic_svc(&db);

        let epic = esvc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let id1 = svc
            .create_task(CreateTaskParams {
                title: "In epic".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: Some(epic.id.0),
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let _id2 = svc
            .create_task(CreateTaskParams {
                title: "No epic".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let tasks = svc
            .list_tasks(ListTasksFilter {
                statuses: None,
                epic_id: Some(epic.id),
            })
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, id1);
    }

    #[test]
    fn list_tasks_excludes_archived_by_default() {
        let db = test_db();
        let svc = task_svc(&db);

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_task(UpdateTaskParams::for_task(id.0).status(TaskStatus::Archived))
            .unwrap();

        let tasks = svc
            .list_tasks(ListTasksFilter {
                statuses: None,
                epic_id: None,
            })
            .unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn validate_send_message_missing_worktree() {
        let db = test_db();
        let svc = task_svc(&db);

        let from_id = svc
            .create_task(CreateTaskParams {
                title: "Sender".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // Target task has no worktree (still backlog)
        let to_id = svc
            .create_task(CreateTaskParams {
                title: "Receiver".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let err = svc.validate_send_message(from_id.0, to_id.0).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
        assert!(err.to_string().contains("no worktree"));
    }

    #[test]
    fn validate_send_message_missing_tmux_window() {
        let db = test_db();
        let svc = task_svc(&db);

        let from_id = svc
            .create_task(CreateTaskParams {
                title: "Sender".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let to_id = svc
            .create_task(CreateTaskParams {
                title: "Receiver".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        // Set worktree but not tmux_window
        svc.update_task(
            UpdateTaskParams::for_task(to_id.0)
                .status(TaskStatus::Running)
                .worktree(FieldUpdate::Set("/repo/.worktrees/feat".into())),
        )
        .unwrap();

        let err = svc.validate_send_message(from_id.0, to_id.0).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
        assert!(err.to_string().contains("no tmux window"));
    }

    #[test]
    fn validate_send_message_target_not_found() {
        let db = test_db();
        let svc = task_svc(&db);

        let from_id = svc
            .create_task(CreateTaskParams {
                title: "Sender".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        let err = svc.validate_send_message(from_id.0, 999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    // -- UpdateTaskParams::updated_field_names ----------------------------------

    #[test]
    fn update_task_params_field_names_returns_str_slices() {
        // Verify return type is Vec<&str> (not Vec<String>) — consistent with UpdateEpicParams.
        let params = UpdateTaskParams::for_task(1).title("x".to_string());
        let names: Vec<&str> = params.updated_field_names();
        assert!(names.contains(&"title"));
    }

    // -- has_any_field / updated_field_names consistency ----------------------

    #[test]
    fn update_task_params_has_any_field_consistent_with_updated_field_names() {
        // When a field is set, both has_any_field() and updated_field_names() must agree.
        // If a new field is added to UpdateTaskParams without updating both methods,
        // this test will catch the divergence.
        let with_field = UpdateTaskParams::for_task(1).title("x".to_string());
        assert!(
            with_field.has_any_field(),
            "has_any_field should be true when title is set"
        );
        assert!(
            !with_field.updated_field_names().is_empty(),
            "updated_field_names should be non-empty when title is set"
        );

        let empty = UpdateTaskParams::for_task(1);
        assert!(
            !empty.has_any_field(),
            "has_any_field should be false when no fields are set"
        );
        assert!(
            empty.updated_field_names().is_empty(),
            "updated_field_names should be empty when no fields are set"
        );
    }

    #[test]
    fn update_epic_params_has_any_field_consistent_with_updated_field_names() {
        // Same consistency guard for UpdateEpicParams.
        let with_field = UpdateEpicParams {
            epic_id: 1,
            title: Some("x".to_string()),
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        };
        assert!(
            with_field.has_any_field(),
            "has_any_field should be true when title is set"
        );
        assert!(
            !with_field.updated_field_names().is_empty(),
            "updated_field_names should be non-empty when title is set"
        );

        let empty = UpdateEpicParams {
            epic_id: 1,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        };
        assert!(
            !empty.has_any_field(),
            "has_any_field should be false when no fields are set"
        );
        assert!(
            empty.updated_field_names().is_empty(),
            "updated_field_names should be empty when no fields are set"
        );
    }

    #[test]
    fn update_task_params_every_field_covered() {
        // Each field set individually must trigger both has_any_field() and
        // updated_field_names(). Add a case here whenever a new field is added
        // to UpdateTaskParams so both methods stay in sync.
        let cases: Vec<UpdateTaskParams> = vec![
            UpdateTaskParams::for_task(1).status(TaskStatus::Backlog),
            UpdateTaskParams::for_task(1).plan_path(Some("p".to_string())),
            UpdateTaskParams::for_task(1).title("t".to_string()),
            UpdateTaskParams::for_task(1).description("d".to_string()),
            UpdateTaskParams::for_task(1).repo_path("r".to_string()),
            UpdateTaskParams::for_task(1).sort_order(0),
            UpdateTaskParams::for_task(1).pr_url(FieldUpdate::Set("u".to_string())),
            UpdateTaskParams::for_task(1).tag(Some(TaskTag::Bug)),
            UpdateTaskParams::for_task(1).sub_status(SubStatus::Active),
            UpdateTaskParams::for_task(1).epic_id(1),
            UpdateTaskParams::for_task(1).worktree(FieldUpdate::Set("w".to_string())),
            UpdateTaskParams::for_task(1).tmux_window(FieldUpdate::Set("tw".to_string())),
            UpdateTaskParams::for_task(1).base_branch(Some("main".to_string())),
            UpdateTaskParams::for_task(1).project_id(1),
        ];
        for params in &cases {
            assert!(
                params.has_any_field(),
                "has_any_field() should be true when a field is set"
            );
            assert!(
                !params.updated_field_names().is_empty(),
                "updated_field_names() should be non-empty when a field is set"
            );
        }
    }

    #[test]
    fn update_epic_params_every_field_covered() {
        // Each field set individually must trigger both has_any_field() and
        // updated_field_names(). Add a case here whenever a new field is added
        // to UpdateEpicParams so both methods stay in sync.
        let base = || UpdateEpicParams {
            epic_id: 1,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
        };
        let cases: Vec<UpdateEpicParams> = vec![
            UpdateEpicParams {
                title: Some("t".to_string()),
                ..base()
            },
            UpdateEpicParams {
                description: Some("d".to_string()),
                ..base()
            },
            UpdateEpicParams {
                status: Some(TaskStatus::Backlog),
                ..base()
            },
            UpdateEpicParams {
                plan_path: Some("p".to_string()),
                ..base()
            },
            UpdateEpicParams {
                sort_order: Some(0),
                ..base()
            },
            UpdateEpicParams {
                repo_path: Some("r".to_string()),
                ..base()
            },
            UpdateEpicParams {
                auto_dispatch: Some(true),
                ..base()
            },
            UpdateEpicParams {
                feed_command: Some(FieldUpdate::Set("cmd".to_string())),
                ..base()
            },
            UpdateEpicParams {
                feed_interval_secs: Some(300),
                ..base()
            },
            UpdateEpicParams {
                project_id: Some(1),
                ..base()
            },
        ];
        for params in &cases {
            assert!(
                params.has_any_field(),
                "has_any_field() should be true when a field is set"
            );
            assert!(
                !params.updated_field_names().is_empty(),
                "updated_field_names() should be non-empty when a field is set"
            );
        }
    }

    // -------------------------------------------------------------------------
    // project_id propagation tests
    // -------------------------------------------------------------------------

    #[test]
    fn create_task_with_explicit_project_id() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let svc = TaskService::new(db.clone() as Arc<dyn db::TaskAndEpicStore>);
        let default_id = db.get_default_project().unwrap().id;
        let other = db.create_project("Other", 1).unwrap();

        let result = svc.create_task(CreateTaskParams {
            title: "T".to_string(),
            description: String::new(),
            repo_path: "/r".to_string(),
            plan_path: None,
            epic_id: None,
            sort_order: None,
            tag: None,
            base_branch: None,
            project_id: other.id,
        });
        assert!(result.is_ok());
        let task_id = result.unwrap();
        let task = db
            .get_task(crate::models::TaskId(task_id.0))
            .unwrap()
            .unwrap();
        assert_eq!(task.project_id, other.id);
        assert_ne!(task.project_id, default_id);
    }

    // -------------------------------------------------------------------------
    // Epic-in-epic service tests
    // -------------------------------------------------------------------------

    #[test]
    fn create_sub_epic_links_parent() {
        let db = test_db();
        let svc = epic_svc(&db);

        let parent = svc
            .create_epic(CreateEpicParams {
                title: "Parent".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let child = svc
            .create_epic(CreateEpicParams {
                title: "Child".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: Some(parent.id),
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        assert_eq!(child.parent_epic_id, Some(parent.id));

        let fetched = svc.get_epic(child.id.0).unwrap();
        assert_eq!(fetched.parent_epic_id, Some(parent.id));
    }

    #[test]
    fn list_root_epics_service() {
        let db = test_db();
        let svc = epic_svc(&db);

        let parent = svc
            .create_epic(CreateEpicParams {
                title: "Root".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();
        svc.create_epic(CreateEpicParams {
            title: "Sub".into(),
            description: "".into(),
            repo_path: "/repo".into(),
            sort_order: None,
            parent_epic_id: Some(parent.id),
            feed_command: None,
            feed_interval_secs: None,
            project_id: 1,
        })
        .unwrap();

        let roots = svc.list_root_epics().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, parent.id);
    }

    #[test]
    fn list_sub_epics_service() {
        let db = test_db();
        let svc = epic_svc(&db);

        let parent = svc
            .create_epic(CreateEpicParams {
                title: "Parent".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();
        let child = svc
            .create_epic(CreateEpicParams {
                title: "Child".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: Some(parent.id),
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        let subs = svc.list_sub_epics(parent.id).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].id, child.id);
    }

    // -- project_id in update_task --------------------------------------------

    #[test]
    fn update_task_project_id_moves_task() {
        let db = test_db();
        let svc = task_svc(&db);
        let d: Arc<dyn db::ProjectCrud> = db.clone();
        let other = d.create_project("Dispatch", 1).unwrap();

        let id = svc
            .create_task(CreateTaskParams {
                title: "T".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                plan_path: None,
                epic_id: None,
                sort_order: None,
                tag: None,
                base_branch: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_task(UpdateTaskParams::for_task(id.0).project_id(other.id))
            .unwrap();

        let db2: Arc<dyn db::TaskCrud> = db.clone();
        let task = db2.get_task(id).unwrap().unwrap();
        assert_eq!(task.project_id, other.id);
    }

    // -- project_id in update_epic --------------------------------------------

    #[test]
    fn update_epic_project_id_moves_epic() {
        let db = test_db();
        let svc = epic_svc(&db);
        let d: Arc<dyn db::ProjectCrud> = db.clone();
        let other = d.create_project("Dispatch", 1).unwrap();

        let epic = svc
            .create_epic(CreateEpicParams {
                title: "E".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
                parent_epic_id: None,
                feed_command: None,
                feed_interval_secs: None,
                project_id: 1,
            })
            .unwrap();

        svc.update_epic(UpdateEpicParams {
            epic_id: epic.id.0,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: Some(other.id),
        })
        .unwrap();

        let d2: Arc<dyn db::EpicCrud> = db.clone();
        let epics = d2.list_epics().unwrap();
        let updated = epics.iter().find(|e| e.id == epic.id).unwrap();
        assert_eq!(updated.project_id, other.id);
    }
}

// ---------------------------------------------------------------------------
// LearningService tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod learning_tests {
    use super::{CreateLearningParams, LearningService, ServiceError, UpdateLearningParams};
    use crate::db::Database;
    use crate::models::{LearningKind, LearningScope, LearningStatus};
    use std::sync::Arc;

    fn service() -> LearningService {
        let db = Arc::new(Database::open_in_memory().unwrap());
        LearningService::new(db)
    }

    #[test]
    fn create_learning_rejects_empty_summary() {
        let svc = service();
        let err = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn create_learning_rejects_user_scope_with_scope_ref() {
        let svc = service();
        let err = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Preference,
                summary: "Some preference".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: Some("should-be-null".to_string()),
                tags: vec![],
                source_task_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn create_learning_rejects_non_user_scope_without_scope_ref() {
        let svc = service();
        let err = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::Repo,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn create_learning_succeeds_with_valid_params() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "Use Arc for shared state".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Proposed);
    }

    #[test]
    fn approve_learning_from_proposed_succeeds() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.approve_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Approved);
    }

    #[test]
    fn approve_learning_from_approved_fails() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.approve_learning(id).unwrap();
        let err = svc.approve_learning(id).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn reject_learning_from_proposed_succeeds() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Pitfall,
                summary: "A pitfall".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.reject_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Rejected);
    }

    #[test]
    fn reject_learning_from_archived_fails() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.approve_learning(id).unwrap();
        svc.archive_learning(id).unwrap();
        let err = svc.reject_learning(id).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn archive_learning_from_approved_succeeds() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.approve_learning(id).unwrap();
        svc.archive_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Archived);
    }

    #[test]
    fn archive_learning_from_proposed_fails() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        let err = svc.archive_learning(id).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn update_learning_on_rejected_fails() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.reject_learning(id).unwrap();
        let err = svc
            .update_learning(UpdateLearningParams {
                id,
                summary: Some("Updated".to_string()),
                detail: None,
                kind: None,
                tags: None,
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn update_learning_rejects_empty_summary() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        let err = svc
            .update_learning(UpdateLearningParams {
                id,
                summary: Some("".to_string()),
                detail: None,
                kind: None,
                tags: None,
            })
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn confirm_learning_on_proposed_fails() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        let err = svc.confirm_learning(id).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn confirm_learning_on_approved_succeeds() {
        let svc = service();
        let id = svc
            .create_learning(CreateLearningParams {
                kind: LearningKind::Convention,
                summary: "A convention".to_string(),
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: vec![],
                source_task_id: None,
            })
            .unwrap();
        svc.approve_learning(id).unwrap();
        svc.confirm_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.confirmed_count, 1);
    }

    #[test]
    fn get_learning_not_found_returns_error() {
        let svc = service();
        let err = svc.get_learning(99999).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }
}
