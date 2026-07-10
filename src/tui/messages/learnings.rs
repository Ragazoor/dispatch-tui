//! Knowledge-base (Learning) overlay messages.

use super::super::types::TreeNav;
use crate::models::{Learning, LearningId};

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
