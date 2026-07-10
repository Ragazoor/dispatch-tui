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
    /// Background stale-learning sweep: archive approved entries with a
    /// non-positive score that have gone untouched past the configured
    /// threshold. Emitted from the tick loop, gated by
    /// [`crate::tui::STALE_LEARNING_CLEANUP_ENABLED`] and
    /// [`crate::tui::STALE_CLEANUP_INTERVAL`]. See
    /// docs/specs/learnings.allium: ArchiveStaleLearning.
    ArchiveStale,
}
