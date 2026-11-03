//! System-level side-effect commands: notifications, browser-opens and the
//! keep-awake hold.

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
    /// Ask the operating system to hold this machine awake. Its answers come
    /// back as `SystemMessage::KeepAwake` tagged with `generation`
    /// (keep-awake.allium: RequestKeepAwake).
    RequestKeepAwake { generation: u64 },
    /// Let go of the keep-awake hold (keep-awake.allium: ReleaseKeepAwake).
    ReleaseKeepAwake,
}
