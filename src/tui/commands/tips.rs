//! Tips persistence commands.

use crate::models::TipsShowMode;

/// Side-effect commands for tips state persistence.
///
/// Wrapped by [`crate::tui::types::Command::Tips`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum TipsCommand {
    SaveState { seen_up_to: u32, show_mode: TipsShowMode },
}
