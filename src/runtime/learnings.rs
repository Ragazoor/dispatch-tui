use super::*;
use crate::db;
#[cfg(test)]
use crate::models::ProjectId;
use crate::models::{LearningId, LearningStatus};
use crate::service::embeddings::EmbeddingService;
use crate::service::LearningService;

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
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        let svc = LearningService::new(db, EmbeddingService::new_noop());
        let result = svc.archive_learning(id).await;
        Self::handle_action_result(app, id, "archive", result);
    }

    pub(super) async fn exec_reject_learning(&self, app: &mut App, id: LearningId) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        let svc = LearningService::new(db, EmbeddingService::new_noop());
        let result = svc.reject_learning(id).await;
        Self::handle_action_result(app, id, "reject", result);
    }

    /// Approve a learning. Unlike archive/reject, the entry stays visible in
    /// the overlay (it transitions to `Approved`). After the patch we re-read
    /// the learning and dispatch `LearningEdited` so the in-memory row picks
    /// up the new status.
    pub(super) async fn exec_approve_learning(&self, app: &mut App, id: LearningId) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        let svc = LearningService::new(db.clone(), EmbeddingService::new_noop());
        match svc.approve_learning(id).await {
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
    use crate::db::{CreateLearningRow, Database, LearningStore};
    use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
    use crate::tui::ViewMode;
    use chrono::Utc;
    use std::sync::Arc;

    fn make_runtime(db: Arc<Database>) -> TuiRuntime {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (feed_tx, _) = mpsc::unbounded_channel();
        let db_arc: Arc<dyn crate::db::TaskStore> = db.clone();
        let runner: Arc<dyn crate::process::ProcessRunner> =
            Arc::new(crate::process::MockProcessRunner::new(vec![]));
        TuiRuntime {
            task_svc: crate::service::TaskService::new(db_arc.clone()),
            epic_svc: crate::service::EpicService::new(db_arc.clone()),
            feed_runner: Some(crate::feed::FeedRunner::new(
                db_arc.clone(),
                feed_tx,
                runner.clone(),
            )),
            database: db_arc,
            msg_tx: tx,
            runner,
            editor_session: Arc::new(std::sync::Mutex::new(None)),
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
        let mut app = App::new(vec![], ProjectId(1));
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
        let mut app = App::new(vec![], ProjectId(1));
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
        db.upvote_learning(id2).await.unwrap();
        db.upvote_learning(id2).await.unwrap();
        db.upvote_learning(id2).await.unwrap();

        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], ProjectId(1));

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
}
