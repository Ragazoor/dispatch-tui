use std::sync::Arc;

use crate::db::{self, EpicPatch, TaskPatch};
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
    pub group_by_repo: Option<bool>,
    /// Triple-state: None = no change, Some(Some(id)) = reparent, Some(None) = make root.
    pub parent_epic_id: Option<Option<EpicId>>,
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
        if self.group_by_repo.is_some() {
            names.push("group_by_repo");
        }
        if self.parent_epic_id.is_some() {
            names.push("parent_epic_id");
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
    pub db: Arc<dyn db::TaskAndEpicStore>,
}

impl EpicService {
    pub fn new(db: Arc<dyn db::TaskAndEpicStore>) -> Self {
        Self { db }
    }

    pub async fn create_epic(&self, params: CreateEpicParams) -> Result<Epic, ServiceError> {
        let repo_path = crate::models::expand_tilde(&params.repo_path);

        if let Some(parent_id) = params.parent_epic_id {
            let parent = match self.db.get_epic(parent_id).await {
                Ok(Some(p)) => p,
                Ok(None) => {
                    return Err(ServiceError::NotFound(format!(
                        "Parent epic {} not found",
                        parent_id.0
                    )))
                }
                Err(e) => {
                    return Err(ServiceError::Internal(format!(
                        "Database error looking up parent epic: {e}"
                    )))
                }
            };
            if params.project_id != parent.project_id {
                return Err(ServiceError::Validation(format!(
                    "sub-epic project_id ({}) must match parent epic project_id ({})",
                    params.project_id.0, parent.project_id.0
                )));
            }
        }

        let epic = self
            .db
            .create_epic(
                &params.title,
                &params.description,
                &repo_path,
                params.parent_epic_id,
                params.project_id,
            )
            .await
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
            let _ = self.db.patch_epic(epic.id, &patch).await;
        }

