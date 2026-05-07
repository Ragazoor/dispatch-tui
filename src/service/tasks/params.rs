//! Parameter structs and builders for `TaskService` operations.
//!
//! Transport-agnostic input shapes: callers (MCP handlers, CLI subcommands,
//! TUI commands) construct one of these and pass it to the corresponding
//! `TaskService` method.

use crate::models::{EpicId, ProjectId, SubStatus, TaskId, TaskStatus, TaskTag};
use crate::service::FieldUpdate;

// ---------------------------------------------------------------------------
// UpdateTaskParams — transport-agnostic input for update_task
// ---------------------------------------------------------------------------

pub struct UpdateTaskParams {
    pub task_id: TaskId,
    pub status: Option<TaskStatus>,
    pub plan_path: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub repo_path: Option<String>,
    pub sort_order: Option<i64>,
    pub pr_url: Option<FieldUpdate>,
    pub tag: Option<TaskTag>,
    pub sub_status: Option<SubStatus>,
    pub epic_id: Option<EpicId>,
    pub worktree: Option<FieldUpdate>,
    pub tmux_window: Option<FieldUpdate>,
    pub base_branch: Option<String>,
    pub project_id: Option<ProjectId>,
}

impl UpdateTaskParams {
    pub(super) fn has_any_field(&self) -> bool {
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
    pub fn for_task(task_id: TaskId) -> Self {
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

    pub fn epic_id(mut self, epic_id: EpicId) -> Self {
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

#[derive(Default)]
pub struct ListTasksFilter {
    pub statuses: Option<Vec<TaskStatus>>,
    pub epic_id: Option<EpicId>,
    pub project_id: Option<ProjectId>,
    pub repo_paths: Option<Vec<String>>,
    pub exclude_task_id: Option<TaskId>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EpicId, ProjectId, SubStatus, TaskId, TaskStatus, TaskTag};
    use crate::service::FieldUpdate;

    #[test]
    fn update_task_params_field_names_returns_str_slices() {
        // Verify return type is Vec<&str> (not Vec<String>) — consistent with UpdateEpicParams.
        let params = UpdateTaskParams::for_task(TaskId(1)).title("x".to_string());
        let names: Vec<&str> = params.updated_field_names();
        assert!(names.contains(&"title"));
    }

    #[test]
    fn update_task_params_has_any_field_consistent_with_updated_field_names() {
        // When a field is set, both has_any_field() and updated_field_names() must agree.
        // If a new field is added to UpdateTaskParams without updating both methods,
        // this test will catch the divergence.
        let with_field = UpdateTaskParams::for_task(TaskId(1)).title("x".to_string());
        assert!(
            with_field.has_any_field(),
            "has_any_field should be true when title is set"
        );
        assert!(
            !with_field.updated_field_names().is_empty(),
            "updated_field_names should be non-empty when title is set"
        );

        let empty = UpdateTaskParams::for_task(TaskId(1));
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
            UpdateTaskParams::for_task(TaskId(1)).status(TaskStatus::Backlog),
            UpdateTaskParams::for_task(TaskId(1)).plan_path(Some("p".to_string())),
            UpdateTaskParams::for_task(TaskId(1)).title("t".to_string()),
            UpdateTaskParams::for_task(TaskId(1)).description("d".to_string()),
            UpdateTaskParams::for_task(TaskId(1)).repo_path("r".to_string()),
            UpdateTaskParams::for_task(TaskId(1)).sort_order(0),
            UpdateTaskParams::for_task(TaskId(1)).pr_url(FieldUpdate::Set("u".to_string())),
            UpdateTaskParams::for_task(TaskId(1)).tag(Some(TaskTag::Bug)),
            UpdateTaskParams::for_task(TaskId(1)).sub_status(SubStatus::Active),
            UpdateTaskParams::for_task(TaskId(1)).epic_id(EpicId(1)),
            UpdateTaskParams::for_task(TaskId(1)).worktree(FieldUpdate::Set("w".to_string())),
            UpdateTaskParams::for_task(TaskId(1)).tmux_window(FieldUpdate::Set("tw".to_string())),
            UpdateTaskParams::for_task(TaskId(1)).base_branch(Some("main".to_string())),
            UpdateTaskParams::for_task(TaskId(1)).project_id(ProjectId(1)),
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
}
