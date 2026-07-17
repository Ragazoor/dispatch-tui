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
    PaneOpened {
        pane_id: String,
        task_id: Option<TaskId>,
    },
    PaneClosed,
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
            SplitMessage::PaneOpened { pane_id, task_id } => {
                app.handle_split_pane_opened(pane_id, task_id)
            }
            SplitMessage::PaneClosed => app.handle_split_pane_closed(),
        }
    }
}
