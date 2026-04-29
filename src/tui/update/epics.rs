//! Epic-related message handlers: lifecycle, wrap-up, batch ops, creation flow.

use std::collections::HashSet;

use crate::dispatch;
use crate::models::{DispatchMode, Epic, EpicId, Task, TaskId, TaskStatus, TaskUsage};

use super::super::types::*;
use super::super::{truncate_title, App, TITLE_DISPLAY_LENGTH};

impl App {
    pub(in crate::tui) fn handle_start_epic_wrap_up(&mut self, epic_id: EpicId) -> Vec<Command> {
        let review_count = self
            .board
            .tasks
            .iter()
            .filter(|t| {
                t.epic_id == Some(epic_id) && t.status == TaskStatus::Review && t.worktree.is_some()
            })
            .count();

        if review_count == 0 {
            return self.update(Message::StatusInfo(
                "No review tasks to wrap up".to_string(),
            ));
        }

        self.input.mode = InputMode::ConfirmEpicWrapUp(epic_id);
        self.set_status(format!(
            "Wrap up {} review task{}: [r] rebase all  [p] PR all  [Esc] cancel",
            review_count,
            if review_count == 1 { "" } else { "s" },
        ));
        vec![]
    }

    pub(in crate::tui) fn handle_epic_wrap_up(&mut self, action: MergeAction) -> Vec<Command> {
        let epic_id = match self.input.mode {
            InputMode::ConfirmEpicWrapUp(id) => id,
            _ => return vec![],
        };
        self.input.mode = InputMode::Normal;

        let mut review_tasks: Vec<&Task> = self
            .board
            .tasks
            .iter()
            .filter(|t| {
                t.epic_id == Some(epic_id) && t.status == TaskStatus::Review && t.worktree.is_some()
            })
            .collect();
        review_tasks.sort_by_key(|t| t.sort_order.unwrap_or(t.id.0));

        let task_ids: Vec<TaskId> = review_tasks.iter().map(|t| t.id).collect();

        if task_ids.is_empty() {
            return vec![];
        }

        self.merge_queue = Some(MergeQueue {
            epic_id,
            action,
            task_ids,
            completed: 0,
            current: None,
            failed: None,
        });

        self.advance_merge_queue()
    }

    pub(in crate::tui) fn advance_merge_queue(&mut self) -> Vec<Command> {
        loop {
            let (total, next_idx, next_id, action) = match &self.merge_queue {
                Some(q) if q.completed < q.task_ids.len() => (
                    q.task_ids.len(),
                    q.completed,
                    q.task_ids[q.completed],
                    q.action.clone(),
                ),
                Some(q) => {
                    let total = q.task_ids.len();
                    self.merge_queue = None;
                    self.set_status(format!("Epic merge complete: {total}/{total} done"));
                    return vec![];
                }
                None => return vec![],
            };

            // Validate the task is still eligible
            let task_data = match self.find_task(next_id) {
                Some(t) if t.status == TaskStatus::Review => match t.worktree {
                    Some(ref worktree) => {
                        let worktree = worktree.clone();
                        let branch = dispatch::branch_from_worktree(&worktree);
                        let repo_path = t.repo_path.clone();
                        let base_branch = t.base_branch.clone();
                        let title = t.title.clone();
                        let description = t.description.clone();
                        let tmux_window = t.tmux_window.clone();
                        branch.map(|b| {
                            (
                                worktree,
                                b,
                                repo_path,
                                base_branch,
                                title,
                                description,
                                tmux_window,
                            )
                        })
                    }
                    None => None,
                },
                _ => None,
            };

            let Some((worktree, branch, repo_path, base_branch, title, description, tmux_window)) =
                task_data
            else {
                // Skip this task — no longer eligible
                if let Some(q) = &mut self.merge_queue {
                    q.completed += 1;
                }
                continue;
            };

            if let Some(q) = &mut self.merge_queue {
                q.current = Some(next_id);
            }

            self.set_status(format!(
                "Epic merge: {next_idx}/{total} done \u{2014} processing #{}",
                next_id
            ));

            return match action {
                MergeAction::Rebase => {
                    vec![Command::Finish {
                        id: next_id,
                        repo_path,
                        branch,
                        base_branch,
                        worktree,
                        tmux_window,
                    }]
                }
                MergeAction::Pr => vec![Command::CreatePr {
                    id: next_id,
                    repo_path,
                    branch,
                    base_branch,
                    title,
                    description,
                }],
            };
        }
    }

