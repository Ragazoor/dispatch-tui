use std::sync::Arc;

use crate::db::{self, LearningFilter, LearningPatch};
use crate::models::{
    Learning, LearningId, LearningKind, LearningScope, LearningStatus, LearningVerdict,
    RetrievalSource, TaskId,
};

use super::{FieldUpdate, ServiceError};

// ---------------------------------------------------------------------------
// CreateLearningParams / UpdateLearningParams
// ---------------------------------------------------------------------------

pub struct CreateLearningParams {
    pub kind: LearningKind,
    pub summary: String,
    pub detail: Option<String>,
    pub scope: LearningScope,
    pub scope_ref: Option<String>,
    pub tags: Vec<String>,
    pub source_task_id: Option<TaskId>,
}

pub struct UpdateLearningParams {
    pub id: LearningId,
    pub summary: Option<String>,
    /// `None` = don't change; `Some(FieldUpdate::Clear)` = clear; `Some(FieldUpdate::Set(v))` = set.
    pub detail: Option<FieldUpdate>,
    pub kind: Option<LearningKind>,
    pub tags: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// LearningService
// ---------------------------------------------------------------------------

pub struct LearningService {
    pub db: Arc<dyn db::TaskStore>,
}

impl LearningService {
    pub fn new(db: Arc<dyn db::TaskStore>) -> Self {
        Self { db }
    }

    pub fn create_learning(
        &self,
        params: CreateLearningParams,
    ) -> Result<LearningId, ServiceError> {
        if params.summary.trim().is_empty() {
            return Err(ServiceError::Validation("summary must not be empty".into()));
        }
        match params.scope {
            LearningScope::User => {
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

    pub fn get_learning(&self, id: LearningId) -> Result<Learning, ServiceError> {
        self.db
            .get_learning(id)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))?
            .ok_or_else(|| ServiceError::NotFound(format!("learning {id} not found")))
    }

    pub fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>, ServiceError> {
        self.db
            .list_learnings(filter)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    /// Approve a learning. Allowed from any non-terminal status, so this also
    /// transitions a `needs_review` learning back to `approved`.
    pub fn approve_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if learning.status.is_terminal() {
            return Err(ServiceError::Validation(format!(
                "cannot approve a {} learning",
                learning.status
            )));
        }
        if learning.status == LearningStatus::Approved {
            return Ok(());
        }
        self.db
            .patch_learning(id, &LearningPatch::new().status(LearningStatus::Approved))
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn reject_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if learning.status.is_terminal() {
            return Err(ServiceError::Validation(format!(
                "cannot reject a {} learning",
                learning.status
            )));
        }
        self.db
            .patch_learning(id, &LearningPatch::new().status(LearningStatus::Rejected))
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn archive_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        let learning = self.get_learning(id)?;
        if !matches!(
            learning.status,
            LearningStatus::Approved | LearningStatus::NeedsReview
        ) {
            return Err(ServiceError::Validation(format!(
                "can only archive an approved or needs_review learning (current status: {})",
                learning.status
            )));
        }
        self.db
            .patch_learning(id, &LearningPatch::new().status(LearningStatus::Archived))
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn update_learning(&self, params: UpdateLearningParams) -> Result<(), ServiceError> {
        let learning = self.get_learning(params.id)?;
        if learning.status.is_terminal() {
            return Err(ServiceError::Validation(
                "cannot edit a rejected or archived learning".to_string(),
            ));
        }
        if let Some(ref s) = params.summary {
            if s.trim().is_empty() {
                return Err(ServiceError::Validation("summary must not be empty".into()));
            }
        }
        let mut patch = LearningPatch::new();
        if let Some(ref s) = params.summary {
            patch = patch.summary(s.as_str());
        }
        if let Some(ref d) = params.detail {
            patch = match d {
                FieldUpdate::Set(v) => patch.detail(Some(v.as_str())),
                FieldUpdate::Clear => patch.detail(None),
            };
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

    pub fn upvote_learning(&self, id: LearningId) -> Result<(), ServiceError> {
        self.db
            .upvote_learning(id)
            .map_err(|e| ServiceError::Validation(format!("cannot upvote: {e}")))
    }

    pub fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<(), ServiceError> {
        self.db
            .record_retrieval(task_id, learning_id, source)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }

    pub fn apply_verdicts(
        &self,
        task_id: TaskId,
        verdicts: Vec<(LearningId, LearningVerdict)>,
    ) -> Result<(), ServiceError> {
        if verdicts.is_empty() {
            return Ok(());
        }
        let retrieved: std::collections::HashSet<LearningId> = self
            .db
            .list_retrievals_for_task(task_id)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))?
            .into_iter()
            .map(|r| r.learning_id)
            .collect();
        for (lid, _) in &verdicts {
            if !retrieved.contains(lid) {
                return Err(ServiceError::Validation(format!(
                    "learning {} was not retrieved during task {}",
                    lid, task_id
                )));
            }
        }
        self.db
            .apply_verdicts_tx(task_id, &verdicts)
            .map_err(|e| ServiceError::Internal(format!("database error: {e}")))
    }
}

// ---------------------------------------------------------------------------
// LearningService tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod learning_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use super::{CreateLearningParams, LearningService, UpdateLearningParams};
    use crate::db::{CreateTaskRequest, Database, TaskStore};
    use crate::models::{
        LearningId, LearningKind, LearningScope, LearningStatus, LearningVerdict, ProjectId,
        RetrievalSource, TaskId, TaskStatus,
    };
    use crate::service::ServiceError;

    fn service() -> LearningService {
        let db = Arc::new(Database::open_in_memory().unwrap());
        LearningService::new(db)
    }

    fn service_with_db() -> (LearningService, Arc<dyn TaskStore>) {
        let db: Arc<dyn TaskStore> = Arc::new(Database::open_in_memory().unwrap());
        (LearningService::new(db.clone()), db)
    }

    fn seed_task(db: &Arc<dyn TaskStore>) -> TaskId {
        db.create_task(CreateTaskRequest {
            title: "test task",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            project_id: ProjectId(1),
        })
        .unwrap()
    }

    fn seed_approved_learning(svc: &LearningService) -> LearningId {
        svc.create_learning(CreateLearningParams {
            kind: LearningKind::Convention,
            summary: "A convention".to_string(),
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: vec![],
            source_task_id: None,
        })
        .unwrap()
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
        assert_eq!(learning.status, LearningStatus::Approved);
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
        svc.archive_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Archived);
    }

