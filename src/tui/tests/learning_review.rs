use super::*;
use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
use chrono::Utc;
use crossterm::event::KeyCode;

pub(super) fn make_learning(id: LearningId) -> Learning {
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

#[test]
fn open_proposed_learnings_returns_load_command() {
    let mut app = make_app();
    let cmds = app.update(Message::OpenProposedLearnings);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::LoadProposedLearnings))
    );
}
