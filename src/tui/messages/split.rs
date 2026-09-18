//! Split-pane mode messages.

use crate::models::TaskId;
use crate::tui::types::Command;
use crate::tui::App;

/// Messages targeting the split-pane mode.
///
/// Wrapped by [`crate::tui::types::Message::Split`] for dispatch.
#[derive(Debug, Clone)]
pub enum SplitMessage {
    Toggle,
    Swap(TaskId),
    /// Split-mode entry opened a pane. Settles the entry (see
    /// `docs/specs/split-pane.allium`'s `SplitPaneEntrySettles`).
    ///
    /// Distinct from [`SplitMessage::SwapOpened`] below because the producer
    /// knows statically which rearrangement it finished, and the settle must
    /// not have to work that out from whichever flag happened to be set. The
    /// failure side has always been two variants for the same reason, and the
    /// spec has always named two events.
    ///
    /// `task_id` is optional here and not on `SwapOpened`: entry can open a
    /// bare, unpinned shell pane, whereas a swap always names the task it is
    /// swapping in.
    EntryOpened {
        pane_id: String,
        task_id: Option<TaskId>,
    },
    /// A swap exchanged the pane's occupant. Settles the swap (see
    /// `docs/specs/split-pane.allium`'s `SplitPaneSwapSettles`).
    SwapOpened {
        pane_id: String,
        task_id: TaskId,
    },
    /// A swap that could not be carried out. Settles the swap (see
    /// `docs/specs/split-pane.allium`'s `SplitPaneSwapSettles`) and reports
    /// `error` to the user; the pane keeps showing what it showed before.
    ///
    /// Distinct from a plain `SystemMessage::Error` precisely so the settle
    /// happens: a failure that only raised the error popup would leave the swap
    /// in flight and wedge every later swap.
    SwapFailed {
        error: String,
    },
    /// An entry that did not open a pane. Settles the entry (see
    /// `docs/specs/split-pane.allium`'s `SplitPaneEntrySettles`) and reports
    /// `failure` to the user; split mode stays inactive.
    ///
    /// The failure is carried here rather than raised on its own for the same
    /// reason `SwapFailed` carries its error: a failure that only told the user
    /// would leave the entry in flight and wedge `[s]` for the session.
    EnterFailed {
        failure: EnterFailure,
    },
    PaneClosed,
}

/// Why an entry did not open a pane.
///
/// The two cases differ in how loudly they are reported, not just in wording:
/// pressing `[s]` outside tmux is an ordinary thing to do and earns a status
/// hint, while a tmux command that failed is an error worth a popup.
#[derive(Debug, Clone)]
pub enum EnterFailure {
    /// Not running under tmux, so there is nothing to split.
    NoTmux,
    /// A tmux command failed. Carries the message to show, already formatted.
    Failed(String),
}

impl SplitMessage {
    /// Route this message to its handler on [`App`].
    ///
    /// Co-locating routing with the enum keeps each variant adjacent to the
    /// arm that wires it, so adding an interaction is a single-file edit here
    /// plus its `update/*` handler — no separate arm in `dispatcher.rs`.
    ///
    /// Named `route` (not `dispatch`) to stay grep-distinct from the top-level
    /// [`crate::tui::dispatcher::dispatch`] router that calls it. This makes
    /// `messages/*.rs` deliberately *not* a pure-data layer: each domain enum
    /// owns the wiring to `App`'s `handle_*` methods.
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            SplitMessage::Toggle => app.handle_toggle_split_mode(),
            SplitMessage::Swap(task_id) => app.handle_swap_split_pane(task_id),
            SplitMessage::EntryOpened { pane_id, task_id } => {
                app.handle_split_pane_entry_opened(pane_id, task_id)
            }
            SplitMessage::SwapOpened { pane_id, task_id } => {
                app.handle_split_pane_swap_opened(pane_id, task_id)
            }
            SplitMessage::SwapFailed { error } => app.handle_split_pane_swap_failed(error),
            SplitMessage::EnterFailed { failure } => app.handle_split_pane_enter_failed(failure),
            SplitMessage::PaneClosed => app.handle_split_pane_closed(),
        }
    }
}
