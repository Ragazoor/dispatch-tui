//! Agent lifecycle handlers: tmux output, refresh, tick, stale/crash, resume.

use std::collections::HashSet;
use std::time::Instant;

use crate::models::{SubStatus, Task, TaskId, TaskStatus};

use super::super::types::*;
use super::super::{App, PR_POLL_INTERVAL, STATUS_MESSAGE_TTL};

impl App {
    pub(in crate::tui) fn handle_tmux_output(
        &mut self,
        id: TaskId,
        output: String,
        activity_ts: u64,
    ) -> Vec<Command> {
        let mut cmds = Vec::new();
        let activity_changed = self
            .agents
            .prev_tmux_activity
            .get(&id)
            .is_none_or(|&prev| prev != activity_ts);
        if activity_changed {
            self.agents.mark_active(id);
            // Recovery: reset stale/crashed sub_status when activity resumes
            let needs_recovery = self
                .find_task(id)
                .is_some_and(|t| matches!(t.sub_status, SubStatus::Stale | SubStatus::Crashed));
            if needs_recovery {
                if let Some(task) = self.find_task_mut(id) {
                    task.sub_status = SubStatus::Active;
                }
                if let Some(task) = self.find_task(id) {
                    cmds.push(Command::PersistTask(task.clone()));
                }
            }
            self.agents.prev_tmux_activity.insert(id, activity_ts);
        }
        self.agents.tmux_outputs.insert(id, output);
        cmds
    }

    pub(in crate::tui) fn handle_window_gone(&mut self, id: TaskId) -> Vec<Command> {
        // Ignore WindowGone for the split-pinned task — its window is joined as
        // a pane and isn't missing, just not a standalone window right now.
        if self.board.split.active && self.board.split.pinned_task_id == Some(id) {
            return vec![];
        }
        if let Some(task) = self.find_task(id) {
            if task.status == TaskStatus::Running {
                // Running task lost its window — likely crashed
                return self.handle_agent_crashed(id);
            }
        }
        // Non-running task: existing behavior
        if let Some(task) = self.find_task_mut(id) {
            task.tmux_window = None;
            let task_clone = task.clone();
            vec![Command::PersistTask(task_clone)]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_refresh_tasks(&mut self, new_tasks: Vec<Task>) -> Vec<Command> {
        let mut cmds = Vec::new();

        for new_task in &new_tasks {
            // Extract old state before any mutable borrows
            let old_task = self.find_task(new_task.id);
            let was_needs_input = old_task.is_some_and(|t| t.sub_status == SubStatus::NeedsInput);
            let was_review = old_task.is_some_and(|t| t.status == TaskStatus::Review);

            // Reset stale timer when a task recovers from Stale/Crashed via DB refresh
            let was_stale_or_crashed = old_task
                .is_some_and(|t| matches!(t.sub_status, SubStatus::Stale | SubStatus::Crashed));
            let is_recovered = !matches!(
                new_task.sub_status,
                SubStatus::Stale | SubStatus::Crashed | SubStatus::Conflict
            );
            if was_stale_or_crashed && is_recovered {
                self.agents.mark_active(new_task.id);
            }

            if self.notifications_enabled {
                // Detect NeedsInput transition (running tasks only)
                if new_task.sub_status == SubStatus::NeedsInput
                    && !was_needs_input
                    && new_task.status == TaskStatus::Running
                    && !self.agents.notified_needs_input.contains(&new_task.id)
                {
                    self.agents.notified_needs_input.insert(new_task.id);
                    cmds.push(Command::SendNotification {
                        title: format!("Task #{}: {}", new_task.id.0, new_task.title),
                        body: "Agent needs your input".to_string(),
                        urgent: true,
                    });
                }

                // Detect review transition (notification)
                if new_task.status == TaskStatus::Review
                    && !was_review
                    && !self.agents.notified_review.contains(&new_task.id)
                {
                    self.agents.notified_review.insert(new_task.id);
                    cmds.push(Command::SendNotification {
                        title: format!("Task #{}: {}", new_task.id.0, new_task.title),
                        body: "Ready for review".to_string(),
                        urgent: false,
                    });
                }
            }

            // Always clear notified state when task leaves the triggering state,
            // even when notifications are disabled. This prevents stale entries from
            // suppressing notifications after re-enabling.
            if new_task.status != TaskStatus::Review {
                self.agents.notified_review.remove(&new_task.id);
            }
            if new_task.sub_status != SubStatus::NeedsInput {
                self.agents.notified_needs_input.remove(&new_task.id);
            }
        }

        // Merge DB state into in-memory state, preserving tmux_outputs
        // Prune selections for tasks that no longer exist
        let valid_ids: HashSet<TaskId> = new_tasks.iter().map(|t| t.id).collect();
        self.select.tasks.retain(|id| valid_ids.contains(id));
        self.board.tasks = new_tasks;
        self.sync_board_selection();
        cmds
    }

    pub(in crate::tui) fn handle_tick(&mut self) -> Vec<Command> {
        // Auto-clear transient status messages after 5 seconds (only in Normal mode)
        if self.input.mode == InputMode::Normal {
            if let Some(set_at) = self.status.message_set_at {
                if set_at.elapsed() > STATUS_MESSAGE_TTL {
                    self.clear_status();
                }
            }
        }

        // Clear expired message flash indicators
        self.agents
            .message_flash
            .retain(|_, t| t.elapsed().as_secs() < 3);

        // Skip capturing the split-pinned task: its window has been joined as a
        // pane and is no longer visible to `has_window`, which would falsely
        // trigger WindowGone → Crashed.
        let split_pinned = self
            .board
            .split
            .pinned_task_id
            .filter(|_| self.board.split.active);

        let mut cmds: Vec<Command> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.tmux_window.is_some())
            .filter(|t| Some(t.id) != split_pinned)
            .filter_map(|t| {
                t.tmux_window
                    .clone()
                    .map(|window| Command::CaptureTmux { id: t.id, window })
            })
            .collect();

        // Check for stale agents
        let timeout = self.agents.inactivity_timeout;
        let newly_stale: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Running && t.tmux_window.is_some())
            .filter(|t| {
                !matches!(
                    t.sub_status,
                    SubStatus::Stale | SubStatus::Crashed | SubStatus::Conflict
                )
            })
            .filter(|t| {
                self.agents
                    .inactive_duration(t.id)
                    .is_some_and(|d| d > timeout)
            })
            .map(|t| t.id)
            .collect();

        for id in newly_stale {
            let stale_cmds = self.handle_stale_agent(id);
            cmds.extend(stale_cmds);
        }

        // Poll PR status for review tasks with open PRs
        let pr_tasks: Vec<(TaskId, String)> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Review)
            .filter(|t| {
                self.agents
                    .last_pr_poll
                    .get(&t.id)
                    .is_none_or(|last| last.elapsed() > PR_POLL_INTERVAL)
            })
            .filter_map(|t| t.pr_url.clone().map(|url| (t.id, url)))
            .collect();

