//! Knowledge-base (Learning) overlay messages.

use super::super::types::{Command, TreeNav};
use crate::models::{Learning, LearningId};
use crate::tui::App;

/// Messages targeting the Knowledge Base overlay.
///
/// Wrapped by [`crate::tui::types::Message::Learning`] for dispatch.
#[derive(Debug, Clone)]
pub enum LearningMessage {
    Open,
    Show(Vec<Learning>),
    Close,
    Navigate(isize),
    Archive(LearningId),
    Reject(LearningId),
    Edit(LearningId),
    Actioned(LearningId),
    Edited(Learning),
    ToggleView,
    NavigateTree(TreeNav),
}

impl LearningMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            LearningMessage::Open => app.handle_open_learnings(),
            LearningMessage::Show(learnings) => app.handle_show_learnings(learnings),
            LearningMessage::Close => app.handle_close_learnings(),
            LearningMessage::Navigate(delta) => app.handle_navigate_learning(delta),
            LearningMessage::Archive(id) => app.handle_archive_learning(id),
            LearningMessage::Reject(id) => app.handle_reject_learning(id),
            LearningMessage::Edit(id) => app.handle_edit_learning(id),
            LearningMessage::Actioned(id) => app.handle_learning_actioned(id),
            LearningMessage::Edited(updated) => app.handle_learning_edited(updated),
            LearningMessage::ToggleView => app.handle_toggle_learnings_view(),
            LearningMessage::NavigateTree(nav) => app.handle_navigate_tree_learning(nav),
        }
    }
}
