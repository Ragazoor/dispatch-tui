//! System-level side-effect commands: notifications and browser-opens.

/// System-level side-effect commands.
///
/// Wrapped by [`crate::tui::types::Command::System`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum SystemCommand {
    /// Send a desktop notification.
    SendNotification {
        title: String,
        body: String,
        urgent: bool,
    },
    /// Open a URL in the user's browser.
    OpenInBrowser { url: String },
}
