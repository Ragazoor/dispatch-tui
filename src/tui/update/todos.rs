//! Personal TODO overlay handlers.

use crate::models::{Todo, TodoId};
use crate::tui::types::{BoardSelection, Command, InputMode, ViewMode};
use crate::tui::App;

/// Sort todos for display: open items first by sort_order, done items at the bottom.
fn sort_todos(todos: &mut [Todo]) {
    todos.sort_by_key(|t| (t.done, t.sort_order, t.id.0));
}

impl App {
    pub(in crate::tui) fn handle_show_todos(
        &mut self,
        mut todos: Vec<Todo>,
    ) -> Vec<Command> {
        sort_todos(&mut todos);
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

    pub(in crate::tui) fn handle_todo_move_selection(&mut self, delta: isize) -> Vec<Command> {
        if let ViewMode::Todos {
            todos, selected, ..
        } = &mut self.board.view_mode
        {
            if todos.is_empty() {
                return vec![];
            }
            let max = todos.len() - 1;
            let next = (*selected as isize + delta).clamp(0, max as isize) as usize;
            *selected = next;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_todo_add(&mut self) -> Vec<Command> {
        self.pending_todo_edit = None;
        self.input.buffer.clear();
        self.input.mode = InputMode::TodoTitle;
        vec![]
    }

    pub(in crate::tui) fn handle_todo_quick_add(&mut self) -> Vec<Command> {
        self.input.buffer.clear();
        self.input.mode = InputMode::TodoQuickAdd;
        vec![]
    }

    pub(in crate::tui) fn handle_todo_edit(&mut self, id: TodoId) -> Vec<Command> {
        if let ViewMode::Todos { todos, .. } = &self.board.view_mode {
            if let Some(t) = todos.iter().find(|t| t.id == id) {
                self.input.buffer = t.title.clone();
                self.input.mode = InputMode::TodoTitle;
                self.pending_todo_edit = Some(id);
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_todo_submit_title(&mut self, title: String) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.input.buffer.clear();
        let title = title.trim().to_string();
        if title.is_empty() {
            self.pending_todo_edit = None;
            return vec![];
        }
        if let Some(id) = self.pending_todo_edit.take() {
            // Edit: apply optimistically to the in-memory Vec, then persist.
            if let ViewMode::Todos { todos, .. } = &mut self.board.view_mode {
                if let Some(t) = todos.iter_mut().find(|t| t.id == id) {
                    t.title = title.clone();
                }
            }
            return vec![Command::Todo(crate::tui::commands::TodoCommand::Update {
                id,
                update: crate::service::TodoUpdate {
                    title: Some(title),
                    ..Default::default()
                },
            })];
        }
        // Add: the new id is unknown until the DB insert, so reload the view after create.
        vec![Command::Todo(crate::tui::commands::TodoCommand::Create {
            title,
            reopen: true,
        })]
    }

    pub(in crate::tui) fn handle_todo_submit_quick_add(&mut self, title: String) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.input.buffer.clear();
        let title = title.trim().to_string();
        if title.is_empty() {
            return vec![];
        }
        // Stays on the board; only refreshes the open-count.
        vec![Command::Todo(crate::tui::commands::TodoCommand::Create {
            title,
            reopen: false,
        })]
    }

    pub(in crate::tui) fn handle_todo_toggle(&mut self, id: TodoId) -> Vec<Command> {
        let mut new_done = None;
        if let ViewMode::Todos {
            todos, selected, ..
        } = &mut self.board.view_mode
        {
            if let Some(t) = todos.iter_mut().find(|t| t.id == id) {
                t.done = !t.done;
                new_done = Some(t.done);
            }
            sort_todos(todos);
            *selected = (*selected).min(todos.len().saturating_sub(1));
        }
        self.refresh_todo_count_from_view();
        match new_done {
            Some(done) => vec![Command::Todo(crate::tui::commands::TodoCommand::Update {
                id,
                update: crate::service::TodoUpdate {
                    done: Some(done),
                    ..Default::default()
                },
            })],
            None => vec![],
        }
    }

    pub(in crate::tui) fn handle_todo_reorder(&mut self, delta: isize) -> Vec<Command> {
        let mut cmds = vec![];
        if let ViewMode::Todos {
            todos, selected, ..
        } = &mut self.board.view_mode
        {
            let i = *selected;
            let j_signed = i as isize + delta;
            if j_signed < 0 || j_signed as usize >= todos.len() {
                return vec![];
            }
            let j = j_signed as usize;
            // Swap sort_order values, then swap positions so the Vec stays sorted.
            let (so_i, so_j) = (todos[i].sort_order, todos[j].sort_order);
            todos[i].sort_order = so_j;
            todos[j].sort_order = so_i;
            todos.swap(i, j);
            *selected = j;
            // After swap: todos[j] is the moved item, todos[i] is the displaced one.
            cmds.push(Command::Todo(crate::tui::commands::TodoCommand::Update {
                id: todos[j].id,
                update: crate::service::TodoUpdate {
                    sort_order: Some(todos[j].sort_order),
                    ..Default::default()
                },
            }));
            cmds.push(Command::Todo(crate::tui::commands::TodoCommand::Update {
                id: todos[i].id,
                update: crate::service::TodoUpdate {
                    sort_order: Some(todos[i].sort_order),
                    ..Default::default()
                },
            }));
        }
        cmds
    }

    pub(in crate::tui) fn handle_todo_clear_done(&mut self) -> Vec<Command> {
        if let ViewMode::Todos {
            todos, selected, ..
        } = &mut self.board.view_mode
        {
            todos.retain(|t| !t.done);
            *selected = (*selected).min(todos.len().saturating_sub(1));
        }
        self.refresh_todo_count_from_view();
        vec![Command::Todo(crate::tui::commands::TodoCommand::ClearDone)]
    }

    pub(in crate::tui) fn handle_todo_delete(&mut self, id: TodoId) -> Vec<Command> {
        if let ViewMode::Todos {
            todos, selected, ..
        } = &mut self.board.view_mode
        {
            todos.retain(|t| t.id != id);
            *selected = (*selected).min(todos.len().saturating_sub(1));
        }
        self.refresh_todo_count_from_view();
        vec![Command::Todo(crate::tui::commands::TodoCommand::Delete(id))]
    }
}