    pub(in crate::tui) fn handle_cancel_epic_wrap_up(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    pub(in crate::tui) fn handle_cancel_merge_queue(&mut self) -> Vec<Command> {
        self.merge_queue = None;
        self.set_status("Merge queue cancelled".to_string());
        vec![]
    }

    // -----------------------------------------------------------------------
    // Epic handlers
    // -----------------------------------------------------------------------

    pub(in crate::tui) fn handle_dispatch_epic(&mut self, id: EpicId) -> Vec<Command> {
        let Some(epic) = self.board.epics.iter().find(|e| e.id == id) else {
            return vec![];
        };
        let status = epic.status;

        if status != TaskStatus::Backlog {
            self.set_status("No backlog tasks in epic".to_string());
            return vec![];
        }

        if epic.plan_path.is_some() {
            // Epic has a plan — dispatch the next backlog subtask sorted by sort_order
            let mut backlog_subtasks: Vec<&Task> = self
                .board
                .tasks
                .iter()
                .filter(|t| {
                    t.epic_id == Some(id)
                        && t.status == TaskStatus::Backlog
                        && !self.dispatching.contains(&t.id)
                })
                .collect();
            backlog_subtasks.sort_by_key(|t| (t.sort_order.unwrap_or(t.id.0), t.id.0));

            match backlog_subtasks.first() {
                Some(task) => {
                    self.dispatching.insert(task.id);
                    let mode = DispatchMode::for_task(task);
                    vec![Command::DispatchAgent {
                        task: (*task).clone(),
                        mode,
                    }]
                }
                None => {
                    self.set_status("No backlog subtasks in epic".to_string());
                    vec![]
                }
            }
        } else {
            // No plan — only spawn planning subtask if epic has no active subtasks
            let has_subtasks = self
                .board
                .tasks
                .iter()
                .any(|t| t.epic_id == Some(id) && t.status != TaskStatus::Archived);
            if has_subtasks {
                self.set_status("Epic has subtasks but no plan".to_string());
                vec![]
            } else {
                vec![Command::DispatchEpic { epic: epic.clone() }]
            }
        }
    }

    pub(in crate::tui) fn handle_enter_epic(&mut self, epic_id: EpicId) -> Vec<Command> {
        let parent = Box::new(self.board.view_mode.clone());
        self.board.view_mode = ViewMode::Epic {
            epic_id,
            selection: BoardSelection::new_for_epic(),
            parent,
        };
        vec![]
    }

    pub(in crate::tui) fn handle_exit_epic(&mut self) -> Vec<Command> {
        if let ViewMode::Epic { parent, .. } = std::mem::take(&mut self.board.view_mode) {
            self.board.view_mode = *parent;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_refresh_epics(&mut self, epics: Vec<Epic>) -> Vec<Command> {
        self.board.epics = epics;
        let valid_ids: HashSet<EpicId> = self.board.epics.iter().map(|e| e.id).collect();
        self.select.epics.retain(|id| valid_ids.contains(id));
        vec![]
    }

    pub(in crate::tui) fn handle_refresh_usage(&mut self, usage: Vec<TaskUsage>) -> Vec<Command> {
        self.board.usage = usage.into_iter().map(|u| (u.task_id, u)).collect();
        vec![]
    }

    pub(in crate::tui) fn handle_epic_created(&mut self, epic: Epic) -> Vec<Command> {
        self.board.epics.push(epic);
        vec![]
    }

    pub(in crate::tui) fn handle_edit_epic(&mut self, id: EpicId) -> Vec<Command> {
        if let Some(epic) = self.board.epics.iter().find(|e| e.id == id) {
            vec![Command::PopOutEditor(EditKind::EpicEdit(epic.clone()))]
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_epic_edited(&mut self, epic: Epic) -> Vec<Command> {
        if let Some(e) = self.board.epics.iter_mut().find(|e| e.id == epic.id) {
            e.title = epic.title;
            e.description = epic.description;
            e.repo_path = epic.repo_path;
            e.updated_at = chrono::Utc::now();
        }
        vec![]
    }

    pub(in crate::tui) fn handle_delete_epic(&mut self, id: EpicId) -> Vec<Command> {
        let mut cmds = Vec::new();
        // Clean up worktrees/tmux for subtasks before deleting
        let subtask_ids: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(id))
            .map(|t| t.id)
            .collect();
        for task_id in subtask_ids {
            if let Some(task) = self.find_task_mut(task_id) {
                let cleanup = Self::take_cleanup(task);
                if let Some(c) = cleanup {
                    cmds.push(c);
                }
                self.clear_agent_tracking(task_id);
            }
        }
        self.board.epics.retain(|e| e.id != id);
        self.board.tasks.retain(|t| t.epic_id != Some(id));
        // If we were viewing this epic, exit
        if matches!(&self.board.view_mode, ViewMode::Epic { epic_id, .. } if *epic_id == id) {
            self.handle_exit_epic();
        }
        self.sync_board_selection();
        cmds.push(Command::DeleteEpic(id));
        cmds
    }

    pub(in crate::tui) fn handle_confirm_delete_epic(&mut self) -> Vec<Command> {
        if let Some(ColumnItem::Epic(epic)) = self.selected_column_item() {
            let title = truncate_title(&epic.title, TITLE_DISPLAY_LENGTH);
            self.input.mode = InputMode::ConfirmDeleteEpic;
            self.set_status(format!("Delete epic {title} and subtasks? [y/n]"));
        }
        vec![]
    }

    pub(in crate::tui) fn handle_move_epic_status(
        &mut self,
        id: EpicId,
        direction: MoveDirection,
    ) -> Vec<Command> {
        let Some(epic) = self.board.epics.iter_mut().find(|e| e.id == id) else {
            return vec![];
        };
        let new_status = match direction {
            MoveDirection::Forward => epic.status.next(),
            MoveDirection::Backward => epic.status.prev(),
        };
        if new_status == epic.status {
            return vec![];
        }
        epic.status = new_status;
        let mut cmds = vec![Command::PersistEpic {
            id,
            status: Some(new_status),
            sort_order: None,
        }];

        // Moving to Done cleans up all subtask tmux windows
        if new_status == TaskStatus::Done {
            let subtask_ids: Vec<TaskId> = self
                .board
                .tasks
                .iter()
                .filter(|t| t.epic_id == Some(id) && t.tmux_window.is_some())
                .map(|t| t.id)
                .collect();
            for task_id in subtask_ids {
                if let Some(task) = self.find_task_mut(task_id) {
                    if let Some(window) = task.tmux_window.take() {
                        cmds.push(Command::KillTmuxWindow { window });
                        cmds.push(Command::PersistTask(task.clone()));
                    }
                }
            }
        }
        self.sync_board_selection();
        cmds
    }

    pub(in crate::tui) fn handle_archive_epic(&mut self, id: EpicId) -> Vec<Command> {
        let mut cmds = Vec::new();
        let subtask_ids: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(id) && t.status != TaskStatus::Archived)
            .map(|t| t.id)
            .collect();
        for task_id in subtask_ids {
            cmds.extend(self.handle_archive_task(task_id));
        }
        self.board.epics.retain(|e| e.id != id);
        if matches!(&self.board.view_mode, ViewMode::Epic { epic_id, .. } if *epic_id == id) {
            self.handle_exit_epic();
        }
        self.sync_board_selection();
        cmds.push(Command::DeleteEpic(id));
        cmds
    }

    pub(in crate::tui) fn handle_confirm_archive_epic(&mut self) -> Vec<Command> {
        if let Some(ColumnItem::Epic(epic)) = self.selected_column_item() {
            let id = epic.id;
            let not_done_count = self
                .subtask_statuses(id)
                .iter()
                .filter(|s| **s != TaskStatus::Done)
                .count();
            if not_done_count > 0 {
                let noun = if not_done_count == 1 {
                    "subtask"
                } else {
                    "subtasks"
                };
                self.set_status(format!(
                    "Cannot archive epic: {} {} not done",
                    not_done_count, noun
                ));
                return vec![];
            }
            self.input.mode = InputMode::ConfirmArchiveEpic;
            self.set_status("Archive epic and all subtasks? [y/n]".to_string());
        }
        vec![]
    }

    pub(in crate::tui) fn handle_start_new_epic(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::InputEpicTitle;
        self.input.buffer.clear();
        let parent_epic_id = if let ViewMode::Epic { epic_id, .. } = self.board.view_mode {
            Some(epic_id)
        } else {
            None
        };
        self.input.epic_draft = Some(EpicDraft {
            parent_epic_id,
            ..Default::default()
        });
        self.set_status("Epic title: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_submit_epic_title(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.input.mode = InputMode::Normal;
            self.clear_status();
            vec![]
        } else {
            let parent_epic_id = self
                .input
                .epic_draft
                .as_ref()
                .and_then(|d| d.parent_epic_id);
            self.input.epic_draft = Some(EpicDraft {
                title: value,
                description: String::new(),
                repo_path: String::new(),
                parent_epic_id,
            });
            self.input.mode = InputMode::InputEpicDescription;
            self.set_status("Opening editor for description...".to_string());
            vec![Command::PopOutEditor(EditKind::Description {
                is_epic: true,
            })]
        }
    }

    pub(in crate::tui) fn handle_submit_epic_description(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.epic_draft {
            draft.description = value;
        }
        self.input.repo_cursor = 0;
        self.input.mode = InputMode::InputEpicRepoPath;
        self.set_status("Epic repo path: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_submit_epic_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.set_status("Repo path required".to_string());
            return vec![];
        }
        if let Err(msg) = crate::dispatch::validate_repo_path(&value) {
            self.set_status(msg);
            return vec![];
        }
        self.finish_epic_creation(value)
    }
}
