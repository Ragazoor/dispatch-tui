//! Key dispatch through the keybinding table
//! (`KeypressRunsItsRowsAction` / `KeyWithoutARowCannotAct` in
//! `docs/specs/keybindings.allium`).
//!
//! A press is answered by finding the row in the active namespace that lists
//! the key and whose context holds, then running that row's action id. The
//! runners below map an action id to its effect; which key runs which action
//! is the table's business alone, so rebinding a key is an edit to one row.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};

use super::super::types::*;
use super::super::{App, GG_CHORD_TIMEOUT};
use super::{key_event, key_label, tree_nav_for};
use crate::keybindings::{
    key_has_ctrl_or_alt, key_name, lookup, KeyBinding, KeyContext, KeyNamespace,
};
use crate::models::{SubStatus, TaskStatus, TaskTag};

impl App {
    /// The namespace a key press is looked up in right now.
    pub(in crate::tui) fn key_namespace(&self) -> KeyNamespace {
        use KeyNamespace as N;
        if self.status.error_popup.is_some() {
            return N::BoardError;
        }
        match &self.input.mode {
            InputMode::Normal => {
                if matches!(self.board.view_mode, ViewMode::TaskDetail { .. }) {
                    N::BoardDetail
                } else {
                    N::BoardNormal
                }
            }
            InputMode::SearchTasks => N::BoardSearch,
            InputMode::InputTitle
            | InputMode::InputDescription
            | InputMode::InputEpicTitle
            | InputMode::InputEpicDescription => N::BoardText,
            InputMode::InputRepoPath => N::BoardPickerRepoPath,
            InputMode::InputBaseBranch => N::BoardPickerBaseBranch,
            InputMode::InputTag => N::BoardPickerTag,
            InputMode::InputWrapUpMode => N::BoardPickerWrapUpMode,
            InputMode::QuickDispatch => N::BoardPickerQuickDispatch,
            InputMode::MoveTaskToEpic(_) => N::BoardPickerMoveToEpic,
            InputMode::ReparentEpic(_) => N::BoardPickerReparentEpic,
            InputMode::Help => N::BoardHelp,
            InputMode::RepoFilter => N::BoardRepoFilter,
            InputMode::ConfirmQuit => N::BoardConfirmQuit,
            InputMode::ConfirmDeleteTask(_) => N::BoardConfirmDeleteTask,
            InputMode::ConfirmBatchDelete => N::BoardConfirmBatchDelete,
            InputMode::ConfirmDeleteEpic => N::BoardConfirmDeleteEpic,
            InputMode::ConfirmDone => N::BoardConfirmDone,
            InputMode::ConfirmRetry(_) => N::BoardConfirmRetry,
            InputMode::ConfirmDetachTmux(_) => N::BoardConfirmDetachTmux,
            InputMode::ConfirmOverrideFeedOwner { .. } => N::BoardConfirmOverrideFeedOwner,
            InputMode::ConfirmMoveTaskToEpic { .. } => N::BoardConfirmMoveToEpic,
            InputMode::ConfirmReparentEpic { .. } => N::BoardConfirmReparentEpic,
            InputMode::ConfirmDeleteRepoPath => N::BoardConfirmDeleteRepoPath,
            InputMode::ConfirmTrustRepo { .. } => N::BoardConfirmTrustRepo,
            InputMode::ConfirmTrustRepoQuickDispatch { .. } => {
                N::BoardConfirmTrustRepoQuickDispatch
            }
            InputMode::ConfirmRepoSync { .. } => N::BoardConfirmRepoSync,
        }
    }

