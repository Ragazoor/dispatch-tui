use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::define_id_newtype;

define_id_newtype!(TodoId, todo_id_tests);

/// A personal TODO item. Flat, ordered, separate from kanban Tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    pub id: TodoId,
    pub title: String,
    pub done: bool,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
}
