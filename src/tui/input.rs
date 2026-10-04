mod confirm;
mod normal;
mod repo_filter;
mod table;

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, ColumnItem, Command, InputMode, Message, MoveDirection, ViewMode};
use crate::models::{DispatchMode, EpicId, TaskId, UsageActor, UsageCategory, UsageEvent};
use crate::tui::commands::UsageCommand;

fn key_event(action: &str, key: &str) -> Command {
    Command::Usage(UsageCommand::Record(UsageEvent {
        category: UsageCategory::Keybinding,
        action: action.to_string(),
        detail: Some(key.to_string()),
        actor: UsageActor::Human,
    }))
}

use crate::keybindings::key_label;

/// The tree movement a key requests in either tree picker (reparent-epic,
/// move-task-to-epic), or `None` for a key that is not a movement. Both
/// pickers navigate identically; only the message they wrap it in differs.
fn tree_nav_for(key: KeyEvent) -> Option<crate::tui::types::TreeNav> {
    use crate::tui::types::TreeNav;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(TreeNav::Down),
        KeyCode::Char('k') | KeyCode::Up => Some(TreeNav::Up),
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') => Some(TreeNav::Right),
        KeyCode::Char('h') | KeyCode::Left => Some(TreeNav::Left),
        _ => None,
    }
}

impl App {
    /// Dispatch `msg` through [`Self::update`], then record the keybinding usage
    /// event. Collapses the update-then-`key_event`-push pattern shared by the
    /// message-dispatch arms of every key handler into a single call, so those
    /// arms can't silently forget the telemetry push. Arms that delegate to a
    /// `handle_key_*` sub-handler use [`Self::dispatch_handler_keyed`] instead.
    pub(in crate::tui) fn dispatch_keyed(
        &mut self,
        msg: Message,
        action: &str,
        key: &str,
    ) -> Vec<Command> {
        let mut cmds = self.update(msg);
        cmds.push(key_event(action, key));
        cmds
    }

    /// Like [`Self::dispatch_handler_keyed`], for a handler whose effect can be
    /// opening a confirmation: that returns no commands but changes the input
    /// mode, and is still a keypress that took effect (`x`, `T`).
    pub(in crate::tui) fn dispatch_prompting_handler_keyed(
        &mut self,
        handler: impl FnOnce(&mut Self) -> Vec<Command>,
        action: &str,
        key: &str,
    ) -> Vec<Command> {
        let mode_before = self.input.mode.clone();
        let mut cmds = handler(self);
        if !cmds.is_empty() || self.input.mode != mode_before {
            cmds.push(key_event(action, key));
        }
        cmds
    }

    /// Run a `handle_key_*` sub-handler, then record the keybinding usage event
    /// only if the handler produced commands. Collapses the run-then-conditional-
    /// `key_event`-push pattern shared by the sub-handler arms (where a no-op
    /// handler must not emit telemetry), mirroring [`Self::dispatch_keyed`] for
    /// that cluster.
    pub(in crate::tui) fn dispatch_handler_keyed(
        &mut self,
        handler: impl FnOnce(&mut Self) -> Vec<Command>,
        action: &str,
        key: &str,
    ) -> Vec<Command> {
        let mut cmds = handler(self);
        if !cmds.is_empty() {
            cmds.push(key_event(action, key));
        }
        cmds
    }

    /// Translate a terminal key event into zero or more commands, depending on current mode.
    ///
    /// Always sets `self.dirty = true` after handling a key. An earlier revision tried to
    /// skip the redraw for no-op keys (e.g. `j` at the last row) by snapshotting which
    /// fields changed, but that opt-in mechanism proved fragile: popup/overlay handlers
    /// routinely mutate state invisible to the snapshot (tree-view open/collapse state,
    /// edit buffers, cursor positions in popups) and silently drop frames when they forget
    /// to set dirty themselves. The `frame_ready` 16ms cap already bounds the cost of
    /// redrawing on a true no-op, so unconditionally marking dirty is both correct and cheap.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Command> {
        // TEMPORARY: debugging a copy-task overlay that reportedly appears
        // without a 'c' keypress. Remove once root-caused.
        tracing::debug!(code = ?key.code, modifiers = ?key.modifiers, mode = ?self.input.mode, "handle_key");
        let cmds = self.dispatch_key(key);

