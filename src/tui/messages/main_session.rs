//! Main session setup messages.

/// Messages targeting the main session setup flow.
///
/// Wrapped by [`crate::tui::types::Message::MainSession`] for dispatch.
#[derive(Debug, Clone)]
pub enum MainSessionMessage {
    /// Open the repo picker (MainSessionDir mode) to (re)select the directory.
    /// Emitted by the runtime when `:` is pressed and no main-session window is
    /// alive.
    Configure,
    /// The user confirmed a directory in the picker.
    SubmitDir(String),
}
