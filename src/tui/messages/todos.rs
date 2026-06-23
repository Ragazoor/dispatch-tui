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
    /// Board quick-add triggered from a selected item. `title` pre-fills the input
    /// buffer; `linked` is stored as the pending link for the created todo.
    QuickAdd { title: String, linked: Option<crate::models::TodoLink> },
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
    /// Enter board-pick mode to link this todo to a task or epic.
    LinkToTask(TodoId),
    /// Jump the board cursor to the linked task or epic, closing the overlay.
    JumpToLinked(crate::models::TodoLink),
}
