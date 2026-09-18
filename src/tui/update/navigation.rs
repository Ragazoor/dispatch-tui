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
        // Resolved once and shared by the item build and the key resolution
        // below. Both take `Option<&EpicPlacementMap>`, and a cold cache makes
        // `cached_placements()` return `None` — so asking twice would build the
        // whole map twice on one keypress and throw the first away.
        let cached = self.cached_placements();
        let placements = self.placements_or_compute(cached.as_deref());
        let items: Vec<_> = self
            .column_items_for_status_with_placements(status, Some(&placements))
            .into_iter()
            .filter(|i| i.is_selectable())
            .collect();
        let target_row = row as isize + direction;
        if target_row < 0 || target_row >= items.len() as isize {
            return vec![];
        }
        let target_row = target_row as usize;

        // Resolved while `items` is still borrowed from `self`, so the mutating
        // half below can take `&mut self`. `None` from either card means there
        // is nothing to swap — a non-card row, or a Done card the column cannot
        // date — and the reorder is refused rather than persisting a value that
        // would leave the card exactly where it was.
        let resolved = [row, target_row].map(|i| reorder_target(&items[i], status, &placements));
        drop(placements);
        drop(cached);
        drop(items);
        let [Some((a_task_id, a_epic_id, a_eff)), Some((b_task_id, b_epic_id, b_eff))] = resolved
        else {
            return vec![];
        };

        // Swap the two keys, or nudge the moved card when they tie.
        let (new_a, new_b) = if a_eff == b_eff {
            (a_eff.nudged(direction), b_eff)
        } else {
            (b_eff, a_eff)
        };

        let mut cmds = vec![];
        for (task_id, epic_id, key) in
            [(a_task_id, a_epic_id, new_a), (b_task_id, b_epic_id, new_b)]
        {
            if let Some(tid) = task_id {
                if let Some(t) = self.find_task_mut(tid) {
                    key.apply_to_task(t);
                    cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                        crate::tui::commands::PersistFields::from_task(t),
                    )));
                }
            }
            if let Some(eid) = epic_id {
                if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                    key.apply_to_epic(e);
                    cmds.push(Command::Epic(crate::tui::commands::EpicCommand::Persist {
                        id: eid,
                        status: None,
                        sort_order: key.sort_order(),
                        completed_at: key.completed_at(),
                    }));
                }
            }
        }

        // Cursor follows the moved item.
        self.selection_mut().set_row(col, target_row);
        // An ordering key changed — discard cached stats so the next render
        // re-sorts correctly.
        self.invalidate_layout_cache();

        cmds
    }
}

/// The value a manual reorder swaps between two cards.
///
/// One variant per column basis, and the whole of what differs between them:
/// every column but Done orders on `sort_order ?? id` ascending, and Done
/// orders on `completed_at` descending (`board-layout.allium`, "Done Column
/// Ordering"). The reorder must agree with the render on both counts or it
/// writes a value the next frame draws somewhere else — see
/// [`CardOrderKey`](crate::tui::types::CardOrderKey), which makes the same
/// split for the render side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReorderKey {
    Generic(i64),
    Completed(DateTime<Utc>),
}

impl ReorderKey {
    /// This key moved one unit in `direction`, for the case where two cards
    /// tie and there is no distinct value to swap.
    ///
    /// The unit and the SIGN both come off the variant: a generic column sorts
    /// ascending, so moving down (`direction > 0`) means a larger key, while
    /// Done sorts descending, so moving down means an earlier timestamp.
    fn nudged(self, direction: isize) -> Self {
        match self {
            Self::Generic(v) => Self::Generic(v + direction as i64),
            Self::Completed(at) => {
                Self::Completed(at - chrono::Duration::milliseconds(direction as i64))
            }
        }
    }

    fn apply_to_task(self, task: &mut crate::models::Task) {
        match self {
            Self::Generic(v) => task.sort_order = Some(v),
            Self::Completed(at) => task.completed_at = Some(at),
        }
    }

    fn apply_to_epic(self, epic: &mut crate::models::Epic) {
        match self {
            Self::Generic(v) => epic.sort_order = Some(v),
            Self::Completed(at) => epic.completed_at = Some(at),
        }
    }

    /// The `sort_order` this key persists, if it is that kind of key.
    fn sort_order(self) -> Option<i64> {
        match self {
            Self::Generic(v) => Some(v),
            Self::Completed(_) => None,
        }
    }

    /// The `completed_at` this key persists, if it is that kind of key.
    fn completed_at(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Generic(_) => None,
            Self::Completed(at) => Some(at),
        }
    }
}

/// The card a reorder is about to move: which row it is, and the key the
/// column currently orders it by.
type ReorderTarget = (Option<TaskId>, Option<EpicId>, ReorderKey);

/// Resolve one card's [`ReorderTarget`], or `None` when there is nothing to
/// swap — a non-card row, or a Done card carrying no completion time.
fn reorder_target(
    item: &ColumnItem<'_>,
    status: TaskStatus,
    placements: &EpicPlacementMap,
) -> Option<ReorderTarget> {
    let in_done = status == TaskStatus::Done;
    match item {
        ColumnItem::Task(t) => {
            let key = if in_done {
                ReorderKey::Completed(t.completed_at?)
            } else {
                ReorderKey::Generic(t.sort_key())
            };
            Some((Some(t.id), None, key))
        }
        ColumnItem::Epic(e) => {
            let key = if in_done {
                // An epic's Done key is `done_sort_key`, not its bare
                // `completed_at`: a still-running epic has none of its own and
                // is rendered at its newest done subtask's time, so that is the
                // value the swap must reason about — otherwise the write and
                // the next render disagree.
                ReorderKey::Completed(placements.get(&e.id)?.done_completion(e)?)
            } else {
                ReorderKey::Generic(e.sort_key())
            };
            Some((None, Some(e.id), key))
        }
        ColumnItem::EpicHeader(_)
        | ColumnItem::SubstatusLabel(_)
        | ColumnItem::FoldedSection(_)
        | ColumnItem::OrphanSeparator => None,
    }
}
