//! Learnings overlay handlers.

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_show_learnings(
        &mut self,
        mut learnings: Vec<crate::models::Learning>,
    ) -> Vec<Command> {
        // Sort by confirmed_count DESC; stable sort preserves insertion order as a tiebreaker.
        learnings.sort_by_key(|b| std::cmp::Reverse(b.confirmed_count));
        let previous = Box::new(std::mem::replace(
            &mut self.board.view_mode,
            ViewMode::Board(BoardSelection::default()),
        ));
        self.board.view_mode = ViewMode::Learnings {
            selected: 0,
            learnings,
            view: LearningsView::List,
            tree_state: std::cell::RefCell::new(tui_tree_widget::TreeState::default()),
            previous,
        };
        vec![]
    }

    pub(in crate::tui) fn handle_close_learnings(&mut self) -> Vec<Command> {
        if let ViewMode::Learnings { previous, .. } = std::mem::take(&mut self.board.view_mode) {
            self.board.view_mode = *previous;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_learning(&mut self, delta: isize) -> Vec<Command> {
        if let ViewMode::Learnings {
            ref mut selected,
            ref learnings,
            ..
        } = self.board.view_mode
        {
            if !learnings.is_empty() {
                let count = learnings.len() as isize;
                *selected = (*selected as isize + delta).clamp(0, count - 1) as usize;
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_reject_learning(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::Learnings { ref learnings, .. } = self.board.view_mode {
            if learnings.iter().any(|l| l.id == id) {
                return vec![Command::RejectLearning(id)];
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_archive_learning(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::Learnings { ref learnings, .. } = self.board.view_mode {
            if learnings.iter().any(|l| l.id == id) {
                return vec![Command::ArchiveLearning(id)];
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_edit_learning(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::Learnings { ref learnings, .. } = self.board.view_mode {
            if let Some(learning) = learnings.iter().find(|l| l.id == id).cloned() {
                return vec![Command::PopOutEditor(EditKind::Learning(learning))];
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_learning_actioned(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::Learnings {
            ref mut learnings,
            ref mut selected,
            ref tree_state,
            ..
        } = self.board.view_mode
        {
            learnings.retain(|l| l.id != id);
            *selected = (*selected).min(learnings.len().saturating_sub(1));
            // Reset tree cursor to first valid node after removal
            tree_state.borrow_mut().select_first();
        }
        vec![]
    }

    pub(in crate::tui) fn handle_learning_edited(
        &mut self,
        updated: crate::models::Learning,
    ) -> Vec<Command> {
        if let ViewMode::Learnings {
            ref mut learnings, ..
        } = self.board.view_mode
        {
            if let Some(entry) = learnings.iter_mut().find(|l| l.id == updated.id) {
                *entry = updated;
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_learnings_view(&mut self) -> Vec<Command> {
        if let ViewMode::Learnings { ref mut view, .. } = self.board.view_mode {
            *view = match view {
                LearningsView::List => LearningsView::Tree,
                LearningsView::Tree => LearningsView::List,
            };
        }
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_tree_learning(&mut self, nav: TreeNav) -> Vec<Command> {
        if let ViewMode::Learnings { ref tree_state, .. } = self.board.view_mode {
            let mut state = tree_state.borrow_mut();
            match nav {
                TreeNav::Up => {
                    state.key_up();
                }
                TreeNav::Down => {
                    state.key_down();
                }
                TreeNav::Left => {
                    state.key_left();
                }
                TreeNav::Right => {
                    state.key_right();
                }
            }
        }
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Learning, LearningId, LearningKind, LearningScope, LearningStatus};
    use crate::tui::tests::make_app;
    use chrono::Utc;

    fn make_learning(id: LearningId) -> Learning {
        Learning {
            id,
            kind: LearningKind::Convention,
            summary: format!("Learning {}", id.0),
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
        app.handle_show_learnings(learnings);
        app
    }

    #[test]
    fn show_learnings_sets_view_mode() {
        let app = make_app_with_learnings();
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings { learnings, .. } if learnings.len() == 3
        ));
    }

    #[test]
    fn show_learnings_stores_previous() {
        let app = make_app_with_learnings();
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings { previous, .. }
                if matches!(previous.as_ref(), ViewMode::Board(_))
        ));
    }

    #[test]
    fn close_learnings_restores_board() {
        let mut app = make_app_with_learnings();
        app.handle_close_learnings();
        assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
    }

    #[test]
    fn toggle_view_switches_list_to_tree() {
        let mut app = make_app_with_learnings();
        app.handle_toggle_learnings_view();
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings {
                view: LearningsView::Tree,
                ..
            }
        ));
    }

    #[test]
    fn toggle_view_switches_tree_to_list() {
        let mut app = make_app_with_learnings();
        app.handle_toggle_learnings_view();
        app.handle_toggle_learnings_view();
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings {
                view: LearningsView::List,
                ..
            }
        ));
    }

    #[test]
    fn toggle_view_is_noop_when_not_in_learnings() {
        let mut app = make_app();
        app.handle_toggle_learnings_view();
        assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
    }

    #[test]
    fn archive_learning_returns_command() {
        let mut app = make_app_with_learnings();
        let cmds = app.handle_archive_learning(LearningId(1));
        assert!(cmds
            .iter()
            .any(|c| matches!(c, Command::ArchiveLearning(id) if *id == LearningId(1))));
    }

    #[test]
    fn archive_learning_on_unknown_id_is_noop() {
        let mut app = make_app_with_learnings();
        let cmds = app.handle_archive_learning(LearningId(99));
        assert!(cmds.is_empty());
    }

    #[test]
    fn learning_actioned_removes_entry() {
        let mut app = make_app_with_learnings();
        app.handle_learning_actioned(LearningId(2));
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings { learnings, .. }
                if learnings.len() == 2 && !learnings.iter().any(|l| l.id == LearningId(2))
        ));
    }

    #[test]
    fn learning_actioned_clamps_selected() {
        let mut app = make_app_with_learnings();
        app.handle_navigate_learning(10);
        app.handle_learning_actioned(LearningId(3));
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings { selected, learnings, .. }
                if *selected == learnings.len().saturating_sub(1)
        ));
    }

    #[test]
    fn learning_actioned_on_single_entry_empties_list() {
        let mut app = make_app();
        app.handle_show_learnings(vec![make_learning(LearningId(1))]);
        app.handle_learning_actioned(LearningId(1));
        assert!(matches!(
            &app.board.view_mode,
            ViewMode::Learnings { learnings, selected, .. }
                if learnings.is_empty() && *selected == 0
        ));
    }

    #[test]
    fn show_learnings_sorts_by_confirmed_count_desc() {
        let mut app = make_app();
        // Assign distinct counts so sort order is deterministic.
        let mut l1 = make_learning(LearningId(1));
        l1.confirmed_count = 5;
        let mut l2 = make_learning(LearningId(2));
        l2.confirmed_count = 10;
        let mut l3 = make_learning(LearningId(3));
        l3.confirmed_count = 1;
        app.handle_show_learnings(vec![l1, l2, l3]);
        if let ViewMode::Learnings { learnings, .. } = &app.board.view_mode {
            assert_eq!(learnings[0].id, LearningId(2), "highest count first");
            assert_eq!(learnings[1].id, LearningId(1), "middle count second");
            assert_eq!(learnings[2].id, LearningId(3), "lowest count last");
        } else {
            panic!("expected Learnings view mode");
        }
    }

    #[test]
    fn show_learnings_preserves_order_for_equal_counts() {
        let mut app = make_app();
        // All equal confirmed_count — stable sort preserves insertion order.
        let l1 = make_learning(LearningId(1));
        let l2 = make_learning(LearningId(2));
        let l3 = make_learning(LearningId(3));
        app.handle_show_learnings(vec![l1, l2, l3]);
        if let ViewMode::Learnings { learnings, .. } = &app.board.view_mode {
            assert_eq!(learnings[0].id, LearningId(1), "insertion order preserved");
            assert_eq!(learnings[1].id, LearningId(2));
            assert_eq!(learnings[2].id, LearningId(3));
        } else {
            panic!("expected Learnings view mode");
        }
    }
}
