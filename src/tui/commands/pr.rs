//! PR flow side-effect commands (creation is agent-driven via the
//! `/wrap-up` skill).

use crate::models::TaskId;

/// Side-effect commands for the PR flow.
///
/// Wrapped by [`crate::tui::types::Command::Pr`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum PrCommand {
    /// Poll PR status for a task in review.
    CheckStatus { id: TaskId, pr_url: String },
}
