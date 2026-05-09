//! Pop-out `$EDITOR` flow side-effect commands.

use super::super::types::{EditKind, EditorOutcome};

/// Side-effect commands for the pop-out editor flow.
///
/// Wrapped by [`crate::tui::types::Command::Editor`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum EditorCommand {
    /// Launch `$EDITOR` in a new tmux window. The [`EditKind`] decides both
    /// what to put in the initial file and what post-processing to apply
    /// when the editor closes.
    PopOut(EditKind),
    /// Finalize an editor session: apply the user's edits (if any) to the
    /// database via the appropriate service. Dispatches on the [`EditKind`]
    /// to reach the right code path.
    ///
    /// Unique among commands: the handler returns follow-on commands from
    /// `app.update(...)` invocations made while applying the edit (e.g. DB
    /// persistence, status messages), which the runtime queue then drains.
    FinalizeResult {
        kind: EditKind,
        outcome: EditorOutcome,
    },
}
