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
        // Column range [1, 5]: 1=Backlog, 2=Running, 3=Review, 4=Done, 5=Archive.
        // In Epic view, Archive is not shown; clamp to [1, COLUMN_COUNT].
        let (min_col, max_col) = if matches!(self.effective_view_mode(), ViewMode::Epic { .. }) {
            (1isize, TaskStatus::COLUMN_COUNT as isize) // [1, 4] in epic view
        } else {
            (1isize, TaskStatus::COLUMN_COUNT as isize + 1) // [1, 5] on main board
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
        let stats = self.compute_epic_stats();
        self.update_anchor_from_current(&stats);
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_row(&mut self, delta: isize) -> Vec<Command> {
        let col = self.selection().column();

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

        if col == 0 {
            return vec![];
        }
        let status = match TaskStatus::from_column_index(col - 1) {
            Some(s) => s,
            None => return vec![],
        };
        let count = self.column_item_count(status);

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
        let stats = self.compute_epic_stats();
        self.update_anchor_from_current(&stats);
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_row_first(&mut self) -> Vec<Command> {
        let col = self.selection().column();
        if col == 0 {
            return vec![];
        }
        if TaskStatus::from_column_index(col - 1).is_none() {
            return vec![];
        }
        self.selection_mut().on_select_all = false;
        self.selection_mut().set_row(col, 0);
        let stats = self.compute_epic_stats();
        self.update_anchor_from_current(&stats);
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_row_last(&mut self) -> Vec<Command> {
        let col = self.selection().column();
        if col == 0 {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let count = self.column_item_count(status);
        if count == 0 {
            return vec![];
        }
        self.selection_mut().on_select_all = false;
        self.selection_mut().set_row(col, count - 1);
        let stats = self.compute_epic_stats();
        self.update_anchor_from_current(&stats);
        vec![]
    }

    pub(in crate::tui) fn handle_reorder_item(&mut self, direction: isize) -> Vec<Command> {
        let col = self.selection().column();
        if col == 0 || is_edge_column(col) {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let row = self.selection().row(col);
        let stats = self.compute_epic_stats();
        let items: Vec<_> = self
            .column_items_for_status_with_stats(status, Some(&stats))
            .into_iter()
            .filter(|i| i.is_selectable())
            .collect();
        let target_row = row as isize + direction;
        if target_row < 0 || target_row >= items.len() as isize {
            return vec![];
        }
        let target_row = target_row as usize;

        // Get IDs and effective sort values
        let (a_task_id, a_epic_id, a_eff) = match &items[row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_order.unwrap_or(t.id.0)),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_order.unwrap_or(e.id.0)),
            ColumnItem::EpicHeader(_)
            | ColumnItem::SubstatusLabel(_)
            | ColumnItem::OrphanSeparator => return vec![],
        };
        let (b_task_id, b_epic_id, b_eff) = match &items[target_row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_order.unwrap_or(t.id.0)),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_order.unwrap_or(e.id.0)),
            ColumnItem::EpicHeader(_)
            | ColumnItem::SubstatusLabel(_)
            | ColumnItem::OrphanSeparator => return vec![],
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
                cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                    t.clone(),
                )));
            }
        }
        if let Some(eid) = a_epic_id {
            if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                e.sort_order = Some(new_a);
                cmds.push(Command::Epic(crate::tui::commands::EpicCommand::Persist {
                    id: eid,
                    status: None,
                    sort_order: Some(new_a),
                }));
            }
        }
        if let Some(tid) = b_task_id {
            if let Some(t) = self.find_task_mut(tid) {
                t.sort_order = Some(new_b);
                cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                    t.clone(),
                )));
            }
        }
        if let Some(eid) = b_epic_id {
            if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                e.sort_order = Some(new_b);
                cmds.push(Command::Epic(crate::tui::commands::EpicCommand::Persist {
                    id: eid,
                    status: None,
                    sort_order: Some(new_b),
                }));
            }
        }

        // Cursor follows the moved item
        self.selection_mut().set_row(col, target_row);

        cmds
    }
}