    /// Which rung of Space's activation ladder the card under the cursor is on
    /// (the `task_*` contexts), or `None` when the cursor is not on a task.
    /// One evaluation yields one rung, so the contexts are mutually exclusive
    /// by construction.
    fn activation_context(&self) -> Option<KeyContext> {
        use KeyContext as C;
        let Some(ColumnItem::Task(task)) = self.selected_column_item() else {
            return None;
        };
        let local_host_id = self.local_host_id();
        if !task.is_locally_owned(local_host_id) {
            return Some(C::TaskOnOtherMachine);
        }
        let split = &self.board.split;
        if split.active && split.pinned_task_id == Some(task.id) && split.right_pane_id.is_some() {
            return Some(C::TaskPinnedInSplit);
        }
        if task.tmux_window.is_some() {
            return Some(if split.active {
                C::TaskWindowSplitOpen
            } else {
                C::TaskWithWindow
            });
        }
        if task.status == TaskStatus::Backlog {
            return Some(C::BacklogTask);
        }
        let now = chrono::Utc::now();
        let in_flight = self.dispatch_may_be_in_flight(task, now);
        // Stale/Crashed, or Running with nothing provisioned behind it and no
        // dispatch possibly in flight. See RetryReachableInPlace in
        // docs/specs/dispatch.allium.
        let stuck = task.sub_status == SubStatus::Stale
            || task.sub_status == SubStatus::Crashed
            || (task.status == TaskStatus::Running && task.is_unprovisioned() && !in_flight);
        if stuck {
            return Some(C::StuckTask);
        }
        let has_worktree = task.worktree.is_some();
        if !has_worktree && in_flight {
            return Some(C::TaskDispatching);
        }
        Some(if has_worktree {
            C::TaskWithWorktree
        } else {
            C::TaskWithoutWorktree
        })
    }

    /// Whether one [`KeyContext`] holds against the board right now. The drift
    /// gate uses the same evaluation to set a row's context up, so a row and
    /// its handler cannot read a condition differently.
    #[cfg(test)]
    pub(in crate::tui) fn context_holds(&self, context: KeyContext) -> bool {
        self.context_holds_cached(context, &std::cell::OnceCell::new())
    }

    /// [`Self::context_holds`] with the activation rung memoised in `rung`, so
    /// a lookup that tests several `task_*` rows rebuilds the column once.
    fn context_holds_cached(
        &self,
        context: KeyContext,
        rung: &std::cell::OnceCell<Option<KeyContext>>,
    ) -> bool {
        use KeyContext as C;
        let in_epic = matches!(self.board.view_mode, ViewMode::Epic { .. });
        match context {
            C::SearchActive => self.search_active(),
            C::InsideEpicView => in_epic,
            C::EpicViewNoSearch => in_epic && !self.search_active(),
            C::TopLevel => !in_epic,
            C::WithSelection => {
                !in_epic
                    && !self.search_active()
                    && (self.has_selection() || self.selection().on_select_all)
            }
            C::OnColumnSelectAll => self.selection().on_select_all,
            C::OnFoldedSection => self.cursor_is_on_folded_header(),
            C::OnFoldedEpicGroup => self.cursor_is_on_folded_epic_header(),
            C::OnEpicCard => self.selected_epic_id().is_some(),
            C::OnTaskCard => matches!(self.selected_column_item(), Some(ColumnItem::Task(_))),
            C::OnFlattenedTaskCard | C::OnUnflattenedTaskCard => {
                self.cursor_in_flattened_column() == (context == C::OnFlattenedTaskCard)
                    && matches!(self.selected_column_item(), Some(ColumnItem::Task(_)))
            }
            C::OffEpicCard => self.selected_epic_id().is_none(),
            // Agent-tree pane contexts: the board has no file tree.
            C::OnDirectory | C::OnFile => false,
            C::TaskOnOtherMachine
            | C::TaskPinnedInSplit
            | C::TaskWindowSplitOpen
            | C::TaskWithWindow
            | C::BacklogTask
            | C::StuckTask
            | C::TaskDispatching
            | C::TaskWithWorktree
            | C::TaskWithoutWorktree => {
                *rung.get_or_init(|| self.activation_context()) == Some(context)
            }
            C::DetailZoomed => matches!(
                self.board.view_mode,
                ViewMode::TaskDetail { zoomed: true, .. }
            ),
        }
    }

    /// Replace the table the board looks keys up in. Production boards use
    /// [`crate::keybindings::KEY_BINDINGS`]; tests install a modified copy to
    /// show that rebinding is an edit to one row.
    #[cfg(test)]
    pub(in crate::tui) fn set_key_table(&mut self, table: &'static [KeyBinding]) {
        self.key_table = table;
    }

