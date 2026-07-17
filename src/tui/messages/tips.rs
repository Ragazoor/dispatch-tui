//! Tips overlay messages.

use crate::models::TipsShowMode;

use crate::tui::types::Command;
use crate::tui::App;

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

impl TipsMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            TipsMessage::Show {
                tips,
                starting_index,
                max_seen_id,
                show_mode,
            } => app.handle_show_tips(tips, starting_index, max_seen_id, show_mode),
            TipsMessage::Next => app.handle_next_tip(),
            TipsMessage::Prev => app.handle_prev_tip(),
            TipsMessage::SetMode(mode) => app.handle_set_tips_mode(mode),
            TipsMessage::Close => app.handle_close_tips(),
        }
    }
}
