//! Personal TODO overlay messages.

use crate::models::{Todo, TodoId};

use crate::tui::types::Command;
use crate::tui::App;

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
    QuickAdd {
        title: String,
        linked: Option<crate::models::TodoLink>,
    },
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
    /// Nest the selected root todo under the nearest root item above it in the list.
    Nest(TodoId),
    /// Promote the selected child todo to a root item.
    Unnest(TodoId),
}

impl TodoMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            TodoMessage::Open => app.handle_open_todos(),
            TodoMessage::Show(todos) => app.handle_show_todos(todos),
            TodoMessage::Close => app.handle_close_todos(),
            TodoMessage::MoveSelection(delta) => app.handle_todo_move_selection(delta),
            TodoMessage::Add => app.handle_todo_add(),
            TodoMessage::QuickAdd { title, linked } => app.handle_todo_quick_add(title, linked),
            TodoMessage::Edit(id) => app.handle_todo_edit(id),
            TodoMessage::SubmitTitle(title) => app.handle_todo_submit_title(title),
            TodoMessage::SubmitQuickAdd(title) => app.handle_todo_submit_quick_add(title),
            TodoMessage::ToggleDone(id) => app.handle_todo_toggle(id),
            TodoMessage::Reorder(delta) => app.handle_todo_reorder(delta),
            TodoMessage::ClearDone => app.handle_todo_clear_done(),
            TodoMessage::Delete(id) => app.handle_todo_delete(id),
            TodoMessage::CountUpdated(n) => app.handle_todo_count_updated(n),
            TodoMessage::LinkToTask(todo_id) => app.handle_todo_link_to_task(todo_id),
            TodoMessage::JumpToLinked(link) => app.handle_todo_jump_to_linked(link),
            TodoMessage::Nest(id) => app.handle_todo_nest(id),
            TodoMessage::Unnest(id) => app.handle_todo_unnest(id),
        }
    }
}
