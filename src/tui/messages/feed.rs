//! Feed-epic refresh messages.

use crate::models::EpicId;

/// Messages produced by the feed-epic refresh flow.
///
/// Wrapped by [`crate::tui::types::Message::Feed`] for dispatch.
#[derive(Debug, Clone)]
pub enum FeedMessage {
    /// User-triggered refresh of a feed epic.
    TriggerEpic(EpicId),
    /// Feed refresh succeeded.
    Refreshed { epic_title: String, count: usize },
    /// Feed refresh failed.
    Failed { epic_title: String, error: String },
}
