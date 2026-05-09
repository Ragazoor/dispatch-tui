//! Pop-out `$EDITOR` flow messages.

use super::super::types::{EditKind, EditorOutcome};

/// Messages produced by the pop-out editor flow.
///
/// Wrapped by [`crate::tui::types::Message::Editor`] for dispatch.
///
/// `EditKind` is large; this inner enum is always carried inside the wider
/// [`crate::tui::types::Message`] enum, which already absorbs the size, so
/// boxing here would only shift cost without saving anything.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum EditorMessage {
    /// Editor closed for a description-only edit during task/epic creation.
    DescriptionResult(String),
    /// Editor closed for any other [`EditKind`] (full task/epic/learning edit).
    Result {
        kind: EditKind,
        outcome: EditorOutcome,
    },
}
