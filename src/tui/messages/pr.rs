//! PR flow messages (creation is agent-driven via the `/wrap-up` skill;
//! these messages cover status polling and the `P` merge action).

use crate::models::{ReviewDecision, TaskId};

/// Messages targeting the PR flow.
///
/// Wrapped by [`crate::tui::types::Message::Pr`] for dispatch.
#[derive(Debug, Clone)]
pub enum PrMessage {
    /// PR for a task has merged upstream.
    Merged(TaskId),
    /// User-triggered merge confirmation flow.
    StartMerge(TaskId),
    /// User confirmed the merge.
    ConfirmMerge,
    /// User cancelled the merge prompt.
    CancelMerge,
    /// PR merge failed.
    MergeFailed { id: TaskId, error: String },
    /// Review-state poll result for a task's PR.
    ReviewState {
        id: TaskId,
        review_decision: Option<ReviewDecision>,
    },
}
