//! Split pane mode handlers: toggle, swap, open/close, focus tracking.

use crate::models::TaskId;

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_toggle_split_mode(&mut self) -> Vec<Command> {
        // A rearrangement already in flight owns `active`. An entry keeps it
        // false until the pane reports back, so a second press acted on here
        // opens a second pane the board never records; a swap can have had it
        // cleared under it by a pane close, so a press acted on there starts an
        // entry beside a live swap — and both then settle on the same pane
        // report, the swap's settle consuming it, leaving the entry to hang and
        // `[s]` dead for the session. Hold it for the settle instead. This is
        // what makes the entry/swap mutual exclusion the specs rest on true
        // rather than assumed — see `HoldToggleWhileRearrangementInFlight` in
        // docs/specs/split-pane.allium.
        if self.board.split.rearrangement_in_flight() {
            self.board.split.pending_toggle = true;
            return vec![];
        }
        if self.board.split.active {
            return self.exit_split_if_active();
        }
        // Both branches below start an entry; they differ only in whether there
        // is a task window to join or a bare shell to open.
        self.board.split.entry_in_flight = true;
        match self
            .selected_task()
            .and_then(|t| t.tmux_window.clone().map(|w| (t.id, w)))
        {
            Some((task_id, window)) => vec![Command::Split(
                crate::tui::commands::SplitCommand::EnterWithTask { task_id, window },
            )],
            None => vec![Command::Split(crate::tui::commands::SplitCommand::Enter)],
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

    /// Settle a swap: clear the in-flight mark and act on whatever was held
    /// while it ran. See `SplitPaneSwapSettles` in
    /// `docs/specs/split-pane.allium`.
    ///
    /// A held swap is replayed through [`Self::handle_swap_split_pane`], so a
    /// request whose task has since become the pinned one, or has lost its
    /// window, is refused by the same guards a fresh keypress meets.
    ///
    /// A held quit wins over a held swap and is performed here, exit first and
    /// then quit — the same order a held quit is performed in on the entry
    /// side. The pending swap is dropped rather than replayed: it would move a
    /// pane into a board nobody will look at again, and the replayed swap is
    /// itself a rearrangement the quit does not wait for, reintroducing the
    /// exact half-finished-rename race the hold exists to close.
    ///
    /// A no-op when no swap was in flight — entering split mode reports its
    /// pane through the same message, and a quit held for *that* belongs to
    /// [`Self::settle_entry`].
    fn settle_swap(&mut self) -> Vec<Command> {
        if !self.board.split.swap_in_flight {
            return vec![];
        }
        self.board.split.swap_in_flight = false;
        let pending = self.board.split.pending_swap.take();
        let toggling = std::mem::take(&mut self.board.split.pending_toggle);
        // Performed whether or not the swap succeeded. The user asked to leave;
        // a failed rearrangement changes what there is to tidy up, never
        // whether the application goes away — and on failure the pane still
        // shows the previous occupant, so there is still an agent to restore.
        // A held toggle is dropped here: the exit below already takes the pane
        // away, and a toggle and a quit held during one swap must exit once
        // between them, not twice.
        if std::mem::take(&mut self.board.split.pending_quit) {
            self.should_quit = true;
            return self.exit_split_if_active();
        }
        // The toggle is replayed as an exit, not as a fresh press. The press
        // was made with the pane open, so it meant "close this split";
        // replaying it as a press would re-read `active` and, where a pane
        // close landed mid-swap, ENTER instead — the opposite of what was
        // asked. As an exit it is a no-op in exactly that case, which is right:
        // the close already did what the press asked for.
        //
        // It also drops a held swap, on the same reasoning that drops one
        // alongside a quit. The two are contradictory instructions, and acting
        // on both would start a fresh rearrangement and then break out the very
        // pane it is exchanging — after which that swap's own report would set
        // `active` back to true, re-opening the split the press asked to close.
        if toggling {
            return self.exit_split_if_active();
        }
        match pending {
            Some(task_id) => self.handle_swap_split_pane(task_id),
            None => vec![],
        }
    }

    /// Settle the entry in flight, if there is one: take it, and act on
    /// whatever it was holding. One function for both outcomes because the
    /// spec models them as one rule — see `SplitPaneEntrySettles` in
    /// `docs/specs/split-pane.allium`.
    ///
    /// A no-op when no entry was in flight: a swap reports its pane through
    /// the same message, and nothing can be held except by an entry.
    fn settle_entry(&mut self, succeeded: bool) -> Vec<Command> {
        if !self.board.split.entry_in_flight {
            return vec![];
        }
        self.board.split.entry_in_flight = false;
        // Taken only once there is an entry to settle: a hold belonging to a
        // swap is not this settle's to act on, and `handle_split_pane_opened`
        // runs both settles over the same message.
        let toggle = std::mem::take(&mut self.board.split.pending_toggle);
        let quitting = std::mem::take(&mut self.board.split.pending_quit);
        let mut cmds = vec![];
        if succeeded {
            // Entry is the only settle that claims focus. A swap reports its
            // pane through the same message but must leave the border where it
            // was: tmux focus does not transfer on a swap (split-pane.allium:
            // PinTaskInSplitPane).
            self.board.split.focused = true;
            // One exit between them: a held toggle and a held quit both want
            // the pane gone, and a held quit additionally wants the pinned
            // agent restored before the board goes away. A held toggle is
            // dropped on failure instead — nothing opened, so replaying it
            // would restart the attempt that just failed and repeat its error
            // rather than undo anything.
            if toggle || quitting {
                cmds = self.exit_split_if_active();
            }
        }
        // A held quit is performed either way. The user asked to leave; a
        // failed entry changes what there is to tidy up, never whether the
        // application goes away.
        if quitting {
            self.should_quit = true;
        }
        cmds
    }

    pub(in crate::tui) fn handle_split_pane_opened(
        &mut self,
        pane_id: String,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.board.split.active = true;
        // Both halves of the pane's identity move together: a swap exchanges
        // pane objects between windows, so which pane the board holds changes
        // along with which task it shows.
        self.board.split.right_pane_id = Some(pane_id);
        self.board.split.pinned_task_id = task_id;
        let mut cmds = self.settle_swap();
        cmds.extend(self.settle_entry(true));
        cmds
    }

    /// An entry that could not open a pane. Split mode stays inactive, which is
    /// the truth — but the entry settles all the same, or `[s]` would do
    /// nothing for the rest of the session.
    pub(in crate::tui) fn handle_split_pane_enter_failed(
        &mut self,
        failure: crate::tui::messages::EnterFailure,
    ) -> Vec<Command> {
        let mut cmds = self.settle_entry(false);
        cmds.extend(match failure {
            crate::tui::messages::EnterFailure::NoTmux => {
                self.handle_status_info("Split mode requires tmux".to_string())
            }
            crate::tui::messages::EnterFailure::Failed(error) => self.handle_error(error),
        });
        cmds
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
        // Reset to `SplitState`'s `Default`, which already *is* the no-split
        // state — except for the rearrangement in flight, which this close is
        // not about, and the quit held for it. A liveness poll issued while the
        // previous pane was open can land after the user closed it and pressed
        // [s] again; resetting over that entry would leave it settling with
        // nothing watching. Clearing `swap_in_flight` is the same mistake in
        // the other half: `settle_swap` is gated on it, so the swap's own
        // report would no longer settle anything and the held quit would be
        // dropped silently — one the user cannot notice, having already asked
        // the application to close. `pending_swap` does reset: it names an
        // occupant the user asked to see, and there is no occupant. See
        // `SplitPaneClosedResets` in `docs/specs/split-pane.allium`.
        let SplitState {
            entry_in_flight,
            swap_in_flight,
            pending_toggle,
            pending_quit,
            ..
        } = self.board.split;
        self.board.split = SplitState {
            entry_in_flight,
            swap_in_flight,
            pending_toggle,
            pending_quit,
            ..SplitState::default()
        };
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
