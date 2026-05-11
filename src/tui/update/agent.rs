//! Agent lifecycle handlers: tmux output, refresh, tick, stale/crash, resume.

use std::collections::HashSet;
use std::time::Instant;

use crate::models::{SubStatus, Task, TaskId, TaskStatus};

use super::super::types::*;
use super::super::{
    App, DISPATCH_SPINNER_FRAMES, DISPATCH_WATCHDOG_TIMEOUT, PR_POLL_INTERVAL, STATUS_MESSAGE_TTL,
};

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
                    cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                        task.clone(),
                    )));
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
            vec![Command::Task(crate::tui::commands::TaskCommand::Persist(
                task_clone,
            ))]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_refresh_tasks(&mut self, new_tasks: Vec<Task>) -> Vec<Command> {
        let mut cmds = Vec::new();
        for new_task in &new_tasks {
            cmds.extend(self.detect_task_transition_notifications(new_task));
        }

        // Merge DB state into in-memory state, preserving tmux_outputs
        // Prune selections for tasks that no longer exist
        let valid_ids: HashSet<TaskId> = new_tasks.iter().map(|t| t.id).collect();
        self.select.tasks.retain(|id| valid_ids.contains(id));
        self.board.tasks = new_tasks;
        self.sync_board_selection();
        cmds
    }

    /// Splice a single fresh task into the in-memory list, replacing the row
    /// with a matching id or appending if it's a newly-created task.
    pub(in crate::tui) fn handle_task_updated(&mut self, new_task: Task) -> Vec<Command> {
        let cmds = self.detect_task_transition_notifications(&new_task);
        if let Some(slot) = self.board.tasks.iter_mut().find(|t| t.id == new_task.id) {
            *slot = new_task;
        } else {
            self.board.tasks.push(new_task);
        }
        self.sync_board_selection();
        cmds
    }

    /// Per-task transition logic shared between full and targeted refresh:
    /// fires notifications on NeedsInput / Review entry, resets stale timers
    /// on recovery, and clears notified state when the task leaves the
    /// triggering state.
    fn detect_task_transition_notifications(&mut self, new_task: &Task) -> Vec<Command> {
        let mut cmds = Vec::new();
        let old_task = self.find_task(new_task.id);
        let was_needs_input = old_task.is_some_and(|t| t.sub_status == SubStatus::NeedsInput);
        let was_review = old_task.is_some_and(|t| t.status == TaskStatus::Review);

        let was_stale_or_crashed =
            old_task.is_some_and(|t| matches!(t.sub_status, SubStatus::Stale | SubStatus::Crashed));
        let is_recovered = !matches!(
            new_task.sub_status,
            SubStatus::Stale | SubStatus::Crashed | SubStatus::Conflict
        );
        if was_stale_or_crashed && is_recovered {
            self.agents.mark_active(new_task.id);
        }

        if self.notifications_enabled {
            if new_task.sub_status == SubStatus::NeedsInput
                && !was_needs_input
                && new_task.status == TaskStatus::Running
                && !self.agents.notified_needs_input.contains(&new_task.id)
            {
                self.agents.notified_needs_input.insert(new_task.id);
                cmds.push(Command::System(
                    crate::tui::commands::SystemCommand::SendNotification {
                        title: format!("Task #{}: {}", new_task.id.0, new_task.title),
                        body: "Agent needs your input".to_string(),
                        urgent: true,
                    },
                ));
            }

            if new_task.status == TaskStatus::Review
                && !was_review
                && !self.agents.notified_review.contains(&new_task.id)
            {
                self.agents.notified_review.insert(new_task.id);
                cmds.push(Command::System(
                    crate::tui::commands::SystemCommand::SendNotification {
                        title: format!("Task #{}: {}", new_task.id.0, new_task.title),
                        body: "Ready for review".to_string(),
                        urgent: false,
                    },
                ));
            }
        }

        if new_task.status != TaskStatus::Review {
            self.agents.notified_review.remove(&new_task.id);
        }
        if new_task.sub_status != SubStatus::NeedsInput {
            self.agents.notified_needs_input.remove(&new_task.id);
        }
        cmds
    }

    pub(in crate::tui) fn handle_tick(&mut self) -> Vec<Command> {
        // Auto-clear transient status messages after 5 seconds (only in Normal
        // mode). Sticky messages (in-flight dispatch feedback) are exempt.
        if self.input.mode == InputMode::Normal && !self.status.message_sticky {
            if let Some(set_at) = self.status.message_set_at {
                if set_at.elapsed() > STATUS_MESSAGE_TTL {
                    self.clear_status();
                }
            }
        }

        if !self.dispatching.is_empty() {
            // Drop dispatching IDs whose task has been deleted from the list.
            let live_ids: HashSet<TaskId> = self.board.tasks.iter().map(|t| t.id).collect();
            let before = self.dispatching.len();
            self.dispatching.retain(|id, _| live_ids.contains(id));
            if self.dispatching.len() != before {
                self.refresh_dispatching_status();
            }

            // Watchdog: force-fail any dispatch that has exceeded the timeout.
            let timed_out: Vec<TaskId> = self
                .dispatching
                .iter()
                .filter(|(_, started)| started.elapsed() > DISPATCH_WATCHDOG_TIMEOUT)
                .map(|(id, _)| *id)
                .collect();
            for id in &timed_out {
                self.dispatching.remove(id);
            }
            if !timed_out.is_empty() {
                self.refresh_dispatching_status();
                let label = if timed_out.len() == 1 {
                    format!("Dispatch for task #{} timed out", timed_out[0].0)
                } else {
                    format!("{} dispatches timed out", timed_out.len())
                };
                self.status.error_popup = Some(label);
            }

            self.spinner_tick = (self.spinner_tick + 1) % DISPATCH_SPINNER_FRAMES;
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
                t.tmux_window.clone().map(|window| {
                    Command::Task(crate::tui::commands::TaskCommand::CaptureTmux {
                        id: t.id,
                        window,
                    })
                })
            })
            .collect();

        let now = chrono::Utc::now();
        let updates: Vec<(TaskId, SubStatus)> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Running && t.tmux_window.is_some())
            .filter(|t| !matches!(t.sub_status, SubStatus::Crashed | SubStatus::Conflict))
            .filter_map(|t| {
                let activity = crate::models::classify_agent_activity(
                    t.last_pre_tool_use_at,
                    t.last_notification_at,
                    now,
                );
                let target = activity.to_sub_status();
                (t.sub_status != target).then_some((t.id, target))
            })
            .collect();

        for (id, target) in updates {
            let cloned = self.find_task_mut(id).map(|t| {
                t.sub_status = target;
                t.clone()
            });
            if let Some(task) = cloned {
                cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                    task,
                )));
            }
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
            cmds.push(Command::Pr(crate::tui::commands::PrCommand::CheckStatus {
                id,
                pr_url,
            }));
        }

        // Check if split mode right pane still exists
        if self.board.split.active {
            if let Some(pane_id) = &self.board.split.right_pane_id {
                cmds.push(Command::CheckSplitPaneExists {
                    pane_id: pane_id.clone(),
                });
            }
        }

        cmds.push(Command::Task(
            crate::tui::commands::TaskCommand::RefreshFromDb,
        ));
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
            cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                task.clone(),
            )));
        }
        self.set_status(format!("Task {id} agent crashed - press d to retry",));

        if self.notifications_enabled {
            if let Some(task) = self.find_task(id) {
                cmds.push(Command::System(
                    crate::tui::commands::SystemCommand::SendNotification {
                        title: format!("Task #{}: {}", task.id.0, task.title),
                        body: "Agent crashed".to_string(),
                        urgent: true,
                    },
                ));
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
                vec![Command::Task(crate::tui::commands::TaskCommand::Resume {
                    task: task.clone(),
                })]
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
            // Match DispatchTask: seed last_pre_tool_use_at so the tick
            // classifier does not flip the freshly resumed task into Stale
            // before the agent emits its first PreToolUse hook. The DB write
            // is split off into SeedActivity so a later generic Persist
            // cannot clobber a hook-written stamp.
            let seed_at = chrono::Utc::now();
            task.last_pre_tool_use_at = Some(seed_at);
            let task_clone = task.clone();
            self.agents.mark_active(id);
            self.agents.last_error.remove(&id);
            self.sync_board_selection();
            self.set_status(format!("Task {id} resumed"));
            vec![
                Command::Task(crate::tui::commands::TaskCommand::Persist(task_clone)),
                Command::Task(crate::tui::commands::TaskCommand::SeedActivity { id, at: seed_at }),
            ]
        } else {
            vec![]
        }
    }
}
