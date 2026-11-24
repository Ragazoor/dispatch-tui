//! board.normal: the actions the table's rows run on the board / epic view.

use super::super::types::*;
use super::super::App;
use crate::keybindings::KeyBinding;

use super::key_event;

/// The board.normal actions that act on the selection, or on a task or epic
/// card, and so do nothing in the orchestrator view.
const ACTS_ON_CARDS_OR_SELECTION: &[&str] = &[
    "delete_task",
    "move_task_forward",
    "move_task_backward",
    "move_task_to_epic",
    "reparent_epic",
    "link_work_package",
    "reorder_task_down",
    "reorder_task_up",
    "toggle_select",
    "select_all",
    "clear_selection",
    "toggle_ready",
    "toggle_bookmark",
    "copy_task",
    "dispatch_task",
    "detach_tmux",
];

impl App {
    /// Run the action of the board.normal row a key resolved to
    /// (`KeypressRunsItsRowsAction`). The row says which action; this says
    /// what the action does. An arm that takes effect records the row's action
    /// id under the key as typed (`label`); one that finds nothing to act on
    /// records nothing. Navigation is the exception: it records even at an
    /// edge (keybindings.allium, KeyInput's guidance).
    pub(in crate::tui) fn run_normal(&mut self, b: &KeyBinding, label: &str) -> Vec<Command> {
        self.try_run_normal(b, label).unwrap_or_else(|| {
            tracing::warn!(action = b.action, "board.normal action has no handler arm");
            Vec::new()
        })
    }

    /// [`Self::run_normal`] without the fallback: `None` when no handler arm
    /// claims the action.
    pub(in crate::tui) fn try_run_normal(
        &mut self,
        b: &KeyBinding,
        label: &str,
    ) -> Option<Vec<Command>> {
        if self.view().orchestrator_view() && ACTS_ON_CARDS_OR_SELECTION.contains(&b.action) {
            // A destructive key must not act on cards this view does not
            // show; the selection is kept for the other lenses
            // (`board-layout.allium`, "Orchestrator View").
            return Some(vec![]);
        }
        self.run_normal_board(b, label)
            .or_else(|| self.run_normal_epic(b, label))
            .or_else(|| self.run_normal_task(b, label))
    }

