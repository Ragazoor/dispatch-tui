//! Selection state and batch operation handlers.

use crate::models::{EpicId, TaskId, TaskStatus};

use super::super::types::*;
use super::super::{is_edge_column, App};

impl App {
    pub(in crate::tui) fn handle_toggle_select(&mut self, id: TaskId) -> Vec<Command> {
        if self.select.tasks.contains(&id) {
            self.select.tasks.remove(&id);
        } else {
            self.select.tasks.insert(id);
        }
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_select_epic(&mut self, id: EpicId) -> Vec<Command> {
        if self.select.epics.contains(&id) {
            self.select.epics.remove(&id);
        } else {
            self.select.epics.insert(id);
        }
        vec![]
    }

    pub(in crate::tui) fn handle_clear_selection(&mut self) -> Vec<Command> {
        self.select.tasks.clear();
        self.select.epics.clear();
        self.selection_mut().on_select_all = false;
        vec![]
    }

    /// Fold or unfold the section the cursor is in
    /// (`docs/specs/tasks.allium`: ToggleSectionCollapse).
    ///
    /// Writes the *recorded* set, so a section a live search query is holding
    /// open still toggles — the change then shows when the query clears. The
    /// cursor target is chosen here and written before reconciling, never left
    /// to `sync_board_selection`'s anchor search: after a fold the anchor names
    /// a card that is no longer rendered, that search fails, and the fallback
    /// clamp leaves an in-bounds row *numerically unchanged* — which for a card
    /// anywhere but first in its section is a card in the next section.
    pub(in crate::tui) fn handle_toggle_section_collapse(&mut self) -> Vec<Command> {
        let Some(section) = self.cursor_section() else {
            // No section under the cursor: an unsectioned column, the archive,
            // an empty column, or the select-all row. Nothing to fold.
            return vec![];
        };
        let col = self.selection().column();
        let Some(status) = TaskStatus::from_column_index(col.wrapping_sub(1)) else {
            return vec![];
        };

        self.toggle_section_collapse(status, section);

        // Where the cursor goes: onto the section's own header when folding,
        // onto its first card when unfolding. Either way it stays in the
        // section the user acted on. Resolved before the write so the anchor
        // lookup and `selection_mut()` do not overlap.
        let target = if self.is_section_collapsed(status, section) {
            Some(ColumnAnchor::Section(SectionRef::new(status, section)))
        } else {
            // `None` here means the section holds nothing, so unfolding
            // revealed no card to land on — leave the cursor to the clamp.
            self.first_card_anchor_in_section(status, section)
        };
        if let Some(target) = target {
            self.selection_mut().anchor = Some(target);
        }
        self.sync_board_selection();
        self.persist_section_folds()
    }

    /// The section the cursor is in — the section of the card under it, or the
    /// section a folded header under it names. `None` anywhere else.
    fn cursor_section(&self) -> Option<crate::models::ColumnSection> {
        let item = self.selected_column_item()?;
        match item {
            // A folded section names its own; every card is asked.
            ColumnItem::FoldedSection(header) => Some(header.at.section),
            _ => {
                // An epic card's section is per column, so the column the
                // cursor is in is part of the question.
                let status = TaskStatus::from_column_index(self.selection().column() - 1)?;
                let cached = self.cached_placements();
                self.item_section(&item, status, cached.as_deref())
            }
        }
    }

    /// The anchor of the first card in `section`, or `None` when it holds
    /// none.
    ///
    /// Asks each card its own section rather than watching for the section's
    /// header and taking whatever follows: the answer then does not depend on
    /// the builder emitting a header immediately before its cards.
    fn first_card_anchor_in_section(
        &mut self,
        status: TaskStatus,
        section: crate::models::ColumnSection,
    ) -> Option<ColumnAnchor> {
        let cached = self.cached_placements();
        let placements = match cached {
            Some(ref p) => std::sync::Arc::clone(p),
            None => std::sync::Arc::new(self.compute_epic_placements()),
        };
        self.column_items_for_status_with_placements(status, Some(&placements))
            .into_iter()
            .find(|item| self.item_section(item, status, Some(&placements)) == Some(section))
            .and_then(|item| item.anchor())
    }

    /// The section a card renders under, or `None` for anything that is not a
    /// card. One resolver for tasks and epics alike, so the two cannot answer
    /// the same question differently.
    fn item_section(
        &self,
        item: &ColumnItem<'_>,
        status: TaskStatus,
        placements: Option<&EpicPlacementMap>,
    ) -> Option<crate::models::ColumnSection> {
        match item {
            ColumnItem::Task(t) => crate::models::ColumnSection::for_task(t),
            ColumnItem::Epic(e) => self.epic_column_section(e, status, placements),
            ColumnItem::FoldedSection(_)
            | ColumnItem::SubstatusLabel(_)
            | ColumnItem::EpicHeader(_)
            | ColumnItem::OrphanSeparator => None,
        }
    }

    fn persist_section_folds(&self) -> Vec<Command> {
        vec![Command::Settings(
            crate::tui::commands::SettingsCommand::PersistStringSetting {
                key: crate::tui::COLLAPSED_SECTIONS_KEY.to_string(),
                value: self.folds.serialise(),
            },
        )]
    }

    pub(in crate::tui) fn handle_select_all_column(&mut self) -> Vec<Command> {
        let col = self.selection().column();
        if is_edge_column(col) {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let placements = self.compute_epic_placements();
        let items = self.column_items_for_status_with_placements(status, Some(&placements));
        let mut task_ids = Vec::new();
        let mut epic_ids = Vec::new();
        for item in &items {
            match item {
                ColumnItem::Task(t) => task_ids.push(t.id),
                ColumnItem::Epic(e) => epic_ids.push(e.id),
                ColumnItem::FoldedSection(_)
                | ColumnItem::EpicHeader(_)
                | ColumnItem::SubstatusLabel(_)
                | ColumnItem::OrphanSeparator => {}
            }
        }
        if task_ids.is_empty() && epic_ids.is_empty() {
            return vec![];
        }
        let all_tasks_selected = task_ids.iter().all(|id| self.select.tasks.contains(id));
        let all_epics_selected = epic_ids.iter().all(|id| self.select.epics.contains(id));
        if all_tasks_selected && all_epics_selected {
            for id in &task_ids {
                self.select.tasks.remove(id);
            }
            for id in &epic_ids {
                self.select.epics.remove(id);
            }
        } else {
            for id in task_ids {
                self.select.tasks.insert(id);
            }
            for id in epic_ids {
                self.select.epics.insert(id);
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_batch_archive_epics(&mut self, ids: Vec<EpicId>) -> Vec<Command> {
        let mut cmds = Vec::new();
        let mut skipped = 0usize;
        for id in ids {
            let not_done = self
                .subtask_statuses(id)
                .iter()
                .filter(|s| **s != TaskStatus::Done)
                .count();
            if not_done > 0 {
                skipped += 1;
                continue;
            }
            cmds.extend(self.handle_archive_epic(id));
        }
        if skipped > 0 {
            let noun = if skipped == 1 { "epic" } else { "epics" };
            self.set_status(format!("Skipped {skipped} {noun} with non-done subtasks"));
        }
        self.select.epics.clear();
        self.select.tasks.clear();
        cmds
    }

    pub(in crate::tui) fn handle_toggle_epic_auto_dispatch(&mut self, id: EpicId) -> Vec<Command> {
        if let Some(epic) = self.board.epics.iter_mut().find(|e| e.id == id) {
            let new_val = !epic.auto_dispatch;
            epic.auto_dispatch = new_val;
            vec![Command::Epic(
                crate::tui::commands::EpicCommand::ToggleAutoDispatch {
                    id,
                    auto_dispatch: new_val,
                },
            )]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_toggle_epic_group_by_repo(&mut self, id: EpicId) -> Vec<Command> {
        if let Some(epic) = self.board.epics.iter_mut().find(|e| e.id == id) {
            let new_val = !epic.group_by_repo;
            epic.group_by_repo = new_val;
            vec![Command::Epic(
                crate::tui::commands::EpicCommand::ToggleGroupByRepo {
                    id,
                    group_by_repo: new_val,
                },
            )]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_batch_move_tasks(
        &mut self,
        ids: Vec<TaskId>,
        direction: MoveDirection,
    ) -> Vec<Command> {
        if matches!(direction, MoveDirection::Forward) {
            // Review is the only status whose forward step lands on Done —
            // next() saturates, so Done.next() is Done and a literal status
            // check is what keeps already-Done tasks out of the prompt.
            let review_ids: Vec<TaskId> = ids
                .iter()
                .copied()
                .filter(|id| {
                    self.find_task(*id)
                        .is_some_and(|t| t.status == TaskStatus::Review)
                })
                .collect();

            if !review_ids.is_empty() {
                // Move non-Review tasks immediately
                let mut cmds = Vec::new();
                for id in &ids {
                    if !review_ids.contains(id) {
                        cmds.extend(self.handle_move_task(*id, direction));
                    }
                }
                // Enter confirmation for Review→Done tasks
                self.prompt_move_to_done(review_ids);
                return cmds;
            }
        }

        let mut cmds = Vec::new();
        for id in ids {
            cmds.extend(self.handle_move_task(id, direction));
        }
        self.select.tasks.clear();
        cmds
    }

    pub(in crate::tui) fn handle_batch_archive_tasks(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
        let mut cmds = Vec::new();
        for id in ids {
            cmds.extend(self.handle_archive_task(id));
        }
        self.select.tasks.clear();
        cmds
    }
}
