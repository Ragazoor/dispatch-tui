//! Selection state and batch operation handlers.

use crate::models::{EpicId, TaskId, TaskStatus};

use super::super::types::*;
use super::super::App;

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
            // No section under the cursor: an unsectioned column, an empty
            // column, or the select-all row. Nothing to fold.
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
            | ColumnItem::FoldedEpic(_)
            | ColumnItem::SubstatusLabel(_)
            | ColumnItem::EpicHeader(_)
            | ColumnItem::OrphanSeparator => None,
        }
    }

    fn persist_section_folds(&self) -> Vec<Command> {
        Self::persist_fold_setting(crate::tui::COLLAPSED_SECTIONS_KEY, self.folds.serialise())
    }

    /// Fold or unfold the flattened epic group the cursor is in
    /// (`docs/specs/tasks.allium`: ToggleEpicFold). Same shape as
    /// `handle_toggle_section_collapse`, over an epic group instead of a
    /// substatus section.
    pub(in crate::tui) fn handle_toggle_epic_fold(&mut self) -> Vec<Command> {
        let Some(fold_ref) = self.cursor_epic_fold_ref() else {
            // Nowhere else is an epic group to fold: a card with no epic, an
            // unflattened column, a flattened column with no header for the
            // group at all, and the select-all cursor position all leave the
            // key doing nothing.
            return vec![];
        };

        self.toggle_epic_fold(fold_ref.status, fold_ref.epic);

        let target = if self.is_epic_folded(fold_ref.status, fold_ref.epic) {
            Some(ColumnAnchor::EpicFold(fold_ref))
        } else {
            self.first_card_anchor_in_epic_group(fold_ref.status, fold_ref.epic)
        };
        if let Some(target) = target {
            self.selection_mut().anchor = Some(target);
        }
        self.sync_board_selection();
        self.persist_epic_folds()
    }

    /// The flattened epic group the cursor is in — the epic of the card under
    /// it, or the epic a folded header under it names. `None` anywhere else,
    /// including a card in a column that is not being flattened: there is no
    /// epic-header row there to fold (board-layout.allium, "Epic Folding").
    fn cursor_epic_fold_ref(&self) -> Option<EpicFoldRef> {
        match self.selected_column_item()? {
            ColumnItem::FoldedEpic(header) => Some(header.at),
            ColumnItem::Task(t) => {
                let epic_id = t.epic_id?;
                let status = TaskStatus::from_column_index(self.selection().column() - 1)?;
                if !self.is_flattened_for_status(status) {
                    return None;
                }
                self.board.epics.iter().find(|e| e.id == epic_id)?;
                Some(EpicFoldRef::new(status, epic_id))
            }
            _ => None,
        }
    }

    /// The anchor of the first card in `epic`'s flattened group within
    /// `status`, or `None` when it holds none.
    fn first_card_anchor_in_epic_group(
        &mut self,
        status: TaskStatus,
        epic: EpicId,
    ) -> Option<ColumnAnchor> {
        let cached = self.cached_placements();
        let placements = self.placements_or_compute(cached.as_deref());
        self.column_items_for_status_with_placements(status, Some(&placements))
            .into_iter()
            .find_map(|item| match item {
                ColumnItem::Task(t) if t.epic_id == Some(epic) => item.anchor(),
                _ => None,
            })
    }

    fn persist_epic_folds(&self) -> Vec<Command> {
        Self::persist_fold_setting(crate::tui::COLLAPSED_EPICS_KEY, self.epic_folds.serialise())
    }

    /// Wrap a fold state's serialised form as the settings-write command both
    /// `persist_section_folds` and `persist_epic_folds` return.
    fn persist_fold_setting(key: &str, value: String) -> Vec<Command> {
        vec![Command::Settings(
            crate::tui::commands::SettingsCommand::PersistStringSetting {
                key: key.to_string(),
                value,
            },
        )]
    }

    pub(in crate::tui) fn handle_select_all_column(&mut self) -> Vec<Command> {
        let col = self.selection().column();
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
                | ColumnItem::FoldedEpic(_)
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

    /// `tasks.allium: BatchDelete` — permanently deletes the selected tasks
    /// and epics in one operation, or nothing at all. Every task must be
    /// `done`, and every epic's whole subtree must be done (an empty subtree
    /// qualifies); one failing item refuses the whole batch, and the status
    /// bar names it ("No partial batches" in the spec's guidance).
    pub(in crate::tui) fn handle_batch_delete(&mut self) -> Vec<Command> {
        let task_ids: Vec<TaskId> = self.select.tasks.iter().copied().collect();
        let epic_ids: Vec<EpicId> = self.select.epics.iter().copied().collect();

        let bad_task = task_ids.iter().copied().find(|id| {
            self.find_task(*id)
                .is_some_and(|t| t.status != TaskStatus::Done)
        });
        if let Some(id) = bad_task {
            let title = self
                .find_task(id)
                .map(|t| crate::tui::truncate_title(&t.title, crate::tui::TITLE_DISPLAY_LENGTH))
                .unwrap_or_default();
            self.set_status(format!("Cannot delete: {title} is not done"));
            return vec![];
        }
        let bad_epic = epic_ids
            .iter()
            .copied()
            .find(|id| !self.epic_subtree_all_done(*id));
        if let Some(id) = bad_epic {
            let title = self
                .board
                .epics
                .iter()
                .find(|e| e.id == id)
                .map(|e| crate::tui::truncate_title(&e.title, crate::tui::TITLE_DISPLAY_LENGTH))
                .unwrap_or_default();
            self.set_status(format!("Cannot delete: epic {title} has unfinished work"));
            return vec![];
        }

        let mut cmds = Vec::new();
        for &id in &epic_ids {
            cmds.extend(self.teardown_epic_subtree(id));
        }
        let mut surviving_task_ids = Vec::new();
        for id in task_ids {
            // A task inside a deleted epic's subtree is already gone — the
            // epic teardown above dropped it from the board.
            if self.find_task(id).is_some() {
                cmds.extend(self.teardown_task_for_batch(id));
                surviving_task_ids.push(id);
            }
        }
        // One atomic call for the whole batch — `tasks.allium: BatchDelete`'s
        // "one operation, or nothing at all" — rather than a
        // `Delete`/`EpicCommand::Delete` issued once per item.
        cmds.push(Command::Task(
            crate::tui::commands::TaskCommand::BatchDelete {
                task_ids: surviving_task_ids,
                epic_ids,
            },
        ));
        self.select.tasks.clear();
        self.select.epics.clear();
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
}