    /// Board-wide actions: navigation, reordering, selection, folding, filters, search and view toggles.
    fn run_normal_board(&mut self, b: &KeyBinding, label: &str) -> Option<Vec<Command>> {
        use crate::tui::messages::{
            BoardFilterMessage, EpicMessage, InputMessage, SplitMessage, SystemMessage, TaskMessage,
        };
        let action = b.action;
        let keyed = |app: &mut App, msg: Message| app.dispatch_keyed(msg, action, label);
        // For a key that can find nothing to act on: it records only when the
        // message issued a command or changed the mode.
        let keyed_if_effect = |app: &mut App, msg: Message| {
            app.dispatch_prompting_handler_keyed(|a| a.update(msg), action, label)
        };
        Some(match action {
            "quit" => keyed(self, Message::System(SystemMessage::Quit)),
            "navigate_column_left" => keyed(self, Message::NavigateColumn(-1)),
            "navigate_column_right" => keyed(self, Message::NavigateColumn(1)),
            "navigate_row_down" => keyed(self, Message::NavigateRow(1)),
            "navigate_row_up" => keyed(self, Message::NavigateRow(-1)),
            "navigate_row_first" => keyed(self, Message::NavigateRowFirst),
            "navigate_row_last" => keyed(self, Message::NavigateRowLast),
            "reorder_task_down" => {
                keyed_if_effect(self, Message::Task(TaskMessage::ReorderItem(1)))
            }
            "reorder_task_up" => keyed_if_effect(self, Message::Task(TaskMessage::ReorderItem(-1))),
            "toggle_notifications" => {
                keyed(self, Message::System(SystemMessage::ToggleNotifications))
            }
            "toggle_keep_awake" => keyed(
                self,
                Message::KeepAwake(crate::tui::messages::KeepAwakeMessage::Toggle),
            ),
            "set_display_name" => keyed(self, Message::Input(InputMessage::StartSetDisplayName)),
            "open_board_filter" => keyed(self, Message::BoardFilter(BoardFilterMessage::Start)),
            "search_tasks" => {
                self.search.saved = Some(self.search.query.clone());
                self.input.mode = InputMode::SearchTasks;
                vec![key_event(action, label)]
            }
            "select_all" => keyed(self, Message::SelectAllColumn),
            "toggle_section_collapse" => keyed_if_effect(self, Message::ToggleSectionCollapse),
            "toggle_epic_fold" => keyed_if_effect(self, Message::ToggleEpicFold),
            "toggle_select" => {
                // Only a task or epic card can be selected; anywhere else the
                // press selects nothing and records nothing.
                if !matches!(
                    self.selected_column_item(),
                    Some(ColumnItem::Task(_) | ColumnItem::Epic(_))
                ) {
                    return Some(vec![]);
                }
                let mut cmds = self.dispatch_selection(
                    |s, id| s.update(Message::Task(TaskMessage::ToggleSelect(id))),
                    |s, id| s.update(Message::Epic(EpicMessage::ToggleSelect(id))),
                );
                cmds.push(key_event(action, label));
                cmds
            }
            "cycle_lens_forward" => keyed(
                self,
                Message::Task(TaskMessage::CycleLens(MoveDirection::Forward)),
            ),
            "cycle_lens_backward" => keyed(
                self,
                Message::Task(TaskMessage::CycleLens(MoveDirection::Backward)),
            ),
            "toggle_help" => keyed(self, Message::System(SystemMessage::ToggleHelp)),
            "toggle_split_mode" => keyed(self, Message::Split(SplitMessage::Toggle)),
            "toggle_bookmark" => keyed(self, Message::Task(TaskMessage::ToggleBookmark)),
            "detach_tmux" => {
                self.dispatch_prompting_handler_keyed(Self::handle_key_detach, action, label)
            }
            "refresh_feed" => {
                self.dispatch_handler_keyed(Self::handle_key_feed_refresh, action, label)
            }
            "clear_search" => {
                self.search.query.clear();
                self.sync_board_selection();
                vec![key_event(action, label)]
            }
            "clear_selection" => keyed(self, Message::ClearSelection),
            _ => return None,
        })
    }

    /// Epic actions. `None` when the action is not one of them.
    fn run_normal_epic(&mut self, b: &KeyBinding, label: &str) -> Option<Vec<Command>> {
        use crate::tui::messages::{EpicMessage, TaskMessage};
        let action = b.action;
        let keyed = |app: &mut App, msg: Message| app.dispatch_keyed(msg, action, label);
        Some(match action {
            "exit_epic" => keyed(self, Message::Epic(EpicMessage::Exit)),
            "create_epic" => keyed(self, Message::Epic(EpicMessage::StartNew)),
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
            "enter_epic" => match self
                .selected_epic_id()
                .or_else(|| self.selected_orchestrator_epic_id())
            {
                Some(id) => keyed(self, Message::Epic(EpicMessage::Enter(id))),
                None => vec![],
            },
            "enter_work_package" => match self.selected_work_package_id() {
                Some(id) => keyed(
                    self,
                    Message::WorkPackage(crate::tui::messages::WorkPackageMessage::Enter(id)),
                ),
                None => vec![],
            },
            "link_work_package" => {
                let item = match self.selected_column_item() {
                    Some(ColumnItem::Task(t)) => crate::models::WorkPackageItem::Task(t.id),
                    Some(ColumnItem::Epic(e)) => crate::models::WorkPackageItem::Epic(e.id),
                    _ => return Some(vec![]),
                };
                keyed(
                    self,
                    Message::WorkPackage(crate::tui::messages::WorkPackageMessage::OpenLinkPicker(
                        item,
                    )),
                )
            }
            "toggle_ready" => keyed(self, Message::Epic(EpicMessage::ToggleReady)),
            "open_orchestrator" => keyed(self, Message::Epic(EpicMessage::OpenOrchestrator)),
            "resolve_waiting_item" => keyed(self, Message::Epic(EpicMessage::ResolveItemKey)),
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
            _ => return None,
        })
    }

