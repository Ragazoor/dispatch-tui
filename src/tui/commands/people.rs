//! People side-effect commands (`docs/specs/workspaces.allium`).

/// Writes the board asks the runtime to make about the person using it.
///
/// Wrapped by [`crate::tui::types::Command::People`].
#[derive(Debug, Clone)]
pub enum PeopleCommand {
    /// `SetDisplayName`: set the caller's own name (`SetOwnNameFromBoard`).
    SetDisplayName(String),
    /// Read the person's workspaces for a create form that just opened; the
    /// answer is `InputMessage::WorkspacesLoaded`.
    LoadWorkspaces,
}
