//! PR flow side-effect commands (creation is agent-driven via the
//! `/wrap-up` skill).

use crate::models::TaskId;

/// Side-effect commands for the PR flow.
///
/// Wrapped by [`crate::tui::types::Command::Pr`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum PrCommand {
    /// Poll PR status for a task in review whose worktree makes `task.host`
    /// this machine — ownership is already unambiguous, so this polls
    /// unconditionally.
    CheckStatus { id: TaskId, url: String },
    /// Poll PR status for a host-less review task (`task.host = null`) —
    /// `core.allium: PollOwner` decides whether this machine may act.
    /// `pr-workflow.allium: PollPrStatus`.
    CheckStatusIfOwned { id: TaskId, url: String },
}
