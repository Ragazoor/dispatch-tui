//! Normal-mode (default board / epic view) key handler.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::{App, GG_CHORD_TIMEOUT};

use super::key_event;

impl App {
    pub(in crate::tui) fn handle_key_normal(&mut self, key: KeyEvent) -> Vec<Command> {
        // TaskDetail overlay captures all input when visible
        if matches!(self.board.view_mode, ViewMode::TaskDetail { .. }) {
            self.clear_pending_g_chord();
            return self.handle_key_task_detail(key);
        }

        self.handle_key_board_normal(key)
    }

    /// Abandon an armed `gg` chord, if one is pending. Called on the
    /// overlay-entry guards.
    fn clear_pending_g_chord(&mut self) {
        self.interaction.pending_g = None;
    }

    /// The main board/epic key match, split out from [`Self::handle_key_normal`]
    /// so the `gg`-chord pre-check can recurse into it for the current key
    /// once a pending `g` has been resolved (see `InteractionState::pending_g`).
    fn handle_key_board_normal(&mut self, key: KeyEvent) -> Vec<Command> {
        if let Some(started) = self.interaction.pending_g.take() {
            if key.code == KeyCode::Char('g') && started.elapsed() <= GG_CHORD_TIMEOUT {
                // Completed `gg` chord: jump to top of column. Recorded under
                // the chord, not the key, so it stays separable from `[`.
                return self.dispatch_keyed(Message::NavigateRowFirst, "navigate_row_first", "gg");
            }
            // Either a different key arrived, or the chord window expired:
            // the pending chord is simply abandoned (no action fires for the
            // lone `g`), then this key is processed normally.
            return self.handle_key_board_normal(key);
        }

        let label = super::key_label(key);
        match key.code {
            KeyCode::Char('q') => {
                if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
                    self.dispatch_keyed(
                        Message::Epic(crate::tui::messages::EpicMessage::Exit),
                        "exit_epic",
                        &label,
                    )
                } else {
                    self.dispatch_keyed(
                        Message::System(crate::tui::messages::SystemMessage::Quit),
                        "quit",
                        &label,
                    )
                }
            }

            KeyCode::Char('h') | KeyCode::Left => {
                self.dispatch_keyed(Message::NavigateColumn(-1), "navigate_column", &label)
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.dispatch_keyed(Message::NavigateColumn(1), "navigate_column", &label)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.dispatch_keyed(Message::NavigateRow(1), "navigate_row", &label)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.dispatch_keyed(Message::NavigateRow(-1), "navigate_row", &label)
            }
            KeyCode::Char('[') => {
                self.dispatch_keyed(Message::NavigateRowFirst, "navigate_row_first", &label)
            }
            KeyCode::Char(']') => {
                self.dispatch_keyed(Message::NavigateRowLast, "navigate_row_last", &label)
            }
            KeyCode::Char('J') => self.dispatch_keyed(
                Message::Task(crate::tui::messages::TaskMessage::ReorderItem(1)),
                "reorder_task_down",
                "J",
            ),
            KeyCode::Char('K') => self.dispatch_keyed(
                Message::Task(crate::tui::messages::TaskMessage::ReorderItem(-1)),
                "reorder_task_up",
                "K",
            ),

            KeyCode::Char('n') => self.dispatch_keyed(
                Message::Input(crate::tui::messages::InputMessage::StartNewTask),
                "create_task",
                "n",
            ),
            KeyCode::Char('c') => self.dispatch_keyed(
                Message::Input(crate::tui::messages::InputMessage::CopyTask),
                "copy_task",
                "c",
            ),
            KeyCode::Char('N') => self.dispatch_keyed(
                Message::System(crate::tui::messages::SystemMessage::ToggleNotifications),
                "toggle_notifications",
                "N",
            ),
            KeyCode::Char('E') => self.dispatch_keyed(
                Message::Epic(crate::tui::messages::EpicMessage::StartNew),
                "create_epic",
                "E",
            ),
            KeyCode::Char('f') => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::Start),
                "filter_repos",
                "f",
            ),
            KeyCode::Char('/') => {
                self.search.saved = Some(self.search.query.clone());
                self.input.mode = InputMode::SearchTasks;
                vec![key_event("search_tasks", "/")]
            }
            KeyCode::Char('L') => {
                if let Some(id) = self.selected_epic_id() {
                    return self.dispatch_keyed(
                        Message::Epic(crate::tui::messages::EpicMessage::MoveStatus(
                            id,
                            MoveDirection::Forward,
                        )),
                        "move_task_forward",
                        "L",
                    );
                }
                let mut cmds = self.handle_key_move(MoveDirection::Forward);
                cmds.push(key_event("move_task_forward", "L"));
                cmds
            }
            KeyCode::Char('H') => {
                if let Some(id) = self.selected_epic_id() {
                    return self.dispatch_keyed(
                        Message::Epic(crate::tui::messages::EpicMessage::MoveStatus(
                            id,
                            MoveDirection::Backward,
                        )),
                        "move_task_backward",
                        "H",
                    );
                }
                let mut cmds = self.handle_key_move(MoveDirection::Backward);
                cmds.push(key_event("move_task_backward", "H"));
                cmds
            }

            // [o] for origin: open the sync confirmation for the selected task's
            // repository (docs/specs/repo-sync.allium: rule PromptRepoSync).
            // Offered only while the drift segment is lit; with no drift the key
            // does nothing. [O] is left unbound for a future sync-all.
            KeyCode::Char('o') => self.dispatch_keyed(
                Message::RepoSync(crate::tui::messages::RepoSyncMessage::OpenPrompt),
                "open_repo_sync_prompt",
                "o",
            ),

            KeyCode::Char('g') => {
                // Start a pending `gg` chord; resolved by the next keypress
                // (above) or by `handle_tick` if the user goes idle.
                self.interaction.pending_g = Some(Instant::now());
                vec![]
            }
            KeyCode::Char('G') => {
                self.dispatch_keyed(Message::NavigateRowLast, "navigate_row_last", &label)
            }

            KeyCode::Char('p') => {
                self.dispatch_handler_keyed(Self::handle_key_open_pr, "open_pr_url", "p")
            }
            KeyCode::Char('a') => self.dispatch_keyed(Message::SelectAllColumn, "select_all", "a"),

            // [z] for fold, the vim idiom. Only bound on the board: the
            // TaskDetail overlay has its own `z` (zoom), and `handle_key_normal`
            // routes that away before this arm is reached.
            KeyCode::Char('z') => self.dispatch_keyed(
                Message::ToggleSectionCollapse,
                "toggle_section_collapse",
                "z",
            ),

            // [Z] for folding an epic group instead of a section — a
            // different key from `z` because a card can carry both a section
            // and an epic group at once (tasks.allium: ToggleEpicFold).
            KeyCode::Char('Z') => {
                self.dispatch_keyed(Message::ToggleEpicFold, "toggle_epic_fold", "Z")
            }

            KeyCode::Char('v') => {
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
                cmds.push(key_event("toggle_select", "v"));
                cmds
            }

            KeyCode::Char(' ') => self.handle_key_activate(),

            KeyCode::Enter => self.handle_key_enter_normal(),

            KeyCode::Char('e') => {
                self.dispatch_handler_keyed(Self::handle_key_edit, "edit_task", "e")
            }

            KeyCode::Char('x') => {
                self.dispatch_handler_keyed(Self::handle_key_delete_item, "delete_task", "x")
            }

            KeyCode::Char('D') => {
                let mut cmds = self.handle_key_quick_dispatch_trigger();
                cmds.push(key_event("quick_dispatch", "D"));
                cmds
            }

            KeyCode::Char('U') => {
                if let Some(id) = self.current_epic_id() {
                    self.dispatch_keyed(
                        Message::Epic(crate::tui::messages::EpicMessage::ToggleAutoDispatch(id)),
                        "toggle_auto_dispatch",
                        "U",
                    )
                } else {
                    vec![]
                }
            }

            KeyCode::Char('R') => {
                if let Some(id) = self.current_epic_id() {
                    self.dispatch_keyed(
                        Message::Epic(crate::tui::messages::EpicMessage::ToggleGroupByRepo(id)),
                        "toggle_group_by_repo",
                        "R",
                    )
                } else {
                    vec![]
                }
            }

            KeyCode::Char('A') => self.dispatch_keyed(
                Message::RepoFilter(crate::tui::messages::RepoFilterMessage::ToggleOnlyActive),
                "filter_active",
                "A",
            ),

            KeyCode::Char('F') => self.dispatch_keyed(
                Message::Task(crate::tui::messages::TaskMessage::ToggleFlattened),
                "toggle_flattened",
                "F",
            ),

            KeyCode::Char('?') => self.dispatch_keyed(
                Message::System(crate::tui::messages::SystemMessage::ToggleHelp),
                "toggle_help",
                "?",
            ),

            KeyCode::Char('s') => self.dispatch_keyed(
                Message::Split(crate::tui::messages::SplitMessage::Toggle),
                "toggle_split_mode",
                "s",
            ),

            KeyCode::Char('T') => {
                self.dispatch_handler_keyed(Self::handle_key_detach, "detach_tmux", "T")
            }

            KeyCode::Char('r') => {
                self.dispatch_handler_keyed(Self::handle_key_feed_refresh, "refresh_feed", "r")
            }

            KeyCode::Char('m') => {
                if let Some(id) = self.selected_epic_id() {
                    self.dispatch_keyed(
                        Message::Epic(crate::tui::messages::EpicMessage::StartReparent(id)),
                        "reparent_epic",
                        "m",
                    )
                } else if let Some(task) = self.selected_task() {
                    // `m` on a task card moves it to another epic (or detaches it).
                    let id = task.id;
                    self.dispatch_keyed(
                        Message::Task(crate::tui::messages::TaskMessage::StartMoveToEpic(id)),
                        "move_task_to_epic",
                        "m",
                    )
                } else {
                    vec![]
                }
            }

            KeyCode::Esc => self.handle_key_esc_normal(),

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

    /// `Enter` — open task detail, or, with the cursor on a column header,
    /// toggle that column's select-all.
    ///
    /// It toggles rather than clears: from a column that is not fully selected it
    /// *selects* (`SelectAllColumn` in `docs/specs/tasks.allium`), exactly as `a`
    /// does. It therefore records the same `select_all` action as `a` and is told
    /// apart by the recorded key, per the convention on `key_event` — two bindings
    /// for one action share an action name.
    ///
    /// allow-phantom-symbol: the superseded label is the subject of the next line.
    /// `clear_select_all` both misdescribed the behaviour — it clears only when the
    /// column is already fully selected — and broke that convention by inventing a
    /// second action name for one action.
    fn handle_key_enter_normal(&mut self) -> Vec<Command> {
        if self.selection().on_select_all {
            return self.dispatch_keyed(Message::SelectAllColumn, "select_all", "Enter");
        }
        // On a folded section header, Enter unfolds it. There is no task under
        // the cursor there for the detail panel to open.
        if self.cursor_is_on_folded_header() {
            return self.dispatch_keyed(
                Message::ToggleSectionCollapse,
                "toggle_section_collapse",
                "Enter",
            );
        }
        // Same reasoning, for a folded epic group.
        if self.cursor_is_on_folded_epic_header() {
            return self.dispatch_keyed(Message::ToggleEpicFold, "toggle_epic_fold", "Enter");
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

    /// `Esc` — clear an active search, exit epic view, clear selection, or no-op.
    fn handle_key_esc_normal(&mut self) -> Vec<Command> {
        if self.search_active() {
            self.search.query.clear();
            self.sync_board_selection();
            return vec![key_event("clear_search", "Esc")];
        }
        if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
            self.dispatch_keyed(
                Message::Epic(crate::tui::messages::EpicMessage::Exit),
                "exit_epic",
                "Esc",
            )
        } else if self.has_selection() || self.selection().on_select_all {
            self.dispatch_keyed(Message::ClearSelection, "clear_selection", "Esc")
        } else {
            vec![]
        }
    }

    pub(in crate::tui) fn handle_key_search(&mut self, key: KeyEvent) -> Vec<Command> {
        // Typing is not an action: only leaving the mode (either way) is
        // recorded, so the count reads as "searches run", not "characters
        // typed".
        let mut cmds = vec![];
        match key.code {
            KeyCode::Esc => {
                self.search.query = self.search.saved.take().unwrap_or_default();
                self.input.mode = InputMode::Normal;
                cmds.push(key_event("search_cancel", "Esc"));
            }
            KeyCode::Enter => {
                self.search.saved = None;
                self.input.mode = InputMode::Normal;
                cmds.push(key_event("search_commit", "Enter"));
            }
            KeyCode::Backspace => {
                self.search.query.pop();
            }
            KeyCode::Char(c) => {
                self.search.query.push(c);
            }
            _ => return vec![],
        }
        // Query may have changed → recompute filtered columns and re-clamp the cursor.
        self.sync_board_selection();
        cmds
    }
}
