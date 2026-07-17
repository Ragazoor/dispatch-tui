//! Wrap-up flow messages (rebase only — PR creation is agent-driven via the
//! `/wrap-up` skill).

use crate::models::{EpicId, TaskId};

use crate::tui::types::Command;
use crate::tui::App;

/// Messages targeting the wrap-up flow.
///
/// Wrapped by [`crate::tui::types::Message::WrapUp`] for dispatch.
#[derive(Debug, Clone)]
pub enum WrapUpMessage {
    /// Begin per-task wrap-up confirmation flow.
    Start(TaskId),
    /// Confirm per-task rebase wrap-up.
    Rebase,
    /// Confirm per-task done wrap-up (mark done, no git ops).
    Done,
    /// Cancel per-task wrap-up.
    Cancel,
    /// Begin per-epic batch wrap-up confirmation flow.
    EpicStart(EpicId),
    /// Confirm per-epic batch rebase wrap-up.
    EpicRebase,
    /// Cancel per-epic batch wrap-up.
    EpicCancel,
    /// Cancel an in-progress merge queue.
    CancelMergeQueue,
}

impl WrapUpMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            WrapUpMessage::Start(id) => app.handle_start_wrap_up(id),
            WrapUpMessage::Rebase => app.handle_wrap_up_rebase(),
            WrapUpMessage::Done => app.handle_wrap_up_done(),
            WrapUpMessage::Cancel => app.handle_cancel_wrap_up(),
            WrapUpMessage::EpicStart(id) => app.handle_start_epic_wrap_up(id),
            WrapUpMessage::EpicRebase => app.handle_epic_wrap_up(),
            WrapUpMessage::EpicCancel => app.handle_cancel_epic_wrap_up(),
            WrapUpMessage::CancelMergeQueue => app.handle_cancel_merge_queue(),
        }
    }
}
