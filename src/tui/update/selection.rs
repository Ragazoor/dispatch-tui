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

    pub(in crate::tui) fn handle_select_all_column(&mut self) -> Vec<Command> {
        let col = self.selection().column();
        if is_edge_column(col) {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let items = self.column_items_for_status(status);
        let mut task_ids = Vec::new();
        let mut epic_ids = Vec::new();
        for item in &items {
            match item {
                ColumnItem::Task(t) => task_ids.push(t.id),
                ColumnItem::Epic(e) => epic_ids.push(e.id),
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
            vec![Command::ToggleEpicAutoDispatch {
                id,
                auto_dispatch: new_val,
            }]
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
                self.select.pending_done = review_ids;
                let count = self.select.pending_done.len();
                self.input.mode = InputMode::ConfirmDone(self.select.pending_done[0]);
                self.set_status(format!(
                    "Move {} {} to Done? [y/n]",
                    count,
                    if count == 1 { "task" } else { "tasks" }
                ));
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
