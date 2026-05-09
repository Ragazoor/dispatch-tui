//! Wrap-up flow messages (rebase only — PR creation is agent-driven via the
//! `/wrap-up` skill).

use crate::models::{EpicId, TaskId};

/// Messages targeting the wrap-up flow.
///
/// Wrapped by [`crate::tui::types::Message::WrapUp`] for dispatch.
#[derive(Debug, Clone)]
pub enum WrapUpMessage {
    /// Begin per-task wrap-up confirmation flow.
    Start(TaskId),
    /// Confirm per-task rebase wrap-up.
    Rebase,
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
