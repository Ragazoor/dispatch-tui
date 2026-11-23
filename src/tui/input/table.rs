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
use super::super::{App, PendingChord, CHORD_TIMEOUT};
use super::key_event;
use crate::keybindings::{
    key_has_ctrl_or_alt, lookup, resolve_chord, KeyBinding, KeyContext, KeyNamespace,
};
use crate::models::{SubStatus, TaskStatus, TaskTag};

impl App {
    /// The namespace a key press is looked up in right now.
    pub(in crate::tui) fn key_namespace(&self) -> KeyNamespace {
        use KeyNamespace as N;
        if self.error_popup().is_some() {
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
            InputMode::InputDescription | InputMode::InputEpicDescription => {
                N::BoardDescriptionEditor
            }
            InputMode::InputTitle
            | InputMode::InputEpicTitle
            | InputMode::InputWorkPackageTitle(_)
            | InputMode::RenameWorkPackage(_)
            | InputMode::InputDisplayName
            | InputMode::InputAdminText => N::BoardText,
            InputMode::LinkWorkPackage { .. } => N::BoardPickerWorkPackage,
            InputMode::InputPlacement => N::BoardPickerPlacement,
            InputMode::Admin => N::BoardAdmin,
            InputMode::ConfirmAdmin => N::BoardConfirmAdmin,
            InputMode::ConfirmDeleteWorkPackage(_) => N::BoardConfirmDeleteWorkPackage,
            InputMode::InputRepoPath => N::BoardPickerRepoPath,
            InputMode::InputBaseBranch => N::BoardPickerBaseBranch,
            InputMode::InputTag => N::BoardPickerTag,
            InputMode::InputWrapUpMode => N::BoardPickerWrapUpMode,
            InputMode::InputModel => N::BoardPickerModel,
            InputMode::QuickDispatch => N::BoardPickerQuickDispatch,
            InputMode::MoveTaskToEpic(_) => N::BoardPickerMoveToEpic,
            InputMode::ReparentEpic(_) => N::BoardPickerReparentEpic,
            InputMode::Help => N::BoardHelp,
            InputMode::BoardFilter => N::BoardFilter,
            InputMode::BoardFilterSearch => N::BoardFilterSearch,
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
        }
    }

