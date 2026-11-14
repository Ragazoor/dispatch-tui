//! Move-task-to-epic tree picker handlers (the `m` key on a task card).
//!
//! Mirrors the epic reparent flow in [`super::epics`] but targets a task: it
//! sets `task.epic_id` (or clears it for the "— no parent —" sentinel) rather
//! than reparenting an epic.

use crate::models::{EpicId, TaskId};

use super::super::types::*;
use super::super::{truncate_title, App, MoveTaskPickerState, TITLE_DISPLAY_LENGTH};

impl App {
    pub(in crate::tui) fn handle_start_move_to_epic(&mut self, task_id: TaskId) -> Vec<Command> {
        let eligible = self.view().move_task_target_epics();
        let items = crate::tui::ui::build_reparent_tree(&eligible);
        let current = self
            .board
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .and_then(|t| t.epic_id);
        let tree_state = crate::tui::ui::tree_state_at(&items, current);
        let batch: Vec<TaskId> = if self.select.tasks.is_empty() {
            vec![]
        } else {
            self.board
                .tasks
                .iter()
                .filter(|t| self.select.tasks.contains(&t.id))
                .map(|t| t.id)
                .collect()
        };
        self.interaction.move_task_picker = Some(MoveTaskPickerState {
            task_id,
            batch,
            tree_state: std::cell::RefCell::new(tree_state),
            items,
        });
        self.input.mode = InputMode::MoveTaskToEpic(task_id);
        vec![]
    }

    pub(in crate::tui) fn handle_move_to_epic_navigate(&mut self, nav: TreeNav) -> Vec<Command> {
        if let Some(picker) = &self.interaction.move_task_picker {
            crate::tui::types::apply_tree_nav(&mut picker.tree_state.borrow_mut(), nav);
            self.dirty = true;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_move_to_epic_confirm(&mut self) -> Vec<Command> {
        let task_id = match self.input.mode {
            InputMode::MoveTaskToEpic(id) => id,
            _ => return vec![],
        };

        let selected_id: Option<String> = self
            .interaction
            .move_task_picker
            .as_ref()
            .and_then(|p| p.tree_state.borrow().selected().last().cloned());

        let new_epic: Option<EpicId> = match selected_id.as_deref() {
            Some(s) if s != crate::tui::types::REPARENT_NO_PARENT_SENTINEL => s
                .strip_prefix("epic:")
                .and_then(|n| n.parse::<i64>().ok())
                .map(EpicId),
            _ => None,
        };

        let moving_title = self
            .board
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .map(|t| truncate_title(&t.title, TITLE_DISPLAY_LENGTH))
            .unwrap_or_default();

        let batch_len = self
            .interaction
            .move_task_picker
            .as_ref()
            .map_or(0, |p| p.batch.len());

        let msg = match new_epic {
            None if batch_len > 0 => {
                format!("Detach {batch_len} tasks from their epics? [y/n]")
            }
            None => format!("Detach {moving_title} from its epic? [y/n]"),
            Some(eid) => {
                let epic_label = self
                    .board
                    .epics
                    .iter()
                    .find(|e| e.id == eid)
                    .map(|e| truncate_title(&e.title, TITLE_DISPLAY_LENGTH))
                    .unwrap_or_else(|| "the epic".to_string());
                if batch_len > 0 {
                    format!("Move {batch_len} tasks to {epic_label}? [y/n]")
                } else {
                    format!("Move {moving_title} to {epic_label}? [y/n]")
                }
            }
        };

        self.input.mode = InputMode::ConfirmMoveTaskToEpic { task_id, new_epic };
        self.status.set(msg);
        vec![]
    }

    fn clear_move_task_state(&mut self) {
        self.input.mode = InputMode::Normal;
        self.interaction.move_task_picker = None;
        self.status.clear();
    }

    pub(in crate::tui) fn handle_move_to_epic_execute(&mut self) -> Vec<Command> {
        let (task_id, new_epic) = match self.input.mode {
            InputMode::ConfirmMoveTaskToEpic { task_id, new_epic } => (task_id, new_epic),
            _ => return vec![],
        };
        let ids = match self.interaction.move_task_picker.as_ref() {
            Some(p) if !p.batch.is_empty() => p.batch.clone(),
            _ => vec![task_id],
        };
        self.clear_move_task_state();
        if ids.len() > 1 || self.select.tasks.iter().any(|t| ids.contains(t)) {
            self.select.tasks.clear();
            self.dirty = true;
        }
        ids.into_iter()
            .map(|id| Command::Task(crate::tui::commands::TaskCommand::MoveToEpic { id, new_epic }))
            .collect()
    }

    /// Cancel the move flow entirely (Esc/q from the confirm prompt), returning
    /// to Normal mode and clearing the picker.
    pub(in crate::tui) fn handle_move_to_epic_cancel_all(&mut self) -> Vec<Command> {
        self.clear_move_task_state();
        vec![]
    }

    pub(in crate::tui) fn handle_move_to_epic_cancel(&mut self) -> Vec<Command> {
        match self.input.mode {
            InputMode::ConfirmMoveTaskToEpic { task_id, .. } => {
                self.input.mode = InputMode::MoveTaskToEpic(task_id);
                self.status.clear();
            }
            InputMode::MoveTaskToEpic(_) => {
                self.input.mode = InputMode::Normal;
                self.interaction.move_task_picker = None;
            }
            _ => {}
        }
        vec![]
    }
}
