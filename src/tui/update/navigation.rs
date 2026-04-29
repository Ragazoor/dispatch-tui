//! Board navigation handlers: quit, column/row navigation, reorder.

use crate::models::TaskStatus;

use super::super::types::*;
use super::super::{is_edge_column, App};

impl App {
    pub(in crate::tui) fn handle_quit(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::ConfirmQuit;
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_column(&mut self, delta: isize) -> Vec<Command> {
        // Column range [0, 5]: 0=Projects, 1=Backlog, 2=Running, 3=Review, 4=Done, 5=Archive.
        // In Epic view, Projects and Archive are not shown; clamp to [1, COLUMN_COUNT].
        let (min_col, max_col) = if matches!(self.effective_view_mode(), ViewMode::Epic { .. }) {
            (1isize, TaskStatus::COLUMN_COUNT as isize) // [1, 4] in epic view
        } else {
            (0isize, TaskStatus::COLUMN_COUNT as isize + 1) // [0, 5] on main board
        };
        let new_col = (self.selection().column() as isize + delta).clamp(min_col, max_col) as usize;
        self.selection_mut().set_column(new_col);

        // Reset archive cursor when entering the archive column.
        if new_col == TaskStatus::COLUMN_COUNT + 1 {
            self.selection_mut()
                .set_row(TaskStatus::COLUMN_COUNT + 1, 0);
            *self.archive.list_state.selected_mut() = Some(0);
        }

        self.clamp_selection();
        self.update_anchor_from_current();
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_row(&mut self, delta: isize) -> Vec<Command> {
        let col = self.selection().column();

        if col == 0 {
            let count = self.board.projects.len();
            if count == 0 {
                return vec![];
            }
            let new_row =
                (self.selection().row(0) as isize + delta).clamp(0, count as isize - 1) as usize;
            self.selection_mut().set_row(0, new_row);
            self.projects_panel.list_state.select(Some(new_row));
            return vec![];
        }
        if col == TaskStatus::COLUMN_COUNT + 1 {
            let count = self.archived_tasks().len();
            if count == 0 {
                return vec![];
            }
            let new_row = (self.selection().row(TaskStatus::COLUMN_COUNT + 1) as isize + delta)
                .clamp(0, count as isize - 1) as usize;
            self.selection_mut()
                .set_row(TaskStatus::COLUMN_COUNT + 1, new_row);
            self.archive.list_state.select(Some(new_row));
            return vec![];
        }

        let status = match TaskStatus::from_column_index(col - 1) {
            Some(s) => s,
            None => return vec![],
        };
        let count = self.column_items_for_status(status).len();

        if self.selection().on_select_all {
            // On the toggle row
            if delta > 0 && count > 0 {
                // Move down into task list
                self.selection_mut().on_select_all = false;
                self.selection_mut().set_row(col, 0);
            }
            // delta <= 0 or empty column: stay on toggle (already at top)
        } else if count > 0 {
            let current = self.selection().row(col);
            if current == 0 && delta < 0 {
                // Move up from first task to toggle row
                self.selection_mut().on_select_all = true;
            } else {
                let new_row = (current as isize + delta).clamp(0, count as isize - 1) as usize;
                self.selection_mut().set_row(col, new_row);
            }
        } else {
            // Empty column: move to toggle
            if delta < 0 {
                self.selection_mut().on_select_all = true;
            }
        }
        self.update_anchor_from_current();
        vec![]
    }

    pub(in crate::tui) fn handle_reorder_item(&mut self, direction: isize) -> Vec<Command> {
        let col = self.selection().column();
        if is_edge_column(col) {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let row = self.selection().row(col);
        let items = self.column_items_for_status(status);
        let target_row = row as isize + direction;
        if target_row < 0 || target_row >= items.len() as isize {
            return vec![];
        }
        let target_row = target_row as usize;

        // Get IDs and effective sort values
        let (a_task_id, a_epic_id, a_eff) = match &items[row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_order.unwrap_or(t.id.0)),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_order.unwrap_or(e.id.0)),
        };
        let (b_task_id, b_epic_id, b_eff) = match &items[target_row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_order.unwrap_or(t.id.0)),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_order.unwrap_or(e.id.0)),
        };

        // Swap effective values; offset if equal
        let (new_a, new_b) = if a_eff == b_eff {
            if direction > 0 {
                (a_eff + 1, b_eff)
            } else {
                (a_eff - 1, b_eff)
            }
        } else {
            (b_eff, a_eff)
        };

        // Drop the borrowed items before mutating
        drop(items);

        let mut cmds = vec![];

        if let Some(tid) = a_task_id {
            if let Some(t) = self.find_task_mut(tid) {
                t.sort_order = Some(new_a);
                cmds.push(Command::PersistTask(t.clone()));
            }
        }
        if let Some(eid) = a_epic_id {
            if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                e.sort_order = Some(new_a);
                cmds.push(Command::PersistEpic {
                    id: eid,
                    status: None,
                    sort_order: Some(new_a),
                });
            }
        }
        if let Some(tid) = b_task_id {
            if let Some(t) = self.find_task_mut(tid) {
                t.sort_order = Some(new_b);
                cmds.push(Command::PersistTask(t.clone()));
            }
        }
        if let Some(eid) = b_epic_id {
            if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                e.sort_order = Some(new_b);
                cmds.push(Command::PersistEpic {
                    id: eid,
                    status: None,
                    sort_order: Some(new_b),
                });
            }
        }

        // Cursor follows the moved item
        self.selection_mut().set_row(col, target_row);

        cmds
    }
}
