//! Admin-screen messages (`docs/specs/admin-surfaces.allium`): the keys of the
//! screen and what comes back from the writes it makes.

use crate::models::AdminSnapshot;
use crate::tui::types::Command;
use crate::tui::App;

/// Messages targeting the admin screen.
///
/// Wrapped by [`crate::tui::types::Message::Admin`] for dispatch.
#[derive(Debug, Clone)]
pub enum AdminMessage {
    /// `W` on the board: open the screen and read the snapshot.
    Open,
    /// The snapshot, read when the screen opened and after every write.
    Loaded(Box<AdminSnapshot>),
    /// Esc on the screen.
    Close,
    /// `j` (+1) / `k` (-1) inside the pane.
    Move(isize),
    /// Tab (+1) / Shift-Tab (-1) between panes.
    Pane(isize),
    /// `n`: new workspace.
    NewWorkspace,
    /// `r`: rename the highlighted workspace.
    RenameWorkspace,
    /// `d`: delete the highlighted workspace.
    DeleteWorkspace,
    /// `a`: add a member to the highlighted workspace.
    AddMember,
    /// `m`: flip the highlighted member between admin and member.
    ToggleRole,
    /// `x`: remove the highlighted member.
    RemoveMember,
    /// `g`: make someone a super-admin.
    Grant,
    /// `v`: revoke the highlighted super-admin.
    Revoke,
    /// `c`: claim the first super-admin.
    Claim,
    /// `D`: delete the highlighted unseen item.
    DeleteUnseen,
    /// Enter in the screen's one-line input.
    SubmitText(String),
    /// `y` or `n` in the screen's confirmation.
    Confirm(bool),
}

impl AdminMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            AdminMessage::Open => app.handle_admin_open(),
            AdminMessage::Loaded(snapshot) => app.handle_admin_loaded(*snapshot),
            AdminMessage::Close => app.handle_admin_close(),
            AdminMessage::Move(delta) => app.handle_admin_move(delta),
            AdminMessage::Pane(delta) => app.handle_admin_pane(delta),
            AdminMessage::NewWorkspace => app.handle_admin_new_workspace(),
            AdminMessage::RenameWorkspace => app.handle_admin_rename_workspace(),
            AdminMessage::DeleteWorkspace => app.handle_admin_delete_workspace(),
            AdminMessage::AddMember => app.handle_admin_add_member(),
            AdminMessage::ToggleRole => app.handle_admin_toggle_role(),
            AdminMessage::RemoveMember => app.handle_admin_remove_member(),
            AdminMessage::Grant => app.handle_admin_grant(),
            AdminMessage::Revoke => app.handle_admin_revoke(),
            AdminMessage::Claim => app.handle_admin_claim(),
            AdminMessage::DeleteUnseen => app.handle_admin_delete_unseen(),
            AdminMessage::SubmitText(text) => app.handle_admin_submit_text(text),
            AdminMessage::Confirm(yes) => app.handle_admin_confirm(yes),
        }
    }
}
