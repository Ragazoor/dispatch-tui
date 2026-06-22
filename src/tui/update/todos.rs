//! Personal TODO overlay handlers.

use crate::models::TodoId;
use crate::tui::types::{BoardSelection, Command, ViewMode};
use crate::tui::App;

impl App {
    pub(in crate::tui) fn handle_show_todos(
        &mut self,
        mut todos: Vec<crate::models::Todo>,
    ) -> Vec<Command> {
        // Open items keep their sort_order; done items sink to the bottom.
        todos.sort_by_key(|t| (t.done, t.sort_order, t.id.0));
        let previous = Box::new(std::mem::replace(
            &mut self.board.view_mode,
            ViewMode::Board(BoardSelection::default()),
        ));
        self.board.view_mode = ViewMode::Todos {
            todos,
            selected: 0,
            previous,
        };
        self.refresh_todo_count_from_view();
        vec![]
    }

    pub(in crate::tui) fn handle_close_todos(&mut self) -> Vec<Command> {
        // Mirror handle_close_learnings: take the view out (so nothing is borrowed)
        // before reassigning. `&self.board.view_mode` + assign-inside is E0506.
        if let ViewMode::Todos { previous, .. } = std::mem::take(&mut self.board.view_mode) {
            self.board.view_mode = *previous;
        }
        vec![]
    }

    /// Recompute the board footer's open-todo count from the in-memory list.
    /// Called after every in-view mutation so the board count never goes stale
    /// when the user returns to it (no DB round-trip needed).
    pub(in crate::tui) fn refresh_todo_count_from_view(&mut self) {
        if let ViewMode::Todos { todos, .. } = &self.board.view_mode {
            self.board.todo_open_count = todos.iter().filter(|t| !t.done).count() as i64;
        }
    }

    // ── Stubs — real bodies added in Tasks 10 / 11 ──────────────────────────

    pub(in crate::tui) fn handle_todo_move_selection(&mut self, _delta: isize) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_add(&mut self) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_quick_add(&mut self) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_edit(&mut self, _id: TodoId) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_submit_title(&mut self, _title: String) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_submit_quick_add(&mut self, _title: String) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_toggle(&mut self, _id: TodoId) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_reorder(&mut self, _delta: isize) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_clear_done(&mut self) -> Vec<Command> {
        vec![]
    }

    pub(in crate::tui) fn handle_todo_delete(&mut self, _id: TodoId) -> Vec<Command> {
        vec![]
    }
}
