use super::*;
use crate::db;
use crate::models::{LearningId, LearningStatus};
#[cfg(test)]
use crate::service::embeddings::EmbeddingService;

impl TuiRuntime {
    pub(super) async fn exec_load_learnings(&self, app: &mut App) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        let approved = db
            .list_learnings(db::LearningFilter {
                status: Some(LearningStatus::Approved),
                ..Default::default()
            })
            .await;
        let needs_review = db
            .list_learnings(db::LearningFilter {
                status: Some(LearningStatus::NeedsReview),
                ..Default::default()
            })
            .await;
        match (approved, needs_review) {
            (Ok(mut a), Ok(mut nr)) => {
                let mut all = Vec::with_capacity(a.len() + nr.len());
                all.append(&mut nr);
                all.append(&mut a);
                app.update(Message::Learning(LearningMessage::Show(all)));
            }
            (Err(e), _) | (_, Err(e)) => {
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(format!(
                        "Failed to load learnings: {e}"
                    )),
                ));
            }
        }
    }

    /// Refresh the count of `NeedsReview` learnings and dispatch
    /// [`LearningMessage::NeedsReviewCountUpdated`] so the `[KB:N]` status-bar badge
    /// stays current. Best-effort — DB errors are logged but not surfaced to
    /// the status bar (the badge simply won't update on this tick).
    pub(super) async fn exec_refresh_needs_review_count(&self, app: &mut App) {
        match self.database.count_learnings_needs_review().await {
            Ok(n) => {
                app.update(Message::Learning(LearningMessage::NeedsReviewCountUpdated(
                    n,
                )));
            }
            Err(e) => {
                tracing::warn!(error = ?e, "failed to count needs_review learnings");
            }
        }
    }

    pub(super) async fn exec_archive_learning(&self, app: &mut App, id: LearningId) {
        let result = self.learning_svc.archive_learning(id).await;
        Self::handle_action_result(app, id, "archive", result);
    }

    pub(super) async fn exec_reject_learning(&self, app: &mut App, id: LearningId) {
        let result = self.learning_svc.reject_learning(id).await;
        Self::handle_action_result(app, id, "reject", result);
    }

    /// Approve a learning. Unlike archive/reject, the entry stays visible in
    /// the overlay (it transitions to `Approved`). After the patch we re-read
    /// the learning and dispatch `LearningEdited` so the in-memory row picks
    /// up the new status.
    pub(super) async fn exec_approve_learning(&self, app: &mut App, id: LearningId) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        match self.learning_svc.approve_learning(id).await {
            Ok(()) => match db.get_learning(id).await {
                Ok(Some(updated)) => {
                    app.update(Message::Learning(LearningMessage::Edited(updated)));
                    app.update(Message::System(
                        crate::tui::messages::SystemMessage::StatusInfo(format!(
                            "Learning {id} approved"
                        )),
                    ));
                }
                Ok(None) => {
                    app.update(Message::System(
                        crate::tui::messages::SystemMessage::StatusInfo(format!(
                            "Learning {id} not found"
                        )),
                    ));
                }
                Err(e) => {
                    app.update(Message::System(
                        crate::tui::messages::SystemMessage::StatusInfo(format!(
                            "Could not refresh learning: {e}"
                        )),
                    ));
                }
            },
            Err(e) => {
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(format!(
                        "Could not approve learning: {e}"
                    )),
                ));
            }
        }
    }

    fn handle_action_result(
        app: &mut App,
        id: LearningId,
        verb: &str,
        result: Result<(), crate::service::ServiceError>,
    ) {
        match result {
            Ok(()) => {
                app.update(Message::Learning(LearningMessage::Actioned(id)));
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(format!(
                        "Learning {id} {verb}ed"
                    )),
                ));
            }
            Err(e) => {
                app.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(format!(
                        "Could not {verb} learning: {e}"
                    )),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::db::{
        CreateLearningRow, CreateTaskRequest, Database, LearningRetrievalStore, LearningStore,
        TaskCrud,
    };
    use crate::models::{
        Learning, LearningId, LearningKind, LearningScope, LearningStatus, LearningVerdict,
        TaskStatus,
    };
    use crate::tui::ViewMode;
    use chrono::Utc;
    use std::sync::Arc;

    fn make_runtime(db: Arc<Database>) -> TuiRuntime {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (feed_tx, _) = mpsc::unbounded_channel();
        let db_arc: Arc<dyn crate::db::TaskStore> = db.clone();
        let runner: Arc<dyn crate::process::ProcessRunner> =
            Arc::new(crate::process::MockProcessRunner::new(vec![]));
        let emb_svc = EmbeddingService::new_noop();
        TuiRuntime {
            task_svc: Arc::new(crate::service::TaskService::new(db_arc.clone())),
            epic_svc: Arc::new(crate::service::EpicService::new(db_arc.clone())),
            todo_svc: Arc::new(crate::service::TodoService::new(db.clone())),
            learning_svc: Arc::new(crate::service::LearningService::new(
                db_arc.clone(),
                emb_svc.clone(),
            )),
            feed_runner: Some(crate::feed::FeedRunner::new(
                db_arc.clone(),
                feed_tx,
                runner.clone(),
            )),
            feed_invalidate_tx: None,
            database: db_arc,
            msg_tx: tx,
            runner,
            editor_session: Arc::new(std::sync::Mutex::new(None)),
            emb_svc,
        }
    }

    fn make_learning(id: LearningId) -> Learning {
        Learning {
            id,
            kind: LearningKind::Convention,
            summary: format!("Learning {id}"),
            detail: None,
            scope: LearningScope::Repo,
            scope_ref: Some("/repo".to_string()),
            tags: vec![],
            status: LearningStatus::Approved,
            source_task_id: None,
            upvote_count: 0,
            last_upvoted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    async fn insert_learning(db: &Arc<Database>) -> LearningId {
        db.create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "test learning",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn exec_archive_learning_updates_db_and_sends_actioned_message() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let id = insert_learning(&db).await;
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![]);
        // Put the app in Learnings view with the learning
        let learning = make_learning(id);
        app.update(Message::Learning(LearningMessage::Show(vec![learning])));

        rt.exec_archive_learning(&mut app, id).await;

        // Learning should be removed from the overlay list
        assert!(matches!(
            app.view_mode(),
            ViewMode::Learnings { learnings, .. } if learnings.is_empty()
        ));
        // DB should show archived
        let updated = db.get_learning(id).await.unwrap().unwrap();
        assert_eq!(updated.status, LearningStatus::Archived);
    }

    #[tokio::test]
    async fn exec_reject_learning_updates_db_and_sends_actioned_message() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let id = insert_learning(&db).await;
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![]);
        let learning = make_learning(id);
        app.update(Message::Learning(LearningMessage::Show(vec![learning])));

        rt.exec_reject_learning(&mut app, id).await;

        assert!(matches!(
            app.view_mode(),
            ViewMode::Learnings { learnings, .. } if learnings.is_empty()
        ));
        let updated = db.get_learning(id).await.unwrap().unwrap();
        assert_eq!(updated.status, LearningStatus::Rejected);
    }

    #[tokio::test]
    async fn exec_load_passes_learnings_to_show_learnings_sorted_by_upvote_count() {
        let db = Arc::new(Database::open_in_memory().await.unwrap());

        // Insert two learnings; bump the second one's upvote_count via patch.
        let id1 = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "learning 1",
                detail: None,
                scope: LearningScope::Repo,
                scope_ref: Some("/repo"),
                tags: &[],
                source_task_id: None,
                embedding: None,
            })
            .await
            .unwrap();
        let id2 = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "learning 2",
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: None,
            })
            .await
            .unwrap();
        // Seed upvote_count=3 on id2 via the production helped-verdict path.
        let task_id = db
            .create_task(CreateTaskRequest {
                title: "t",
                description: "",
                repo_path: "/repo",
                plan: None,
                status: TaskStatus::Backlog,
                base_branch: "main",
                epic_id: None,
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
            })
            .await
            .unwrap();
        db.apply_verdicts_tx(
            task_id,
            &[
                (id2, LearningVerdict::Helped),
                (id2, LearningVerdict::Helped),
                (id2, LearningVerdict::Helped),
            ],
        )
        .await
        .unwrap();

        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![]);

        rt.exec_load_learnings(&mut app).await;

        // TUI handler sorts by upvote_count DESC: id2 (count=3) before id1 (count=0).
        if let ViewMode::Learnings { learnings, .. } = app.view_mode() {
            assert_eq!(learnings.len(), 2);
            assert_eq!(
                learnings[0].id, id2,
                "higher upvote_count should sort first"
            );
            assert_eq!(
                learnings[1].id, id1,
                "lower upvote_count should sort second"
            );
        } else {
            panic!("expected Learnings view mode");
        }
    }

    /// Verify the `LearningServiceApi` seam is injectable without a real database.
    ///
    /// A mock that always returns `ServiceError::Validation` is wired in as
    /// `learning_svc`. `exec_archive_learning` must surface the error through
    /// the status bar — not panic or construct its own `LearningService`.
    #[tokio::test]
    async fn exec_archive_learning_uses_injected_learning_svc_not_ad_hoc_construction() {
        use crate::db::LearningFilter;
        use crate::models::{Learning, LearningVerdict, RetrievalSource};
        use crate::service::{
            CreateLearningParams, LearningServiceApi, ServiceError, UpdateLearningParams,
        };

        struct AlwaysFailLearningService;

        #[async_trait::async_trait]
        impl LearningServiceApi for AlwaysFailLearningService {
            async fn create_learning(
                &self,
                _: CreateLearningParams,
            ) -> Result<LearningId, ServiceError> {
                Err(ServiceError::Validation("mock".into()))
            }
            async fn get_learning(&self, _: LearningId) -> Result<Learning, ServiceError> {
                Err(ServiceError::Validation("mock".into()))
            }
            async fn list_learnings(
                &self,
                _: LearningFilter,
            ) -> Result<Vec<Learning>, ServiceError> {
                Ok(vec![])
            }
            async fn approve_learning(&self, _: LearningId) -> Result<(), ServiceError> {
                Err(ServiceError::Validation("mock".into()))
            }
            async fn reject_learning(&self, _: LearningId) -> Result<(), ServiceError> {
                Err(ServiceError::Validation("mock".into()))
            }
            async fn archive_learning(&self, _: LearningId) -> Result<(), ServiceError> {
                Err(ServiceError::Validation("injected mock error".into()))
            }
            async fn update_learning(&self, _: UpdateLearningParams) -> Result<(), ServiceError> {
                Err(ServiceError::Validation("mock".into()))
            }
            async fn record_retrieval(
                &self,
                _: crate::models::TaskId,
                _: LearningId,
                _: RetrievalSource,
            ) -> Result<(), ServiceError> {
                Ok(())
            }
            async fn apply_verdicts(
                &self,
                _: crate::models::TaskId,
                _: Vec<(LearningId, LearningVerdict)>,
            ) -> Result<(), ServiceError> {
                Ok(())
            }
        }

        let db = Arc::new(Database::open_in_memory().await.unwrap());
        let db_arc: Arc<dyn crate::db::TaskStore> = db.clone();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (feed_tx, _) = mpsc::unbounded_channel();
        let runner: Arc<dyn crate::process::ProcessRunner> =
            Arc::new(crate::process::MockProcessRunner::new(vec![]));
        let emb_svc = EmbeddingService::new_noop();
        let rt = TuiRuntime {
            task_svc: Arc::new(crate::service::TaskService::new(db_arc.clone())),
            epic_svc: Arc::new(crate::service::EpicService::new(db_arc.clone())),
            todo_svc: Arc::new(crate::service::TodoService::new(db.clone())),
            learning_svc: Arc::new(AlwaysFailLearningService),
            feed_runner: Some(crate::feed::FeedRunner::new(
                db_arc.clone(),
                feed_tx,
                runner.clone(),
            )),
            feed_invalidate_tx: None,
            database: db_arc,
            msg_tx: tx,
            runner,
            editor_session: Arc::new(std::sync::Mutex::new(None)),
            emb_svc,
        };
        let mut app = App::new(vec![]);

        rt.exec_archive_learning(&mut app, LearningId(99)).await;

        // The mock returns an error; it must surface in the status bar.
        let msg = app.status_message().unwrap_or_default();
        assert!(
            msg.contains("injected mock error"),
            "expected mock error in status bar, got: {msg:?}"
        );
    }
}
