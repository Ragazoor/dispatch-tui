//! Board navigation handlers: quit, column/row navigation, reorder.

use chrono::{DateTime, Utc};

use crate::models::{EpicId, TaskId, TaskStatus};

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
        let (min_col, max_col) = if matches!(self.effective_view_mode(), BoardViewMode::Epic { .. })
        {
            (1isize, TaskStatus::COLUMN_COUNT as isize) // [1, 4] in epic view
        } else {
            (1isize, TaskStatus::COLUMN_COUNT as isize + 1) // [1, 5] on main board
        };
        // One board scan for both the destination-column emptiness test below
        // and the closing clamp, instead of one each.
        let counts = self.column_item_counts();
        let old_col = self.selection().column();
        let new_col = (old_col as isize + delta).clamp(min_col, max_col) as usize;
        let column_changed = new_col != old_col;
        self.selection_mut().set_column(new_col);

        // Reset archive cursor when entering the archive column.
        if new_col == TaskStatus::COLUMN_COUNT + 1 {
            self.selection_mut().reset_to_top(new_col);
            *self.archive.list_state.selected_mut() = Some(0);
        } else if column_changed {
            // Always default the cursor to the first card in the destination
            // column (never the sticky row left over from a prior visit), and
            // scroll the column back to the top so the first card is visible.
            // An empty destination column is left untouched, matching the `[`/`]`
            // empty-column no-op semantics.
            let entering_nonempty = counts.get(new_col - 1).is_some_and(|&count| count > 0);
            if entering_nonempty {
                self.selection_mut().reset_to_top(new_col);
            }
        }

        self.clamp_selection_to(counts);
        self.update_anchor_from_current();
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
        self.update_anchor_from_current();
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_row_first(&mut self) -> Vec<Command> {
        let col = self.selection().column();

        if col == TaskStatus::COLUMN_COUNT + 1 {
            let count = self.archived_tasks().len();
            if count == 0 {
                return vec![];
            }
            self.selection_mut()
                .set_row(TaskStatus::COLUMN_COUNT + 1, 0);
            self.archive.list_state.select(Some(0));
            return vec![];
        }

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
        self.selection_mut().set_row(col, 0);
        self.update_anchor_from_current();
        vec![]
    }

    pub(in crate::tui) fn handle_navigate_row_last(&mut self) -> Vec<Command> {
        let col = self.selection().column();

        if col == TaskStatus::COLUMN_COUNT + 1 {
            let count = self.archived_tasks().len();
            if count == 0 {
                return vec![];
            }
            let last = count - 1;
            self.selection_mut()
                .set_row(TaskStatus::COLUMN_COUNT + 1, last);
            self.archive.list_state.select(Some(last));
            return vec![];
        }

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
        self.update_anchor_from_current();
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
        let cached = self.cached_placements();
        let items: Vec<_> = self
            .column_items_for_status_with_placements(status, cached.as_deref())
            .into_iter()
            .filter(|i| i.is_selectable())
            .collect();
        let target_row = row as isize + direction;
        if target_row < 0 || target_row >= items.len() as isize {
            return vec![];
        }
        let target_row = target_row as usize;

        // Done writes a different field from every other column. Its cards are
        // ordered by `completed_at` descending, so the swap lands there — for
        // an EPIC card too, whose own `completed_at` outranks the key derived
        // from its done subtasks (`EpicPlacement::sort_key`). See "Manual
        // reorder in Done" in `docs/specs/board-layout.allium`.
        if status == TaskStatus::Done {
            // Resolved while `items` is still borrowed from `self`, so the
            // mutating half below can take `&mut self`. Computed rather than
            // read from the cache when the cache is cold: a `None` here would
            // read as "this epic card has no completion time" and silently
            // refuse a reorder the column would have honoured.
            let placements = self.placements_or_compute(cached.as_deref());
            let resolved = [row, target_row].map(|i| done_reorder_key(&items[i], &placements));
            drop(placements);
            drop(cached);
            drop(items);
            let [Some(a), Some(b)] = resolved else {
                return vec![];
            };
            return self.reorder_in_done(col, target_row, direction, a, b);
        }

        // Get IDs and effective sort values
        let (a_task_id, a_epic_id, a_eff) = match &items[row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_key()),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_key()),
            ColumnItem::EpicHeader(_)
            | ColumnItem::SubstatusLabel(_)
            | ColumnItem::FoldedSection(_)
            | ColumnItem::OrphanSeparator => return vec![],
        };
        let (b_task_id, b_epic_id, b_eff) = match &items[target_row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_key()),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_key()),
            ColumnItem::EpicHeader(_)
            | ColumnItem::SubstatusLabel(_)
            | ColumnItem::FoldedSection(_)
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
                    crate::tui::commands::PersistFields::from_task(t),
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
                    completed_at: None,
                }));
            }
        }
        if let Some(tid) = b_task_id {
            if let Some(t) = self.find_task_mut(tid) {
                t.sort_order = Some(new_b);
                cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                    crate::tui::commands::PersistFields::from_task(t),
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
                    completed_at: None,
                }));
            }
        }

        // Cursor follows the moved item
        self.selection_mut().set_row(col, target_row);

        // sort_order changed — discard cached stats so the next render re-sorts correctly.
        self.invalidate_layout_cache();

        cmds
    }

    /// The Done column's half of [`Self::handle_reorder_item`]: swap the two
    /// cards' `completed_at` values and persist them.
    ///
    /// Done orders on `completed_at` DESCENDING, which is the only thing that
    /// differs from the generic branch — including in the equal-keys case,
    /// where moving DOWN (`direction > 0`) means an EARLIER timestamp, so the
    /// offset is subtracted rather than added.
    ///
    /// Refused when either card carries no completion time: there is nothing
    /// to swap, and persisting one anyway would write a value that leaves the
    /// card exactly where it was. After `migrate_v99_add_completed_at` no done
    /// row should be in that state.
    fn reorder_in_done(
        &mut self,
        col: usize,
        target_row: usize,
        direction: isize,
        (a_task_id, a_epic_id, a_eff): DoneReorderKey,
        (b_task_id, b_epic_id, b_eff): DoneReorderKey,
    ) -> Vec<Command> {
        let (new_a, new_b) = if a_eff == b_eff {
            let nudge = chrono::Duration::milliseconds(direction as i64);
            (a_eff - nudge, b_eff)
        } else {
            (b_eff, a_eff)
        };

        let mut cmds = vec![];
        for (task_id, epic_id, at) in [(a_task_id, a_epic_id, new_a), (b_task_id, b_epic_id, new_b)]
        {
            if let Some(tid) = task_id {
                if let Some(t) = self.find_task_mut(tid) {
                    t.completed_at = Some(at);
                    cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                        crate::tui::commands::PersistFields::from_task(t),
                    )));
                }
            }
            if let Some(eid) = epic_id {
                if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                    e.completed_at = Some(at);
                    cmds.push(Command::Epic(crate::tui::commands::EpicCommand::Persist {
                        id: eid,
                        status: None,
                        sort_order: None,
                        completed_at: Some(at),
                    }));
                }
            }
        }

        self.selection_mut().set_row(col, target_row);
        self.invalidate_layout_cache();
        cmds
    }
}

/// The card a Done-column reorder is about to move: which row it is, and the
/// completion time the column currently dates it by.
type DoneReorderKey = (Option<TaskId>, Option<EpicId>, DateTime<Utc>);

/// Resolve one Done-column card's [`DoneReorderKey`], or `None` when the card
/// carries no completion time and so has nothing to swap.
fn done_reorder_key(
    item: &ColumnItem<'_>,
    placements: &EpicPlacementMap,
) -> Option<DoneReorderKey> {
    match item {
        ColumnItem::Task(t) => Some((Some(t.id), None, t.completed_at?)),
        // An epic's key is `done_sort_key`, not its bare `completed_at`: a
        // still-running epic has none of its own and is rendered at its newest
        // done subtask's time, so that is the value the swap must reason about
        // — otherwise the write and the next render disagree.
        ColumnItem::Epic(e) => {
            let at = placements.get(&e.id)?.done_completion(e)?;
            Some((None, Some(e.id), at))
        }
        ColumnItem::EpicHeader(_)
        | ColumnItem::SubstatusLabel(_)
        | ColumnItem::FoldedSection(_)
        | ColumnItem::OrphanSeparator => None,
    }
}
