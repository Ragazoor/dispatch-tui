//! Main session side-effect commands.

/// Side-effect commands for the main session flow.
///
/// Wrapped by [`crate::tui::types::Command::MainSession`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum MainSessionCommand {
    Open,
}
