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
    /// Check which of these repo paths are no longer directories, off the
    /// update loop (a stalled mount must not freeze the board). Answers with
    /// `SystemMessage::BrokenRepoPathsFound`.
    CheckRepoPaths { paths: Vec<String> },
}
