//! Keep-awake hold side-effect commands (docs/specs/keep-awake.allium).

/// Wrapped by [`crate::tui::types::Command::KeepAwake`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum KeepAwakeCommand {
    /// Ask the operating system to hold this machine awake. Its answers come
    /// back as `KeepAwakeMessage::Answer` tagged with `generation`
    /// (RequestKeepAwake).
    Request { generation: u64 },
    /// Let go of the hold (ReleaseKeepAwake).
    Release,
}
