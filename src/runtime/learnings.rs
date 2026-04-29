use super::*;
use crate::db;
use crate::models::LearningStatus;
use crate::service::LearningService;

impl TuiRuntime {
    pub(super) fn exec_load_proposed_learnings(&self, app: &mut App) {
        let db: Arc<dyn db::LearningStore> = self.database.clone();
        let filter = db::LearningFilter {
            status: Some(LearningStatus::Proposed),
            ..Default::default()
        };
        match db.list_learnings(filter) {
            Ok(mut learnings) => {
                // Sort: user < project < repo < epic < task, then created_at desc within scope
                learnings.sort_by(|a, b| {
                    let scope_ord = |s: crate::models::LearningScope| match s {
                        crate::models::LearningScope::User => 0,
                        crate::models::LearningScope::Project => 1,
                        crate::models::LearningScope::Repo => 2,
                        crate::models::LearningScope::Epic => 3,
                        crate::models::LearningScope::Task => 4,
                    };
                    scope_ord(a.scope)
                        .cmp(&scope_ord(b.scope))
                        .then(b.created_at.cmp(&a.created_at))
                });
                app.update(Message::ShowProposedLearnings(learnings));
            }
            Err(e) => {
                app.update(Message::StatusInfo(format!(
                    "Failed to load learnings: {e}"
                )));
            }
        }
    }

    pub(super) fn exec_approve_learning(
        &self,
        app: &mut App,
        id: crate::models::LearningId,
    ) {
        let db: Arc<dyn db::LearningStore> = self.database.clone();
        let svc = LearningService::new(db);
        match svc.approve_learning(id) {
            Ok(()) => {
                app.update(Message::LearningActioned(id));
                app.update(Message::StatusInfo(format!("Learning {id} approved")));
            }
            Err(e) => {
                app.update(Message::StatusInfo(format!(
                    "Could not approve learning: {e}"
                )));
            }
        }
    }

    pub(super) fn exec_reject_learning(
        &self,
        app: &mut App,
        id: crate::models::LearningId,
    ) {
        let db: Arc<dyn db::LearningStore> = self.database.clone();
        let svc = LearningService::new(db);
        match svc.reject_learning(id) {
            Ok(()) => {
                app.update(Message::LearningActioned(id));
                app.update(Message::StatusInfo(format!("Learning {id} rejected")));
            }
            Err(e) => {
                app.update(Message::StatusInfo(format!(
                    "Could not reject learning: {e}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LearningStore};
    use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
    use crate::tui::ViewMode;
    use chrono::Utc;
    use std::sync::Arc;

    fn make_runtime(db: Arc<Database>) -> TuiRuntime {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (feed_tx, _) = mpsc::unbounded_channel();
        let db_arc: Arc<dyn crate::db::TaskStore> = db.clone();
        TuiRuntime {
            task_svc: crate::service::TaskService::new(db_arc.clone()),
            epic_svc: crate::service::EpicService::new(db_arc.clone()),
            feed_runner: crate::feed::FeedRunner::new(db_arc.clone(), feed_tx),
            database: db_arc,
            msg_tx: tx,
            runner: Arc::new(crate::process::MockProcessRunner::new(vec![])),
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
            status: LearningStatus::Proposed,
            source_task_id: None,
            confirmed_count: 0,
            last_confirmed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn insert_proposed_learning(db: &Arc<Database>) -> LearningId {
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
    fn exec_approve_learning_updates_db_and_sends_actioned_message() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = insert_proposed_learning(&db);
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], 1, std::time::Duration::from_secs(300));
        // Put the app in ProposedLearnings view with the learning
        let learning = make_learning(id);
        app.update(Message::ShowProposedLearnings(vec![learning]));

        rt.exec_approve_learning(&mut app, id);

        // Learning should be removed from the overlay list
        assert!(matches!(
            app.view_mode(),
            ViewMode::ProposedLearnings { learnings, .. } if learnings.is_empty()
        ));
        // DB should show approved
        let updated = db.get_learning(id).unwrap().unwrap();
        assert_eq!(updated.status, LearningStatus::Approved);
    }

    #[test]
    fn exec_reject_learning_updates_db_and_sends_actioned_message() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let id = insert_proposed_learning(&db);
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], 1, std::time::Duration::from_secs(300));
        let learning = make_learning(id);
        app.update(Message::ShowProposedLearnings(vec![learning]));

        rt.exec_reject_learning(&mut app, id);

        assert!(matches!(
            app.view_mode(),
            ViewMode::ProposedLearnings { learnings, .. } if learnings.is_empty()
        ));
        let updated = db.get_learning(id).unwrap().unwrap();
        assert_eq!(updated.status, LearningStatus::Rejected);
    }

    #[test]
    fn exec_approve_on_nonexistent_id_shows_status_info() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let rt = make_runtime(db);
        let mut app = App::new(vec![], 1, std::time::Duration::from_secs(300));

        rt.exec_approve_learning(&mut app, 999);

        // Should show a status message, not panic
        assert!(app.status_message().is_some());
    }
}
