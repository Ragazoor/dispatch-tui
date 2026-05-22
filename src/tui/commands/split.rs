//! Split-pane mode side-effect commands.

use crate::models::TaskId;

/// Side-effect commands for the split-pane mode.
///
/// Wrapped by [`crate::tui::types::Command::Split`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum SplitCommand {
    Enter,
    EnterWithTask {
        task_id: TaskId,
        window: String,
    },
    Exit {
        pane_id: String,
        restore_window: Option<String>,
    },
    Swap {
        task_id: TaskId,
        new_window: String,
        old_pane_id: Option<String>,
        old_window: Option<String>,
    },
    FocusPane {
        pane_id: String,
    },
    CheckPaneExists {
        pane_id: String,
    },
    RespawnPane {
        pane_id: String,
    },
}