    /// Answer one key press: look it up in the active namespace and run the
    /// row's action, or — when no row applies — let a text-bearing mode take
    /// the key as data, or ignore it.
    pub(in crate::tui) fn dispatch_key(&mut self, key: KeyEvent) -> Vec<Command> {
        let ns = self.key_namespace();
        let mut name = key_name(key);
        let mut label = key_label(key);
        if ns != KeyNamespace::BoardNormal {
            self.interaction.pending_g = None;
        } else if let Some(started) = self.interaction.pending_g.take() {
            // A pending `g` resolves with this press: a second `g` inside the
            // window completes the `gg` chord, which is looked up as one key;
            // anything else abandons the chord and is processed normally.
            if name == "g" && started.elapsed() <= GG_CHORD_TIMEOUT {
                name = "gg".to_string();
                label = "gg".to_string();
            }
        }
        if ns == KeyNamespace::BoardNormal
            && name == "g"
            && self
                .key_table
                .iter()
                .any(|b| b.namespace == ns && b.keys.contains(&"gg"))
        {
            // First half of a chord: pending input, not a lookup.
            self.interaction.pending_g = Some(Instant::now());
            return vec![];
        }

        let table = self.key_table;
        let rung = std::cell::OnceCell::new();
        let binding = lookup(table, ns, &name, |c| self.context_holds_cached(c, &rung));
        match binding {
            Some(b) => self.run_row(ns, b, key, &label),
            None => self.unbound_key(ns, key),
        }
    }

