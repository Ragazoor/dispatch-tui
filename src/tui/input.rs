mod confirm;
mod normal;
mod projects;
mod repo_filter;

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, ColumnItem, Command, InputMode, Message, MoveDirection, ViewMode};
use crate::models::{DispatchMode, EpicId, SubStatus, TaskId, TaskStatus, TaskTag, TipsShowMode};

impl App {
    /// Translate a terminal key event into zero or more commands, depending on current mode.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Command> {
        if self.status.error_popup.is_some() {
            return self.update(Message::DismissError);
        }

        // Tips overlay captures all input when visible
        if self.tips.is_some() {
            return self.handle_key_tips(key);
        }

        match self.input.mode.clone() {
            InputMode::Normal => self.handle_key_normal(key),
            InputMode::InputTitle
            | InputMode::InputDescription
            | InputMode::InputRepoPath
            | InputMode::InputEpicTitle
            | InputMode::InputEpicDescription
            | InputMode::InputEpicRepoPath
            | InputMode::InputBaseBranch => self.handle_key_text_input(key),
            InputMode::ConfirmDelete => self.handle_key_confirm_delete(key),
            InputMode::InputTag => self.handle_key_tag(key),
            InputMode::QuickDispatch => self.handle_key_quick_dispatch(key),
            InputMode::ConfirmRetry(id) => self.handle_key_confirm_retry(key, id),
            InputMode::ConfirmArchive(task_id) => self.handle_key_confirm_archive(key, task_id),
            InputMode::ConfirmDeleteEpic => self.handle_key_confirm_delete_epic(key),
            InputMode::ConfirmArchiveEpic => self.handle_key_confirm_archive_epic(key),

            InputMode::ConfirmDone(_) => self.handle_key_confirm_done(key),
            InputMode::ConfirmMergePr(_) => self.handle_key_confirm_merge_pr(key),
            InputMode::ConfirmWrapUp(_) => self.handle_key_confirm_wrap_up(key),
            InputMode::ConfirmEpicWrapUp(_) => self.handle_key_confirm_epic_wrap_up(key),
            InputMode::ConfirmDetachTmux(_) => self.handle_key_confirm_detach_tmux(key),
            InputMode::ConfirmEditTask(id) => self.handle_key_confirm_edit_task(key, id),
            InputMode::Help => self.handle_key_help(key),
            InputMode::RepoFilter => self.handle_key_repo_filter(key),
            InputMode::InputPresetName => self.handle_key_input_preset_name(key),
            InputMode::ConfirmDeletePreset => self.handle_key_confirm_delete_preset(key),
            InputMode::ConfirmDeleteRepoPath => self.handle_key_confirm_delete_repo_path(key),
            InputMode::ConfirmQuit => self.handle_key_confirm_quit(key),
            InputMode::InputProjectName { editing_id } => {
                self.handle_key_input_project_name(key, editing_id)
            }
            InputMode::ConfirmDeleteProject1 { id } => {
                self.handle_key_confirm_delete_project1(key, id)
            }
            InputMode::ConfirmDeleteProject2 { id, item_count } => {
                self.handle_key_confirm_delete_project2(key, id, item_count)
            }
        }
    }

    pub(in crate::tui) fn handle_key_tips(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('l') | KeyCode::Right => self.update(Message::NextTip),
            KeyCode::Char('h') | KeyCode::Left => self.update(Message::PrevTip),
            KeyCode::Char('n') => {
                let current_mode = self.tips.as_ref().map(|t| t.show_mode);
                let new_mode = match current_mode {
                    Some(TipsShowMode::NewOnly) => TipsShowMode::Always,
                    _ => TipsShowMode::NewOnly,
                };
                let label = match new_mode {
                    TipsShowMode::NewOnly => "Tips: will only show when there are new tips",
                    TipsShowMode::Always => "Tips: will show on every startup",
                    TipsShowMode::Never => {
                        unreachable!("n key only toggles between Always and NewOnly")
                    }
                };
                let mut cmds = self.update(Message::SetTipsMode(new_mode));
                cmds.extend(self.update(Message::StatusInfo(label.to_string())));
                cmds
            }
            KeyCode::Char('x') => {
                let mut cmds = self.update(Message::SetTipsMode(TipsShowMode::Never));
                cmds.extend(
                    self.update(Message::StatusInfo("Tips: disabled on startup".to_string())),
                );
                cmds
            }
            KeyCode::Char('q') | KeyCode::Esc => self.update(Message::CloseTips),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_task_detail(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => {
                return self.update(Message::CloseTaskDetail);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let ViewMode::TaskDetail {
                    scroll, max_scroll, ..
                } = &mut self.board.view_mode
                {
                    *scroll = scroll.saturating_add(1).min(*max_scroll);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let ViewMode::TaskDetail { scroll, .. } = &mut self.board.view_mode {
                    *scroll = scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('z') => {
                if let ViewMode::TaskDetail { zoomed, .. } = &mut self.board.view_mode {
                    *zoomed = !*zoomed;
                }
            }
            _ => {}
        }
        vec![]
    }

    /// Handle keys when the Archive column is focused.
    pub(in crate::tui) fn handle_key_archive(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let count = self.archived_tasks().len();
                if count > 0 {
                    let archive_col = TaskStatus::COLUMN_COUNT + 1;
                    let next = (self.selection().row(archive_col) + 1).min(count - 1);
                    self.selection_mut().set_row(archive_col, next);
                    *self.archive.list_state.selected_mut() = Some(next);
                }
                vec![]
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let archive_col = TaskStatus::COLUMN_COUNT + 1;
                let prev = self.selection().row(archive_col).saturating_sub(1);
                self.selection_mut().set_row(archive_col, prev);
                *self.archive.list_state.selected_mut() = Some(prev);
                vec![]
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => {
                self.update(Message::NavigateColumn(-1))
            }
            KeyCode::Char('x') => {
                let archived = self.archived_tasks();
                if let Some(task) = archived.get(self.selected_archive_row()) {
                    let title = super::truncate_title(&task.title, 30);
                    self.input.mode = InputMode::ConfirmDelete;
                    self.set_status(format!("Delete {title}? [y/n]"));
                }
                vec![]
            }
            KeyCode::Char('e') => {
                let archived = self.archived_tasks();
                if let Some(task) = archived.get(self.selected_archive_row()) {
                    let title = super::truncate_title(&task.title, 30);
                    self.input.mode = InputMode::ConfirmEditTask(task.id);
                    self.set_status(format!("Edit {title}? [y/n]"));
                }
                vec![]
            }
            KeyCode::Char('q') => self.update(Message::Quit),
            _ => vec![],
        }
    }

    /// Handle the 'd' key: dispatch, brainstorm, resume, or retry depending on item type/status.
    pub(in crate::tui) fn handle_key_dispatch(&mut self) -> Vec<Command> {
        match self.selected_column_item() {
            Some(ColumnItem::Epic(epic)) => {
                let id = epic.id;
                self.update(Message::DispatchEpic(id))
            }
            Some(ColumnItem::Task(task)) => {
                let id = task.id;
                let status = task.status;
                let has_window = task.tmux_window.is_some();
                let has_worktree = task.worktree.is_some();
                let is_problematic = self.find_task(id).is_some_and(|t| {
                    t.sub_status == SubStatus::Stale || t.sub_status == SubStatus::Crashed
                });

                match status {
                    TaskStatus::Backlog => {
                        let mode = DispatchMode::for_task(task);
                        self.update(Message::DispatchTask(id, mode))
                    }
                    TaskStatus::Running | TaskStatus::Review => {
                        if is_problematic {
                            self.update(Message::KillAndRetry(id))
                        } else if has_window {
                            self.update(Message::StatusInfo(
                                "Agent already running, press g to jump".to_string(),
                            ))
                        } else if has_worktree {
                            self.update(Message::ResumeTask(id))
                        } else {
                            self.update(Message::StatusInfo(
                                "No worktree to resume, move to Backlog and re-dispatch"
                                    .to_string(),
                            ))
                        }
                    }
                    TaskStatus::Done => {
                        self.update(Message::StatusInfo("Task is done".to_string()))
                    }
                    TaskStatus::Archived => {
                        self.update(Message::StatusInfo("Task is archived".to_string()))
                    }
                }
            }
            None => {
                if let ViewMode::Epic { epic_id, .. } = self.board.view_mode {
                    self.update(Message::DispatchEpic(epic_id))
                } else {
                    vec![]
                }
            }
        }
    }

    /// Handle 'm'/'M' key: move selected task(s) forward or backward.
    pub(in crate::tui) fn handle_key_move(&mut self, direction: MoveDirection) -> Vec<Command> {
        if self.has_selection() {
            if self.select.tasks.is_empty() {
                // Only epics selected — can't move since status is derived
                return self.update(Message::StatusInfo(
                    "Epic status is derived from subtasks".to_string(),
                ));
            }
            let ids: Vec<_> = self.select.tasks.iter().copied().collect();
            self.update(Message::BatchMoveTasks { ids, direction })
        } else if let Some(task) = self.selected_task() {
            let id = task.id;
            self.update(Message::MoveTask { id, direction })
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_key_text_input(&mut self, key: KeyEvent) -> Vec<Command> {
        // In repo path modes, j/k navigate the filtered repo list
        let is_repo_mode = matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        );
        if is_repo_mode {
            match key.code {
                KeyCode::Down => return self.update(Message::MoveRepoCursor(1)),
                KeyCode::Up => return self.update(Message::MoveRepoCursor(-1)),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc => self.update(Message::CancelInput),
            KeyCode::Enter => {
                // In repo path modes, Enter selects from the filtered list if there are matches,
                // otherwise falls through to submit the literal buffer value as a new path.
                if is_repo_mode {
                    let filtered =
                        super::filtered_repos(&self.board.repo_paths, &self.input.buffer);
                    if !filtered.is_empty() {
                        let idx = self.input.repo_cursor.min(filtered.len() - 1);
                        let path = filtered[idx].clone();
                        let msg = match self.input.mode {
                            InputMode::InputEpicRepoPath => Message::SubmitEpicRepoPath(path),
                            _ => Message::SubmitRepoPath(path),
                        };
                        return self.update(msg);
                    }
                    // No filtered matches — fall through to submit literal buffer value
                }
                let value = self.input.buffer.trim().to_string();
                match self.input.mode.clone() {
                    InputMode::InputTitle => self.update(Message::SubmitTitle(value)),
                    InputMode::InputDescription => self.update(Message::SubmitDescription(value)),
                    InputMode::InputRepoPath => self.update(Message::SubmitRepoPath(value)),
                    InputMode::InputEpicTitle => self.update(Message::SubmitEpicTitle(value)),
                    InputMode::InputEpicDescription => {
                        self.update(Message::SubmitEpicDescription(value))
                    }
                    InputMode::InputEpicRepoPath => self.update(Message::SubmitEpicRepoPath(value)),
                    InputMode::InputBaseBranch => self.update(Message::SubmitBaseBranch(value)),
                    _ => vec![],
                }
            }
            KeyCode::Backspace => self.update(Message::InputBackspace),
            KeyCode::Char(c) => self.update(Message::InputChar(c)),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_tag(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('b') => self.update(Message::SubmitTag(Some(TaskTag::Bug))),
            KeyCode::Char('f') => self.update(Message::SubmitTag(Some(TaskTag::Feature))),
            KeyCode::Char('c') => self.update(Message::SubmitTag(Some(TaskTag::Chore))),
            KeyCode::Char('e') => self.update(Message::SubmitTag(Some(TaskTag::Epic))),
            KeyCode::Enter => self.update(Message::SubmitTag(None)),
            KeyCode::Esc => self.update(Message::CancelInput),
            _ => vec![],
        }
    }

    /// Generic y/n confirm dialog: on y/Y resets mode, clears status, and runs `on_confirm`;
    /// on any other key just resets mode and clears status.
    pub(in crate::tui) fn handle_key_quick_dispatch(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Esc => self.update(Message::CancelInput),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::MoveRepoCursor(1)),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::MoveRepoCursor(-1)),
            KeyCode::Enter => {
                let idx = self.input.repo_cursor;
                self.update(Message::SelectQuickDispatchRepo(idx))
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as usize) - ('1' as usize);
                self.update(Message::SelectQuickDispatchRepo(idx))
            }
            KeyCode::Char(c) => {
                self.input.buffer.push(c);
                self.input.repo_cursor = 0;
                vec![]
            }
            KeyCode::Backspace => {
                self.input.buffer.pop();
                self.input.repo_cursor = 0;
                vec![]
            }
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_help(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc => self.update(Message::ToggleHelp),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn dispatch_selection<F, G>(
        &mut self,
        on_task: F,
        on_epic: G,
    ) -> Vec<Command>
    where
        F: FnOnce(&mut Self, TaskId) -> Vec<Command>,
        G: FnOnce(&mut Self, EpicId) -> Vec<Command>,
    {
        match self.selected_column_item() {
            Some(ColumnItem::Task(task)) => {
                let id = task.id;
                on_task(self, id)
            }
            Some(ColumnItem::Epic(epic)) => {
                let id = epic.id;
                on_epic(self, id)
            }
            None => vec![],
        }
    }

    /// Calls `f` with the selected task's ID, or returns `vec![]` if the cursor is not on a task.
    pub(in crate::tui) fn with_selected_task<F>(&mut self, f: F) -> Vec<Command>
    where
        F: FnOnce(&mut Self, TaskId) -> Vec<Command>,
    {
        if let Some(id) = self.selected_task().map(|t| t.id) {
            f(self, id)
        } else {
            vec![]
        }
    }

    /// Returns the ID of the currently selected epic, or `None` if the cursor is not on an epic.
    pub(in crate::tui) fn selected_epic_id(&self) -> Option<EpicId> {
        match self.selected_column_item() {
            Some(ColumnItem::Epic(epic)) => Some(epic.id),
            _ => None,
        }
    }
}