        self.dirty = true;
        cmds
    }

    /// `Space` on a task card — the unified "activate task" action (see
    /// docs/specs/split-pane.allium: JumpToAgentWindow). Which rung of the
    /// ladder applies is the row's context (`task_on_other_machine` …
    /// `task_without_worktree`, evaluated by `App::context_holds`); this runs
    /// the rung's action. Split mode overrides only the jump, so a windowless
    /// card still dispatches or resumes with the pane open.
    pub(in crate::tui) fn run_activation(
        &mut self,
        b: &crate::keybindings::KeyBinding,
        label: &str,
    ) -> Vec<Command> {
        use crate::keybindings::KeyContext as C;
        let Some(ColumnItem::Task(task)) = self.selected_column_item() else {
            return vec![];
        };
        let id = task.id;
        let action = b.action;
        match b.context {
            Some(C::TaskOnOtherMachine) => self.dispatch_keyed(
                Message::System(crate::tui::messages::SystemMessage::StatusInfo(
                    crate::tui::foreign_worktree_refusal(None),
                )),
                action,
                label,
            ),
            Some(C::TaskPinnedInSplit) => {
                let Some(pane_id) = self.board.split.right_pane_id.clone() else {
                    return vec![];
                };
                vec![
                    Command::Split(crate::tui::commands::SplitCommand::FocusPane { pane_id }),
                    key_event(action, label),
                ]
            }
            Some(C::TaskWindowSplitOpen) => self.dispatch_handler_keyed(
                |app| app.update(Message::Split(crate::tui::messages::SplitMessage::Swap(id))),
                action,
                label,
            ),
            Some(C::TaskWithWindow) => {
                let Some(window) = task.tmux_window.clone() else {
                    return vec![];
                };
                vec![
                    Command::Task(crate::tui::commands::TaskCommand::JumpToTmux { window }),
                    key_event(action, label),
                ]
            }
            Some(C::BacklogTask) => {
                let mode = DispatchMode::for_task(task);
                let repo_path = task.repo_path.clone();
                vec![
                    Command::Task(crate::tui::commands::TaskCommand::CheckTrustAndDispatch {
                        id,
                        repo_path,
                        mode,
                    }),
                    key_event(action, label),
                ]
            }
            Some(C::StuckTask) => {
                // Windowless Stale/Crashed, or an unprovisioned Running task:
                // open the kill-and-retry dialog.
                let mut cmds = self.update(Message::Task(
                    crate::tui::messages::TaskMessage::KillAndRetry(id),
                ));
                cmds.push(key_event(action, label));
                cmds
            }
            Some(C::TaskDispatching) => {
                // Space did something — it answered — even though the answer
                // is "not yet". Counting it keeps the key's total honest.
                self.dispatch_keyed(
                    Message::System(crate::tui::messages::SystemMessage::StatusInfo(
                        "Dispatch in progress\u{2026}".to_string(),
                    )),
                    action,
                    label,
                )
            }
            Some(C::TaskWithWorktree) => {
                let mut cmds =
                    self.update(Message::Task(crate::tui::messages::TaskMessage::Resume(id)));
                cmds.push(key_event(action, label));
                cmds
            }
            Some(C::TaskWithoutWorktree) => self.dispatch_keyed(
                Message::System(crate::tui::messages::SystemMessage::StatusInfo(
                    "No worktree to resume, move to Backlog and re-dispatch".to_string(),
                )),
                action,
                label,
            ),
            _ => vec![],
        }
    }

    /// Handle the 'L'/'H' keys: move selected task(s) forward or backward.
    /// (`m` is the move-to-epic tree picker, not a status move.)
    pub(in crate::tui) fn handle_key_move(&mut self, direction: MoveDirection) -> Vec<Command> {
        if self.has_selection() {
            if self.select.tasks.is_empty() {
                // Only epics selected — can't move since status is derived
                return self.update(Message::System(
                    crate::tui::messages::SystemMessage::StatusInfo(
                        "Epic status is derived from subtasks".to_string(),
                    ),
                ));
            }
            let ids: Vec<_> = self.select.tasks.iter().copied().collect();
            self.update(Message::Task(
                crate::tui::messages::TaskMessage::BatchMove { ids, direction },
            ))
        } else if let Some(task) = self.selected_task() {
            let id = task.id;
            self.update(Message::Task(crate::tui::messages::TaskMessage::Move {
                id,
                direction,
            }))
        } else {
            vec![]
        }
    }

    /// `Enter` in a text-entry mode: submit the picker selection when the mode
    /// has one, otherwise the trimmed buffer, routed by mode.
    pub(in crate::tui) fn submit_text_input(&mut self) -> Vec<Command> {
        // In picker modes, Enter selects the item at the cursor position in
        // the effective list (filtered candidates + optional new entry at
        // the end) — see docs/specs/dispatch.allium: RepoPathPicker,
        // BaseBranchPicker.
        if let Some(candidates) = self.picker_candidates() {
            let selected = super::resolve_picker_selection(
                candidates,
                &self.input.buffer,
                self.input.repo_cursor,
            );
            if let Some(value) = selected {
                let msg = match self.input.mode {
                    InputMode::InputBaseBranch => {
                        Message::Input(crate::tui::messages::InputMessage::SubmitBaseBranch(value))
                    }
                    _ => Message::Input(crate::tui::messages::InputMessage::SubmitRepoPath(value)),
                };
                return self.update(msg);
            }
            // effective is empty — fall through to submit the empty buffer and
            // let the mode-specific submit handler apply its fallback/error.
        }
        let value = self.input.buffer.trim().to_string();
        match self.input.mode.clone() {
            InputMode::InputTitle => self.update(Message::Input(
                crate::tui::messages::InputMessage::SubmitTitle(value),
            )),
            InputMode::InputDescription => self.update(Message::Input(
                crate::tui::messages::InputMessage::SubmitDescription(value),
            )),
            InputMode::InputRepoPath => self.update(Message::Input(
                crate::tui::messages::InputMessage::SubmitRepoPath(value),
            )),
            InputMode::InputEpicTitle => self.update(Message::Epic(
                crate::tui::messages::EpicMessage::SubmitTitle(value),
            )),
            InputMode::InputEpicDescription => self.update(Message::Epic(
                crate::tui::messages::EpicMessage::SubmitDescription(value),
            )),
            InputMode::InputBaseBranch => self.update(Message::Input(
                crate::tui::messages::InputMessage::SubmitBaseBranch(value),
            )),
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
            Some(
                ColumnItem::EpicHeader(_)
                | ColumnItem::SubstatusLabel(_)
                | ColumnItem::FoldedSection(_)
                | ColumnItem::FoldedEpic(_)
                | ColumnItem::OrphanSeparator,
            ) => vec![],
            None => vec![],
        }
    }

    /// Returns the ID of the currently selected epic, or `None` if the cursor is not on an epic.
    /// Whether the cursor is resting on a folded section header. An expanded
    /// header cannot hold the cursor, so this is only ever true for a folded
    /// one.
    pub(in crate::tui) fn cursor_is_on_folded_header(&self) -> bool {
        matches!(
            self.selected_column_item(),
            Some(ColumnItem::FoldedSection(_))
        )
    }

    /// Whether the cursor is resting on a folded epic group's header, on the
    /// same terms as [`Self::cursor_is_on_folded_header`].
    pub(in crate::tui) fn cursor_is_on_folded_epic_header(&self) -> bool {
        matches!(self.selected_column_item(), Some(ColumnItem::FoldedEpic(_)))
    }

    pub(in crate::tui) fn selected_epic_id(&self) -> Option<EpicId> {
        match self.selected_column_item() {
            Some(ColumnItem::Epic(epic)) => Some(epic.id),
            _ => None,
        }
    }

    /// Returns the epic ID when inside an epic view, or `None` in board view.
    pub(in crate::tui) fn current_epic_id(&self) -> Option<EpicId> {
        match &self.board.view_mode {
            ViewMode::Epic { epic_id, .. } => Some(*epic_id),
            _ => None,
        }
    }
}
