use super::*;
use crate::db;
use crate::models::{LearningId, LearningScope, LearningStatus};
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
                learnings.sort_by_key(|l| {
                    let scope_ord = match l.scope {
                        LearningScope::User => 0,
                        LearningScope::Project => 1,
                        LearningScope::Repo => 2,
                        LearningScope::Epic => 3,
                        LearningScope::Task => 4,
                    };
                    (scope_ord, std::cmp::Reverse(l.created_at))
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

    pub(super) fn exec_approve_learning(&self, app: &mut App, id: LearningId) {
        self.exec_action_learning(app, id, "approve", |svc, id| svc.approve_learning(id));
    }

    pub(super) fn exec_reject_learning(&self, app: &mut App, id: LearningId) {
        self.exec_action_learning(app, id, "reject", |svc, id| svc.reject_learning(id));
    }

    fn exec_action_learning(
        &self,
        app: &mut App,
        id: LearningId,
        verb: &str,
        action: impl Fn(&LearningService, LearningId) -> Result<(), crate::service::ServiceError>,
    ) {
        let db: Arc<dyn db::LearningStore> = self.database.clone();
        let svc = LearningService::new(db);
        match action(&svc, id) {
            Ok(()) => {
                app.update(Message::LearningActioned(id));
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

    #[test]
    fn exec_load_sorts_by_scope_then_created_at_desc() {
        let db = Arc::new(Database::open_in_memory().unwrap());

        // Insert two repo learnings and one user learning.
        // We can't control created_at directly via create_learning (it uses NOW()),
        // so insert them in order and verify scope ordering overrides insertion order.
        let repo_id = db
            .create_learning(
                LearningKind::Convention,
                "repo learning 1",
                None,
                LearningScope::Repo,
                Some("/repo"),
                &[],
                None,
            )
            .unwrap();
        let user_id = db
            .create_learning(
                LearningKind::Convention,
                "user learning",
                None,
                LearningScope::User,
                None,
                &[],
                None,
            )
            .unwrap();

        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![], 1, std::time::Duration::from_secs(300));

        rt.exec_load_proposed_learnings(&mut app);

        // User scope (0) must come before Repo scope (2)
        if let ViewMode::ProposedLearnings { learnings, .. } = app.view_mode() {
            assert_eq!(learnings.len(), 2);
            assert_eq!(learnings[0].id, user_id, "user scope should sort first");
            assert_eq!(learnings[1].id, repo_id, "repo scope should sort second");
        } else {
            panic!("expected ProposedLearnings");
        }
    }
}
