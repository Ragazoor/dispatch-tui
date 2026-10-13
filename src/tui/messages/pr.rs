//! PR flow messages (creation is agent-driven via the `/wrap-up` skill;
//! these messages cover status polling).

use crate::models::{TaskId, TmuxWindow};

use crate::tui::types::Command;
use crate::tui::App;

/// Messages targeting the PR flow.
///
/// Wrapped by [`crate::tui::types::Message::Pr`] for dispatch.
#[derive(Debug, Clone)]
pub enum PrMessage {
    /// The module accepted a report that the task's PR merged and moved the
    /// task to Done. `window` is the tmux window the task had when the report
    /// was posted — the module cleared it, and this host still kills it.
    Merged {
        id: TaskId,
        window: Option<TmuxWindow>,
    },
    /// The module accepted a report that the PR was closed without merging.
    Closed(TaskId),
    /// The module accepted a report of an open PR and set the review
    /// sub-status itself; this only retires the task's earlier failures.
    ReviewState(TaskId),
    /// A PR read failed. `permanent` is the whole reason this message exists:
    /// it says whether repeating the call could ever answer differently, and so
    /// whether this failure counts towards giving up on the task
    /// (`pr-workflow.allium`: `PollPrStatus`).
    CheckFailed {
        id: TaskId,
        permanent: bool,
        error: String,
    },
}

impl PrMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            PrMessage::Merged { id, window } => app.handle_pr_merged(id, window),
            PrMessage::Closed(id) => app.handle_pr_closed(id),
            PrMessage::ReviewState(id) => app.handle_pr_review_state(id),
            PrMessage::CheckFailed {
                id,
                permanent,
                error,
            } => app.handle_pr_check_failed(id, permanent, error),
        }
    }
}
