use super::*;
use crate::db;
#[cfg(test)]
use crate::models::ProjectId;
use crate::models::{LearningId, LearningStatus};
use crate::service::LearningService;

impl TuiRuntime {
    pub(super) fn exec_load_learnings(&self, app: &mut App) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        // Load both `approved` and `needs_review` learnings. The overlay groups
        // them with `needs_review` surfaced at the top so humans can curate
        // entries demoted by a `wrong` verdict at wrap-up.
        let approved = db.list_learnings(db::LearningFilter {
            status: Some(LearningStatus::Approved),
            ..Default::default()
        });
        let needs_review = db.list_learnings(db::LearningFilter {
            status: Some(LearningStatus::NeedsReview),
            ..Default::default()
        });
        match (approved, needs_review) {
            (Ok(mut a), Ok(mut nr)) => {
                let mut all = Vec::with_capacity(a.len() + nr.len());
                all.append(&mut nr);
                all.append(&mut a);
                app.update(Message::Learning(LearningMessage::Show(all)));
            }
            (Err(e), _) | (_, Err(e)) => {
                app.update(Message::StatusInfo(format!(
                    "Failed to load learnings: {e}"
                )));
            }
        }
    }

    /// Refresh the count of `NeedsReview` learnings and dispatch
    /// [`LearningMessage::NeedsReviewCountUpdated`] so the `[KB:N]` status-bar badge
    /// stays current. Best-effort — DB errors are logged but not surfaced to
    /// the status bar (the badge simply won't update on this tick).
    pub(super) fn exec_refresh_needs_review_count(&self, app: &mut App) {
        match self.database.count_learnings_needs_review() {
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

    pub(super) fn exec_archive_learning(&self, app: &mut App, id: LearningId) {
        self.exec_action_learning(app, id, "archive", |svc, id| svc.archive_learning(id));
    }

    pub(super) fn exec_reject_learning(&self, app: &mut App, id: LearningId) {
        self.exec_action_learning(app, id, "reject", |svc, id| svc.reject_learning(id));
    }

    /// Approve a learning. Unlike archive/reject, the entry stays visible in
    /// the overlay (it transitions to `Approved`). After the patch we re-read
    /// the learning and dispatch `LearningEdited` so the in-memory row picks
    /// up the new status.
    pub(super) fn exec_approve_learning(&self, app: &mut App, id: LearningId) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        let svc = LearningService::new(db.clone());
        match svc.approve_learning(id) {
            Ok(()) => match db.get_learning(id) {
                Ok(Some(updated)) => {
                    app.update(Message::Learning(LearningMessage::Edited(updated)));
                    app.update(Message::StatusInfo(format!("Learning {id} approved")));
                }
                Ok(None) => {
                    app.update(Message::StatusInfo(format!("Learning {id} not found")));
                }
                Err(e) => {
                    app.update(Message::StatusInfo(format!(
                        "Could not refresh learning: {e}"
                    )));
                }
            },
            Err(e) => {
                app.update(Message::StatusInfo(format!(
                    "Could not approve learning: {e}"
                )));
            }
        }
    }

    fn exec_action_learning(
        &self,
        app: &mut App,
        id: LearningId,
        verb: &str,
        action: impl Fn(&LearningService, LearningId) -> Result<(), crate::service::ServiceError>,
    ) {
        let db: Arc<dyn db::TaskStore> = self.database.clone();
        let svc = LearningService::new(db);
        match action(&svc, id) {
            Ok(()) => {
                app.update(Message::Learning(LearningMessage::Actioned(id)));
                app.update(Message::StatusInfo(format!("Learning {id} {verb}ed")));
            }
            Err(e) => {
                app.update(Message::StatusInfo(format!(
                    "Could not {verb} learning: {e}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::db::{Database, LearningStore};
    use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
    use crate::tui::ViewMode;
    use chrono::Utc;
    use std::sync::Arc;

    const APP_INACTIVITY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

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
            confirmed_count: 0,
            last_confirmed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn insert_learning(db: &Arc<Database>) -> LearningId {
        db.create_learning(
            LearningKind::Convention,
            "test learning",
            None,
            LearningScope::User,
            None,
            &[],
            None,
        )
        .unwrap()
    }

    #[test]
    fn exec_archive_learning_updates_db_and_sends_actioned_message() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = insert_learning(&db);
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], ProjectId(1), APP_INACTIVITY_TIMEOUT);
        // Put the app in Learnings view with the learning
        let learning = make_learning(id);
        app.update(Message::Learning(LearningMessage::Show(vec![learning])));

        rt.exec_archive_learning(&mut app, id);

        // Learning should be removed from the overlay list
        assert!(matches!(
            app.view_mode(),
            ViewMode::Learnings { learnings, .. } if learnings.is_empty()
        ));
        // DB should show archived
        let updated = db.get_learning(id).unwrap().unwrap();
        assert_eq!(updated.status, LearningStatus::Archived);
    }

    #[test]
    fn exec_reject_learning_updates_db_and_sends_actioned_message() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = insert_learning(&db);
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], ProjectId(1), APP_INACTIVITY_TIMEOUT);
        let learning = make_learning(id);
        app.update(Message::Learning(LearningMessage::Show(vec![learning])));

        rt.exec_reject_learning(&mut app, id);

        assert!(matches!(
            app.view_mode(),
            ViewMode::Learnings { learnings, .. } if learnings.is_empty()
        ));
        let updated = db.get_learning(id).unwrap().unwrap();
        assert_eq!(updated.status, LearningStatus::Rejected);
    }

    #[test]
    fn exec_load_passes_learnings_to_show_learnings_sorted_by_confirmed_count() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        // Insert two learnings; bump the second one's confirmed_count via patch.
        let id1 = db
            .create_learning(
                LearningKind::Convention,
                "learning 1",
                None,
                LearningScope::Repo,
                Some("/repo"),
                &[],
                None,
            )
            .unwrap();
        let id2 = db
            .create_learning(
                LearningKind::Convention,
                "learning 2",
                None,
                LearningScope::User,
                None,
                &[],
                None,
            )
            .unwrap();
        // Bump id2's confirmed_count so it sorts first.
        db.upvote_learning(id2).unwrap();
        db.upvote_learning(id2).unwrap();
        db.upvote_learning(id2).unwrap();

        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], ProjectId(1), APP_INACTIVITY_TIMEOUT);

        rt.exec_load_learnings(&mut app);

        // TUI handler sorts by confirmed_count DESC: id2 (count=3) before id1 (count=0).
        if let ViewMode::Learnings { learnings, .. } = app.view_mode() {
            assert_eq!(learnings.len(), 2);
            assert_eq!(
                learnings[0].id, id2,
                "higher confirmed_count should sort first"
            );
            assert_eq!(
                learnings[1].id, id1,
                "lower confirmed_count should sort second"
            );
        } else {
            panic!("expected Learnings view mode");
        }
    }
}
