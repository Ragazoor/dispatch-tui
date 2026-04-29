//! Form input, text-entry, creation/edit/delete-flow handlers.

use crate::models::TaskTag;

use super::super::types::*;
use super::super::{filtered_repos, truncate_title, App, TITLE_DISPLAY_LENGTH};

impl App {
    pub(in crate::tui) fn handle_copy_task(&mut self) -> Vec<Command> {
        let task = match self.selected_task() {
            Some(t) => t,
            None => return vec![],
        };
        let title = format!("Copy of: {}", task.title);
        let description = task.description.clone();
        let repo_path = task.repo_path.clone();
        let tag = task.tag;
        self.input.task_draft = Some(TaskDraft {
            title,
            description,
            tag,
            ..Default::default()
        });
        self.input.buffer = repo_path;
        self.input.repo_cursor = 0;
        self.input.mode = InputMode::InputRepoPath;
        self.set_status("Enter repo path: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_start_new_task(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::InputTitle;
        self.input.buffer.clear();
        self.input.task_draft = None;
        self.set_status("Enter title: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_cancel_input(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.input.buffer.clear();
        self.input.task_draft = None;
        self.input.pending_epic_id = None;
        self.clear_status();
        vec![]
    }

    pub(in crate::tui) fn handle_confirm_delete_start(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            let title = truncate_title(&task.title, TITLE_DISPLAY_LENGTH);
            let status = task.status.as_str();
            let warning = if task.worktree.is_some() {
                " (has worktree)"
            } else {
                ""
            };
            self.input.mode = InputMode::ConfirmDelete;
            self.set_status(format!("Delete {title} [{status}]{warning}? [y/n]"));
        }
        vec![]
    }

    pub(in crate::tui) fn handle_confirm_delete_yes(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        if let Some(task) = self.selected_task() {
            let id = task.id;
            self.handle_delete_task(id)
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_cancel_delete(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    pub(in crate::tui) fn handle_submit_title(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.input.mode = InputMode::Normal;
            self.input.task_draft = None;
            self.clear_status();
        } else {
            self.input.task_draft = Some(TaskDraft {
                title: value,
                description: String::new(),
                repo_path: String::new(),
                tag: None,
                base_branch: "main".to_string(),
            });
            self.input.mode = InputMode::InputTag;
            self.set_status("Tag: [b]ug  [f]eature  [c]hore  [e]pic  [Enter] none".to_string());
        }
        vec![]
    }

    pub(in crate::tui) fn handle_submit_description(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.task_draft {
            draft.description = value;
        }
        self.input.repo_cursor = 0;
        self.input.mode = InputMode::InputRepoPath;
        self.set_status("Enter repo path: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_submit_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.set_status("Repo path required (no saved paths available)".to_string());
            return vec![];
        }
        if let Err(msg) = crate::dispatch::validate_repo_path(&value) {
            self.set_status(msg);
            return vec![];
        }
        if let Some(ref mut draft) = self.input.task_draft {
            draft.repo_path = value;
        }
        self.input.buffer = self
            .input
            .task_draft
            .as_ref()
            .map(|d| d.base_branch.clone())
            .unwrap_or_else(|| "main".to_string());
        self.input.mode = InputMode::InputBaseBranch;
        self.set_status("Base branch: ".to_string());
        vec![]
    }

    pub(in crate::tui) fn handle_submit_base_branch(&mut self, value: String) -> Vec<Command> {
        let base_branch = if value.is_empty() {
            self.input
                .task_draft
                .as_ref()
                .map(|d| d.base_branch.clone())
                .unwrap_or_else(|| "main".to_string())
        } else {
            value
        };
        if let Some(ref mut draft) = self.input.task_draft {
            draft.base_branch = base_branch;
        }
        let repo_path = self
            .input
            .task_draft
            .as_ref()
            .map(|d| d.repo_path.clone())
            .unwrap_or_default();
        self.input.buffer.clear();
        self.finish_task_creation(repo_path)
    }

    pub(in crate::tui) fn handle_submit_tag(&mut self, tag: Option<TaskTag>) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.task_draft {
            draft.tag = tag;
        }
        self.input.mode = InputMode::InputDescription;
        self.set_status("Opening editor for description...".to_string());
        vec![Command::PopOutEditor(EditKind::Description {
            is_epic: false,
        })]
    }

    pub(in crate::tui) fn handle_input_char(&mut self, c: char) -> Vec<Command> {
        let is_repo_mode = matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        );
        if is_repo_mode && c.is_ascii_digit() && c != '0' {
            let idx = (c as usize) - ('1' as usize);
            let filtered = filtered_repos(&self.board.repo_paths, &self.input.buffer);
            if idx < filtered.len() {
                let repo_path = filtered[idx].clone();
                self.input.buffer.clear();
                return match self.input.mode {
                    InputMode::InputEpicRepoPath => self.finish_epic_creation(repo_path),
                    _ => self.update(Message::SubmitRepoPath(repo_path)),
                };
            }
        }
        // Per spec: cursor resets to 0 whenever the query changes
        if matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        ) {
            self.input.repo_cursor = 0;
        }
        self.input.buffer.push(c);
        vec![]
    }

    pub(in crate::tui) fn handle_input_backspace(&mut self) -> Vec<Command> {
        // Per spec: cursor resets to 0 whenever the query changes
        if matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        ) {
            self.input.repo_cursor = 0;
        }
        self.input.buffer.pop();
        vec![]
    }

    pub(in crate::tui) fn handle_start_quick_dispatch_selection(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::QuickDispatch;
        self.input.repo_cursor = 0;
        self.input.buffer.clear();
        self.set_status(
            "Type to filter · j/k navigate · Enter select · 1-9 shortcut · Esc cancel".to_string(),
        );
        vec![]
    }

    pub(in crate::tui) fn handle_select_quick_dispatch_repo(&mut self, idx: usize) -> Vec<Command> {
        let repos = filtered_repos(&self.board.repo_paths, &self.input.buffer);
        if idx < repos.len() {
            let repo_path = repos[idx].clone();
            let epic_id = self.input.pending_epic_id.take();
            self.input.mode = InputMode::Normal;
            self.input.buffer.clear();
            self.clear_status();
            self.handle_quick_dispatch(repo_path, epic_id)
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_cancel_retry(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }
}
