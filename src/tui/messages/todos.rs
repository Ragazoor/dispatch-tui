//! Personal TODO overlay messages.

use crate::models::{Todo, TodoId};

/// Messages targeting the personal TODO view.
///
/// Wrapped by [`crate::tui::types::Message::Todo`] for dispatch.
#[derive(Debug, Clone)]
pub enum TodoMessage {
    Open,
    Show(Vec<Todo>),
    Close,
    MoveSelection(isize),
    Add,
    QuickAdd,
    Edit(TodoId),
    /// Commit of the in-view title input (add or edit). Carries the typed buffer.
    SubmitTitle(String),
    /// Commit of the board quick-add input. Carries the typed buffer.
    SubmitQuickAdd(String),
    ToggleDone(TodoId),
    Reorder(isize),
    ClearDone,
    Delete(TodoId),
    CountUpdated(i64),
}