        Ok(epic)
    }

    pub async fn get_epic(&self, epic_id: EpicId) -> Result<Epic, ServiceError> {
        match self.db.get_epic(epic_id).await {
            Ok(Some(epic)) => Ok(epic),
            Ok(None) => Err(ServiceError::NotFound(format!(
                "Epic {} not found",
                epic_id.0
            ))),
            Err(e) => Err(ServiceError::Internal(format!("Database error: {e}"))),
        }
    }

    pub async fn get_epic_with_subtasks(
        &self,
        epic_id: EpicId,
    ) -> Result<(Epic, Vec<Task>), ServiceError> {
        let epic = self.get_epic(epic_id).await?;
        let subtasks = self
            .db
            .list_tasks_for_epic(epic.id)
            .await
            .unwrap_or_default();
        Ok((epic, subtasks))
    }

    pub async fn list_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        self.db
            .list_epics()
            .await
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub async fn list_root_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        self.db
            .list_root_epics()
            .await
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>, ServiceError> {
        self.db
            .list_sub_epics(parent_id)
            .await
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }

    pub async fn list_epics_with_progress(
        &self,
    ) -> Result<Vec<(Epic, usize, usize)>, ServiceError> {
        let epics = self.list_epics().await?;
        let all_subtasks =
            self.db.list_all_tasks_with_epic_id().await.map_err(|e| {
                ServiceError::Internal(format!("Failed to list tasks with epic: {e}"))
            })?;

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

    pub async fn update_epic(&self, params: UpdateEpicParams) -> Result<EpicId, ServiceError> {
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
        if let Some(gbr) = params.group_by_repo {
            patch = patch.group_by_repo(gbr);
        }

        let mut reparent_pid: Option<ProjectId> = None;
        match params.parent_epic_id {
            Some(Some(new_parent_id)) => {
                let parent = self.get_epic(new_parent_id).await?;
                self.check_no_cycle(params.epic_id, &parent).await?;
                patch = patch.parent_epic_id(Some(new_parent_id));
                if params.project_id.is_none() {
                    match self.db.get_epic(params.epic_id).await {
                        Ok(Some(current)) if current.project_id != parent.project_id => {
                            reparent_pid = Some(parent.project_id);
                            patch = patch.project_id(parent.project_id);
                        }
                        Err(e) => tracing::warn!(
                            "update_epic: failed to check project cascade for epic {}: {e}",
                            params.epic_id.0
                        ),
                        _ => {}
                    }
                }
            }
            Some(None) => {
                patch = patch.parent_epic_id(None);
            }
            None => {}
        }

        let epic_id = params.epic_id;
        self.db
            .patch_epic(epic_id, &patch)
            .await
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))?;

        if let Some(pid) = params.project_id.or(reparent_pid) {
            self.cascade_project_id(epic_id, pid).await;
        }

        Ok(epic_id)
    }

    /// Walk the ancestor chain of `proposed_parent` and return a Validation error
    /// if `epic_id` appears in it (which would create a cycle).
    /// Takes a pre-fetched `&Epic` to avoid an extra DB round-trip.
    async fn check_no_cycle(
        &self,
        epic_id: EpicId,
        proposed_parent: &Epic,
    ) -> Result<(), ServiceError> {
        if proposed_parent.id == epic_id {
            return Err(ServiceError::Validation(
                "Setting this parent would create a cycle in the epic hierarchy".into(),
            ));
        }
        let mut current_opt = proposed_parent.parent_epic_id;
        loop {
            let current = match current_opt {
                None => return Ok(()),
                Some(c) => c,
            };
            if current == epic_id {
                return Err(ServiceError::Validation(
                    "Setting this parent would create a cycle in the epic hierarchy".into(),
                ));
            }
            match self.db.get_epic(current).await {
                Ok(Some(e)) => current_opt = e.parent_epic_id,
                Ok(None) => return Ok(()),
                Err(e) => return Err(ServiceError::Internal(format!("Database error: {e}"))),
            }
        }
    }

    /// Recursively update project_id for all direct sub-epics and direct tasks
    /// of the given epic. Called when an epic's project_id changes.
    async fn cascade_project_id(&self, epic_id: EpicId, project_id: ProjectId) {
        let sub_epics = match self.db.list_sub_epics(epic_id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "cascade_project_id: list_sub_epics failed for epic {}: {e}",
                    epic_id.0
                );
                return;
            }
        };
        for sub in sub_epics {
            if let Err(e) = self
                .db
                .patch_epic(sub.id, &EpicPatch::new().project_id(project_id))
                .await
            {
                tracing::warn!("cascade_project_id: patch_epic {} failed: {e}", sub.id.0);
            }
            Box::pin(self.cascade_project_id(sub.id, project_id)).await;
        }

        let tasks = match self.db.list_tasks_for_epic(epic_id).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "cascade_project_id: list_tasks_for_epic failed for epic {}: {e}",
                    epic_id.0
                );
                return;
            }
        };
        for task in tasks {
            if let Err(e) = self
                .db
                .patch_task(task.id, &TaskPatch::new().project_id(project_id))
                .await
            {
                tracing::warn!("cascade_project_id: patch_task {} failed: {e}", task.id.0);
            }
        }
    }

    pub async fn delete_epic(&self, epic_id: EpicId) -> Result<(), ServiceError> {
        // Verify epic exists
        self.get_epic(epic_id).await?;

        self.db
            .delete_epic(epic_id)
            .await
            .map_err(|e| ServiceError::Internal(format!("Database error: {e}")))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::db::{CreateTaskRequest, Database, EpicCrud, ProjectCrud, TaskCrud};

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
            group_by_repo: None,
            parent_epic_id: None,
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
            group_by_repo: None,
            parent_epic_id: None,
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
            group_by_repo: None,
            parent_epic_id: None,
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
            UpdateEpicParams {
                group_by_repo: Some(true),
                ..base()
            },
            UpdateEpicParams {
                parent_epic_id: Some(Some(EpicId(2))),
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

    #[tokio::test]
    async fn update_epic_sets_group_by_repo() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let epic = db
            .create_epic("Test", "", "/repo", None, ProjectId(1))
            .await
            .unwrap();
        assert!(!epic.group_by_repo);
        let svc = EpicService::new(db.clone());
        svc.update_epic(UpdateEpicParams {
            epic_id: epic.id,
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
            group_by_repo: Some(true),
            parent_epic_id: None,
        })
        .await
        .unwrap();
        let updated = db.get_epic(epic.id).await.unwrap().unwrap();
        assert!(updated.group_by_repo);
    }

    #[tokio::test]
    async fn create_sub_epic_with_mismatched_project_id_returns_error() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let parent = db
            .create_epic("Parent", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        let proj2 = db.create_project("P2", 2).await.unwrap();

        let result = svc
            .create_epic(CreateEpicParams {
                title: "Sub".into(),
                description: "".into(),
                repo_path: "/r".into(),
                sort_order: None,
                parent_epic_id: Some(parent.id),
                feed_command: None,
                feed_interval_secs: None,
                project_id: proj2.id,
            })
            .await;

        assert!(
            matches!(result, Err(ServiceError::Validation(_))),
            "expected Validation error for mismatched project_id, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn create_sub_epic_with_correct_project_id_succeeds() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let parent = db
            .create_epic("Parent", "", "/r", None, ProjectId(1))
            .await
            .unwrap();

        let sub = svc
            .create_epic(CreateEpicParams {
                title: "Sub".into(),
                description: "".into(),
                repo_path: "/r".into(),
                sort_order: None,
                parent_epic_id: Some(parent.id),
                feed_command: None,
                feed_interval_secs: None,
                project_id: ProjectId(1),
            })
            .await
            .unwrap();

        assert_eq!(sub.project_id, ProjectId(1));
        assert_eq!(sub.parent_epic_id, Some(parent.id));
    }

    #[tokio::test]
    async fn update_epic_project_id_cascades_to_children() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let proj2 = db.create_project("P2", 2).await.unwrap();

        let root = db
            .create_epic("Root", "", "/r", None, ProjectId(1))
            .await
            .unwrap();

        let sub = db
            .create_epic("Sub", "", "/r", Some(root.id), ProjectId(1))
            .await
            .unwrap();
        assert_eq!(sub.project_id, ProjectId(1));

        let task_id = db
            .create_task(CreateTaskRequest {
                title: "T",
                description: "",
                repo_path: "/r",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: Some(root.id),
                sort_order: None,
                tag: None,
                project_id: ProjectId(1),
                wrap_up_mode: None,
            })
            .await
            .unwrap();

        svc.update_epic(UpdateEpicParams {
            epic_id: root.id,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: Some(proj2.id),
            group_by_repo: None,
            parent_epic_id: None,
        })
        .await
        .unwrap();

        let root_after = db.get_epic(root.id).await.unwrap().unwrap();
        assert_eq!(root_after.project_id, proj2.id, "root epic project_id");

        let sub_after = db.get_epic(sub.id).await.unwrap().unwrap();
        assert_eq!(sub_after.project_id, proj2.id, "sub-epic must follow root");

        let task_after = db.get_task(task_id).await.unwrap().unwrap();
        assert_eq!(
            task_after.project_id, proj2.id,
            "task must follow root epic"
        );
    }

    #[tokio::test]
    async fn update_epic_sets_parent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let parent = db
            .create_epic("Parent", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        let child = db
            .create_epic("Child", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        assert!(child.parent_epic_id.is_none());

        svc.update_epic(UpdateEpicParams {
            epic_id: child.id,
            parent_epic_id: Some(Some(parent.id)),
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
            group_by_repo: None,
        })
        .await
        .unwrap();

        let updated = db.get_epic(child.id).await.unwrap().unwrap();
        assert_eq!(updated.parent_epic_id, Some(parent.id));
    }

    #[tokio::test]
    async fn update_epic_clears_parent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let parent = db
            .create_epic("Parent", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        let child = db
            .create_epic("Child", "", "/r", Some(parent.id), ProjectId(1))
            .await
            .unwrap();
        assert_eq!(child.parent_epic_id, Some(parent.id));

        svc.update_epic(UpdateEpicParams {
            epic_id: child.id,
            parent_epic_id: Some(None),
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
            group_by_repo: None,
        })
        .await
        .unwrap();

        let updated = db.get_epic(child.id).await.unwrap().unwrap();
        assert!(updated.parent_epic_id.is_none());
    }

    #[tokio::test]
    async fn update_epic_parent_id_absent_is_noop() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let parent = db
            .create_epic("Parent", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        let child = db
            .create_epic("Child", "", "/r", Some(parent.id), ProjectId(1))
            .await
            .unwrap();

        svc.update_epic(UpdateEpicParams {
            epic_id: child.id,
            parent_epic_id: None, // omitted — no change
            title: Some("New Title".to_string()),
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            repo_path: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            project_id: None,
            group_by_repo: None,
        })
        .await
        .unwrap();

        let updated = db.get_epic(child.id).await.unwrap().unwrap();
        assert_eq!(updated.parent_epic_id, Some(parent.id), "parent unchanged");
    }

    #[tokio::test]
    async fn update_epic_cycle_detection() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let a = db
            .create_epic("A", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        let b = db
            .create_epic("B", "", "/r", Some(a.id), ProjectId(1))
            .await
            .unwrap();

        // Trying to set A's parent to B would create a cycle: A → B → A
        let result = svc
            .update_epic(UpdateEpicParams {
                epic_id: a.id,
                parent_epic_id: Some(Some(b.id)),
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
                group_by_repo: None,
            })
            .await;

        assert!(
            matches!(result, Err(ServiceError::Validation(_))),
            "expected Validation error for cycle, got: {:?}",
            result
        );
        // DB must be unchanged
        let a_after = db.get_epic(a.id).await.unwrap().unwrap();
        assert!(a_after.parent_epic_id.is_none());
    }

    #[tokio::test]
    async fn update_epic_self_parent_rejected() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let epic = db
            .create_epic("Epic", "", "/r", None, ProjectId(1))
            .await
            .unwrap();

        let result = svc
            .update_epic(UpdateEpicParams {
                epic_id: epic.id,
                parent_epic_id: Some(Some(epic.id)),
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
                group_by_repo: None,
            })
            .await;

        assert!(
            matches!(result, Err(ServiceError::Validation(_))),
            "expected Validation error for self-parent, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn update_epic_reparent_cascades_project_id() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());

        let proj2 = db.create_project("P2", 2).await.unwrap();

        let parent = db
            .create_epic("Parent", "", "/r", None, proj2.id)
            .await
            .unwrap();
        let child = db
            .create_epic("Child", "", "/r", None, ProjectId(1))
            .await
            .unwrap();
        // grandchild to verify deep cascade
        let grandchild = db
            .create_epic("GC", "", "/r", Some(child.id), ProjectId(1))
            .await
            .unwrap();

        svc.update_epic(UpdateEpicParams {
            epic_id: child.id,
            parent_epic_id: Some(Some(parent.id)),
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
            group_by_repo: None,
        })
        .await
        .unwrap();

        let child_after = db.get_epic(child.id).await.unwrap().unwrap();
        assert_eq!(child_after.project_id, proj2.id, "child project cascaded");

        let gc_after = db.get_epic(grandchild.id).await.unwrap().unwrap();
        assert_eq!(gc_after.project_id, proj2.id, "grandchild project cascaded");
    }
}