    /// A key no applicable row lists. Text-bearing modes take it as data;
    /// everything else ignores it (`KeyWithoutARowCannotAct`).
    fn unbound_key(&mut self, ns: KeyNamespace, key: KeyEvent) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        use KeyNamespace as N;
        match ns {
            N::BoardSearch => {
                // Typing is not an action; nothing is recorded.
                match key.code {
                    KeyCode::Char(c) if !key_has_ctrl_or_alt(key) => self.search.query.push(c),
                    _ => return vec![],
                }
                self.sync_board_selection();
                vec![]
            }
            N::BoardText
            | N::BoardPickerRepoPath
            | N::BoardPickerBaseBranch
            | N::BoardPickerQuickDispatch => match key.code {
                KeyCode::Char(c) if !key_has_ctrl_or_alt(key) => {
                    self.update(Message::Input(InputMessage::InputChar(c)))
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// A text-editing row (`records_usage` false): applies the edit to the
    /// field and records nothing, like the typed characters it accompanies.
    fn run_text_edit(&mut self, ns: KeyNamespace, action: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage as I;
        if ns == KeyNamespace::BoardSearch {
            if action == "text_backspace" {
                self.search.query.pop();
                self.sync_board_selection();
            }
            return vec![];
        }
        let msg = match action {
            "text_backspace" => I::InputBackspace,
            "text_delete_forward" => I::InputDeleteForward,
            "text_cursor_left" => I::CursorLeft,
            "text_cursor_right" => I::CursorRight,
            "text_cursor_home" => I::CursorHome,
            "text_cursor_end" => I::CursorEnd,
            "text_cursor_word_left" => I::CursorWordLeft,
            "text_cursor_word_right" => I::CursorWordRight,
            _ => return vec![],
        };
        self.update(Message::Input(msg))
    }

    /// `L` / `H`: an epic card moves that epic's status, otherwise the
    /// selection (or the card) moves.
    pub(in crate::tui) fn move_key(
        &mut self,
        direction: MoveDirection,
        action: &'static str,
        label: &str,
    ) -> Vec<Command> {
        if let Some(id) = self.selected_epic_id() {
            return self.dispatch_keyed(
                Message::Epic(crate::tui::messages::EpicMessage::MoveStatus(id, direction)),
                action,
                label,
            );
        }
        // A press that moves nothing (no selection and no card under the
        // cursor) took no effect and records nothing.
        let mut cmds = self.handle_key_move(direction);
        if !cmds.is_empty() {
            cmds.push(key_event(action, label));
        }
        cmds
    }

    /// Run the action of the row a key resolved to.
    fn run_row(
        &mut self,
        ns: KeyNamespace,
        b: &'static KeyBinding,
        key: KeyEvent,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::SystemMessage;
        use KeyNamespace as N;
        let action = b.action;
        if !b.records_usage {
            return self.run_text_edit(ns, action);
        }
        match ns {
            N::BoardNormal => self.run_normal(b, key, label),
            N::BoardDetail => self.run_detail(action, key, label),
            N::BoardSearch => self.run_search(action, label),
            N::BoardHelp => self.run_help(action, key, label),
            N::BoardError => {
                self.dispatch_keyed(Message::System(SystemMessage::DismissError), action, label)
            }
            N::BoardText | N::BoardPickerRepoPath | N::BoardPickerBaseBranch => {
                self.run_text_mode(action, key, label)
            }
            N::BoardRepoFilter => self.handle_key_repo_filter(key, action, label),
            N::BoardPickerTag => self.run_tag_picker(action, key, label),
            N::BoardPickerWrapUpMode => self.run_wrap_up_picker(action, key, label),
            N::BoardPickerQuickDispatch => self.run_quick_dispatch_picker(action, key, label),
            N::BoardPickerMoveToEpic => self.run_move_to_epic_picker(action, key, label),
            N::BoardPickerReparentEpic => self.run_reparent_picker(action, key, label),
            N::BoardConfirmMoveToEpic => {
                use crate::tui::messages::TaskMessage::*;
                let msg = match action {
                    "confirm_move_task_to_epic_yes" => MoveToEpicExecute,
                    "confirm_move_task_to_epic_no" => MoveToEpicCancel,
                    _ => MoveToEpicCancelAll,
                };
                self.dispatch_keyed(Message::Task(msg), action, label)
            }
            N::BoardConfirmReparentEpic => {
                use crate::tui::messages::EpicMessage::*;
                let msg = match action {
                    "confirm_reparent_epic_yes" => ReparentExecute,
                    "confirm_reparent_epic_no" => ReparentCancel,
                    _ => ReparentCancelAll,
                };
                self.dispatch_keyed(Message::Epic(msg), action, label)
            }
            N::BoardConfirmRetry => match self.input.mode.clone() {
                InputMode::ConfirmRetry(id) => self.handle_key_confirm_retry(label, action, id),
                _ => vec![],
            },
            N::BoardConfirmQuit
            | N::BoardConfirmDeleteTask
            | N::BoardConfirmBatchDelete
            | N::BoardConfirmDeleteEpic
            | N::BoardConfirmDone
            | N::BoardConfirmDetachTmux
            | N::BoardConfirmOverrideFeedOwner
            | N::BoardConfirmDeleteRepoPath
            | N::BoardConfirmTrustRepo
            | N::BoardConfirmTrustRepoQuickDispatch
            | N::BoardConfirmRepoSync => self.run_confirm(action.ends_with("_yes"), label),
            N::AgentTreeTree
            | N::AgentTreeCommits
            | N::AgentTreeAgents
            | N::AgentDiff
            | N::TmuxGlobal => vec![],
        }
    }

    /// The task detail view: close, scroll, zoom.
    fn run_detail(&mut self, action: &'static str, key: KeyEvent, label: &str) -> Vec<Command> {
        use crate::tui::messages::TaskMessage;
        match action {
            "close_detail" => {
                self.dispatch_keyed(Message::Task(TaskMessage::CloseDetail), action, label)
            }
            "scroll_detail" => {
                let down = matches!(key.code, KeyCode::Char('j') | KeyCode::Down);
                if let ViewMode::TaskDetail {
                    scroll, max_scroll, ..
                } = &mut self.board.view_mode
                {
                    *scroll = if down {
                        scroll.saturating_add(1).min(*max_scroll)
                    } else {
                        scroll.saturating_sub(1)
                    };
                }
                vec![key_event(action, label)]
            }
            "zoom_detail" => {
                if let ViewMode::TaskDetail { zoomed, .. } = &mut self.board.view_mode {
                    *zoomed = !*zoomed;
                }
                vec![key_event(action, label)]
            }
            _ => vec![],
        }
    }

    /// The `/` search prompt: commit keeps the query, cancel restores the saved one.
    fn run_search(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        match action {
            "search_cancel" => {
                self.search.query = self.search.saved.take().unwrap_or_default();
                self.input.mode = InputMode::Normal;
            }
            "search_commit" => {
                self.search.saved = None;
                self.input.mode = InputMode::Normal;
            }
            _ => return vec![],
        }
        // The query may have changed: recompute filtered columns.
        self.sync_board_selection();
        vec![key_event(action, label)]
    }

    /// The help overlay: close and scroll.
    fn run_help(&mut self, action: &'static str, key: KeyEvent, label: &str) -> Vec<Command> {
        use crate::tui::messages::SystemMessage;
        match action {
            "close_help" => {
                self.dispatch_keyed(Message::System(SystemMessage::ToggleHelp), action, label)
            }
            "scroll_help" => {
                let down = matches!(key.code, KeyCode::Char('j') | KeyCode::Down);
                let max = self.interaction.help_max_scroll.get().unwrap_or(usize::MAX);
                let now = self.interaction.help_scroll;
                self.interaction.help_scroll = if down {
                    now.saturating_add(1).min(max)
                } else {
                    now.saturating_sub(1)
                };
                vec![key_event(action, label)]
            }
            _ => vec![],
        }
    }

    /// The text-entry modes and the repo / base-branch pickers.
    fn run_text_mode(&mut self, action: &'static str, key: KeyEvent, label: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        match action {
            "picker_move_cursor" => self.move_repo_cursor(action, key, label),
            "cancel_input" => {
                self.dispatch_keyed(Message::Input(InputMessage::CancelInput), action, label)
            }
            "submit_input" => {
                // Typing is data entry, not a keybinding use: only the
                // commit and the cancel of a text mode are recorded.
                let mut cmds = self.submit_text_input();
                cmds.push(key_event(action, label));
                cmds
            }
            _ => vec![],
        }
    }

    /// Move the repo list cursor one row for an Up / Down press.
    fn move_repo_cursor(
        &mut self,
        action: &'static str,
        key: KeyEvent,
        label: &str,
    ) -> Vec<Command> {
        let delta = if key.code == KeyCode::Down { 1 } else { -1 };
        self.dispatch_keyed(
            Message::RepoFilter(crate::tui::messages::RepoFilterMessage::MoveCursor(delta)),
            action,
            label,
        )
    }

    /// The quick-dispatch repo picker.
    fn run_quick_dispatch_picker(
        &mut self,
        action: &'static str,
        key: KeyEvent,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        match action {
            "quick_dispatch_cancel" => {
                self.dispatch_keyed(Message::Input(InputMessage::CancelInput), action, label)
            }
            "quick_dispatch_move_cursor" => self.move_repo_cursor(action, key, label),
            "quick_dispatch_select" => {
                let idx = self.input.repo_cursor;
                self.dispatch_keyed(
                    Message::Input(InputMessage::SelectQuickDispatchRepo(idx)),
                    action,
                    label,
                )
            }
            _ => vec![],
        }
    }

    /// The "move task to epic" tree picker.
    fn run_move_to_epic_picker(
        &mut self,
        action: &'static str,
        key: KeyEvent,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::TaskMessage::*;
        let msg = match action {
            "move_to_epic_picker_navigate" => match tree_nav_for(key) {
                Some(nav) => MoveToEpicNavigate(nav),
                None => return vec![],
            },
            "move_to_epic_picker_confirm" => MoveToEpicConfirm,
            "move_to_epic_picker_cancel" => MoveToEpicCancel,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Task(msg), action, label)
    }

    /// The "reparent epic" tree picker.
    fn run_reparent_picker(
        &mut self,
        action: &'static str,
        key: KeyEvent,
        label: &str,
    ) -> Vec<Command> {
        use crate::tui::messages::EpicMessage::*;
        let msg = match action {
            "reparent_picker_navigate" => match tree_nav_for(key) {
                Some(nav) => ReparentNavigate(nav),
                None => return vec![],
            },
            "reparent_picker_confirm" => ReparentConfirm,
            "reparent_picker_cancel" => ReparentCancel,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Epic(msg), action, label)
    }

    /// The yes / no confirmation modes. The input mode says which confirmation
    /// is open; the key only says yes or no.
    fn run_confirm(&mut self, yes: bool, label: &str) -> Vec<Command> {
        match self.input.mode.clone() {
            InputMode::ConfirmQuit => self.handle_key_confirm_quit(label, yes),
            InputMode::ConfirmDeleteTask(id) => self.handle_key_confirm_delete_task(label, yes, id),
            InputMode::ConfirmBatchDelete => self.handle_key_confirm_batch_delete(label, yes),
            InputMode::ConfirmDeleteEpic => self.handle_key_confirm_delete_epic(label, yes),
            InputMode::ConfirmDone => self.handle_key_confirm_done(label, yes),
            InputMode::ConfirmDetachTmux(_) => self.handle_key_confirm_detach_tmux(label, yes),
            InputMode::ConfirmOverrideFeedOwner { .. } => {
                self.handle_key_confirm_override_feed_owner(label, yes)
            }
            InputMode::ConfirmDeleteRepoPath => {
                self.handle_key_confirm_delete_repo_path(label, yes)
            }
            InputMode::ConfirmTrustRepo { task_id, mode } => {
                self.handle_key_confirm_trust_repo(label, yes, task_id, mode)
            }
            InputMode::ConfirmTrustRepoQuickDispatch { draft, epic_id } => {
                self.handle_key_confirm_trust_repo_quick_dispatch(label, yes, draft, epic_id)
            }
            InputMode::ConfirmRepoSync { repo_path } => {
                self.handle_key_confirm_repo_sync(label, yes, repo_path)
            }
            _ => vec![],
        }
    }

    /// The creation form's tag step. `p` arms phoenix and does not pick a tag;
    /// a second `p` finds it armed and does nothing (CreateTask:
    /// PhoenixArming, `docs/specs/tasks.allium`). Every other accepted key is
    /// a letter of the label it selects.
    fn run_tag_picker(&mut self, action: &str, key: KeyEvent, label: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        match action {
            "phoenix_arm" => {
                if self.input.phoenix_armed() {
                    return vec![];
                }
                self.dispatch_keyed(
                    Message::Input(InputMessage::ArmPhoenix),
                    "phoenix_arm",
                    label,
                )
            }
            "tag_picker_select" => {
                let KeyCode::Char(c) = key.code else {
                    return vec![];
                };
                let tag = match c {
                    'b' => TaskTag::Bug,
                    'f' => TaskTag::Feature,
                    'c' => TaskTag::Chore,
                    'v' => TaskTag::PrReview,
                    'r' => TaskTag::Research,
                    'x' => TaskTag::Fix,
                    _ => return vec![],
                };
                self.dispatch_keyed(
                    Message::Input(InputMessage::SubmitTag(Some(tag))),
                    "tag_picker_select",
                    label,
                )
            }
            "tag_picker_default" => self.dispatch_keyed(
                Message::Input(InputMessage::SubmitTag(None)),
                "tag_picker_default",
                label,
            ),
            "tag_picker_cancel" => self.dispatch_keyed(
                Message::Input(InputMessage::CancelInput),
                "tag_picker_cancel",
                label,
            ),
            _ => vec![],
        }
    }

    fn run_wrap_up_picker(&mut self, action: &str, key: KeyEvent, label: &str) -> Vec<Command> {
        use crate::models::WrapUpMode;
        use crate::tui::messages::InputMessage;
        match action {
            "wrap_up_mode_picker_select" => {
                let KeyCode::Char(c) = key.code else {
                    return vec![];
                };
                let mode = match c {
                    'r' => WrapUpMode::Rebase,
                    'p' => WrapUpMode::Pr,
                    'd' => WrapUpMode::Done,
                    _ => return vec![],
                };
                self.dispatch_keyed(
                    Message::Input(InputMessage::SubmitWrapUpMode(Some(mode))),
                    "wrap_up_mode_picker_select",
                    label,
                )
            }
            "wrap_up_mode_picker_default" => self.dispatch_keyed(
                Message::Input(InputMessage::SubmitWrapUpMode(None)),
                "wrap_up_mode_picker_default",
                label,
            ),
            "wrap_up_mode_picker_cancel" => self.dispatch_keyed(
                Message::Input(InputMessage::CancelInput),
                "wrap_up_mode_picker_cancel",
                label,
            ),
            _ => vec![],
        }
    }
}
