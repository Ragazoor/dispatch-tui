use std::sync::Arc;

use crate::db::{self, EpicPatch, TaskPatch};
use crate::models::{
    Epic, EpicId, SubStatus, Task, TaskId, TaskStatus, TaskTag, UsageReport, DEFAULT_BASE_BRANCH,
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
}

impl UpdateTaskParams {
    fn has_any_field(&self) -> bool {
        self.status.is_some()
            || self.plan_path.is_some()
            || self.title.is_some()
            || self.description.is_some()
            || self.repo_path.is_some()
            || self.sort_order.is_some()
            || self.pr_url.is_some()
            || self.tag.is_some()
            || self.sub_status.is_some()
            || self.epic_id.is_some()
            || self.worktree.is_some()
            || self.tmux_window.is_some()
            || self.base_branch.is_some()
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

pub struct TaskService {
    pub db: Arc<dyn db::TaskStore>,
}

impl TaskService {
    pub fn new(db: Arc<dyn db::TaskStore>) -> Self {
        Self { db }
    }

    pub fn update_task(&self, params: UpdateTaskParams) -> Result<TaskId, ServiceError> {
        if !params.has_any_field() {
            return Err(ServiceError::Validation(
                "At least one field must be provided".into(),
            ));
        }

        let expanded_repo_path = params.repo_path.as_deref().map(crate::models::expand_tilde);

        let mut patch = TaskPatch::new();
        if let Some(s) = params.status {
            patch = patch.status(s);
        }
        if let Some(ref p) = params.plan_path {
            patch = patch.plan_path(Some(p.as_str()));
        }
        if let Some(ref t) = params.title {
            patch = patch.title(t);
        }
        if let Some(ref d) = params.description {
            patch = patch.description(d);
        }
        if let Some(ref r) = expanded_repo_path {
            patch = patch.repo_path(r);
        }
        if let Some(so) = params.sort_order {
            patch = patch.sort_order(Some(so));
        }
        if let Some(ref update) = params.pr_url {
            match update {
                FieldUpdate::Set(url) => patch = patch.pr_url(Some(url.as_str())),
                FieldUpdate::Clear => patch = patch.pr_url(None),
            }
        }
        if let Some(tag) = params.tag {
            patch = patch.tag(Some(tag));
        }
        if let Some(ref update) = params.worktree {
            match update {
                FieldUpdate::Set(wt) => patch = patch.worktree(Some(wt.as_str())),
                FieldUpdate::Clear => patch = patch.worktree(None),
            }
        }
        if let Some(ref update) = params.tmux_window {
            match update {
                FieldUpdate::Set(tw) => patch = patch.tmux_window(Some(tw.as_str())),
                FieldUpdate::Clear => patch = patch.tmux_window(None),
            }
        }
        if let Some(ref bb) = params.base_branch {
            patch = patch.base_branch(bb.as_str());
        }

        if let Some(ss) = params.sub_status {
            let effective_status = params.status.or_else(|| {
                self.db
                    .get_task(TaskId(params.task_id))
                    .ok()
                    .flatten()
                    .map(|t| t.status)
            });
            if let Some(eff) = effective_status {
                if !ss.is_valid_for(eff) {
                    return Err(ServiceError::Validation(format!(
                        "sub_status '{}' is not valid for status '{}'",
                        ss.as_str(),
                        eff.as_str()
                    )));
                }
            }
            patch = patch.sub_status(ss);
        }

        let task_id = TaskId(params.task_id);
        self.db
            .patch_task(task_id, &patch)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        // Update epic linkage if requested
        if let Some(new_epic_id) = params.epic_id {
            // Recalculate old epic before reassignment
            if let Ok(Some(task)) = self.db.get_task(task_id) {
                if let Some(old_epic_id) = task.epic_id {
                    if let Err(err) = self.db.recalculate_epic_status(old_epic_id) {
                        tracing::warn!("failed to recalculate epic status for epic {}: {err}", old_epic_id.0);
                    }
                }
            }
            self.db
                .set_task_epic_id(task_id, Some(EpicId(new_epic_id)))
                .map_err(|e| ServiceError::Internal(format!("Failed to link task to epic: {e}")))?;
            if let Err(err) = self.db.recalculate_epic_status(EpicId(new_epic_id)) {
                tracing::warn!("failed to recalculate epic status for epic {new_epic_id}: {err}");
            }
        }

        // Recalculate parent epic status if subtask status changed
        if params.status.is_some() {
            if let Ok(Some(task)) = self.db.get_task(task_id) {
                if let Some(epic_id) = task.epic_id {
                    if let Err(err) = self.db.recalculate_epic_status(epic_id) {
                        tracing::warn!("failed to recalculate epic status for epic {}: {err}", epic_id.0);
                    }
                }
            }
        }

        Ok(task_id)
    }

    /// CLI update command: change task status with optional condition and sub_status.
    /// Returns true if the update was applied (false if only_if condition didn't match).
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
            if let Ok(Some(task)) = self.db.get_task(task_id) {
                if let Some(epic_id) = task.epic_id {
                    if let Err(err) = self.db.recalculate_epic_status(epic_id) {
                        tracing::warn!("failed to recalculate epic status for epic {}: {err}", epic_id.0);
                    }
                }
            }
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
            )
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        if let Some(eid) = params.epic_id {
            self.db
                .set_task_epic_id(task_id, Some(EpicId(eid)))
                .map_err(|e| ServiceError::Internal(format!("Failed to link task to epic: {e}")))?;
        }
        if let Some(so) = params.sort_order {
            self.db
                .patch_task(task_id, &TaskPatch::new().sort_order(Some(so)))
                .map_err(|e| ServiceError::Internal(format!("Failed to set sort_order: {e}")))?;
        }
        if let Some(tag) = params.tag {
            self.db
                .patch_task(task_id, &TaskPatch::new().tag(Some(tag)))
                .map_err(|e| ServiceError::Internal(format!("Failed to set tag: {e}")))?;
        }

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
}

impl UpdateEpicParams {
    fn has_any_field(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.status.is_some()
            || self.plan_path.is_some()
            || self.sort_order.is_some()
            || self.repo_path.is_some()
            || self.auto_dispatch.is_some()
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
}

// ---------------------------------------------------------------------------
// EpicService
// ---------------------------------------------------------------------------

pub struct EpicService {
    pub db: Arc<dyn db::TaskStore>,
}

impl EpicService {
    pub fn new(db: Arc<dyn db::TaskStore>) -> Self {
        Self { db }
    }

    pub fn create_epic(&self, params: CreateEpicParams) -> Result<Epic, ServiceError> {
        let epic = self
            .db
            .create_epic(&params.title, &params.description, &params.repo_path)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        if let Some(so) = params.sort_order {
            let _ = self
                .db
                .patch_epic(epic.id, &EpicPatch::new().sort_order(Some(so)));
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn test_db() -> Arc<dyn db::TaskStore> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn task_svc(db: &Arc<dyn db::TaskStore>) -> TaskService {
        TaskService::new(Arc::clone(db))
    }

    fn epic_svc(db: &Arc<dyn db::TaskStore>) -> EpicService {
        EpicService::new(Arc::clone(db))
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
            })
            .unwrap();

        task_svc
            .update_task(UpdateTaskParams::for_task(id.0).epic_id(epic.id.0))
            .unwrap();

        let task = task_svc.get_task(id.0).unwrap();
        assert_eq!(task.epic_id, Some(epic.id));
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
            })
            .unwrap();
        let e2 = epic_svc
            .create_epic(CreateEpicParams {
                title: "E2".into(),
                description: "".into(),
                repo_path: "/repo".into(),
                sort_order: None,
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
            })
            .unwrap();

        assert_eq!(task.epic_id, Some(epic.id));
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
}
