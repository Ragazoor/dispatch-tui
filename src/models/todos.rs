use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::define_id_newtype;
use crate::models::{EpicId, TaskId};

define_id_newtype!(TodoId, todo_id_tests);

/// Discriminated link from a todo to a task or epic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoLink {
    Task(TaskId),
    Epic(EpicId),
}

/// A personal TODO item. Flat, ordered, separate from kanban Tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub id: TodoId,
    pub title: String,
    pub done: bool,
    pub sort_order: i64,
    /// `None` = root item; `Some(id)` = child of that root. Depth capped at 1.
    pub parent_id: Option<TodoId>,
    /// Link to a task or epic on the board. `None` = unlinked.
    pub linked: Option<TodoLink>,
    /// The person whose checklist this is (`todo.allium: Todo.owner`), or
    /// `None` on a todo created before this install ever connected to a shared
    /// store. Written once, at creation; nothing on the update path reaches it.
    pub owner: Option<String>,
    pub created_at: DateTime<Utc>,
}
