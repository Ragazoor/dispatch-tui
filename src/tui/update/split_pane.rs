//! Split pane mode handlers: toggle, swap, open/close, focus tracking.

use crate::models::TaskId;

use super::super::types::*;
use super::super::App;

impl App {
    pub(in crate::tui) fn handle_toggle_split_mode(&mut self) -> Vec<Command> {
        // A rearrangement already in flight owns `active`. An entry keeps it
        // false until the pane reports back, so a second press acted on here
        // opens a second pane the board never records; a swap can have had it
        // cleared under it by a pane close, so a press acted on there would
        // start an entry beside a live swap, overwriting it in `in_flight` and
        // leaving the swap's own report to settle the entry instead. Hold it
        // for the settle instead. This is what makes the entry/swap mutual
        // exclusion the specs rest on true rather than assumed — see
        // `HoldToggleWhileRearrangementInFlight` in
        // docs/specs/split-pane.allium.
        if let Some(in_flight) = self.board.split.in_flight.as_mut() {
            in_flight.pending_toggle = true;
            return vec![];
        }
        if self.board.split.active {
            return self.exit_split_if_active();
        }
        // Both branches below start an entry; they differ only in whether there
        // is a task window to join or a bare shell to open.
        self.board.split.in_flight = Some(InFlight::entry());
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
        // Nothing new starts while anything is in flight. During a swap the
        // request is held for its settle; during an entry — unreachable, since
        // a swap request is only raised while split mode is active — it is
        // simply dropped, because starting one would overwrite the entry and
        // leave the entry's own report settling the swap.
        if let Some(in_flight) = self.board.split.in_flight.as_mut() {
            if let Rearrangement::Swap { pending_swap } = &mut in_flight.kind {
                *pending_swap = Some(task_id);
            }
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
        self.board.split.in_flight = Some(InFlight::swap());
        vec![Command::Split(crate::tui::commands::SplitCommand::Swap {
            task_id,
            new_window,
            old_pane_id,
            old_task,
        })]
    }

    /// Settle the rearrangement in flight, if it is the one this report belongs
    /// to: end it, and act on whatever it was holding.
    ///
    /// One function for both rearrangements and both outcomes, because the
    /// specs model them as one thing happening — see `SplitPaneEntrySettles`
    /// and `SplitPaneSwapSettles` in `docs/specs/split-pane.allium`.
    ///
    /// The two differ in exactly two places, both below: an entry claims focus
    /// and a swap does not, and `exit_split_if_active`'s own guard on `active`
    /// decides whether a held toggle survives a failure. Everything else — quit
    /// wins, exit-then-quit, one exit between a held toggle and a held quit,
    /// holds discarded whether or not acted on — is shared.
    ///
    /// `settled` carries the report and which rearrangement raised it (see
    /// [`Settled`]). A report that does not match the rearrangement in flight
    /// settles nothing and cannot consume the other one's holds — an assertion
    /// rather than dispatch, written as a refusal rather than a panic because
    /// this is a `src/tui/update` handler (see "Rendering purity" in
    /// `docs/conventions.md`): the wrong answer is a report ignored, not a
    /// board that dies mid-keypress.
    fn settle(&mut self, settled: Settled) -> Vec<Command> {
        // Ending the rearrangement discards both holds with it. That is what
        // makes "each hold is acted on at most once" structural rather than a
        // list of clearing statements a later field could be left out of.
        let Some(in_flight) = self.board.split.in_flight.take_if(|f| f.settles(&settled)) else {
            return vec![];
        };
        match settled {
            Settled::Entry(Some((pane_id, task_id))) => {
                self.record_open_pane(pane_id, task_id);
                // Entry is the only settle that claims focus. A swap must leave
                // the border where it was, because tmux focus does not transfer
                // on a swap (split-pane.allium: PinTaskInSplitPane).
                self.board.split.focused = true;
            }
            Settled::Swap(Some((pane_id, task_id))) => {
                self.record_open_pane(pane_id, Some(task_id));
            }
            // A failure changes nothing about what the pane shows: an entry
            // never opened one, and a swap's still holds the previous occupant.
            Settled::Entry(None) | Settled::Swap(None) => {}
        }
        // A held quit wins, and is performed whether or not the rearrangement
        // succeeded: the user asked to leave, and a failure changes what there
        // is to tidy up, never whether the application goes away. It exits
        // first so a pinned agent is returned to a standalone window rather
        // than ending with the board.
        //
        // A held toggle alongside it is dropped, and so is a held swap. All
        // three want something done to the pane, and the exit below is that
        // one thing: a toggle and a quit held during one rearrangement must
        // exit once between them, not twice, and replaying the swap would both
        // move a pane into a board nobody will look at again and start a fresh
        // rearrangement the quit does not wait for.
        if in_flight.pending_quit {
            self.should_quit = true;
            return self.exit_split_if_active();
        }
        // The toggle is replayed as an exit, not as a fresh press. The press
        // was made with a pane open, so it meant "close this split"; replaying
        // it as a press would re-read `active` and, where a pane close landed
        // mid-swap, ENTER instead — the opposite of what was asked.
        //
        // It drops a held swap for the same reason the quit above does: "show
        // me this task" and "close this split" are contradictory, and acting on
        // both would break out the very pane the swap is exchanging.
        //
        // `exit_split_if_active` is self-guarded on `active`, and that guard
        // alone gives each rearrangement the right answer on failure. A failed
        // entry leaves `active` false, so the held toggle is dropped —
        // correctly, because nothing opened and replaying would restart the
        // attempt that just failed. A failed swap leaves it true, so the toggle
        // is honoured — correctly, because the pane is still there to close.
        if in_flight.pending_toggle {
            return self.exit_split_if_active();
        }
        // A held swap is replayed as an ordinary request, so a task that has
        // since become the pinned one, or lost its window, is refused by the
        // same guards a fresh keypress meets. Replayed whether or not the swap
        // that held it succeeded: unlike a toggle, "show me this task" is not
        // relative to what the rearrangement before it achieved.
        match in_flight.kind {
            Rearrangement::Swap {
                pending_swap: Some(task_id),
            } => self.handle_swap_split_pane(task_id),
            _ => vec![],
        }
    }

    /// Record the pane a rearrangement just produced.
    ///
    /// Both halves of the pane's identity move together: a swap exchanges pane
    /// objects between windows, so which pane the board holds changes along
    /// with which task it shows.
    fn record_open_pane(&mut self, pane_id: String, task_id: Option<TaskId>) {
        self.board.split.active = true;
        self.board.split.right_pane_id = Some(pane_id);
        self.board.split.pinned_task_id = task_id;
    }

    /// Split-mode entry reported its pane. See [`Self::settle`].
    pub(in crate::tui) fn handle_split_pane_entry_opened(
        &mut self,
        pane_id: String,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.settle(Settled::Entry(Some((pane_id, task_id))))
    }

    /// A swap reported its pane. See [`Self::settle`].
    pub(in crate::tui) fn handle_split_pane_swap_opened(
        &mut self,
        pane_id: String,
        task_id: TaskId,
    ) -> Vec<Command> {
        self.settle(Settled::Swap(Some((pane_id, task_id))))
    }

    /// An entry that could not open a pane. Split mode stays inactive, which is
    /// the truth — but the entry settles all the same, or `[s]` would do
    /// nothing for the rest of the session.
    pub(in crate::tui) fn handle_split_pane_enter_failed(
        &mut self,
        failure: crate::tui::messages::EnterFailure,
    ) -> Vec<Command> {
        let mut cmds = self.settle(Settled::Entry(None));
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
        cmds.extend(self.settle(Settled::Swap(None)));
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
        // not about, and the requests held for it. A liveness poll issued while
        // the previous pane was open can land after the user closed it and
        // pressed [s] again; resetting over that entry would leave it settling
        // with nothing watching. Dropping a swap is the same mistake in the
        // other half: `settle` is gated on finding the rearrangement, so the
        // swap's own report would no longer settle anything and the held quit
        // would be dropped silently — one the user cannot notice, having
        // already asked the application to close.
        //
        // One field carries across, not a list of them, which is the point of
        // the type: a hold added later survives this close because it lives
        // inside the rearrangement, without anyone having to classify it here.
        // A held *swap* is the one exception and is cleared explicitly — it
        // names an occupant the user asked to see, and there is no occupant.
        // See `SplitPaneClosedResets` in `docs/specs/split-pane.allium`.
        let in_flight = self.board.split.in_flight.take().map(|mut in_flight| {
            in_flight.pane_closed();
            in_flight
        });
        self.board.split = SplitState {
            in_flight,
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
