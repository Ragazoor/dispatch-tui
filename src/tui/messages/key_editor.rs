//! Key-editor messages (`KeyEditor` in `docs/specs/keybindings.allium`):
//! what comes back from reading and writing the override file.

use serde_json::Value;

use crate::keybindings::KeyLoadError;
use crate::tui::types::Command;
use crate::tui::App;

/// Wrapped by [`crate::tui::types::Message::KeyEditor`] for dispatch.
#[derive(Debug, Clone)]
pub enum KeyEditorMessage {
    /// The override file's entries, read when the editor opened and after a
    /// save, or why it cannot be read.
    FileRead(Result<Vec<Value>, KeyLoadError>),
    /// A save was accepted (true when it wrote the file, false when the file
    /// already said what was staged), or was refused with these messages.
    Saved(Result<bool, Vec<String>>),
}

impl KeyEditorMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            KeyEditorMessage::FileRead(read) => app.handle_key_editor_file_read(read),
            KeyEditorMessage::Saved(result) => app.handle_key_editor_saved(result),
        }
    }
}
