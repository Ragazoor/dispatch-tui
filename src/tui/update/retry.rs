//! Retry, kill-and-retry, archive task handlers.

use crate::models::{DispatchMode, SubStatus, TaskId, TaskStatus};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_kill_and_retry(&mut self, id: TaskId) -> Vec<Command> {
        self.input.mode = InputMode::ConfirmRetry(id);
        let label = if self
            .find_task(id)
            .is_some_and(|t| t.sub_status == SubStatus::Crashed)
        {
            "crashed"
        } else {
            "stale"
        };
        self.set_status(format!(
            "Agent {} - [r] Resume  [f] Fresh start  [Esc] Cancel",
            label
        ));
        vec![]
    }

    pub(in crate::tui) fn handle_retry_resume(&mut self, id: TaskId) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.clear_agent_tracking(id);

        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Running {
                return vec![];
            }
            if task.worktree.is_none() {
                self.set_status("Cannot resume: task has no worktree".to_string());
                return vec![];
            }
            task.sub_status = SubStatus::Active;
            let old_window = task.tmux_window.take();
            let task_clone = task.clone();

            let mut cmds = Vec::new();
            if let Some(window) = old_window {
                cmds.push(Command::KillTmuxWindow { window });
            }
            cmds.push(Command::Resume { task: task_clone });
            cmds.extend(self.maybe_respawn_split_pane(id));
            cmds
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_retry_fresh(&mut self, id: TaskId) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.clear_agent_tracking(id);

        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Running {
                return vec![];
            }
            let cleanup = Self::take_cleanup(task);
            task.status = TaskStatus::Backlog;
            task.sub_status = SubStatus::None;
            let task_clone = task.clone();

            let mut cmds = Vec::new();
            if let Some(c) = cleanup {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone.clone()));
            self.dispatching.insert(id);
            cmds.push(Command::DispatchAgent {
                task: task_clone,
                mode: DispatchMode::Dispatch,
            });
            cmds
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_archive_task(&mut self, id: TaskId) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            if task.status == TaskStatus::Archived {
                return vec![];
            }
            let cleanup = Self::take_cleanup(task);
            task.status = TaskStatus::Archived;
            task.sub_status = SubStatus::default_for(TaskStatus::Archived);
            let task_clone = task.clone();
            self.clear_agent_tracking(id);
            self.sync_board_selection();

            let mut cmds = Vec::new();
            if let Some(c) = cleanup {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone));
            cmds.extend(self.maybe_respawn_split_pane(id));
            cmds
        } else {
            vec![]
        }
    }
}
