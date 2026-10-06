//! board.normal: the actions the table's rows run on the board / epic view.

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::App;
use crate::keybindings::KeyBinding;

use super::key_event;

impl App {
    /// Run the action of the board.normal row a key resolved to
    /// (`KeypressRunsItsRowsAction`). The row says which action; this says
    /// what the action does. Every arm records the row's action id under the
    /// key as typed (`label`), as it always has.
    pub(in crate::tui) fn run_normal(
        &mut self,
        b: &KeyBinding,
        key: KeyEvent,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::{
            EpicMessage, InputMessage, RepoFilterMessage, RepoSyncMessage, SplitMessage,
            SystemMessage, TaskMessage,
        };
        let action = b.action;
        let keyed = |app: &mut App, msg: Message| app.dispatch_keyed(msg, action, label);
        match action {
            "quit" => keyed(self, Message::System(SystemMessage::Quit)),
            "exit_epic" => keyed(self, Message::Epic(EpicMessage::Exit)),
            "navigate_column" => {
                let d = if matches!(key.code, KeyCode::Char('h') | KeyCode::Left) {
                    -1
                } else {
                    1
                };
                keyed(self, Message::NavigateColumn(d))
            }
            "navigate_row" => {
                let d = if matches!(key.code, KeyCode::Char('j') | KeyCode::Down) {
                    1
                } else {
                    -1
                };
                keyed(self, Message::NavigateRow(d))
            }
            "navigate_row_first" => keyed(self, Message::NavigateRowFirst),
            "navigate_row_last" => keyed(self, Message::NavigateRowLast),
            "reorder_task_down" => keyed(self, Message::Task(TaskMessage::ReorderItem(1))),
            "reorder_task_up" => keyed(self, Message::Task(TaskMessage::ReorderItem(-1))),
            "create_task" => keyed(self, Message::Input(InputMessage::StartNewTask)),
            "copy_task" => keyed(self, Message::Input(InputMessage::CopyTask)),
            "toggle_notifications" => {
                keyed(self, Message::System(SystemMessage::ToggleNotifications))
            }
            "create_epic" => keyed(self, Message::Epic(EpicMessage::StartNew)),
            "filter_repos" => keyed(self, Message::RepoFilter(RepoFilterMessage::Start)),
            "search_tasks" => {
                self.search.saved = Some(self.search.query.clone());
                self.input.mode = InputMode::SearchTasks;
                vec![key_event(action, label)]
            }
            "move_task_forward" => {
                self.move_key(MoveDirection::Forward, "move_task_forward", label)
            }
            "move_task_backward" => {
                self.move_key(MoveDirection::Backward, "move_task_backward", label)
            }
            "open_repo_sync_prompt" => keyed(self, Message::RepoSync(RepoSyncMessage::OpenPrompt)),
            "open_pr_url" => self.dispatch_handler_keyed(Self::handle_key_open_pr, action, label),
            "select_all" => keyed(self, Message::SelectAllColumn),
            "toggle_section_collapse" => keyed(self, Message::ToggleSectionCollapse),
            "toggle_epic_fold" => keyed(self, Message::ToggleEpicFold),
            "toggle_select" => {
                let mut cmds = self.dispatch_selection(
                    |s, id| s.update(Message::Task(TaskMessage::ToggleSelect(id))),
                    |s, id| s.update(Message::Epic(EpicMessage::ToggleSelect(id))),
                );
                cmds.push(key_event(action, label));
                cmds
            }
            "open_task_detail" => {
                let Some(task) = self.selected_task() else {
                    return vec![];
                };
                let id = task.id;
                let mut cmds = self.update(Message::Task(TaskMessage::OpenDetail(id)));
                cmds.push(key_event(action, label));
                cmds
            }
            "exit_all_epics" => keyed(self, Message::Epic(EpicMessage::ExitAll)),
            "jump_to_task_epic" => match self.selected_task().and_then(|t| t.epic_id) {
                Some(id) => keyed(self, Message::Epic(EpicMessage::JumpTo(id))),
                None => vec![],
            },
            "jump_to_deepest_epic" => {
                match self.selected_epic_id().zip(self.selected_column_status()) {
                    Some((id, status)) => {
                        keyed(self, Message::Epic(EpicMessage::JumpToDeepest(id, status)))
                    }
                    None => vec![],
                }
            }
            "enter_epic" => match self.selected_epic_id() {
                Some(id) => keyed(self, Message::Epic(EpicMessage::Enter(id))),
                None => vec![],
            },
            "activate_unavailable"
            | "jump_to_tmux"
            | "swap_split_pane"
            | "dispatch_task"
            | "open_retry_dialog"
            | "resume_task" => self.run_activation(b, label),
            "edit_task" => self.dispatch_handler_keyed(Self::handle_key_edit, action, label),
            "delete_task" => {
                self.dispatch_prompting_handler_keyed(Self::handle_key_delete_item, action, label)
            }
            "quick_dispatch" => {
                let mut cmds = self.handle_key_quick_dispatch_trigger();
                cmds.push(key_event(action, label));
                cmds
            }
            "toggle_auto_dispatch" | "toggle_group_by_repo" => match self.current_epic_id() {
                Some(id) => {
                    let msg = if action == "toggle_auto_dispatch" {
                        EpicMessage::ToggleAutoDispatch(id)
                    } else {
                        EpicMessage::ToggleGroupByRepo(id)
                    };
                    keyed(self, Message::Epic(msg))
                }
                None => vec![],
            },
            "filter_active" => keyed(
                self,
                Message::RepoFilter(RepoFilterMessage::ToggleOnlyActive),
            ),
            "toggle_flattened" => keyed(self, Message::Task(TaskMessage::ToggleFlattened)),
            "toggle_help" => keyed(self, Message::System(SystemMessage::ToggleHelp)),
            "toggle_split_mode" => keyed(self, Message::Split(SplitMessage::Toggle)),
            "detach_tmux" => {
                self.dispatch_prompting_handler_keyed(Self::handle_key_detach, action, label)
            }
            "refresh_feed" => {
                self.dispatch_handler_keyed(Self::handle_key_feed_refresh, action, label)
            }
            "reparent_epic" => match self.selected_epic_id() {
                Some(id) => keyed(self, Message::Epic(EpicMessage::StartReparent(id))),
                None => vec![],
            },
            "move_task_to_epic" => match self.selected_task() {
                // `m` on a task card moves it to another epic (or detaches it).
                Some(task) => {
                    let id = task.id;
                    keyed(self, Message::Task(TaskMessage::StartMoveToEpic(id)))
                }
                None => vec![],
            },
            "clear_search" => {
                self.search.query.clear();
                self.sync_board_selection();
                vec![key_event(action, label)]
            }
            "clear_selection" => keyed(self, Message::ClearSelection),
            _ => vec![],
        }
    }

