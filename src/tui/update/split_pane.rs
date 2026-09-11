//! Split pane mode handlers: toggle, swap, open/close, focus tracking.

use crate::models::TaskId;

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_toggle_split_mode(&mut self) -> Vec<Command> {
        // An entry already in flight owns `active`, which stays false until the
        // pane reports back. Acting on a second press against it opens a second
        // pane the board never records — see `HoldToggleWhileEntryInFlight` in
        // docs/specs/split-pane.allium. Hold the press instead and replay it
        // once the entry settles, so a fast double-tap ends where a slow one
        // does.
        if self.board.split.entry_in_flight {
            self.board.split.pending_toggle = true;
            return vec![];
        }
        if self.board.split.active {
            self.exit_split_if_active()
        } else if let Some((task_id, window)) = self
            .selected_task()
            .and_then(|t| t.tmux_window.clone().map(|w| (t.id, w)))
        {
            self.board.split.entry_in_flight = true;
            vec![Command::Split(
                crate::tui::commands::SplitCommand::EnterWithTask { task_id, window },
            )]
        } else {
            self.board.split.entry_in_flight = true;
            vec![Command::Split(crate::tui::commands::SplitCommand::Enter)]
        }
    }

    /// Swap `task_id`'s tmux window into the split pane.
    ///
    /// The caller establishes the preconditions (see SwapSplitPane in
    /// docs/specs/split-pane.allium): the only producer of
    /// `SplitMessage::Swap` is `handle_key_activate`, which raises it solely
    /// for a task that has a live tmux window while split mode is active. A
    /// windowless task is routed by status there instead — it never reaches
    /// this handler — so there is no user-facing "no session" case to report.
    pub(in crate::tui) fn handle_swap_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
        let task = match self.find_task(task_id) {
            Some(t) => t,
            None => return vec![],
        };
        let new_window = match &task.tmux_window {
            Some(w) => w.clone(),
            None => return vec![],
        };
        // No pane to swap into: nothing downstream would report back, so this
        // must not be marked in flight — a swap that can never settle would
        // wedge every later one.
        let Some(old_pane_id) = self.board.split.right_pane_id.clone() else {
            return vec![];
        };
        // A swap already in flight owns `pinned_task_id` and `right_pane_id`
        // until it settles; both still name the outgoing occupant. Starting a
        // second swap against them swaps the wrong pane and renames a window
        // onto a name the first swap just assigned, leaving two windows
        // sharing it — see `DeferSwapWhileSwapInFlight` in
        // docs/specs/split-pane.allium. Hold the request instead, newest
        // wins, and replay it once the swap settles.
        if self.board.split.swap_in_flight {
            self.board.split.pending_swap = Some(task_id);
            return vec![];
        }
        // Already pinned — nothing to do. Reached on the replay of a held
        // request whose task became the pinned one while the swap ran (the
        // user pressing Space twice on the same card), which is the only way
        // in now that a missing pane id returns above. Checked *after* the
        // in-flight hold: during a swap `pinned_task_id` still names the
        // outgoing task, so that comparison is not yet meaningful.
        if self.board.split.pinned_task_id == Some(task_id) {
            return vec![];
        }
        let old_task = self
            .board
            .split
            .pinned_task_id
            .and_then(|id| self.find_task(id))
            .and_then(|t| t.tmux_window.clone().zip(t.worktree.clone()));
        self.board.split.swap_in_flight = true;
        vec![Command::Split(crate::tui::commands::SplitCommand::Swap {
            task_id,
            new_window,
            old_pane_id,
            old_task,
        })]
    }

    /// Settle a swap: clear the in-flight mark and replay whatever was held
    /// while it ran. See `SplitPaneSwapSettles` in
    /// `docs/specs/split-pane.allium`.
    ///
    /// The replay goes back through [`Self::handle_swap_split_pane`], so a
    /// held request whose task has since become the pinned one, or has lost
    /// its window, is refused by the same guards a fresh keypress meets.
    ///
    /// A no-op when no swap was in flight — entering split mode reports its
    /// pane through the same message — because nothing can be held except by
    /// a swap that is in flight.
    fn settle_swap(&mut self) -> Vec<Command> {
        self.board.split.swap_in_flight = false;
        match self.board.split.pending_swap.take() {
            Some(task_id) => self.handle_swap_split_pane(task_id),
            None => vec![],
        }
    }

    /// Settle an entry: clear the in-flight mark and act on a toggle held
    /// while it ran. See `SplitPaneEntrySettles` in
    /// `docs/specs/split-pane.allium`.
    ///
    /// The held press is acted on as the toggle it is, against a pane that has
    /// just opened — which is an exit. It is not routed back through
    /// [`Self::handle_toggle_split_mode`] only because that would re-read a
    /// selection the user may have moved in between; the branch it would take
    /// is this one.
    ///
    /// A no-op when no entry was in flight — a swap reports its pane through
    /// the same message — because nothing can be held except by an entry that
    /// is in flight.
    fn settle_entry(&mut self) -> Vec<Command> {
        if !self.board.split.entry_in_flight {
            return vec![];
        }
        self.board.split.entry_in_flight = false;
        // Cleared whether or not it is acted on, so a press held during an
        // entry is acted on at most once.
        if std::mem::take(&mut self.board.split.pending_toggle) {
            return self.exit_split_if_active();
        }
        vec![]
    }

    pub(in crate::tui) fn handle_split_pane_opened(
        &mut self,
        pane_id: String,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.board.split.active = true;
        self.board.split.focused = true;
        // Both halves of the pane's identity move together: a swap exchanges
        // pane objects between windows, so which pane the board holds changes
        // along with which task it shows.
        self.board.split.right_pane_id = Some(pane_id);
        self.board.split.pinned_task_id = task_id;
        let mut cmds = self.settle_swap();
        cmds.extend(self.settle_entry());
        cmds
    }

    /// An entry that could not open a pane. Split mode stays inactive, which is
    /// the truth — but the entry settles all the same, or `[s]` would do
    /// nothing for the rest of the session.
    pub(in crate::tui) fn handle_split_pane_enter_failed(
        &mut self,
        failure: crate::tui::messages::EnterFailure,
    ) -> Vec<Command> {
        self.board.split.entry_in_flight = false;
        // A held toggle is dropped rather than replayed: nothing opened, so
        // replaying would restart the attempt that just failed and repeat its
        // error rather than undo anything.
        self.board.split.pending_toggle = false;
        match failure {
            crate::tui::messages::EnterFailure::NoTmux => {
                self.handle_status_info("Split mode requires tmux".to_string())
            }
            crate::tui::messages::EnterFailure::Failed(error) => self.handle_error(error),
        }
    }

    /// A swap that could not be carried out. The pane still shows the previous
    /// occupant, so the pinned task and pane are left alone — but the swap
    /// settles all the same, or the board could never swap again.
    pub(in crate::tui) fn handle_split_pane_swap_failed(&mut self, error: String) -> Vec<Command> {
        let mut cmds = self.handle_error(error);
        cmds.extend(self.settle_swap());
        cmds
    }

    pub(in crate::tui) fn handle_focus_changed(&mut self, focused: bool) -> Vec<Command> {
        if self.board.split.active {
            self.board.split.focused = focused;
        }
        vec![]
    }

    pub(in crate::tui) fn handle_split_pane_closed(&mut self) -> Vec<Command> {
        // Assigned wholesale rather than field by field: `SplitState`'s
        // `Default` already *is* the no-split state, and a hand-written reset
        // is one a later field can be left out of.
        self.board.split = SplitState::default();
        vec![]
    }

    /// If `task_id` is the split-pinned task, clear the pin and respawn the
    /// pane with a fresh shell.  Split mode stays active.
    pub(in crate::tui) fn maybe_respawn_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
        if self.board.split.active && self.board.split.pinned_task_id == Some(task_id) {
            self.board.split.pinned_task_id = None;
            if let Some(pane_id) = self.board.split.right_pane_id.clone() {
                return vec![Command::Split(
                    crate::tui::commands::SplitCommand::RespawnPane { pane_id },
                )];
            }
        }
        vec![]
    }
}
