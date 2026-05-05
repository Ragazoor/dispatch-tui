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
        status: LearningStatus::Approved,
        source_task_id: None,
        confirmed_count: 0,
        last_confirmed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn make_app_with_learnings() -> App {
    let mut app = make_app();
    let learnings = vec![
        make_learning(LearningId(1)),
        make_learning(LearningId(2)),
        make_learning(LearningId(3)),
    ];
    app.update(Message::ShowLearnings(learnings));
    app
}

#[test]
fn open_proposed_learnings_returns_load_command() {
    let mut app = make_app();
    let cmds = app.update(Message::OpenLearnings);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::LoadLearnings)));
}

#[test]
fn show_proposed_learnings_sets_view_mode() {
    let app = make_app_with_learnings();
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { learnings, .. } if learnings.len() == 3
    ));
}

#[test]
fn show_proposed_learnings_stores_previous() {
    let app = make_app_with_learnings();
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { previous, .. }
            if matches!(previous.as_ref(), ViewMode::Board(_))
    ));
}

#[test]
fn close_proposed_learnings_restores_board() {
    let mut app = make_app_with_learnings();
    app.update(Message::CloseLearnings);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn navigate_down_increments_selected() {
    let mut app = make_app_with_learnings();
    app.update(Message::NavigateLearning(1));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { selected, .. } if *selected == 1
    ));
}

#[test]
fn navigate_down_clamps_at_last() {
    let mut app = make_app_with_learnings(); // 3 entries
    app.update(Message::NavigateLearning(100));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { selected, .. } if *selected == 2
    ));
}

#[test]
fn navigate_up_clamps_at_zero() {
    let mut app = make_app_with_learnings();
    app.update(Message::NavigateLearning(-5));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { selected, .. } if *selected == 0
    ));
}

#[test]
fn approve_learning_returns_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.update(Message::ArchiveLearning(LearningId(1)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ArchiveLearning(id) if *id == LearningId(1))));
}

#[test]
fn reject_learning_returns_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.update(Message::RejectLearning(LearningId(1)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::RejectLearning(id) if *id == LearningId(1))));
}

#[test]
fn learning_actioned_removes_entry_from_list() {
    let mut app = make_app_with_learnings();
    app.update(Message::LearningActioned(LearningId(2)));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { learnings, .. } if learnings.len() == 2
            && !learnings.iter().any(|l| l.id == LearningId(2))
    ));
}

#[test]
fn learning_actioned_clamps_selected_when_last_removed() {
    let mut app = make_app_with_learnings();
    // Move cursor to last entry (index 2)
    app.update(Message::NavigateLearning(10));
    // Remove last entry (id=3)
    app.update(Message::LearningActioned(LearningId(3)));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { selected, learnings, .. }
            if *selected == learnings.len() - 1
    ));
}

#[test]
fn learning_edited_replaces_entry_in_snapshot() {
    let mut app = make_app_with_learnings();
    let mut updated = make_learning(LearningId(2));
    updated.summary = "Updated summary".to_string();
    app.update(Message::LearningEdited(updated));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { learnings, .. }
            if learnings.iter().find(|l| l.id == LearningId(2)).map(|l| l.summary.as_str()) == Some("Updated summary")
    ));
}

#[test]
fn learning_edited_with_unknown_id_is_noop() {
    let mut app = make_app_with_learnings();
    let unknown = make_learning(LearningId(99));
    app.update(Message::LearningEdited(unknown));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { learnings, .. } if learnings.len() == 3
    ));
}

#[test]
fn refresh_tasks_does_not_update_learnings_snapshot() {
    let mut app = make_app_with_learnings();
    let original_len = if let ViewMode::Learnings { learnings, .. } = &app.board.view_mode {
        learnings.len()
    } else {
        panic!("expected ProposedLearnings")
    };
    // Simulate a RefreshTasks (MCP tick fires while overlay is open)
    app.update(Message::RefreshTasks(app.board.tasks.clone()));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { learnings, .. } if learnings.len() == original_len
    ));
}

#[test]
fn edit_learning_returns_pop_out_editor_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.update(Message::EditLearning(LearningId(2)));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PopOutEditor(EditKind::Learning(l)) if l.id == LearningId(2)
    )));
}

#[test]
fn learning_actioned_on_single_entry_empties_list() {
    let mut app = make_app();
    app.update(Message::ShowLearnings(vec![make_learning(
        LearningId(1),
    )]));
    app.update(Message::LearningActioned(LearningId(1)));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { learnings, selected, .. }
            if learnings.is_empty() && *selected == 0
    ));
}

#[test]
fn approve_on_empty_list_is_noop() {
    let mut app = make_app();
    app.update(Message::ShowLearnings(vec![]));
    let cmds = app.update(Message::ArchiveLearning(LearningId(1)));
    assert!(cmds.is_empty());
}

#[test]
fn j_key_navigates_down_in_overlay() {
    let mut app = make_app_with_learnings();
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { selected, .. } if *selected == 1
    ));
}

#[test]
fn k_key_at_top_stays_at_zero() {
    let mut app = make_app_with_learnings();
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(matches!(
        &app.board.view_mode,
        ViewMode::Learnings { selected, .. } if *selected == 0
    ));
}

#[test]
fn q_closes_overlay() {
    let mut app = make_app_with_learnings();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn esc_closes_overlay() {
    let mut app = make_app_with_learnings();
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn a_key_emits_approve_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    // selected=0, first learning has id=1
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::ArchiveLearning(id) if *id == LearningId(1))));
}

#[test]
fn r_key_emits_reject_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    // selected=0, first learning has id=1
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::RejectLearning(id) if *id == LearningId(1))));
}

#[test]
fn e_key_emits_pop_out_editor_command() {
    let mut app = make_app_with_learnings();
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    // selected=0, first learning has id=1
    assert!(cmds.iter().any(
        |c| matches!(c, Command::PopOutEditor(EditKind::Learning(l)) if l.id == LearningId(1))
    ));
}

#[test]
fn board_keys_inert_when_overlay_open() {
    let mut app = make_app_with_learnings();
    // 'd' would dispatch a task from the board — must be swallowed
    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Learnings { .. }
    ));
}

#[test]
fn i_key_from_board_emits_load_command() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('I')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::LoadLearnings)));
}

#[test]
fn a_key_on_empty_overlay_is_inert() {
    let mut app = make_app();
    app.update(Message::ShowLearnings(vec![]));
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Learnings { .. }
    ));
}
