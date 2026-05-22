//! Tips overlay messages.

use crate::models::TipsShowMode;

/// Messages targeting the tips overlay.
///
/// Wrapped by [`crate::tui::types::Message::Tips`] for dispatch.
#[derive(Debug, Clone)]
pub enum TipsMessage {
    Show {
        tips: Vec<crate::tips::Tip>,
        starting_index: usize,
        max_seen_id: u32,
        show_mode: TipsShowMode,
    },
    Next,
    Prev,
    SetMode(TipsShowMode),
    Close,
}
