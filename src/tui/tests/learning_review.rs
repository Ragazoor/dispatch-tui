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

fn make_app_with_learnings() -> App {
    let mut app = make_app();
    let learnings = vec![make_learning(1), make_learning(2), make_learning(3)];
    app.update(Message::ShowProposedLearnings(learnings));
    app
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

#[test]
fn show_proposed_learnings_sets_view_mode() {
    let app = make_app_with_learnings();
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { learnings, .. } if learnings.len() == 3
    ));
}

#[test]
fn show_proposed_learnings_stores_previous() {
    let app = make_app_with_learnings();
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { previous, .. }
            if matches!(previous.as_ref(), ViewMode::Board(_))
    ));
}

#[test]
fn close_proposed_learnings_restores_board() {
    let mut app = make_app_with_learnings();
    app.update(Message::CloseProposedLearnings);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn navigate_down_increments_selected() {
    let mut app = make_app_with_learnings();
    app.update(Message::NavigateProposedLearning(1));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { selected, .. } if *selected == 1
    ));
}

#[test]
fn navigate_down_clamps_at_last() {
    let mut app = make_app_with_learnings(); // 3 entries
    app.update(Message::NavigateProposedLearning(100));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { selected, .. } if *selected == 2
    ));
}

#[test]
fn navigate_up_clamps_at_zero() {
    let mut app = make_app_with_learnings();
    app.update(Message::NavigateProposedLearning(-5));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { selected, .. } if *selected == 0
    ));
}

#[test]
fn approve_learning_returns_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.update(Message::ApproveLearning(1));
    assert!(cmds.iter().any(|c| matches!(c, Command::ApproveLearning(id) if *id == 1)));
}

#[test]
fn reject_learning_returns_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.update(Message::RejectLearning(1));
    assert!(cmds.iter().any(|c| matches!(c, Command::RejectLearning(id) if *id == 1)));
}

#[test]
fn learning_actioned_removes_entry_from_list() {
    let mut app = make_app_with_learnings();
    app.update(Message::LearningActioned(2));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { learnings, .. } if learnings.len() == 2
            && !learnings.iter().any(|l| l.id == 2)
    ));
}

#[test]
fn learning_actioned_clamps_selected_when_last_removed() {
    let mut app = make_app_with_learnings();
    // Move cursor to last entry (index 2)
    app.update(Message::NavigateProposedLearning(10));
    // Remove last entry (id=3)
    app.update(Message::LearningActioned(3));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { selected, learnings, .. }
            if *selected == learnings.len() - 1
    ));
}

#[test]
fn learning_edited_replaces_entry_in_snapshot() {
    let mut app = make_app_with_learnings();
    let mut updated = make_learning(2);
    updated.summary = "Updated summary".to_string();
    app.update(Message::LearningEdited(updated));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { learnings, .. }
            if learnings.iter().find(|l| l.id == 2).map(|l| l.summary.as_str()) == Some("Updated summary")
    ));
}

#[test]
fn learning_edited_with_unknown_id_is_noop() {
    let mut app = make_app_with_learnings();
    let unknown = make_learning(99);
    app.update(Message::LearningEdited(unknown));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { learnings, .. } if learnings.len() == 3
    ));
}

#[test]
fn refresh_tasks_does_not_update_learnings_snapshot() {
    let mut app = make_app_with_learnings();
    let original_len = if let ViewMode::ProposedLearnings { learnings, .. } = &app.board.view_mode {
        learnings.len()
    } else {
        panic!("expected ProposedLearnings")
    };
    // Simulate a RefreshTasks (MCP tick fires while overlay is open)
    app.update(Message::RefreshTasks(app.board.tasks.clone()));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { learnings, .. } if learnings.len() == original_len
    ));
}

#[test]
fn edit_learning_returns_pop_out_editor_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.update(Message::EditLearning(2));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PopOutEditor(EditKind::Learning(l)) if l.id == 2
    )));
}

#[test]
fn learning_actioned_on_single_entry_empties_list() {
    let mut app = make_app();
    app.update(Message::ShowProposedLearnings(vec![make_learning(1)]));
    app.update(Message::LearningActioned(1));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::ProposedLearnings { learnings, selected, .. }
            if learnings.is_empty() && *selected == 0
    ));
}

#[test]
fn approve_on_empty_list_is_noop() {
    let mut app = make_app();
    app.update(Message::ShowProposedLearnings(vec![]));
    let cmds = app.update(Message::ApproveLearning(1));
    assert!(cmds.is_empty());
}
