use crossterm::event::{KeyCode, KeyEvent};

use super::{App, ColumnItem, Command, EditKind, InputMode, Message, MoveDirection, ViewMode};
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

    fn handle_key_tips(&mut self, key: KeyEvent) -> Vec<Command> {
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

    fn handle_key_task_detail(&mut self, key: KeyEvent) -> Vec<Command> {
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

    fn handle_key_proposed_learnings(&mut self, key: KeyEvent) -> Vec<Command> {
        let selected_id = if let ViewMode::ProposedLearnings { selected, ref learnings, .. } =
            self.board.view_mode
        {
            learnings.get(selected).map(|l| l.id)
        } else {
            return vec![];
        };

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.update(Message::CloseProposedLearnings),
            KeyCode::Char('j') | KeyCode::Down => {
                self.update(Message::NavigateProposedLearning(1))
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.update(Message::NavigateProposedLearning(-1))
            }
            KeyCode::Char('a') => {
                if let Some(id) = selected_id {
                    self.update(Message::ApproveLearning(id))
                } else {
                    vec![]
                }
            }
            KeyCode::Char('r') => {
                if let Some(id) = selected_id {
                    self.update(Message::RejectLearning(id))
                } else {
                    vec![]
                }
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected_id {
                    self.update(Message::EditLearning(id))
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn handle_key_normal(&mut self, key: KeyEvent) -> Vec<Command> {
        // TaskDetail overlay captures all input when visible
        if matches!(self.board.view_mode, ViewMode::TaskDetail { .. }) {
            return self.handle_key_task_detail(key);
        }

        // ProposedLearnings overlay captures all input when visible
        if matches!(self.board.view_mode, ViewMode::ProposedLearnings { .. }) {
            return self.handle_key_proposed_learnings(key);
        }

        if self.show_archived() {
            return self.handle_key_archive(key);
        }

        // Projects panel intercepts all input when visible (except in Epic view).
        if self.projects_panel_visible() {
            return self.handle_key_projects_panel(key);
        }

        match key.code {
            KeyCode::Char('q') => {
                if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
                    self.update(Message::ExitEpic)
                } else {
                    self.selection_mut().set_column(0);
                    self.clamp_selection();
                    self.update_anchor_from_current();
                    vec![]
                }
            }

            KeyCode::Char('h') | KeyCode::Left => self.update(Message::NavigateColumn(-1)),
            KeyCode::Char('l') | KeyCode::Right => self.update(Message::NavigateColumn(1)),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::NavigateRow(1)),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::NavigateRow(-1)),
            KeyCode::Char('J') => self.update(Message::ReorderItem(1)),
            KeyCode::Char('K') => self.update(Message::ReorderItem(-1)),

            KeyCode::Char('n') => self.update(Message::StartNewTask),
            KeyCode::Char('c') => self.update(Message::CopyTask),
            KeyCode::Char('N') => self.update(Message::ToggleNotifications),
            KeyCode::Char('E') => self.update(Message::StartNewEpic),
            KeyCode::Char('d') => self.handle_key_dispatch(),
            KeyCode::Char('f') => self.update(Message::StartRepoFilter),
            KeyCode::Char('W') => self.dispatch_selection(
                |s, id| s.update(Message::StartWrapUp(id)),
                |s, id| s.update(Message::StartEpicWrapUp(id)),
            ),
            KeyCode::Char('L') => {
                if let Some(id) = self.selected_epic_id() {
                    return self.update(Message::MoveEpicStatus(id, MoveDirection::Forward));
                }
                self.handle_key_move(MoveDirection::Forward)
            }
            KeyCode::Char('H') => {
                if let Some(id) = self.selected_epic_id() {
                    return self.update(Message::MoveEpicStatus(id, MoveDirection::Backward));
                }
                self.handle_key_move(MoveDirection::Backward)
            }

            KeyCode::Char('g') => {
                if let Some(task) = self.selected_task() {
                    // If the task's window is pinned in the split pane, it no longer
                    // exists as a standalone window — focus the pane directly instead.
                    if self.board.split.active && self.board.split.pinned_task_id == Some(task.id) {
                        if let Some(pane_id) = self.board.split.right_pane_id.clone() {
                            return vec![Command::FocusSplitPane { pane_id }];
                        }
                    }
                    if let Some(window) = &task.tmux_window {
                        vec![Command::JumpToTmux {
                            window: window.clone(),
                        }]
                    } else {
                        self.update(Message::StatusInfo("No active session".to_string()))
                    }
                } else if let Some(id) = self.selected_epic_id() {
                    self.update(Message::EnterEpic(id))
                } else {
                    vec![]
                }
            }

            KeyCode::Char('G') => {
                if let Some(task) = self.selected_task() {
                    if self.board.split.active {
                        let id = task.id;
                        self.update(Message::SwapSplitPane(id))
                    } else {
                        vec![]
                    }
                } else if let Some(id) = self.selected_epic_id() {
                    // Prefer blocked Running subtasks, then Review, by sort_order
                    let window = self
                        .board
                        .tasks
                        .iter()
                        .filter(|t| {
                            t.epic_id == Some(id)
                                && t.status == TaskStatus::Running
                                && t.sub_status != SubStatus::Active
                                && t.tmux_window.is_some()
                        })
                        .min_by_key(|t| (t.sort_order.unwrap_or(t.id.0), t.id.0))
                        .or_else(|| {
                            self.board
                                .tasks
                                .iter()
                                .filter(|t| {
                                    t.epic_id == Some(id)
                                        && t.status == TaskStatus::Review
                                        && t.tmux_window.is_some()
                                })
                                .min_by_key(|t| (t.sort_order.unwrap_or(t.id.0), t.id.0))
                        })
                        .and_then(|t| t.tmux_window.clone());

                    if let Some(window) = window {
                        vec![Command::JumpToTmux { window }]
                    } else {
                        self.update(Message::StatusInfo("No active subtask session".to_string()))
                    }
                } else {
                    vec![]
                }
            }

            KeyCode::Char('p') => {
                if let Some(task) = self.selected_task() {
                    if let Some(url) = &task.pr_url {
                        vec![Command::OpenInBrowser { url: url.clone() }]
                    } else {
                        self.update(Message::StatusInfo("No PR URL".to_string()))
                    }
                } else {
                    vec![]
                }
            }
            KeyCode::Char('P') => {
                self.with_selected_task(|s, id| s.update(Message::StartMergePr(id)))
            }

            KeyCode::Char('a') => self.update(Message::SelectAllColumn),

            KeyCode::Char(' ') => self.dispatch_selection(
                |s, id| s.update(Message::ToggleSelect(id)),
                |s, id| s.update(Message::ToggleSelectEpic(id)),
            ),

            KeyCode::Enter => {
                if self.selection().on_select_all {
                    return self.update(Message::SelectAllColumn);
                }
                if let Some(task) = self.selected_task() {
                    let id = task.id.0;
                    return self.update(Message::OpenTaskDetail(id));
                }
                vec![]
            }

            KeyCode::Char('e') => match self.selected_column_item() {
                Some(ColumnItem::Task(task)) => {
                    let title = super::truncate_title(&task.title, 30);
                    self.input.mode = InputMode::ConfirmEditTask(task.id);
                    self.set_status(format!("Edit {title}? [y/n]"));
                    vec![]
                }
                Some(ColumnItem::Epic(epic)) => {
                    let id = epic.id;
                    self.update(Message::EditEpic(id))
                }
                None => {
                    if let ViewMode::Epic { epic_id, .. } = &self.board.view_mode {
                        let id = *epic_id;
                        self.update(Message::EditEpic(id))
                    } else {
                        vec![]
                    }
                }
            },

            KeyCode::Char('x') => {
                if self.has_selection() {
                    let count = self.select.tasks.len() + self.select.epics.len();
                    self.input.mode = InputMode::ConfirmArchive(None);
                    self.set_status(format!("Archive {} items? [y/n]", count));
                    vec![]
                } else {
                    match self.selected_column_item() {
                        Some(ColumnItem::Epic(_)) => self.update(Message::ConfirmArchiveEpic),
                        _ => {
                            if let Some(task) = self.selected_task() {
                                let id = task.id;
                                self.input.mode = InputMode::ConfirmArchive(Some(id));
                                self.set_status("Archive task? [y/n]".to_string());
                            }
                            vec![]
                        }
                    }
                }
            }

            KeyCode::Char('D') => {
                let epic_id = if let ViewMode::Epic { epic_id, .. } = &self.board.view_mode {
                    Some(*epic_id)
                } else {
                    None
                };
                self.input.pending_epic_id = epic_id;
                match self.board.repo_paths.len() {
                    0 => self.update(Message::StatusInfo(
                        "No saved repo paths — create a task first".to_string(),
                    )),
                    1 => {
                        let repo_path = self.board.repo_paths[0].clone();
                        self.update(Message::QuickDispatch { repo_path, epic_id })
                    }
                    _ => self.update(Message::StartQuickDispatchSelection),
                }
            }

            KeyCode::Char('U') => {
                if let ViewMode::Epic { epic_id, .. } = &self.board.view_mode {
                    let id = *epic_id;
                    self.update(Message::ToggleEpicAutoDispatch(id))
                } else {
                    vec![]
                }
            }

            KeyCode::Char('F') => self.update(Message::ToggleFlattened),

            KeyCode::Char('I') => self.update(Message::OpenProposedLearnings),

            KeyCode::Char('?') => self.update(Message::ToggleHelp),

            KeyCode::Char('S') => self.update(Message::ToggleSplitMode),

            KeyCode::Char('T') => {
                if !self.select.tasks.is_empty() {
                    let ids: Vec<_> = self.select.tasks.iter().copied().collect();
                    self.update(Message::BatchDetachTmux(ids))
                } else if let Some(task) = self.selected_task() {
                    if task.status == TaskStatus::Review && task.tmux_window.is_some() {
                        let id = task.id;
                        self.update(Message::DetachTmux(id))
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            }

            KeyCode::Char('r') => {
                let feed_epic_id = match self.selected_column_item() {
                    Some(ColumnItem::Epic(e)) if e.feed_command.is_some() => Some(e.id),
                    _ => None,
                }
                .or_else(|| {
                    if let ViewMode::Epic { epic_id, .. } = &self.board.view_mode {
                        let id = *epic_id;
                        self.find_epic(id)
                            .filter(|e| e.feed_command.is_some())
                            .map(|e| e.id)
                    } else {
                        None
                    }
                });
                if let Some(id) = feed_epic_id {
                    self.update(Message::TriggerEpicFeed(id))
                } else {
                    vec![]
                }
            }

            KeyCode::Esc => {
                if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
                    self.update(Message::ExitEpic)
                } else if self.has_selection() || self.selection().on_select_all {
                    self.update(Message::ClearSelection)
                } else {
                    vec![]
                }
            }

            _ => vec![],
        }
    }

    /// Handle keys when the Archive column is focused.
    fn handle_key_archive(&mut self, key: KeyEvent) -> Vec<Command> {
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
    fn handle_key_dispatch(&mut self) -> Vec<Command> {
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
    fn handle_key_move(&mut self, direction: MoveDirection) -> Vec<Command> {
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

    fn handle_key_text_input(&mut self, key: KeyEvent) -> Vec<Command> {
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

    fn handle_key_tag(&mut self, key: KeyEvent) -> Vec<Command> {
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
    fn confirm_dialog(
        &mut self,
        key: KeyEvent,
        on_confirm: impl FnOnce(&mut Self) -> Vec<Command>,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.input.mode = InputMode::Normal;
                self.clear_status();
                on_confirm(self)
            }
            _ => {
                self.input.mode = InputMode::Normal;
                self.clear_status();
                vec![]
            }
        }
    }

    fn handle_key_confirm_quit(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            s.should_quit = true;
            s.exit_split_if_active()
        })
    }

    fn handle_key_confirm_delete(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if s.show_archived() {
                s.confirm_delete_archived()
            } else {
                s.confirm_delete_selected()
            }
        })
    }

    fn confirm_delete_archived(&mut self) -> Vec<Command> {
        self.archived_tasks()
            .get(self.selected_archive_row())
            .map(|t| t.id)
            .map(|id| self.update(Message::DeleteTask(id)))
            .unwrap_or_default()
    }

    fn confirm_delete_selected(&mut self) -> Vec<Command> {
        self.selected_task()
            .map(|t| t.id)
            .map(|id| self.update(Message::DeleteTask(id)))
            .unwrap_or_default()
    }

    fn handle_key_quick_dispatch(&mut self, key: KeyEvent) -> Vec<Command> {
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

    fn handle_key_confirm_retry(&mut self, key: KeyEvent, id: TaskId) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.update(Message::RetryResume(id)),
            KeyCode::Char('f') => self.update(Message::RetryFresh(id)),
            KeyCode::Esc => self.update(Message::CancelRetry),
            _ => vec![],
        }
    }

    fn handle_key_confirm_archive(
        &mut self,
        key: KeyEvent,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if s.has_selection() {
                let mut cmds = Vec::new();
                if !s.select.tasks.is_empty() {
                    let ids: Vec<_> = s.select.tasks.iter().copied().collect();
                    cmds.extend(s.update(Message::BatchArchiveTasks(ids)));
                }
                if !s.select.epics.is_empty() {
                    let ids: Vec<_> = s.select.epics.iter().copied().collect();
                    cmds.extend(s.update(Message::BatchArchiveEpics(ids)));
                }
                cmds
            } else if let Some(id) = task_id {
                s.update(Message::ArchiveTask(id))
            } else {
                vec![]
            }
        })
    }

    fn handle_key_confirm_done(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.update(Message::ConfirmDone),
            _ => self.update(Message::CancelDone),
        }
    }

    fn handle_key_confirm_merge_pr(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.update(Message::ConfirmMergePr),
            _ => self.update(Message::CancelMergePr),
        }
    }

    fn handle_key_confirm_delete_epic(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if let Some(id) = s.selected_epic_id() {
                s.update(Message::DeleteEpic(id))
            } else {
                vec![]
            }
        })
    }

    fn handle_key_confirm_archive_epic(&mut self, key: KeyEvent) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if let Some(id) = s.selected_epic_id() {
                s.update(Message::ArchiveEpic(id))
            } else {
                vec![]
            }
        })
    }

    fn handle_key_help(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc => self.update(Message::ToggleHelp),
            _ => vec![],
        }
    }

    fn handle_key_repo_filter(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                self.update(Message::CloseRepoFilter)
            }
            KeyCode::Char('a') => self.update(Message::ToggleAllRepoFilter),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::MoveRepoCursor(1)),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::MoveRepoCursor(-1)),
            KeyCode::Char(' ') => {
                let idx = self.input.repo_cursor;
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::ToggleRepoFilter(path))
                } else {
                    vec![]
                }
            }
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::ToggleRepoFilter(path))
                } else {
                    vec![]
                }
            }
            KeyCode::Backspace | KeyCode::Delete => self.update(Message::StartDeleteRepoPath),
            KeyCode::Char('s') => self.update(Message::StartSavePreset),
            KeyCode::Char('x') => self.update(Message::StartDeletePreset),
            KeyCode::Char(c @ 'A'..='Z') => {
                let idx = (c as usize) - ('A' as usize);
                if idx < self.filter.presets.len() {
                    let name = self.filter.presets[idx].0.clone();
                    self.update(Message::LoadFilterPreset(name))
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn handle_key_input_preset_name(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Enter => {
                let name = self.input.buffer.clone();
                self.update(Message::SaveFilterPreset(name))
            }
            KeyCode::Esc => self.update(Message::CancelPresetInput),
            KeyCode::Backspace => self.update(Message::InputBackspace),
            KeyCode::Char(c) => self.update(Message::InputChar(c)),
            _ => vec![],
        }
    }

    fn handle_key_confirm_delete_preset(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char(c @ 'A'..='Z') => {
                let idx = (c as usize) - ('A' as usize);
                if idx < self.filter.presets.len() {
                    let name = self.filter.presets[idx].0.clone();
                    self.update(Message::DeleteFilterPreset(name))
                } else {
                    vec![]
                }
            }
            KeyCode::Esc => self.update(Message::CancelPresetInput),
            _ => vec![],
        }
    }

    fn handle_key_confirm_delete_repo_path(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let idx = self.input.repo_cursor;
                if idx < self.board.repo_paths.len() {
                    let path = self.board.repo_paths[idx].clone();
                    self.update(Message::DeleteRepoPath(path))
                } else {
                    self.input.mode = InputMode::RepoFilter;
                    vec![]
                }
            }
            _ => {
                self.input.mode = InputMode::RepoFilter;
                vec![]
            }
        }
    }

    fn handle_key_confirm_detach_tmux(&mut self, key: KeyEvent) -> Vec<Command> {
        let ids = match &self.input.mode {
            InputMode::ConfirmDetachTmux(ids) => ids.clone(),
            _ => return vec![],
        };
        self.confirm_dialog(key, |s| s.detach_tmux_panels(ids))
    }

    fn handle_key_confirm_edit_task(&mut self, key: KeyEvent, id: TaskId) -> Vec<Command> {
        self.confirm_dialog(key, |s| {
            if let Some(task) = s.board.tasks.iter().find(|t| t.id == id) {
                vec![Command::PopOutEditor(EditKind::TaskEdit(task.clone()))]
            } else {
                vec![]
            }
        })
    }

    fn handle_key_confirm_wrap_up(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.update(Message::WrapUpRebase),
            KeyCode::Char('p') => self.update(Message::WrapUpPr),
            KeyCode::Esc => self.update(Message::CancelWrapUp),
            _ => vec![],
        }
    }

    fn handle_key_confirm_epic_wrap_up(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('r') => self.update(Message::EpicWrapUpRebase),
            KeyCode::Char('p') => self.update(Message::EpicWrapUpPr),
            KeyCode::Esc => self.update(Message::CancelEpicWrapUp),
            _ => vec![],
        }
    }

    // -----------------------------------------------------------------------
    // Projects panel input handlers
    // -----------------------------------------------------------------------

    fn handle_key_projects_panel(&mut self, key: KeyEvent) -> Vec<Command> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let len = self.board.projects.len();
                if len > 0 {
                    let next = (self.selection().row(0) + 1).min(len - 1);
                    if next == self.selection().row(0) {
                        return vec![];
                    }
                    self.selection_mut().set_row(0, next);
                    self.projects_panel.list_state.select(Some(next));
                    if let Some(id) = self.selected_project().map(|p| p.id) {
                        return self.update(Message::SelectProject(id));
                    }
                }
                vec![]
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let prev = self.selection().row(0).saturating_sub(1);
                if prev == self.selection().row(0) {
                    return vec![];
                }
                self.selection_mut().set_row(0, prev);
                self.projects_panel.list_state.select(Some(prev));
                if let Some(id) = self.selected_project().map(|p| p.id) {
                    return self.update(Message::SelectProject(id));
                }
                vec![]
            }
            KeyCode::Char('l')
            | KeyCode::Right
            | KeyCode::Enter
            | KeyCode::Char('g')
            | KeyCode::Esc => self.update(Message::NavigateColumn(1)),
            KeyCode::Char('n') => {
                self.input.mode = InputMode::InputProjectName { editing_id: None };
                self.input.buffer.clear();
                vec![]
            }
            KeyCode::Char('r') => {
                if let Some(project) = self.selected_project().cloned() {
                    self.input.mode = InputMode::InputProjectName {
                        editing_id: Some(project.id),
                    };
                    self.input.buffer = project.name;
                }
                vec![]
            }
            KeyCode::Char('d') => {
                if let Some(project) = self.selected_project().cloned() {
                    if !project.is_default {
                        self.input.mode = InputMode::ConfirmDeleteProject1 { id: project.id };
                    } else {
                        return self.update(Message::StatusInfo(
                            "Cannot delete the default project".to_string(),
                        ));
                    }
                }
                vec![]
            }
            KeyCode::Char('J') => {
                if let Some(id) = self.selected_project().map(|p| p.id) {
                    return vec![Command::ReorderProject { id, delta: 1 }];
                }
                vec![]
            }
            KeyCode::Char('K') => {
                if let Some(id) = self.selected_project().map(|p| p.id) {
                    return vec![Command::ReorderProject { id, delta: -1 }];
                }
                vec![]
            }
            KeyCode::Char('q') => self.update(Message::Quit),
            _ => vec![],
        }
    }

    fn handle_key_input_project_name(
        &mut self,
        key: KeyEvent,
        editing_id: Option<i64>,
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Enter => {
                let name = self.input.buffer.trim().to_string();
                self.input.mode = InputMode::Normal;
                self.input.buffer.clear();
                if name.is_empty() {
                    return vec![];
                }
                match editing_id {
                    None => vec![Command::CreateProject { name }],
                    Some(id) => vec![Command::RenameProject { id, name }],
                }
            }
            KeyCode::Esc => {
                self.input.mode = InputMode::Normal;
                self.input.buffer.clear();
                vec![]
            }
            KeyCode::Backspace => {
                self.input.buffer.pop();
                vec![]
            }
            KeyCode::Char(c) => {
                self.input.buffer.push(c);
                vec![]
            }
            _ => vec![],
        }
    }

    fn handle_key_confirm_delete_project1(&mut self, key: KeyEvent, id: i64) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') => {
                let item_count = self
                    .board
                    .tasks
                    .iter()
                    .filter(|t| t.project_id == id)
                    .count() as u64
                    + self
                        .board
                        .epics
                        .iter()
                        .filter(|e| e.project_id == id)
                        .count() as u64;
                self.input.mode = InputMode::ConfirmDeleteProject2 { id, item_count };
                vec![]
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.input.mode = InputMode::Normal;
                vec![]
            }
            _ => vec![],
        }
    }

    fn handle_key_confirm_delete_project2(
        &mut self,
        key: KeyEvent,
        id: i64,
        _item_count: u64, // read by the renderer via InputMode::ConfirmDeleteProject2 { item_count }
    ) -> Vec<Command> {
        match key.code {
            KeyCode::Char('y') => {
                self.input.mode = InputMode::Normal;
                // Navigate away from Projects panel (col 0) to Backlog (col 1)
                let nav = self.update(Message::NavigateColumn(1));
                let mut cmds = vec![Command::DeleteProject { id }];
                cmds.extend(nav);
                cmds
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.input.mode = InputMode::Normal;
                vec![]
            }
            _ => vec![],
        }
    }

    /// Dispatches to `on_task` or `on_epic` based on the current selection, passing only the
    /// item's ID (which is `Copy`). Returns `vec![]` when nothing is selected.
    fn dispatch_selection<F, G>(&mut self, on_task: F, on_epic: G) -> Vec<Command>
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
    fn with_selected_task<F>(&mut self, f: F) -> Vec<Command>
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
    fn selected_epic_id(&self) -> Option<EpicId> {
        match self.selected_column_item() {
            Some(ColumnItem::Epic(epic)) => Some(epic.id),
            _ => None,
        }
    }
}
