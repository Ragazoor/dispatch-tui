//! Feed-epic refresh side-effect commands.

use crate::models::EpicId;

/// Side-effect commands for the feed-epic refresh flow.
///
/// Wrapped by [`crate::tui::types::Command::Feed`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum FeedCommand {
    /// Run the configured shell command for a feed epic and upsert results.
    TriggerEpic {
        epic_id: EpicId,
        epic_title: String,
        feed_command: String,
        group_by_repo: bool,
    },
}
