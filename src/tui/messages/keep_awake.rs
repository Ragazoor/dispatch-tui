//! Keep-awake toggle messages (docs/specs/keep-awake.allium).

use crate::keep_awake::HoldEvent;
use crate::tui::types::Command;
use crate::tui::App;

/// Wrapped by [`crate::tui::types::Message::KeepAwake`] for dispatch.
#[derive(Debug, Clone)]
pub enum KeepAwakeMessage {
    /// Flip the toggle (the `W` key).
    Toggle,
    /// An answer about the hold requested as `generation`.
    Answer { generation: u64, event: HoldEvent },
}

impl KeepAwakeMessage {
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            KeepAwakeMessage::Toggle => app.handle_toggle_keep_awake(),
            KeepAwakeMessage::Answer { generation, event } => {
                app.handle_keep_awake_event(generation, event)
            }
        }
    }
}
