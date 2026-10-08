//! Cursor selection: clamping, anchoring and syncing to the board.

use super::*;

impl App {
    /// Return the item (task or epic) currently under the cursor.
    ///
    /// Uses the cached `EpicStatsMap` when available (avoids the O(subtasks)
    /// clone that `column_items_for_status` incurs with `stats=None`).
    pub fn selected_column_item(&self) -> Option<ColumnItem<'_>> {
        if self.selection().on_select_all {
            return None;
        }
        let col = self.selection().column();
        if col == 0 {
            return None;
        }
        let status = TaskStatus::from_column_index(col - 1)?;
        let cached = self.cached_placements();
        let items = self.column_items_for_status_with_placements(status, cached.as_deref());
        let row = self.selection().row(col);
        items.into_iter().filter(|i| i.is_selectable()).nth(row)
    }

    /// Look up the title of an epic by ID.
    pub fn epic_title(&self, id: EpicId) -> Option<&str> {
        self.board
            .epics
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.title.as_str())
    }

    /// Return the currently selected task (if the cursor is on a task), or None
    /// if the cursor is on an epic or the column is empty.
    pub fn selected_task(&self) -> Option<&Task> {
        match self.selected_column_item() {
            Some(ColumnItem::Task(task)) => Some(task),
            _ => None,
        }
    }

    /// Clamp all selected_row values to be within bounds for each column.
    pub fn clamp_selection(&mut self) {
        // Counts first, mutation second: the search pass borrows the board, so
        // it cannot be held across `selection_mut()`. One pass for all columns.
        self.clamp_selection_to(self.column_item_counts());
    }

    /// [`Self::clamp_selection`] against counts already taken from
    /// [`Self::column_item_counts`], for a caller that needs them for its own
    /// reasons too and would otherwise scan the board a second time. Counts
    /// depend only on board data, so taking them before an unrelated selection
    /// change is equivalent to taking them after.
    pub(in crate::tui) fn clamp_selection_to(&mut self, counts: [usize; TaskStatus::COLUMN_COUNT]) {
        for (idx, &count) in counts.iter().enumerate() {
            let nav_col = idx + 1;
            let sel = self.selection_mut();
            if count == 0 {
                sel.set_row(nav_col, 0);
            } else if sel.row(nav_col) >= count {
                sel.set_row(nav_col, count - 1);
            }
        }
    }

    /// Set the selection anchor to the item currently under the cursor.
    /// Called after every navigation keystroke so that subsequent data refreshes
    /// can restore the cursor to this item.
    /// Sets anchor to None when the cursor is on the select-all header.
    ///
    /// Warms the layout cache if needed, then reads from `column_anchor_cache`
    /// in O(1).
    pub(in crate::tui) fn update_anchor_from_current(&mut self) {
        let on_select_all = self.selection().on_select_all;
        if on_select_all {
            self.selection_mut().anchor = None;
            return;
        }
        let col = self.selection().column();
        if col == 0 || col > TaskStatus::COLUMN_COUNT {
            return;
        }
        let row = self.selection().row(col);
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return;
        };

        let _ = self.cached_epic_stats(); // warms column_anchor_cache if cold
        let new_anchor = self
            .layout
            .column_anchor_cache
            .as_ref()
            .and_then(|m| m.get(&status))
            .and_then(|v| v.get(row))
            .copied();
        self.selection_mut().anchor = new_anchor;
    }

    /// Restore cursor position from the anchor after a data change.
    /// Scans all columns for the anchor item and moves the cursor to its new
    /// position (following it across columns if needed).
    /// Falls back to index clamping if the anchor is not found.
    pub fn sync_board_selection(&mut self) {
        // Board data has changed; discard stale stats and recompute below.
        self.invalidate_layout_cache();

        let anchor = match self.effective_view_mode() {
            BoardViewMode::Board(sel) | BoardViewMode::Epic { selection: sel, .. } => sel.anchor,
        };

        let Some(anchor) = anchor else {
            // on_select_all or no anchor set yet — just clamp
            return self.clamp_selection();
        };

        // Rebuild all layout caches for the fresh board state.
        let _ = self.cached_epic_stats();
        // Search for the anchor in the pre-sorted anchor cache (avoids re-sorting each column).
        let mut found: Option<(usize, usize)> = None;
        if let Some(anchor_map) = &self.layout.column_anchor_cache {
            'outer: for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
                let nav_col = idx + 1;
                if let Some(anchors) = anchor_map.get(&status) {
                    for (row, &item_anchor) in anchors.iter().enumerate() {
                        if item_anchor == anchor {
                            found = Some((nav_col, row));
                            break 'outer;
                        }
                    }
                }
            }
        }

        if let Some((found_col, found_row)) = found {
            // Clamp every column, `found_col` included — the anchor row is
            // overwritten immediately below, so clamping it first is harmless
            // and saves duplicating the clamp body here.
            self.clamp_selection();
            let sel = self.selection_mut();
            sel.set_column(found_col);
            sel.set_row(found_col, found_row);
            sel.on_select_all = false;
        } else {
            self.clamp_selection();
        }
    }

    pub(in crate::tui) fn reset_column_scroll(&mut self) {
        for state in &mut self.selection_mut().list_states {
            *state.offset_mut() = 0;
        }
    }
}
