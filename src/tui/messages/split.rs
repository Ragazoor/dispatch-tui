//! Split-pane mode messages.

use crate::models::TaskId;

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
