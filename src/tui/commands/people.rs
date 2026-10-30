//! People side-effect commands (`docs/specs/workspaces.allium`,
//! `docs/specs/admin-surfaces.allium`).

use crate::models::MemberRole;

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
    /// Read the admin snapshot; the answer is `AdminMessage::Loaded`.
    LoadAdmin,
    /// One write from the admin screen. The runtime makes it, shows the
    /// result on the status line or in the error popup, and on success reads
    /// the snapshot again (`AdminScreenRefreshesAfterAWrite`).
    Admin(AdminWrite),
}

/// The writes of the admin screen, each the board twin of an MCP tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminWrite {
    Claim,
    CreateWorkspace(String),
    RenameWorkspace {
        id: i64,
        name: String,
    },
    DeleteWorkspace(i64),
    /// `person` is what was typed: a display name or an identity.
    AddMember {
        workspace_id: i64,
        person: String,
    },
    SetRole {
        membership_id: i64,
        role: MemberRole,
    },
    RemoveMember(i64),
    /// Grant (`true`) or revoke (`false`) super-admin for `person`.
    SetSuperAdmin {
        person: String,
        granted: bool,
    },
    DeleteUnseen {
        kind: String,
        id: i64,
    },
}
