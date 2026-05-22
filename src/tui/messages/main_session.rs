//! Main session setup messages.

/// Messages targeting the main session setup flow.
///
/// Wrapped by [`crate::tui::types::Message::MainSession`] for dispatch.
#[derive(Debug, Clone)]
pub enum MainSessionMessage {
    SubmitDir(String),
    Created(String),
}