    /// `'p'` — open the selected task's PR URL in the browser.
    fn handle_key_open_pr(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            if let Some(u) = &task.url {
                vec![Command::System(
                    crate::tui::commands::SystemCommand::OpenInBrowser { url: u.url.clone() },
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

    /// `'e'` — edit the selected task or epic.
    fn handle_key_edit(&mut self) -> Vec<Command> {
        match self.selected_column_item() {
            Some(ColumnItem::Task(task)) => {
                vec![Command::Editor(
                    crate::tui::commands::EditorCommand::PopOut(
                        crate::tui::types::EditKind::TaskEdit(Box::new(task.clone())),
                    ),
                )]
            }
            Some(ColumnItem::Epic(epic)) => {
                let id = epic.id;
                self.update(Message::Epic(crate::tui::messages::EpicMessage::Edit(id)))
            }
            Some(
                ColumnItem::EpicHeader(_)
                | ColumnItem::SubstatusLabel(_)
                | ColumnItem::FoldedSection(_)
                | ColumnItem::FoldedEpic(_)
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

    /// `'x'` — complete the selected task(s), or permanently delete them once
    /// Done. (`tasks.allium: DeleteKeyRouting`.)
    ///
    /// Completing is the common case and deleting the exception, so 'x' only
    /// deletes a task that already sits in Done; anything else moves straight
    /// to Done via the ConfirmDone prompt. A selection containing an epic
    /// always goes to the batch-delete confirmation, guarded per item.
    fn handle_key_delete_item(&mut self) -> Vec<Command> {
        if self.has_selection() {
            if self.select.epics.is_empty() {
                let not_done: Vec<_> = self
                    .select
                    .tasks
                    .iter()
                    .copied()
                    .filter(|id| {
                        self.find_task(*id)
                            .is_some_and(|t| t.status != crate::models::TaskStatus::Done)
                    })
                    .collect();
                if !not_done.is_empty() {
                    self.prompt_move_to_done(not_done);
                    return vec![];
                }
            }
            let count = self.select.tasks.len() + self.select.epics.len();
            self.input.mode = InputMode::ConfirmBatchDelete;
            self.set_status(format!("Delete {} items? [y/n]", count));
            vec![]
        } else {
            match self.selected_column_item() {
                Some(ColumnItem::Epic(_)) => self.update(Message::Epic(
                    crate::tui::messages::EpicMessage::ConfirmDelete,
                )),
                _ => {
                    if let Some(task) = self.selected_task() {
                        let id = task.id;
                        if task.status != crate::models::TaskStatus::Done {
                            self.prompt_move_to_done(vec![id]);
                            return vec![];
                        }
                        let title = super::super::truncate_title(
                            &task.title,
                            super::super::TITLE_DISPLAY_LENGTH,
                        );
                        self.input.mode = InputMode::ConfirmDeleteTask(id);
                        self.set_status(format!("Delete {title}? [y/n]"));
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
}
