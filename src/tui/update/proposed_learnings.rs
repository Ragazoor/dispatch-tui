//! Proposed-learnings overlay handlers.

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_show_proposed_learnings(
        &mut self,
        learnings: Vec<crate::models::Learning>,
    ) -> Vec<Command> {
        let previous = Box::new(self.board.view_mode.clone());
        self.board.view_mode = ViewMode::ProposedLearnings {
            selected: 0,
            learnings,
            previous,
        };
        vec![]
    }

    pub(in crate::tui) fn handle_close_proposed_learnings(&mut self) -> Vec<Command> {
        if let ViewMode::ProposedLearnings { previous, .. } =
            std::mem::take(&mut self.board.view_mode)
        {
            self.board.view_mode = *previous;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_proposed_learning(
        &mut self,
        delta: isize,
    ) -> Vec<Command> {
        if let ViewMode::ProposedLearnings {
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

    pub(in crate::tui) fn handle_approve_learning(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::ProposedLearnings { ref learnings, .. } = self.board.view_mode {
            if learnings.iter().any(|l| l.id == id) {
                return vec![Command::ApproveLearning(id)];
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_reject_learning(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::ProposedLearnings { ref learnings, .. } = self.board.view_mode {
            if learnings.iter().any(|l| l.id == id) {
                return vec![Command::RejectLearning(id)];
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_edit_learning(
        &mut self,
        id: crate::models::LearningId,
    ) -> Vec<Command> {
        if let ViewMode::ProposedLearnings { ref learnings, .. } = self.board.view_mode {
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
        if let ViewMode::ProposedLearnings {
            ref mut learnings,
            ref mut selected,
            ..
        } = self.board.view_mode
        {
            learnings.retain(|l| l.id != id);
            if !learnings.is_empty() {
                *selected = (*selected).min(learnings.len() - 1);
            } else {
                *selected = 0;
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_learning_edited(
        &mut self,
        updated: crate::models::Learning,
    ) -> Vec<Command> {
        if let ViewMode::ProposedLearnings {
            ref mut learnings, ..
        } = self.board.view_mode
        {
            if let Some(entry) = learnings.iter_mut().find(|l| l.id == updated.id) {
                *entry = updated;
            }
        }
        vec![]
    }
}