    #[test]
    fn approve_learning_from_needs_review_sets_status_to_approved() {
        use crate::db::LearningPatch;
        let (svc, db) = service_with_db();
        let id = seed_approved_learning(&svc);
        // Move to needs_review (simulating a `wrong` verdict).
        db.patch_learning(
            id,
            &LearningPatch::new().status(LearningStatus::NeedsReview),
        )
        .unwrap();
        svc.approve_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Approved);
    }

    #[test]
    fn approve_learning_from_terminal_status_fails() {
        let svc = service();
        let id = seed_approved_learning(&svc);
        svc.reject_learning(id).unwrap();
        let err = svc.approve_learning(id).unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn archive_learning_from_needs_review_succeeds() {
        use crate::db::LearningPatch;
        let (svc, db) = service_with_db();
        let id = seed_approved_learning(&svc);
        db.patch_learning(
            id,
            &LearningPatch::new().status(LearningStatus::NeedsReview),
        )
        .unwrap();
        svc.archive_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.status, LearningStatus::Archived);
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
    fn upvote_learning_on_approved_succeeds() {
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
        svc.upvote_learning(id).unwrap();
        let learning = svc.get_learning(id).unwrap();
        assert_eq!(learning.confirmed_count, 1);
    }

    #[test]
    fn get_learning_not_found_returns_error() {
        let svc = service();
        let err = svc.get_learning(LearningId(99999)).unwrap_err();
        assert!(matches!(err, ServiceError::NotFound(_)));
    }

    #[test]
    fn apply_verdicts_validation_rejects_unknown_retrieval() {
        let (svc, db) = service_with_db();
        let task_id = seed_task(&db);
        let learning_id = seed_approved_learning(&svc);
        // No retrieval recorded — apply should fail with Validation.
        let err = svc
            .apply_verdicts(task_id, vec![(learning_id, LearningVerdict::Helped)])
            .unwrap_err();
        assert!(matches!(err, ServiceError::Validation(_)));
    }

    #[test]
    fn apply_verdicts_succeeds_when_retrieval_exists() {
        let (svc, db) = service_with_db();
        let task_id = seed_task(&db);
        let learning_id = seed_approved_learning(&svc);
        svc.record_retrieval(task_id, learning_id, RetrievalSource::PromptInjection)
            .unwrap();
        svc.apply_verdicts(task_id, vec![(learning_id, LearningVerdict::Helped)])
            .unwrap();
        let l = svc.get_learning(learning_id).unwrap();
        assert_eq!(l.confirmed_count, 1);
    }

    #[test]
    fn apply_verdicts_empty_is_ok() {
        let (svc, db) = service_with_db();
        let task_id = seed_task(&db);
        svc.apply_verdicts(task_id, vec![]).unwrap();
    }
}
