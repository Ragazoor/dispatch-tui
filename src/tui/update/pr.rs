//! PR-related message handlers: merge flow, review state polling.

use crate::models::{ReviewDecision, SubStatus, TaskId, TaskStatus};

use super::super::types::*;
use super::super::{truncate_title, App, TITLE_DISPLAY_LENGTH};

impl App {
    pub(in crate::tui) fn handle_pr_merged(&mut self, id: TaskId) -> Vec<Command> {
        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Review {
                return cmds;
            }

            let pr_label = task
                .pr_url
                .as_deref()
                .and_then(crate::models::pr_number_from_url)
                .map_or("PR".to_string(), |n| format!("PR #{n}"));
            let title = task.title.clone();

            // Detach: kill tmux window but preserve worktree
            if let Some(window) = task.tmux_window.take() {
                cmds.push(Command::KillTmuxWindow { window });
            }
            task.status = TaskStatus::Done;
            task.sub_status = SubStatus::default_for(TaskStatus::Done);
            let task_clone = task.clone();

            self.clear_agent_tracking(id);
            self.sync_board_selection();
            self.set_status(format!(
                "{pr_label} merged \u{2014} task #{id} moved to Done"
            ));

            cmds.push(Command::PersistTask(task_clone));

            if self.notifications_enabled {
                cmds.push(Command::SendNotification {
                    title: "PR merged".to_string(),
                    body: format!("{pr_label} merged: {title}"),
                    urgent: false,
                });
            }
        }

        cmds.extend(self.maybe_respawn_split_pane(id));

        cmds
    }

    pub(in crate::tui) fn handle_start_merge_pr(&mut self, id: TaskId) -> Vec<Command> {
        let task = match self.find_task(id) {
            Some(t) => t,
            None => return vec![],
        };

        if task.status != TaskStatus::Review {
            return self.update(Message::StatusInfo("Task is not in review".to_string()));
        }
        if task.pr_url.is_none() {
            return self.update(Message::StatusInfo("Task has no PR".to_string()));
        }
        if task.sub_status != SubStatus::Approved {
            let label = match task.sub_status {
                SubStatus::AwaitingReview => "awaiting review",
                SubStatus::ChangesRequested => "changes requested",
                _ => "not approved",
            };
            return self.update(Message::StatusInfo(format!("Cannot merge: PR is {label}")));
        }

        let pr_label = task
            .pr_url
            .as_deref()
            .and_then(crate::models::pr_number_from_url)
            .map_or("PR".to_string(), |n| format!("PR #{n}"));
        let title = truncate_title(&task.title, TITLE_DISPLAY_LENGTH);

        self.input.mode = InputMode::ConfirmMergePr(id);
        self.set_status(format!("Merge {pr_label} for {title}? [y/n]"));
        vec![]
    }

    pub(in crate::tui) fn handle_confirm_merge_pr(&mut self) -> Vec<Command> {
        let id = match self.input.mode {
            InputMode::ConfirmMergePr(id) => id,
            _ => return vec![],
        };
        self.input.mode = InputMode::Normal;

        let pr_url = match self.find_task(id).and_then(|t| t.pr_url.clone()) {
            Some(url) => url,
            None => {
                self.clear_status();
                return vec![];
            }
        };

        let pr_label = crate::models::pr_number_from_url(&pr_url)
            .map_or("PR".to_string(), |n| format!("PR #{n}"));
        self.set_status(format!("Merging {pr_label}..."));
        vec![Command::MergePr { id, pr_url }]
    }

    pub(in crate::tui) fn handle_cancel_merge_pr(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    pub(in crate::tui) fn handle_merge_pr_failed(
        &mut self,
        _id: TaskId,
        error: String,
    ) -> Vec<Command> {
        self.set_status(format!("Merge failed: {error}"));
        vec![]
    }

    pub(in crate::tui) fn handle_pr_review_state(
        &mut self,
        id: TaskId,
        review_decision: Option<ReviewDecision>,
    ) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Review {
                return vec![];
            }
            // Don't overwrite attention-requiring substatuses
            if task.sub_status == SubStatus::Conflict {
                return vec![];
            }
            let new_sub = match review_decision {
                Some(ReviewDecision::Approved) => SubStatus::Approved,
                Some(ReviewDecision::ChangesRequested) => SubStatus::ChangesRequested,
                _ => SubStatus::AwaitingReview,
            };
            if task.sub_status != new_sub {
                task.sub_status = new_sub;
                let task_clone = task.clone();
                return vec![Command::PersistTask(task_clone)];
            }
        }
        vec![]
    }
}