        for (id, pr_url) in pr_tasks {
            self.agents.last_pr_poll.insert(id, Instant::now());
            cmds.push(Command::CheckPrStatus { id, pr_url });
        }

        // Check if split mode right pane still exists
        if self.board.split.active {
            if let Some(pane_id) = &self.board.split.right_pane_id {
                cmds.push(Command::CheckSplitPaneExists {
                    pane_id: pane_id.clone(),
                });
            }
        }

        cmds.push(Command::RefreshFromDb);
        cmds
    }

    pub(in crate::tui) fn handle_stale_agent(&mut self, id: TaskId) -> Vec<Command> {
        // Only applies to Running tasks
        let dominated = match self.find_task(id) {
            Some(t) if t.status == TaskStatus::Running => {
                // Escalation only: don't downgrade Crashed to Stale
                t.sub_status == SubStatus::Crashed
            }
            _ => return vec![],
        };
        if dominated {
            return vec![];
        }

        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            task.sub_status = SubStatus::Stale;
        }
        let elapsed = self
            .agents
            .inactive_duration(id)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        if let Some(task) = self.find_task(id) {
            cmds.push(Command::PersistTask(task.clone()));
        }
        self.set_status(format!(
            "Task {id} inactive for {elapsed}m - press d to retry",
        ));

        if self.notifications_enabled {
            if let Some(task) = self.find_task(id) {
                cmds.push(Command::SendNotification {
                    title: format!("Task #{}: {}", task.id.0, task.title),
                    body: format!("Agent inactive for {elapsed}m"),
                    urgent: false,
                });
            }
        }
        cmds
    }

    pub(in crate::tui) fn handle_agent_crashed(&mut self, id: TaskId) -> Vec<Command> {
        // Only applies to Running tasks
        if !self
            .find_task(id)
            .is_some_and(|t| t.status == TaskStatus::Running)
        {
            return vec![];
        }

        // Capture last tmux output as crash context
        if let Some(output) = self.agents.tmux_outputs.get(&id) {
            if !output.is_empty() {
                self.agents.last_error.insert(id, output.clone());
            }
        }

        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            task.sub_status = SubStatus::Crashed;
            task.tmux_window = None;
        }
        if let Some(task) = self.find_task(id) {
            cmds.push(Command::PersistTask(task.clone()));
        }
        self.set_status(format!("Task {id} agent crashed - press d to retry",));

        if self.notifications_enabled {
            if let Some(task) = self.find_task(id) {
                cmds.push(Command::SendNotification {
                    title: format!("Task #{}: {}", task.id.0, task.title),
                    body: "Agent crashed".to_string(),
                    urgent: true,
                });
            }
        }
        cmds
    }

    pub(in crate::tui) fn handle_resume_task(&mut self, id: TaskId) -> Vec<Command> {
        if let Some(task) = self.find_task(id) {
            if !matches!(task.status, TaskStatus::Running | TaskStatus::Review) {
                return vec![];
            }
            if task.worktree.is_some() && task.tmux_window.is_none() {
                vec![Command::Resume { task: task.clone() }]
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_resumed(
        &mut self,
        id: TaskId,
        tmux_window: String,
    ) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            task.tmux_window = Some(tmux_window);
            task.status = TaskStatus::Running;
            task.sub_status = SubStatus::Active;
            let task_clone = task.clone();
            self.agents.mark_active(id);
            self.agents.last_error.remove(&id);
            self.sync_board_selection();
            self.set_status(format!("Task {id} resumed"));
            vec![Command::PersistTask(task_clone)]
        } else {
            vec![]
        }
    }
}
