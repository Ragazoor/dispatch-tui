//! Normal-mode (default board / epic view) key handler.

use crossterm::event::{KeyCode, KeyEvent};

use crate::models::{SubStatus, TaskStatus};

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_key_proposed_learnings(&mut self, key: KeyEvent) -> Vec<Command> {
        let selected_id = if let ViewMode::ProposedLearnings {
            selected,
            ref learnings,
            ..
        } = self.board.view_mode
        {
            learnings.get(selected).map(|l| l.id)
        } else {
            return vec![];
        };

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.update(Message::CloseProposedLearnings),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::NavigateProposedLearning(1)),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::NavigateProposedLearning(-1)),
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

    pub(in crate::tui) fn handle_key_normal(&mut self, key: KeyEvent) -> Vec<Command> {
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
                    let title = super::super::truncate_title(&task.title, 30);
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
}