    /// Which rung of Space's activation ladder the card under the cursor is on
    /// (the `task_*` contexts), or `None` when the cursor is not on a task.
    /// One evaluation yields one rung, so the contexts are mutually exclusive
    /// by construction.
    pub(in crate::tui) fn activation_context(&self) -> Option<KeyContext> {
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

    /// `Self::context_holds` with the activation rung memoised in `rung`, so
    /// a lookup that tests several `task_*` rows rebuilds the column once.
    fn context_holds_cached(
        &self,
        context: KeyContext,
        rung: &std::cell::OnceCell<Option<KeyContext>>,
    ) -> bool {
        use KeyContext as C;
        // An epic view or a package view: one level below the main board,
        // left by the same keys (`work-packages.allium: ExitPackage`). Only
        // an epic view is an epic, which the epic settings keys need.
        let in_epic = matches!(self.board.view_mode, ViewMode::Epic { .. });
        let nested = in_epic || matches!(self.board.view_mode, ViewMode::Package { .. });
        match context {
            C::SearchActive => self.view().search_active(),
            C::InsideEpicView => in_epic,
            C::InsideNestedView => nested,
            C::NestedViewNoSearch => nested && !self.view().search_active(),
            C::TopLevel => !nested,
            C::WithSelection => {
                !nested
                    && !self.view().search_active()
                    && (self.has_selection() || self.selection().on_select_all)
            }
            C::OnColumnSelectAll => self.selection().on_select_all,
            C::OnFoldedSection => self.cursor_is_on_folded_header(),
            C::OnFoldedEpicGroup => self.cursor_is_on_folded_epic_header(),
            C::OnEpicCard => self.selected_epic_id().is_some(),
            C::OnWorkPackageCard => self.selected_work_package_id().is_some(),
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

    /// Install the table this board loaded at startup (`LoadKeyTable` in
    /// `docs/specs/keybindings.allium`). A load that refused anything opens
    /// the error popup with what it refused (`BoardShowsKeyLoadErrors`); the
    /// popup's one row is a catch-all, so any key dismisses it.
    pub fn install_key_table(&mut self, load: &crate::keybindings::KeyTableLoad) {
        self.key_table = load.table;
        if let Some(summary) = load.error_summary() {
            self.status.key_load_notice = Some(summary);
            self.dirty = true;
        }
    }

    /// Replace the table the board looks keys up in. Tests install a
    /// modified copy to show that rebinding is an edit to one row.
    #[cfg(test)]
    pub(in crate::tui) fn set_key_table(&mut self, table: &'static [KeyBinding]) {
        self.key_table = table;
    }

    /// Answer one key press: look it up in the active namespace and run the
    /// row's action, or — when no row applies — let a text-bearing mode take
    /// the key as data, or ignore it.
    pub(in crate::tui) fn dispatch_key(&mut self, key: KeyEvent) -> Vec<Command> {
        let ns = self.key_namespace();
        let table = self.key_table;
        // A pending first half counts only in the namespace it was pressed in
        // and inside the chord window; past that it is abandoned and this
        // press is processed normally.
        let mut pending = self
            .interaction
            .pending_chord
            .take()
            .filter(|p| p.namespace == ns && p.started.elapsed() <= CHORD_TIMEOUT)
            .map(|p| p.first);
        let Some((name, label)) = resolve_chord(&mut pending, table, ns, key) else {
            // First half of a chord: pending input, not a lookup.
            self.interaction.pending_chord = pending.map(|first| PendingChord {
                namespace: ns,
                first,
                started: Instant::now(),
            });
            return vec![];
        };

        let rung = std::cell::OnceCell::new();
        let binding = lookup(table, ns, &name, |c| self.context_holds_cached(c, &rung));
        match binding {
            Some(b) => self.run_row(ns, b, &label),
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
            N::BoardFilterSearch => match key.code {
                // Typing is not an action; nothing is recorded.
                KeyCode::Char(c) if !key_has_ctrl_or_alt(key) => self.update(Message::BoardFilter(
                    crate::tui::messages::BoardFilterMessage::EpicQueryChar(c),
                )),
                _ => vec![],
            },
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
        if ns == KeyNamespace::BoardFilterSearch {
            if action == "text_backspace" {
                return self.update(Message::BoardFilter(
                    crate::tui::messages::BoardFilterMessage::EpicQueryBackspace,
                ));
            }
            return vec![];
        }
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
    fn run_row(&mut self, ns: KeyNamespace, b: &'static KeyBinding, label: &str) -> Vec<Command> {
        use crate::tui::messages::SystemMessage;
        use KeyNamespace as N;
        let action = b.action;
        if !b.records_usage {
            return self.run_text_edit(ns, action);
        }
        match ns {
            N::BoardNormal => self.run_normal(b, label),
            N::BoardDetail => self.run_detail(action, label),
            N::BoardSearch => self.run_search(action, label),
            N::BoardHelp => self.run_help(action, label),
            N::BoardError => {
                self.dispatch_keyed(Message::System(SystemMessage::DismissError), action, label)
            }
            N::BoardText
            | N::BoardDescriptionEditor
            | N::BoardPickerRepoPath
            | N::BoardPickerBaseBranch => self.run_text_mode(action, label),
            N::BoardFilter => self.handle_key_board_filter(action, label),
            N::BoardFilterSearch => self.handle_key_board_filter_search(action, label),
            N::BoardPickerTag => self.run_tag_picker(action, label),
            N::BoardPickerWrapUpMode => self.run_wrap_up_picker(action, label),
            N::BoardPickerModel => self.run_model_picker(action, label),
            N::BoardPickerQuickDispatch => self.run_quick_dispatch_picker(action, label),
            N::BoardPickerMoveToEpic => self.run_move_to_epic_picker(action, label),
            N::BoardPickerReparentEpic => self.run_reparent_picker(action, label),
            N::BoardPickerWorkPackage => self.run_work_package_picker(action, label),
            N::BoardPickerPlacement => self.run_placement_picker(action, label),
            N::BoardAdmin => self.run_admin(action, label),
            N::BoardConfirmAdmin => self.dispatch_keyed(
                Message::Admin(crate::tui::messages::AdminMessage::Confirm(
                    action.ends_with("_yes"),
                )),
                action,
                label,
            ),
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
            | N::BoardConfirmDeleteWorkPackage => self.run_confirm(action.ends_with("_yes"), label),
            N::AgentTreeTree
            | N::AgentTreeCommits
            | N::AgentTreeAgents
            | N::AgentDiff
            | N::TmuxGlobal => vec![],
        }
    }

    /// The task detail view: close, scroll, zoom.
    fn run_detail(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::TaskMessage;
        match action {
            "close_detail" => {
                self.dispatch_keyed(Message::Task(TaskMessage::CloseDetail), action, label)
            }
            "scroll_detail_down" => self.scroll_detail(true, action, label),
            "scroll_detail_up" => self.scroll_detail(false, action, label),
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
    fn run_help(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::SystemMessage;
        match action {
            "close_help" => {
                self.dispatch_keyed(Message::System(SystemMessage::ToggleHelp), action, label)
            }
            "scroll_help_down" => {
                let max = self.interaction.help_max_scroll.get().unwrap_or(usize::MAX);
                self.interaction.help_scroll =
                    self.interaction.help_scroll.saturating_add(1).min(max);
                vec![key_event(action, label)]
            }
            "scroll_help_up" => {
                self.interaction.help_scroll = self.interaction.help_scroll.saturating_sub(1);
                vec![key_event(action, label)]
            }
            _ => vec![],
        }
    }

    /// Scroll the task detail one line down (`down`) or up, within its range.
    fn scroll_detail(&mut self, down: bool, action: &'static str, label: &str) -> Vec<Command> {
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

    /// The text-entry modes and the repo / base-branch pickers.
    fn run_text_mode(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        match action {
            "picker_cursor_down" => self.move_repo_cursor(1, action, label),
            "picker_cursor_up" => self.move_repo_cursor(-1, action, label),
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

    /// Move the repo list cursor one row: `delta` is 1 for a `_down` row and
    /// -1 for an `_up` row.
    fn move_repo_cursor(
        &mut self,
        delta: isize,
        action: &'static str,
        label: &str,
    ) -> Vec<Command> {
        self.dispatch_keyed(
            Message::BoardFilter(crate::tui::messages::BoardFilterMessage::MoveCursor(delta)),
            action,
            label,
        )
    }

    /// The quick-dispatch repo picker.
    fn run_quick_dispatch_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        match action {
            "quick_dispatch_cancel" => {
                self.dispatch_keyed(Message::Input(InputMessage::CancelInput), action, label)
            }
            "quick_dispatch_cursor_down" => self.move_repo_cursor(1, action, label),
            "quick_dispatch_cursor_up" => self.move_repo_cursor(-1, action, label),
            "quick_dispatch_select" => {
                let idx = self.input.repo_cursor;
                // A cursor past the list with no typed new repo selects
                // nothing: the press records only when it dispatched.
                self.dispatch_prompting_handler_keyed(
                    |a| a.update(Message::Input(InputMessage::SelectQuickDispatchRepo(idx))),
                    action,
                    label,
                )
            }
            _ => vec![],
        }
    }

    /// The "move task to epic" tree picker.
    fn run_move_to_epic_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::TaskMessage::*;
        use crate::tui::types::TreeNav;
        let msg = match action {
            "move_to_epic_picker_down" => MoveToEpicNavigate(TreeNav::Down),
            "move_to_epic_picker_up" => MoveToEpicNavigate(TreeNav::Up),
            "move_to_epic_picker_expand" => MoveToEpicNavigate(TreeNav::Right),
            "move_to_epic_picker_collapse" => MoveToEpicNavigate(TreeNav::Left),
            "move_to_epic_picker_confirm" => MoveToEpicConfirm,
            "move_to_epic_picker_cancel" => MoveToEpicCancel,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Task(msg), action, label)
    }

    /// The work-package link picker (`WorkPackageLinkPicker`).
    fn run_work_package_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::WorkPackageMessage::*;
        let msg = match action {
            "work_package_picker_down" => PickerMove(1),
            "work_package_picker_up" => PickerMove(-1),
            "work_package_picker_toggle" => PickerActivate,
            "work_package_picker_close" => PickerClose,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::WorkPackage(msg), action, label)
    }

    /// The workspace and visibility step ending the new-epic and
    /// new-package forms.
    fn run_placement_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage::*;
        let msg = match action {
            "placement_picker_workspace_down" => PlacementMoveWorkspace(1),
            "placement_picker_workspace_up" => PlacementMoveWorkspace(-1),
            "placement_picker_visibility" => PlacementToggleVisibility,
            "placement_picker_create" => SubmitPlacement,
            "placement_picker_cancel" => CancelInput,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Input(msg), action, label)
    }

    /// The admin screen (`admin-surfaces.allium`).
    fn run_admin(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::AdminMessage::*;
        let msg = match action {
            "admin_navigate_down" => Move(1),
            "admin_navigate_up" => Move(-1),
            "admin_next_pane" => Pane(1),
            "admin_previous_pane" => Pane(-1),
            "admin_new_workspace" => NewWorkspace,
            "admin_rename_workspace" => RenameWorkspace,
            "admin_delete_workspace" => DeleteWorkspace,
            "admin_add_member" => AddMember,
            "admin_toggle_role" => ToggleRole,
            "admin_remove_member" => RemoveMember,
            "admin_grant_super_admin" => Grant,
            "admin_revoke_super_admin" => Revoke,
            "admin_claim_super_admin" => Claim,
            "admin_delete_unseen_item" => DeleteUnseen,
            "admin_close" => Close,
            _ => return vec![],
        };
        if matches!(msg, Move(_) | Pane(_)) {
            return self.dispatch_keyed(Message::Admin(msg), action, label);
        }
        // A per-pane key on another pane, or one the person may not use,
        // does nothing: it records only when it issued a command or opened
        // a prompt.
        self.dispatch_prompting_handler_keyed(|a| a.update(Message::Admin(msg)), action, label)
    }

    /// The "reparent epic" tree picker.
    fn run_reparent_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::EpicMessage::*;
        use crate::tui::types::TreeNav;
        let msg = match action {
            "reparent_picker_down" => ReparentNavigate(TreeNav::Down),
            "reparent_picker_up" => ReparentNavigate(TreeNav::Up),
            "reparent_picker_expand" => ReparentNavigate(TreeNav::Right),
            "reparent_picker_collapse" => ReparentNavigate(TreeNav::Left),
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
            InputMode::ConfirmDeleteWorkPackage(id) => {
                self.handle_key_confirm_delete_work_package(label, yes, id)
            }
            _ => vec![],
        }
    }

    /// The creation form's tag step. `p` arms phoenix and does not pick a tag;
    /// a second `p` finds it armed and does nothing (CreateTask:
    /// PhoenixArming, `docs/specs/tasks.allium`). Every other accepted key is
    /// a letter of the label it selects.
    fn run_tag_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::tui::messages::InputMessage;
        let submit = match action {
            "phoenix_arm" => {
                if self.input.phoenix_armed() {
                    return vec![];
                }
                InputMessage::ArmPhoenix
            }
            "tag_picker_default" => InputMessage::SubmitTag(None),
            "tag_picker_cancel" => InputMessage::CancelInput,
            "tag_picker_select_bug" => InputMessage::SubmitTag(Some(TaskTag::Bug)),
            "tag_picker_select_feature" => InputMessage::SubmitTag(Some(TaskTag::Feature)),
            "tag_picker_select_chore" => InputMessage::SubmitTag(Some(TaskTag::Chore)),
            "tag_picker_select_pr_review" => InputMessage::SubmitTag(Some(TaskTag::PrReview)),
            "tag_picker_select_research" => InputMessage::SubmitTag(Some(TaskTag::Research)),
            "tag_picker_select_fix" => InputMessage::SubmitTag(Some(TaskTag::Fix)),
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Input(submit), action, label)
    }

    fn run_wrap_up_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::models::WrapUpMode;
        use crate::tui::messages::InputMessage;
        let submit = match action {
            "wrap_up_mode_picker_select_rebase" => {
                InputMessage::SubmitWrapUpMode(Some(WrapUpMode::Rebase))
            }
            "wrap_up_mode_picker_select_pr" => InputMessage::SubmitWrapUpMode(Some(WrapUpMode::Pr)),
            "wrap_up_mode_picker_select_done" => {
                InputMessage::SubmitWrapUpMode(Some(WrapUpMode::Done))
            }
            "wrap_up_mode_picker_default" => InputMessage::SubmitWrapUpMode(None),
            "wrap_up_mode_picker_cancel" => InputMessage::CancelInput,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Input(submit), action, label)
    }

    fn run_model_picker(&mut self, action: &'static str, label: &str) -> Vec<Command> {
        use crate::models::TaskModel;
        use crate::tui::messages::InputMessage;
        let submit = match action {
            "model_picker_select_haiku" => InputMessage::SubmitModel(Some(TaskModel::Haiku)),
            "model_picker_select_sonnet" => InputMessage::SubmitModel(Some(TaskModel::Sonnet)),
            "model_picker_select_opus" => InputMessage::SubmitModel(Some(TaskModel::Opus)),
            "model_picker_select_fable" => InputMessage::SubmitModel(Some(TaskModel::Fable)),
            "model_picker_default" => InputMessage::SubmitModel(None),
            "model_picker_cancel" => InputMessage::CancelInput,
            _ => return vec![],
        };
        self.dispatch_keyed(Message::Input(submit), action, label)
    }
}