    /// Task actions. `None` when the action is not one of them.
    fn run_normal_task(&mut self, b: &KeyBinding, label: &str) -> Option<Vec<Command>> {
        use crate::tui::messages::{InputMessage, TaskMessage};
        let action = b.action;
        let keyed = |app: &mut App, msg: Message| app.dispatch_keyed(msg, action, label);
        let keyed_if_effect = |app: &mut App, msg: Message| {
            app.dispatch_prompting_handler_keyed(|a| a.update(msg), action, label)
        };
        Some(match action {
            "create_task" => keyed(self, Message::Input(InputMessage::StartNewTask)),
            // With no task under the cursor there is nothing to copy.
            "copy_task" => keyed_if_effect(self, Message::Input(InputMessage::CopyTask)),
            "move_task_forward" => {
                self.move_key(MoveDirection::Forward, "move_task_forward", label)
            }
            "move_task_backward" => {
                self.move_key(MoveDirection::Backward, "move_task_backward", label)
            }
            "open_pr_url" => self.dispatch_handler_keyed(Self::handle_key_open_pr, action, label),
            "open_task_detail" => {
                let Some(task) = self.selected_task() else {
                    return Some(vec![]);
                };
                let id = task.id;
                let mut cmds = self.update(Message::Task(TaskMessage::OpenDetail(id)));
                cmds.push(key_event(action, label));
                cmds
            }
            "activate_unavailable"
            | "jump_to_tmux"
            | "swap_split_pane"
            | "dispatch_task"
            | "open_retry_dialog"
            | "resume_task" => self.run_activation(b, label),
            // Prompting: on a package card `e` opens the rename input, which
            // changes the mode and issues no command.
            "edit_task" => {
                self.dispatch_prompting_handler_keyed(Self::handle_key_edit, action, label)
            }
            "delete_task" => {
                self.dispatch_prompting_handler_keyed(Self::handle_key_delete_item, action, label)
            }
            "quick_dispatch" => {
                let mut cmds = self.handle_key_quick_dispatch_trigger();
                cmds.push(key_event(action, label));
                cmds
            }
            _ => return None,
        })
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
            Some(ColumnItem::Epic(epic) | ColumnItem::Orchestrator(epic)) => {
                let id = epic.id;
                self.update(Message::Epic(crate::tui::messages::EpicMessage::Edit(id)))
            }
            Some(ColumnItem::WorkPackage(package)) => {
                let id = package.id;
                self.update(Message::WorkPackage(
                    crate::tui::messages::WorkPackageMessage::StartRename(id),
                ))
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
            self.delete_selected_items()
        } else {
            self.delete_cursor_item()
        }
    }

    /// The batch half of [`Self::handle_key_delete_item`].
    fn delete_selected_items(&mut self) -> Vec<Command> {
        if self.select.epics.is_empty() {
            let not_done: Vec<_> = self
                .select
                .tasks
                .iter()
                .copied()
                .filter(|id| {
                    self.board
                        .find_task(*id)
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
        let keys = self.yes_no(crate::keybindings::KeyNamespace::BoardConfirmBatchDelete);
        self.status.set(format!("Delete {count} items? {keys}"));
        vec![]
    }

    /// The single-item half of [`Self::handle_key_delete_item`].
    fn delete_cursor_item(&mut self) -> Vec<Command> {
        let (id, done, title) = match self.selected_column_item() {
            Some(ColumnItem::Epic(_)) => {
                return self.update(Message::Epic(
                    crate::tui::messages::EpicMessage::ConfirmDelete,
                ));
            }
            Some(ColumnItem::WorkPackage(package)) => {
                let id = package.id;
                return self.update(Message::WorkPackage(
                    crate::tui::messages::WorkPackageMessage::ConfirmDelete(id),
                ));
            }
            Some(ColumnItem::Task(task)) => (
                task.id,
                task.status == crate::models::TaskStatus::Done,
                super::super::truncate_title(&task.title, super::super::TITLE_DISPLAY_LENGTH),
            ),
            _ => return vec![],
        };
        if !done {
            self.prompt_move_to_done(vec![id]);
            return vec![];
        }
        self.input.mode = InputMode::ConfirmDeleteTask(id);
        let keys = self.yes_no(crate::keybindings::KeyNamespace::BoardConfirmDeleteTask);
        self.status.set(format!("Delete {title}? {keys}"));
        vec![]
    }

    /// `'D'` — quick-dispatch: immediate for 1 repo, picker otherwise (including none).
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
                self.board
                    .find_epic(id)
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
