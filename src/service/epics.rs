use std::sync::Arc;

use crate::db::{self, EpicPatch};
use crate::models::{Epic, EpicId, ProjectId, Task, TaskStatus};

use super::{FieldUpdate, ServiceError};

// ---------------------------------------------------------------------------
// UpdateEpicParams
// ---------------------------------------------------------------------------

pub struct UpdateEpicParams {
    pub epic_id: EpicId,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub plan_path: Option<String>,
    pub sort_order: Option<i64>,
    pub repo_path: Option<String>,
    pub auto_dispatch: Option<bool>,
    pub feed_command: Option<FieldUpdate>,
    pub feed_interval_secs: Option<Option<i64>>,
    pub project_id: Option<ProjectId>,
}

impl UpdateEpicParams {
    pub(in crate::service) fn has_any_field(&self) -> bool {
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

    pub fn get_epic(&self, epic_id: EpicId) -> Result<Epic, ServiceError> {
        match self.db.get_epic(epic_id) {
            Ok(Some(epic)) => Ok(epic),
            Ok(None) => Err(ServiceError::NotFound(format!(
                "Epic {} not found",
                epic_id.0
            ))),
            Err(e) => Err(ServiceError::Internal(format!("Database error: {e}"))),
        }
    }

    pub fn get_epic_with_subtasks(
        &self,
        epic_id: EpicId,
    ) -> Result<(Epic, Vec<Task>), ServiceError> {
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
            patch = patch.feed_interval_secs(fi);
        }
        if let Some(pid) = params.project_id {
            patch = patch.project_id(pid);
        }

        let epic_id = params.epic_id;
        self.db
            .patch_epic(epic_id, &patch)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        Ok(epic_id)
    }

    pub fn delete_epic(&self, epic_id: EpicId) -> Result<(), ServiceError> {
        // Verify epic exists
        self.get_epic(epic_id)?;

        self.db
            .delete_epic(epic_id)
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_epic_params_has_any_field_consistent_with_updated_field_names() {
        // Same consistency guard for UpdateEpicParams.
        let with_field = UpdateEpicParams {
            epic_id: EpicId(1),
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
            epic_id: EpicId(1),
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
    fn update_epic_params_every_field_covered() {
        // Each field set individually must trigger both has_any_field() and
        // updated_field_names(). Add a case here whenever a new field is added
        // to UpdateEpicParams so both methods stay in sync.
        let base = || UpdateEpicParams {
            epic_id: EpicId(1),
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
                feed_interval_secs: Some(Some(300)),
                ..base()
            },
            UpdateEpicParams {
                project_id: Some(ProjectId(1)),
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
}
