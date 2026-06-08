use std::sync::Arc;

use crate::db::{self, EpicPatch};
use crate::models::{Epic, EpicId, Task, TaskStatus};

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
    pub auto_dispatch: Option<bool>,
    pub feed_command: Option<FieldUpdate>,
    pub feed_interval_secs: Option<Option<i64>>,
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
        if self.auto_dispatch.is_some() {
            names.push("auto_dispatch");
        }
        if self.feed_command.is_some() {
            names.push("feed_command");
        }
        if self.feed_interval_secs.is_some() {
            names.push("feed_interval_secs");
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
    pub sort_order: Option<i64>,
    pub parent_epic_id: Option<EpicId>,
    pub feed_command: Option<String>,
    pub feed_interval_secs: Option<i64>,
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
        if let Some(parent_id) = params.parent_epic_id {
            self.db
                .get_epic(parent_id)
                .await?
                .ok_or_else(|| {
                    ServiceError::NotFound(format!("Parent epic {} not found", parent_id.0))
                })?;
        }

        let epic = self
            .db
            .create_epic(&params.title, &params.description, params.parent_epic_id)
            .await?;

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
        self.db
            .get_epic(epic_id)
            .await?
            .ok_or_else(|| ServiceError::NotFound(format!("Epic {} not found", epic_id.0)))
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
        Ok(self.db.list_epics().await?)
    }

    pub async fn list_root_epics(&self) -> Result<Vec<Epic>, ServiceError> {
        Ok(self.db.list_root_epics().await?)
    }

    pub async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>, ServiceError> {
        Ok(self.db.list_sub_epics(parent_id).await?)
    }

    pub async fn list_epics_with_progress(
        &self,
    ) -> Result<Vec<(Epic, usize, usize)>, ServiceError> {
        let epics = self.list_epics().await?;
        let all_subtasks = self.db.list_all_tasks_with_epic_id().await?;

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
        if let Some(ad) = params.auto_dispatch {
            patch = patch.auto_dispatch(ad);
        }
        if let Some(ref fc) = params.feed_command {
            patch = patch.feed_command(fc.as_option());
        }
        if let Some(fi) = params.feed_interval_secs {
            patch = patch.feed_interval_secs(fi);
        }
        if let Some(gbr) = params.group_by_repo {
            patch = patch.group_by_repo(gbr);
        }

        match params.parent_epic_id {
            Some(Some(new_parent_id)) => {
                let parent = self.get_epic(new_parent_id).await?;
                self.check_no_cycle(params.epic_id, &parent).await?;
                patch = patch.parent_epic_id(Some(new_parent_id));
            }
            Some(None) => {
                patch = patch.parent_epic_id(None);
            }
            None => {}
        }

        let epic_id = params.epic_id;
        self.db.patch_epic(epic_id, &patch).await?;

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
            match self.db.get_epic(current).await? {
                Some(e) => current_opt = e.parent_epic_id,
                None => return Ok(()),
            }
        }
    }

    /// Recursively update project_id for all direct sub-epics and direct tasks
    pub async fn delete_epic(&self, epic_id: EpicId) -> Result<(), ServiceError> {
        // Verify epic exists
        self.get_epic(epic_id).await?;

        self.db.delete_epic(epic_id).await.map_err(ServiceError::from)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::db::{Database, EpicCrud};

    fn base_params(epic_id: EpicId) -> UpdateEpicParams {
        UpdateEpicParams {
            epic_id,
            title: None,
            description: None,
            status: None,
            plan_path: None,
            sort_order: None,
            auto_dispatch: None,
            feed_command: None,
            feed_interval_secs: None,
            group_by_repo: None,
            parent_epic_id: None,
        }
    }

    #[test]
    fn update_epic_params_has_any_field_consistent_with_updated_field_names() {
        let with_field = UpdateEpicParams {
            title: Some("x".to_string()),
            ..base_params(EpicId(1))
        };
        assert!(
            with_field.has_any_field(),
            "has_any_field should be true when title is set"
        );
        assert!(
            !with_field.updated_field_names().is_empty(),
            "updated_field_names should be non-empty when title is set"
        );

        let empty = base_params(EpicId(1));
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
        let cases: Vec<UpdateEpicParams> = vec![
            UpdateEpicParams {
                title: Some("t".to_string()),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                description: Some("d".to_string()),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                status: Some(TaskStatus::Backlog),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                plan_path: Some("p".to_string()),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                sort_order: Some(0),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                auto_dispatch: Some(true),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                feed_command: Some(FieldUpdate::Set("cmd".to_string())),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                feed_interval_secs: Some(Some(300)),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                group_by_repo: Some(true),
                ..base_params(EpicId(1))
            },
            UpdateEpicParams {
                parent_epic_id: Some(Some(EpicId(2))),
                ..base_params(EpicId(1))
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
        let epic = db.create_epic("Test", "", None).await.unwrap();
        assert!(!epic.group_by_repo);
        let svc = EpicService::new(db.clone());
        svc.update_epic(UpdateEpicParams {
            group_by_repo: Some(true),
            ..base_params(epic.id)
        })
        .await
        .unwrap();
        let updated = db.get_epic(epic.id).await.unwrap().unwrap();
        assert!(updated.group_by_repo);
    }

    #[tokio::test]
    async fn create_sub_epic_succeeds() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());
        let parent = db.create_epic("Parent", "", None).await.unwrap();
        let sub = svc
            .create_epic(CreateEpicParams {
                title: "Sub".into(),
                description: "".into(),
                sort_order: None,
                parent_epic_id: Some(parent.id),
                feed_command: None,
                feed_interval_secs: None,
            })
            .await
            .unwrap();
        assert_eq!(sub.parent_epic_id, Some(parent.id));
    }

    #[tokio::test]
    async fn create_sub_epic_missing_parent_returns_not_found() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());
        let result = svc
            .create_epic(CreateEpicParams {
                title: "Sub".into(),
                description: "".into(),
                sort_order: None,
                parent_epic_id: Some(EpicId(9999)),
                feed_command: None,
                feed_interval_secs: None,
            })
            .await;
        assert!(
            matches!(result, Err(ServiceError::NotFound(_))),
            "expected NotFound for missing parent, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn update_epic_sets_parent() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let svc = EpicService::new(db.clone());
        let parent = db.create_epic("Parent", "", None).await.unwrap();
        let child = db.create_epic("Child", "", None).await.unwrap();
        assert!(child.parent_epic_id.is_none());
        svc.update_epic(UpdateEpicParams {
            parent_epic_id: Some(Some(parent.id)),
            ..base_params(child.id)
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
        let parent = db.create_epic("Parent", "", None).await.unwrap();
        let child = db.create_epic("Child", "", Some(parent.id)).await.unwrap();
        assert_eq!(child.parent_epic_id, Some(parent.id));
        svc.update_epic(UpdateEpicParams {
            parent_epic_id: Some(None),
            ..base_params(child.id)
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
        let parent = db.create_epic("Parent", "", None).await.unwrap();
        let child = db.create_epic("Child", "", Some(parent.id)).await.unwrap();
        svc.update_epic(UpdateEpicParams {
            title: Some("New Title".to_string()),
            ..base_params(child.id)
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
        let a = db.create_epic("A", "", None).await.unwrap();
        let b = db.create_epic("B", "", Some(a.id)).await.unwrap();
        // Trying to set A's parent to B would create a cycle: A → B → A
        let result = svc
            .update_epic(UpdateEpicParams {
                parent_epic_id: Some(Some(b.id)),
                ..base_params(a.id)
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
        let epic = db.create_epic("Epic", "", None).await.unwrap();
        let result = svc
            .update_epic(UpdateEpicParams {
                parent_epic_id: Some(Some(epic.id)),
                ..base_params(epic.id)
            })
            .await;
        assert!(
            matches!(result, Err(ServiceError::Validation(_))),
            "expected Validation error for self-parent, got: {:?}",
            result
        );
    }
}
