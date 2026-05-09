//! System-level messages: lifecycle, ticks, focus, errors, status, notifications,
//! help/notification toggles, browser-opens, inter-agent flashes.

use crate::models::TaskId;

/// System-level messages.
///
/// Wrapped by [`crate::tui::types::Message::System`] for dispatch.
#[derive(Debug, Clone)]
pub enum SystemMessage {
    /// Periodic tick (see `TICK_INTERVAL` in `runtime/mod.rs`).
    Tick,
    /// Terminal was resized.
    TerminalResized,
    /// Window focus changed.
    FocusChanged(bool),
    /// User requested quit.
    Quit,
    /// Error popup.
    Error(String),
    /// Dismiss the active error popup.
    DismissError,
    /// Transient status-bar info message.
    StatusInfo(String),
    /// Toggle the help overlay.
    ToggleHelp,
    /// Toggle desktop notifications setting.
    ToggleNotifications,
    /// Open a URL in the user's browser.
    OpenInBrowser { url: String },
    /// Inter-agent message received: flash the target task's card.
    MessageReceived(TaskId),
}
