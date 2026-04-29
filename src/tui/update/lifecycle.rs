//! Task lifecycle handlers: move, dispatch, create, delete, detail, flatten, done.

use crate::models::{DispatchMode, SubStatus, Task, TaskId, TaskStatus};

use super::super::types::*;
use super::super::{truncate_title, App, TITLE_DISPLAY_LENGTH};

impl App {
    pub(in crate::tui) fn handle_move_task(
        &mut self,
        id: TaskId,
        direction: MoveDirection,
    ) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            let new_status = match direction {
                MoveDirection::Forward => task.status.next(),
                MoveDirection::Backward => task.status.prev(),
            };
            if new_status == task.status {
                return vec![];
            }

            // Confirm before moving to Done
            if new_status == TaskStatus::Done {
                let title = truncate_title(&task.title, TITLE_DISPLAY_LENGTH);
                self.input.mode = InputMode::ConfirmDone(id);
                self.set_status(format!("Move {title} to Done? [y/n]"));
                return vec![];
            }

            // Kill tmux window when moving backward, but keep worktree for resume
            let detach = if matches!(direction, MoveDirection::Backward) {
                Self::take_detach(task)
            } else {
                None
            };

            task.status = new_status;
            task.sub_status = SubStatus::default_for(new_status);
            let task_clone = task.clone();
            self.clear_agent_tracking(id);
            self.sync_board_selection();

            let mut cmds = Vec::new();
            if let Some(c) = detach {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone));
            cmds
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_confirm_done(&mut self) -> Vec<Command> {
        let ids = if !self.select.pending_done.is_empty() {
            std::mem::take(&mut self.select.pending_done)
        } else {
            match self.input.mode {
                InputMode::ConfirmDone(id) => vec![id],
                _ => return vec![],
            }
        };
        self.input.mode = InputMode::Normal;
        self.clear_status();

        let mut cmds = Vec::new();
        for id in ids {
            if let Some(task) = self.find_task_mut(id) {
                if task.status != TaskStatus::Review {
                    continue;
                }
                let detach = Self::take_detach(task);
                task.status = TaskStatus::Done;
                task.sub_status = SubStatus::default_for(TaskStatus::Done);
                let task_clone = task.clone();
                self.clear_agent_tracking(id);
                if let Some(c) = detach {
                    cmds.push(c);
                }
                cmds.push(Command::PersistTask(task_clone));
                cmds.extend(self.maybe_respawn_split_pane(id));
            }
        }
        self.select.tasks.clear();
        self.sync_board_selection();
        cmds
    }

    pub(in crate::tui) fn handle_cancel_done(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.select.pending_done.clear();
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_notifications(&mut self) -> Vec<Command> {
        self.notifications_enabled = !self.notifications_enabled;
        let label = if self.notifications_enabled {
            "Notifications enabled"
        } else {
            "Notifications disabled"
        };
        self.set_status(label.to_string());
        vec![Command::PersistSetting {
            key: "notifications_enabled".to_string(),
            value: self.notifications_enabled,
        }]
    }

    pub(in crate::tui) fn handle_dispatch_task(
        &mut self,
        id: TaskId,
        mode: DispatchMode,
    ) -> Vec<Command> {
        if self.dispatching.contains(&id) {
            return vec![];
        }
        let task = self
            .find_task(id)
            .filter(|t| t.status == TaskStatus::Backlog)
            .cloned();
        if let Some(task) = task {
            self.dispatching.insert(id);
            return vec![Command::DispatchAgent { task, mode }];
        }
        vec![]
    }

    pub(in crate::tui) fn handle_dispatched(
        &mut self,
        id: TaskId,
        worktree: String,
        tmux_window: String,
        switch_focus: bool,
    ) -> Vec<Command> {
        self.dispatching.remove(&id);
        if let Some(task) = self.find_task_mut(id) {
            task.worktree = Some(worktree);
            task.tmux_window = Some(tmux_window.clone());
            task.status = TaskStatus::Running;
            task.sub_status = SubStatus::default_for(TaskStatus::Running);
            let task_clone = task.clone();
            self.agents.mark_active(id);
            self.sync_board_selection();
            let mut cmds = vec![Command::PersistTask(task_clone)];
            if switch_focus {
                cmds.push(Command::JumpToTmux {
                    window: tmux_window,
                });
            }
            cmds
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_task_created(&mut self, task: Task) -> Vec<Command> {
        self.board.tasks.push(task);
        self.sync_board_selection();
        vec![]
    }

    pub(in crate::tui) fn handle_delete_task(&mut self, id: TaskId) -> Vec<Command> {
        let cleanup = self.find_task_mut(id).and_then(Self::take_cleanup);
        self.clear_agent_tracking(id);
        self.board.tasks.retain(|t| t.id != id);
        self.sync_board_selection();
        let archive_col = TaskStatus::COLUMN_COUNT + 1;
        let archive_count = self.archived_tasks().len();
        if archive_count > 0 && self.selection().row(archive_col) >= archive_count {
            self.selection_mut().set_row(archive_col, archive_count - 1);
        }
        *self.archive.list_state.selected_mut() = Some(self.selection().row(archive_col));
        let mut cmds = Vec::new();
        if let Some(c) = cleanup {
            cmds.push(c);
        }
        cmds.push(Command::DeleteTask(id));
        cmds
    }

    pub(in crate::tui) fn handle_open_task_detail(&mut self, task_id: i64) -> Vec<Command> {
        let previous = Box::new(self.board.view_mode.clone());
        self.board.view_mode = ViewMode::TaskDetail {
            task_id,
            scroll: 0,
            zoomed: false,
            max_scroll: 0,
            previous,
        };
        vec![]
    }

    pub(in crate::tui) fn handle_close_task_detail(&mut self) -> Vec<Command> {
        if let ViewMode::TaskDetail { previous, .. } = std::mem::take(&mut self.board.view_mode) {
            self.board.view_mode = *previous;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_toggle_flattened(&mut self) -> Vec<Command> {
        self.board.flattened = !self.board.flattened;
        // Column item counts change when toggling (epics hidden / shown, and
        // tasks from the subtree merged in / split out), so selection row
        // indices may be out of bounds. Sync to follow the anchor.
        self.sync_board_selection();
        vec![]
    }
}
