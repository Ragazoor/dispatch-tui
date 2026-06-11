//! Normal-mode (default board / epic view) key handler.

use crossterm::event::{KeyCode, KeyEvent};

use crate::models::LearningId;

use super::super::messages::LearningMessage;
use super::super::types::*;
use super::super::App;

use super::key_event;

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
            KeyCode::Tab => {
                let mut cmds = self.update(Message::Learning(LearningMessage::ToggleView));
                cmds.push(key_event("toggle_learnings_view", "Tab"));
                cmds
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.update(Message::Learning(LearningMessage::Close))
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected_id {
                    let mut cmds = self.update(Message::Learning(LearningMessage::Edit(id)));
                    cmds.push(key_event("edit_learning", "e"));
                    cmds
                } else {
                    vec![]
                }
            }
            KeyCode::Char('a') => {
                if let Some(id) = selected_id {
                    let mut cmds = self.update(Message::Learning(LearningMessage::Approve(id)));
                    cmds.push(key_event("approve_learning", "a"));
                    cmds
                } else {
                    vec![]
                }
            }
            KeyCode::Char('x') => {
                if let Some(id) = selected_id {
                    let mut cmds = self.update(Message::Learning(LearningMessage::Reject(id)));
                    cmds.push(key_event("reject_learning", "x"));
                    cmds
                } else {
                    vec![]
                }
            }
            KeyCode::Char('A') => {
                if let Some(id) = selected_id {
                    let mut cmds = self.update(Message::Learning(LearningMessage::Archive(id)));
                    cmds.push(key_event("archive_learning", "A"));
                    cmds
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

        match key.code {
            KeyCode::Char('q') => {
                if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::Exit))
                } else {
                    self.update(Message::System(crate::tui::messages::SystemMessage::Quit))
                }
            }

            KeyCode::Char('h') | KeyCode::Left => self.update(Message::NavigateColumn(-1)),
            KeyCode::Char('l') | KeyCode::Right => self.update(Message::NavigateColumn(1)),
            KeyCode::Char('j') | KeyCode::Down => self.update(Message::NavigateRow(1)),
            KeyCode::Char('k') | KeyCode::Up => self.update(Message::NavigateRow(-1)),
            KeyCode::Char('[') => self.update(Message::NavigateRowFirst),
            KeyCode::Char(']') => self.update(Message::NavigateRowLast),
            KeyCode::Char('J') => {
                let mut cmds = self.update(Message::Task(
                    crate::tui::messages::TaskMessage::ReorderItem(1),
                ));
                cmds.push(key_event("reorder_task_down", "J"));
                cmds
            }
            KeyCode::Char('K') => {
                let mut cmds = self.update(Message::Task(
                    crate::tui::messages::TaskMessage::ReorderItem(-1),
                ));
                cmds.push(key_event("reorder_task_up", "K"));
                cmds
            }

            KeyCode::Char('n') => {
                let mut cmds = self.update(Message::Input(
                    crate::tui::messages::InputMessage::StartNewTask,
                ));
                cmds.push(key_event("create_task", "n"));
                cmds
            }
            KeyCode::Char('c') => {
                let mut cmds =
                    self.update(Message::Input(crate::tui::messages::InputMessage::CopyTask));
                cmds.push(key_event("copy_task", "c"));
                cmds
            }
            KeyCode::Char('N') => {
                let mut cmds = self.update(Message::System(
                    crate::tui::messages::SystemMessage::ToggleNotifications,
                ));
                cmds.push(key_event("toggle_notifications", "N"));
                cmds
            }
            KeyCode::Char('E') => {
                let mut cmds =
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::StartNew));
                cmds.push(key_event("create_epic", "E"));
                cmds
            }
            KeyCode::Char('d') => {
                let mut cmds = self.handle_key_dispatch();
                cmds.push(key_event("dispatch_task", "d"));
                cmds
            }
            KeyCode::Char('f') => {
                let mut cmds = self.update(Message::RepoFilter(
                    crate::tui::messages::RepoFilterMessage::Start,
                ));
                cmds.push(key_event("filter_repos", "f"));
                cmds
            }
            KeyCode::Char('W') => {
                let mut cmds = self.dispatch_selection(
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
                );
                cmds.push(key_event("wrap_up", "W"));
                cmds
            }
            KeyCode::Char('L') => {
                if let Some(id) = self.selected_epic_id() {
                    let mut cmds = self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::MoveStatus(id, MoveDirection::Forward),
                    ));
                    cmds.push(key_event("move_task_forward", "L"));
                    return cmds;
                }
                let mut cmds = self.handle_key_move(MoveDirection::Forward);
                cmds.push(key_event("move_task_forward", "L"));
                cmds
            }
            KeyCode::Char('H') => {
                if let Some(id) = self.selected_epic_id() {
                    let mut cmds = self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::MoveStatus(id, MoveDirection::Backward),
                    ));
                    cmds.push(key_event("move_task_backward", "H"));
                    return cmds;
                }
                let mut cmds = self.handle_key_move(MoveDirection::Backward);
                cmds.push(key_event("move_task_backward", "H"));
                cmds
            }

            KeyCode::Char(':') => {
                // The runtime decides: jump to the main-session window if it is
                // alive, otherwise open the picker to (re)select a directory.
                vec![
                    Command::MainSession(crate::tui::commands::MainSessionCommand::Open),
                    key_event("open_main_session", ":"),
                ]
            }

            KeyCode::Char('g') => {
                let mut cmds = self.handle_key_jump_window();
                if !cmds.is_empty() {
                    cmds.push(key_event("jump_to_tmux", "g"));
                }
                cmds
            }

            KeyCode::Char('p') => {
                let mut cmds = self.handle_key_open_pr();
                if !cmds.is_empty() {
                    cmds.push(key_event("open_pr_url", "p"));
                }
                cmds
            }
            KeyCode::Char('a') => {
                let mut cmds = self.update(Message::SelectAllColumn);
                cmds.push(key_event("select_all", "a"));
                cmds
            }

            KeyCode::Char(' ') => {
                let mut cmds = self.dispatch_selection(
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
                );
                cmds.push(key_event("toggle_select", " "));
                cmds
            }

            KeyCode::Enter => self.handle_key_enter_normal(),

            KeyCode::Char('e') => {
                let mut cmds = self.handle_key_edit();
                if !cmds.is_empty() {
                    cmds.push(key_event("edit_task", "e"));
                }
                cmds
            }

            KeyCode::Char('x') => {
                let mut cmds = self.handle_key_archive_item();
                if !cmds.is_empty() {
                    cmds.push(key_event("archive_task", "x"));
                }
                cmds
            }

            KeyCode::Char('D') => {
                let mut cmds = self.handle_key_quick_dispatch_trigger();
                cmds.push(key_event("quick_dispatch", "D"));
                cmds
            }

            KeyCode::Char('U') => {
                if let Some(id) = self.current_epic_id() {
                    let mut cmds = self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::ToggleAutoDispatch(id),
                    ));
                    cmds.push(key_event("toggle_auto_dispatch", "U"));
                    cmds
                } else {
                    vec![]
                }
            }

            KeyCode::Char('R') => {
                if let Some(id) = self.current_epic_id() {
                    let mut cmds = self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::ToggleGroupByRepo(id),
                    ));
                    cmds.push(key_event("toggle_group_by_repo", "R"));
                    cmds
                } else {
                    vec![]
                }
            }

            KeyCode::Char('A') => {
                let mut cmds = self.update(Message::RepoFilter(
                    crate::tui::messages::RepoFilterMessage::ToggleOnlyActive,
                ));
                cmds.push(key_event("filter_active", "A"));
                cmds
            }

            KeyCode::Char('F') => {
                let mut cmds = self.update(Message::Task(
                    crate::tui::messages::TaskMessage::ToggleFlattened,
                ));
                cmds.push(key_event("toggle_flattened", "F"));
                cmds
            }

            KeyCode::Char('I') => {
                let mut cmds = self.update(Message::Learning(LearningMessage::Open));
                cmds.push(key_event("open_learnings", "I"));
                cmds
            }

            KeyCode::Char('?') => {
                let mut cmds = self.update(Message::System(
                    crate::tui::messages::SystemMessage::ToggleHelp,
                ));
                cmds.push(key_event("toggle_help", "?"));
                cmds
            }

            KeyCode::Char('s') => {
                let mut cmds =
                    self.update(Message::Split(crate::tui::messages::SplitMessage::Toggle));
                cmds.push(key_event("toggle_split_mode", "s"));
                cmds
            }

            KeyCode::Char('S') => {
                let mut cmds = self.handle_key_swap_split();
                if !cmds.is_empty() {
                    cmds.push(key_event("swap_split_pane", "S"));
                }
                cmds
            }

            KeyCode::Char('T') => {
                let mut cmds = self.handle_key_detach();
                if !cmds.is_empty() {
                    cmds.push(key_event("detach_tmux", "T"));
                }
                cmds
            }

            KeyCode::Char('r') => {
                let mut cmds = self.handle_key_feed_refresh();
                if !cmds.is_empty() {
                    cmds.push(key_event("refresh_feed", "r"));
                }
                cmds
            }

            KeyCode::Char('m') => {
                if let Some(id) = self.selected_epic_id() {
                    let mut cmds = self.update(Message::Epic(
                        crate::tui::messages::EpicMessage::StartReparent(id),
                    ));
                    cmds.push(key_event("reparent_epic", "m"));
                    cmds
                } else {
                    vec![]
                }
            }

            KeyCode::Esc => self.handle_key_esc_normal(),

            _ => vec![],
        }
    }

    /// `'g'` — jump to the selected task's tmux window, or enter an epic.
    fn handle_key_jump_window(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            // If the task's window is pinned in the split pane, it no longer
            // exists as a standalone window — focus the pane directly instead.
            if self.board.split.active && self.board.split.pinned_task_id == Some(task.id) {
                if let Some(pane_id) = self.board.split.right_pane_id.clone() {
                    return vec![Command::Split(
                        crate::tui::commands::SplitCommand::FocusPane { pane_id },
                    )];
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

    /// `'S'` — swap the selected task's tmux window into the split pane.
    /// In split mode this pins/swaps the task in-place (no focus transfer).
    /// Outside split mode it shows a hint instead of silently doing nothing.
    fn handle_key_swap_split(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            if self.board.split.active {
                let id = task.id;
                self.update(Message::Split(crate::tui::messages::SplitMessage::Swap(id)))
            } else {
                self.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(
                        "Split view not active — press s to open".to_string(),
                    ),
                ))
            }
        } else {
            vec![]
        }
    }

    /// `'p'` — open the selected task's PR URL in the browser.
    fn handle_key_open_pr(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            if let Some(u) = &task.url {
                vec![Command::System(
                    crate::tui::commands::SystemCommand::OpenInBrowser {
                        url: u.url.clone(),
                    },
                )]
            } else {
                self.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo("No URL set".to_string()),
                ))
            }
        } else {
            vec![]
        }
    }

    /// `Enter` — open task detail, or toggle off select-all.
    fn handle_key_enter_normal(&mut self) -> Vec<Command> {
        if self.selection().on_select_all {
            return self.update(Message::SelectAllColumn);
        }
        if let Some(task) = self.selected_task() {
            let id = task.id;
            let mut cmds = self.update(Message::Task(
                crate::tui::messages::TaskMessage::OpenDetail(id),
            ));
            cmds.push(key_event("open_task_detail", "Enter"));
            return cmds;
        }
        vec![]
    }

    /// `'e'` — edit the selected task or epic.
    fn handle_key_edit(&mut self) -> Vec<Command> {
        match self.selected_column_item() {
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
            Some(
                ColumnItem::EpicHeader(_)
                | ColumnItem::SubstatusLabel(_)
                | ColumnItem::OrphanSeparator,
            ) => vec![],
            None => {
                if let Some(id) = self.current_epic_id() {
                    self.update(Message::Epic(crate::tui::messages::EpicMessage::Edit(id)))
                } else {
                    vec![]
                }
            }
        }
    }

    /// `'x'` — archive the selected item or selection.
    fn handle_key_archive_item(&mut self) -> Vec<Command> {
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
                        vec![]
                    } else {
                        vec![]
                    }
                }
            }
        }
    }

    /// `'D'` — quick-dispatch: immediate for 1 repo, picker for multiple, error for none.
    fn handle_key_quick_dispatch_trigger(&mut self) -> Vec<Command> {
        let epic_id = self.current_epic_id();
        self.input.pending_epic_id = epic_id;
        match self.board.repo_paths.len() {
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

    /// `'T'` — detach tmux window(s): batch if selection active, single otherwise.
    fn handle_key_detach(&mut self) -> Vec<Command> {
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

    /// `'r'` — trigger feed refresh for the selected or current epic.
    fn handle_key_feed_refresh(&mut self) -> Vec<Command> {
        let feed_epic_id = match self.selected_column_item() {
            Some(ColumnItem::Epic(e)) if e.feed_command.is_some() => Some(e.id),
            _ => None,
        }
        .or_else(|| {
            self.current_epic_id().and_then(|id| {
                self.find_epic(id)
                    .filter(|e| e.feed_command.is_some())
                    .map(|e| e.id)
            })
        });
        if let Some(id) = feed_epic_id {
            self.update(Message::Feed(
                crate::tui::messages::FeedMessage::TriggerEpic(id),
            ))
        } else {
            vec![]
        }
    }

    /// `Esc` — exit epic view, clear selection, or no-op.
    fn handle_key_esc_normal(&mut self) -> Vec<Command> {
        if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
            self.update(Message::Epic(crate::tui::messages::EpicMessage::Exit))
        } else if self.has_selection() || self.selection().on_select_all {
            self.update(Message::ClearSelection)
        } else {
            vec![]
        }
    }
}
