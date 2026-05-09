//! Knowledge-base (Learning) overlay side-effect commands.

use crate::models::LearningId;

/// Side-effect commands for the Knowledge Base overlay.
///
/// Wrapped by [`crate::tui::types::Command::Learning`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum LearningCommand {
    Load,
    Archive(LearningId),
    Reject(LearningId),
    Approve(LearningId),
}
