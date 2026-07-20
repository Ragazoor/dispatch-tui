//! Main session side-effect commands.

/// Side-effect commands for the main session flow.
///
/// Wrapped by [`crate::tui::types::Command::MainSession`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum MainSessionCommand {
    /// Decide what `:` does: jump to the main-session window if it is alive,
    /// otherwise open the repo picker to (re)select a directory.
    Open,
    /// Create a fresh main-session window in the configured directory and jump
    /// to it. Emitted after the picker confirms a non-empty path.
    Create,
    /// Poll whether the "dispatch-main" window is alive (a live tmux check off
    /// the event loop) and report the result via
    /// [`crate::tui::messages::MainSessionMessage::LivenessChanged`]. Emitted by
    /// the tick loop every `MAIN_SESSION_POLL_TICKS`.
    CheckLiveness,
}
