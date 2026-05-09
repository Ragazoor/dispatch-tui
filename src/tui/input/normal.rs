//! Normal-mode (default board / epic view) key handler.

use crossterm::event::{KeyCode, KeyEvent};

use crate::models::{LearningId, SubStatus, TaskStatus};

use super::super::messages::LearningMessage;
use super::super::types::*;
use super::super::App;

/// Extract the learning id of the currently-selected node in the tree view.
///
/// Leaf node identifiers are encoded as `"learning:<id>"`. Returns `None` when
/// nothing is selected or the selected item is a scope-group header.
fn selected_learning_id_from_tree(
    tree_state: &std::cell::RefCell<tui_tree_widget::TreeState<String>>,
) -> Option<LearningId> {
    let state = tree_state.borrow();
    let selected = state.selected();
    selected
        .last()?
        .strip_prefix("learning:")?
        .parse::<i64>()
        .ok()
        .map(LearningId)
}

impl App {
    pub(in crate::tui) fn handle_key_learnings(&mut self, key: KeyEvent) -> Vec<Command> {
        // Extract view and selected-id data before any mutable borrows.
        let (current_view, selected_id) = if let ViewMode::Learnings {
            selected,
            ref learnings,
            view,
            ref tree_state,
            ..
        } = self.board.view_mode
        {
            let id = match view {
                LearningsView::List => learnings.get(selected).map(|l| l.id),
                LearningsView::Tree => selected_learning_id_from_tree(tree_state),
            };
            (view, id)
        } else {
            return vec![];
        };

        match key.code {
            KeyCode::Tab => self.update(Message::Learning(LearningMessage::ToggleView)),
            KeyCode::Char('q') | KeyCode::Esc => {
                self.update(Message::Learning(LearningMessage::Close))
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected_id {
                    self.update(Message::Learning(LearningMessage::Edit(id)))
                } else {
                    vec![]
                }
            }
            KeyCode::Char('a') => {
                if let Some(id) = selected_id {
                    self.update(Message::Learning(LearningMessage::Approve(id)))
                } else {
                    vec![]
                }
            }
            KeyCode::Char('x') => {
                if let Some(id) = selected_id {
                    self.update(Message::Learning(LearningMessage::Reject(id)))
                } else {
                    vec![]
                }
            }
            KeyCode::Char('A') => {
                if let Some(id) = selected_id {
                    self.update(Message::Learning(LearningMessage::Archive(id)))
                } else {
                    vec![]
                }
            }
            // List-view navigation
            KeyCode::Char('j') | KeyCode::Down if matches!(current_view, LearningsView::List) => {
                self.update(Message::Learning(LearningMessage::Navigate(1)))
            }
            KeyCode::Char('k') | KeyCode::Up if matches!(current_view, LearningsView::List) => {
                self.update(Message::Learning(LearningMessage::Navigate(-1)))
            }
            // Tree-view navigation (j/k/Up/Down fall through here when in Tree view)
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::Learning(
                LearningMessage::NavigateTree(TreeNav::Down),
            )),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::Learning(
                LearningMessage::NavigateTree(TreeNav::Up),
            )),
            KeyCode::Char('l') | KeyCode::Right => self.update(Message::Learning(
                LearningMessage::NavigateTree(TreeNav::Right),
            )),
            KeyCode::Char('h') | KeyCode::Left => self.update(Message::Learning(
                LearningMessage::NavigateTree(TreeNav::Left),
            )),
            _ => vec![],
        }
    }

    pub(in crate::tui) fn handle_key_normal(&mut self, key: KeyEvent) -> Vec<Command> {
        // TaskDetail overlay captures all input when visible
        if matches!(self.board.view_mode, ViewMode::TaskDetail { .. }) {
            return self.handle_key_task_detail(key);
        }

        // Learnings overlay captures all input when visible
        if matches!(self.board.view_mode, ViewMode::Learnings { .. }) {
            return self.handle_key_learnings(key);
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
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::Exit))
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
            KeyCode::Char('J') => self.update(Message::Task(
                crate::tui::messages::TaskMessage::ReorderItem(1),
            )),
            KeyCode::Char('K') => self.update(Message::Task(
                crate::tui::messages::TaskMessage::ReorderItem(-1),
            )),

            KeyCode::Char('n') => self.update(Message::Input(
                crate::tui::messages::InputMessage::StartNewTask,
            )),
            KeyCode::Char('c') => {
                self.update(Message::Input(crate::tui::messages::InputMessage::CopyTask))
            }
            KeyCode::Char('N') => self.update(Message::System(
                crate::tui::messages::SystemMessage::ToggleNotifications,
            )),
            KeyCode::Char('E') => {
                self.update(Message::Epic(crate::tui::messages::EpicMessage::StartNew))
            }
            KeyCode::Char('d') => self.handle_key_dispatch(),
            KeyCode::Char('f') => self.update(Message::StartRepoFilter),
            KeyCode::Char('W') => self.dispatch_selection(
                |s, id| {
                    s.update(Message::WrapUp(crate::tui::messages::WrapUpMessage::Start(
                        id,
                    )))
                },
                |s, id| {
                    s.update(Message::WrapUp(
                        crate::tui::messages::WrapUpMessage::EpicStart(id),
                    ))
                },
            ),
            KeyCode::Char('L') => {
                if let Some(id) = self.selected_epic_id() {
                    return self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::MoveStatus(id, MoveDirection::Forward),
                    ));
                }
                self.handle_key_move(MoveDirection::Forward)
            }
            KeyCode::Char('H') => {
                if let Some(id) = self.selected_epic_id() {
                    return self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::MoveStatus(id, MoveDirection::Backward),
                    ));
                }
                self.handle_key_move(MoveDirection::Backward)
            }

            KeyCode::Char(':') => {
                if self.main_session_dir.is_none() {
                    self.input.mode = InputMode::MainSessionDir;
                    self.set_status(
                        "Type to filter · ↑/↓ navigate · Enter select · Esc cancel".to_string(),
                    );
                    vec![]
                } else {
                    vec![Command::OpenMainSession]
                }
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
                        vec![Command::Task(
                            crate::tui::commands::TaskCommand::JumpToTmux {
                                window: window.clone(),
                            },
                        )]
                    } else {
                        self.update(Message::System(
                            crate::tui::messages::SystemMessage::StatusInfo(
                                "No active session".to_string(),
                            ),
                        ))
                    }
                } else if let Some(id) = self.selected_epic_id() {
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(id)))
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
                        vec![Command::Task(
                            crate::tui::commands::TaskCommand::JumpToTmux { window },
                        )]
                    } else {
                        self.update(Message::System(
                            crate::tui::messages::SystemMessage::StatusInfo(
                                "No active subtask session".to_string(),
                            ),
                        ))
                    }
                } else {
                    vec![]
                }
            }

            KeyCode::Char('p') => {
                if let Some(task) = self.selected_task() {
                    if let Some(url) = &task.pr_url {
                        vec![Command::System(
                            crate::tui::commands::SystemCommand::OpenInBrowser { url: url.clone() },
                        )]
                    } else {
                        self.update(Message::System(
                            crate::tui::messages::SystemMessage::StatusInfo(
                                "No PR URL".to_string(),
                            ),
                        ))
                    }
                } else {
                    vec![]
                }
            }
            KeyCode::Char('P') => self.with_selected_task(|s, id| {
                s.update(Message::Pr(crate::tui::messages::PrMessage::StartMerge(id)))
            }),

            KeyCode::Char('a') => self.update(Message::SelectAllColumn),

            KeyCode::Char(' ') => self.dispatch_selection(
                |s, id| {
                    s.update(Message::Task(
                        crate::tui::messages::TaskMessage::ToggleSelect(id),
                    ))
                },
                |s, id| {
                    s.update(Message::Epic(
                        crate::tui::messages::EpicMessage::ToggleSelect(id),
                    ))
                },
            ),

            KeyCode::Enter => {
                if self.selection().on_select_all {
                    return self.update(Message::SelectAllColumn);
                }
                if let Some(task) = self.selected_task() {
                    let id = task.id.0;
                    return self.update(Message::Task(
                        crate::tui::messages::TaskMessage::OpenDetail(id),
                    ));
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
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::Edit(id)))
                }
                Some(ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_)) => vec![],
                None => {
                    if let ViewMode::Epic { epic_id, .. } = &self.board.view_mode {
                        let id = *epic_id;
                        self.update(Message::Epic(crate::tui::messages::EpicMessage::Edit(id)))
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
                        Some(ColumnItem::Epic(_)) => self.update(Message::Epic(
                            crate::tui::messages::EpicMessage::ConfirmArchive,
                        )),
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
                    0 => self.update(Message::System(
                        crate::tui::messages::SystemMessage::StatusInfo(
                            "No saved repo paths — create a task first".to_string(),
                        ),
                    )),
                    1 => {
                        let repo_path = self.board.repo_paths[0].clone();
                        self.update(Message::Task(
                            crate::tui::messages::TaskMessage::QuickDispatch { repo_path, epic_id },
                        ))
                    }
                    _ => self.update(Message::Input(
                        crate::tui::messages::InputMessage::StartQuickDispatchSelection,
                    )),
                }
            }

            KeyCode::Char('U') => {
                if let ViewMode::Epic { epic_id, .. } = &self.board.view_mode {
                    let id = *epic_id;
                    self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::ToggleAutoDispatch(id),
                    ))
                } else {
                    vec![]
                }
            }

            KeyCode::Char('F') => self.update(Message::Task(
                crate::tui::messages::TaskMessage::ToggleFlattened,
            )),

            KeyCode::Char('I') => self.update(Message::Learning(LearningMessage::Open)),

            KeyCode::Char('?') => self.update(Message::System(
                crate::tui::messages::SystemMessage::ToggleHelp,
            )),

            KeyCode::Char('S') => self.update(Message::ToggleSplitMode),

            KeyCode::Char('T') => {
                if !self.select.tasks.is_empty() {
                    let ids: Vec<_> = self.select.tasks.iter().copied().collect();
                    self.update(Message::Task(
                        crate::tui::messages::TaskMessage::BatchDetachTmux(ids),
                    ))
                } else if let Some(task) = self.selected_task() {
                    if task.tmux_window.is_some() {
                        let id = task.id;
                        self.update(Message::Task(
                            crate::tui::messages::TaskMessage::DetachTmux(id),
                        ))
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
                    self.update(Message::Feed(
                        crate::tui::messages::FeedMessage::TriggerEpic(id),
                    ))
                } else {
                    vec![]
                }
            }

            KeyCode::Esc => {
                if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::Exit))
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
