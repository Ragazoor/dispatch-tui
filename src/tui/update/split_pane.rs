//! Split pane mode handlers: toggle, swap, open/close, focus tracking.

use crate::models::TaskId;

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_toggle_split_mode(&mut self) -> Vec<Command> {
        if self.board.split.active {
            self.exit_split_if_active()
        } else if let Some(window) = self.selected_task().and_then(|t| t.tmux_window.clone()) {
            let task_id = self.selected_task().unwrap().id;
            vec![Command::EnterSplitModeWithTask { task_id, window }]
        } else {
            vec![Command::EnterSplitMode]
        }
    }

    pub(in crate::tui) fn handle_swap_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
        // Already pinned — nothing to do
        if self.board.split.pinned_task_id == Some(task_id) {
            return vec![];
        }

        let task = match self.find_task(task_id) {
            Some(t) => t,
            None => return vec![],
        };
        let new_window = match &task.tmux_window {
            Some(w) => w.clone(),
            None => {
                return self.update(Message::StatusInfo(
                    "No agent session for this task".to_string(),
                ))
            }
        };
        let old_pane_id = self.board.split.right_pane_id.clone();
        let old_window = self
            .board
            .split
            .pinned_task_id
            .and_then(|id| self.find_task(id))
            .and_then(|t| t.tmux_window.clone());
        vec![Command::SwapSplitPane {
            task_id,
            new_window,
            old_pane_id,
            old_window,
        }]
    }

    pub(in crate::tui) fn handle_split_pane_opened(
        &mut self,
        pane_id: String,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.board.split.active = true;
        self.board.split.focused = true;
        self.board.split.right_pane_id = Some(pane_id);
        self.board.split.pinned_task_id = task_id;
        vec![]
    }

    pub(in crate::tui) fn handle_focus_changed(&mut self, focused: bool) -> Vec<Command> {
        if self.board.split.active {
            self.board.split.focused = focused;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_split_pane_closed(&mut self) -> Vec<Command> {
        self.board.split.active = false;
        self.board.split.focused = true;
        self.board.split.right_pane_id = None;
        self.board.split.pinned_task_id = None;
        vec![]
    }

    /// If `task_id` is the split-pinned task, clear the pin and respawn the
    /// pane with a fresh shell.  Split mode stays active.
    pub(in crate::tui) fn maybe_respawn_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
        if self.board.split.active && self.board.split.pinned_task_id == Some(task_id) {
            self.board.split.pinned_task_id = None;
            if let Some(pane_id) = self.board.split.right_pane_id.clone() {
                return vec![Command::RespawnSplitPane { pane_id }];
            }
        }
        vec![]
    }
}
