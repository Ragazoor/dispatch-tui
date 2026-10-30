//! The admin screen (`docs/specs/admin-surfaces.allium`): which key opens what,
//! and what each answer sends. The screen decides nothing about who may do
//! what; it hides a key the person's role cannot use and sends the rest, and
//! the store's refusal is what the person reads.

use crate::models::{AdminSnapshot, WorkspaceDetail, WorkspaceMember};
use crate::tui::commands::{AdminWrite, PeopleCommand};

use super::super::types::*;
use super::super::App;

impl AdminScreen {
    /// The workspaces the person administers: all of them for a super-admin.
    pub fn administered(&self) -> Vec<&WorkspaceDetail> {
        self.snapshot
            .workspaces
            .iter()
            .filter(|w| self.snapshot.administers(w))
            .collect()
    }

    pub fn current_workspace(&self) -> Option<&WorkspaceDetail> {
        self.administered().get(self.workspace).copied()
    }

    pub fn current_member(&self) -> Option<&WorkspaceMember> {
        self.current_workspace()?.members.get(self.member)
    }

    /// The panes Tab visits: the last two are a super-admin's.
    pub fn panes(&self) -> Vec<AdminPane> {
        let mut panes = vec![AdminPane::Workspaces, AdminPane::Members];
        if self.snapshot.i_am_super_admin {
            panes.extend([AdminPane::SuperAdmins, AdminPane::Unseen]);
        }
        panes
    }

    fn len_of(&self, pane: AdminPane) -> usize {
        match pane {
            AdminPane::Workspaces => self.administered().len(),
            AdminPane::Members => self.current_workspace().map_or(0, |w| w.members.len()),
            AdminPane::SuperAdmins => self.snapshot.super_admins().count(),
            AdminPane::Unseen => self.snapshot.unseen.len(),
        }
    }

    fn cursor_mut(&mut self, pane: AdminPane) -> &mut usize {
        match pane {
            AdminPane::Workspaces => &mut self.workspace,
            AdminPane::Members => &mut self.member,
            AdminPane::SuperAdmins => &mut self.super_admin,
            AdminPane::Unseen => &mut self.unseen,
        }
    }

    fn clamp_cursors(&mut self) {
        for pane in [
            AdminPane::Workspaces,
            AdminPane::Members,
            AdminPane::SuperAdmins,
            AdminPane::Unseen,
        ] {
            let last = self.len_of(pane).saturating_sub(1);
            let cursor = self.cursor_mut(pane);
            *cursor = (*cursor).min(last);
        }
        if !self.panes().contains(&self.pane) {
            self.pane = AdminPane::Workspaces;
        }
    }
}

impl App {
    /// `W` in the board's normal mode: open the screen and read the snapshot.
    pub(in crate::tui) fn handle_admin_open(&mut self) -> Vec<Command> {
        self.input.clear_buffer();
        self.input.admin = Some(AdminScreen::default());
        self.input.mode = InputMode::Admin;
        self.status.clear();
        vec![Command::People(PeopleCommand::LoadAdmin)]
    }

    /// The snapshot, come back from a read. Ignored once the screen is closed.
    /// The highlight keeps its workspace by id, falling back to the nearest
    /// row when that workspace is gone.
    pub(in crate::tui) fn handle_admin_loaded(&mut self, snapshot: AdminSnapshot) -> Vec<Command> {
        let Some(screen) = self.input.admin.as_mut() else {
            return vec![];
        };
        let kept = screen.current_workspace().map(|w| w.id);
        screen.snapshot = snapshot;
        if let Some(index) =
            kept.and_then(|id| screen.administered().iter().position(|w| w.id == id))
        {
            screen.workspace = index;
        }
        screen.clamp_cursors();
        vec![]
    }

    pub(in crate::tui) fn handle_admin_close(&mut self) -> Vec<Command> {
        self.input.admin = None;
        self.input.mode = InputMode::Normal;
        self.status.clear();
        vec![]
    }

    pub(in crate::tui) fn handle_admin_move(&mut self, delta: isize) -> Vec<Command> {
        if let Some(screen) = self.input.admin.as_mut() {
            let pane = screen.pane;
            let last = screen.len_of(pane).saturating_sub(1);
            let cursor = screen.cursor_mut(pane);
            *cursor = cursor.saturating_add_signed(delta).min(last);
            if pane == AdminPane::Workspaces {
                screen.member = 0;
            }
        }
        vec![]
    }

    pub(in crate::tui) fn handle_admin_pane(&mut self, delta: isize) -> Vec<Command> {
        if let Some(screen) = self.input.admin.as_mut() {
            let panes = screen.panes();
            let at = panes.iter().position(|p| *p == screen.pane).unwrap_or(0);
            let next = (at as isize + delta).rem_euclid(panes.len() as isize) as usize;
            screen.pane = panes[next];
        }
        vec![]
    }

    fn open_admin_prompt(&mut self, prompt: AdminPrompt, prefill: String) -> Vec<Command> {
        if let Some(screen) = self.input.admin.as_mut() {
            screen.prompt = Some(prompt);
        }
        self.input.set_buffer(prefill);
        self.input.mode = InputMode::InputAdminText;
        self.status.clear();
        vec![]
    }

    fn open_admin_confirm(&mut self, confirm: AdminConfirm) -> Vec<Command> {
        if let Some(screen) = self.input.admin.as_mut() {
            screen.confirm = Some(confirm);
        }
        self.input.mode = InputMode::ConfirmAdmin;
        self.status.clear();
        vec![]
    }

    fn admin_write(write: AdminWrite) -> Vec<Command> {
        vec![Command::People(PeopleCommand::Admin(write))]
    }

    /// The pane the key was pressed in, when it is one of `panes`.
    fn admin_screen_in(&self, panes: &[AdminPane]) -> Option<&AdminScreen> {
        self.input
            .admin
            .as_ref()
            .filter(|s| panes.contains(&s.pane))
    }

    pub(in crate::tui) fn handle_admin_new_workspace(&mut self) -> Vec<Command> {
        let allowed = self
            .admin_screen_in(&[AdminPane::Workspaces])
            .is_some_and(|s| s.snapshot.i_am_super_admin);
        if !allowed {
            return vec![];
        }
        self.open_admin_prompt(AdminPrompt::NewWorkspace, String::new())
    }

    pub(in crate::tui) fn handle_admin_rename_workspace(&mut self) -> Vec<Command> {
        let Some((id, name)) = self
            .admin_screen_in(&[AdminPane::Workspaces])
            .and_then(|s| s.current_workspace())
            .map(|w| (w.id, w.name.clone()))
        else {
            return vec![];
        };
        self.open_admin_prompt(AdminPrompt::RenameWorkspace(id), name)
    }

    pub(in crate::tui) fn handle_admin_delete_workspace(&mut self) -> Vec<Command> {
        let Some(confirm) = self
            .admin_screen_in(&[AdminPane::Workspaces])
            .filter(|s| s.snapshot.i_am_super_admin)
            .and_then(|s| s.current_workspace())
            .map(|w| AdminConfirm::DeleteWorkspace {
                id: w.id,
                name: w.name.clone(),
                members: w.members.len(),
            })
        else {
            return vec![];
        };
        self.open_admin_confirm(confirm)
    }

    pub(in crate::tui) fn handle_admin_add_member(&mut self) -> Vec<Command> {
        let Some(id) = self
            .admin_screen_in(&[AdminPane::Workspaces, AdminPane::Members])
            .and_then(|s| s.current_workspace())
            .map(|w| w.id)
        else {
            return vec![];
        };
        self.open_admin_prompt(AdminPrompt::AddMember(id), String::new())
    }

    pub(in crate::tui) fn handle_admin_toggle_role(&mut self) -> Vec<Command> {
        let Some((membership_id, role)) = self
            .admin_screen_in(&[AdminPane::Members])
            .and_then(|s| s.current_member())
            .map(|m| (m.membership_id, m.role.toggled()))
        else {
            return vec![];
        };
        Self::admin_write(AdminWrite::SetRole {
            membership_id,
            role,
        })
    }

    pub(in crate::tui) fn handle_admin_remove_member(&mut self) -> Vec<Command> {
        let Some(confirm) = self.admin_screen_in(&[AdminPane::Members]).and_then(|s| {
            let member = s.current_member()?;
            Some(AdminConfirm::RemoveMember {
                membership_id: member.membership_id,
                person: member.shown_name.clone(),
                workspace: s.current_workspace()?.name.clone(),
            })
        }) else {
            return vec![];
        };
        self.open_admin_confirm(confirm)
    }

    pub(in crate::tui) fn handle_admin_grant(&mut self) -> Vec<Command> {
        let allowed = self
            .admin_screen_in(&[AdminPane::SuperAdmins])
            .is_some_and(|s| s.snapshot.i_am_super_admin);
        if !allowed {
            return vec![];
        }
        self.open_admin_prompt(AdminPrompt::GrantSuperAdmin, String::new())
    }

    pub(in crate::tui) fn handle_admin_revoke(&mut self) -> Vec<Command> {
        let Some(identity) = self
            .admin_screen_in(&[AdminPane::SuperAdmins])
            .filter(|s| s.snapshot.i_am_super_admin)
            .and_then(|s| s.snapshot.super_admins().nth(s.super_admin))
            .map(|p| p.identity.clone())
        else {
            return vec![];
        };
        self.open_admin_confirm(AdminConfirm::RevokeSuperAdmin { identity })
    }

    pub(in crate::tui) fn handle_admin_claim(&mut self) -> Vec<Command> {
        let claimable = self
            .input
            .admin
            .as_ref()
            .is_some_and(|s| s.snapshot.claimable);
        if !claimable {
            return vec![];
        }
        Self::admin_write(AdminWrite::Claim)
    }

    pub(in crate::tui) fn handle_admin_delete_unseen(&mut self) -> Vec<Command> {
        let Some(confirm) = self
            .admin_screen_in(&[AdminPane::Unseen])
            .filter(|s| s.snapshot.i_am_super_admin)
            .and_then(|s| s.snapshot.unseen.get(s.unseen))
            .map(|i| AdminConfirm::DeleteUnseen {
                kind: i.kind.clone(),
                id: i.id,
            })
        else {
            return vec![];
        };
        self.open_admin_confirm(confirm)
    }

    /// Enter in the screen's one-line input. A blank entry sends nothing.
    pub(in crate::tui) fn handle_admin_submit_text(&mut self, text: String) -> Vec<Command> {
        let prompt = self
            .input
            .admin
            .as_mut()
            .and_then(|screen| screen.prompt.take());
        self.input.clear_buffer();
        self.input.mode = InputMode::Admin;
        self.status.clear();
        let text = text.trim().to_string();
        let (Some(prompt), false) = (prompt, text.is_empty()) else {
            return vec![];
        };
        Self::admin_write(match prompt {
            AdminPrompt::NewWorkspace => AdminWrite::CreateWorkspace(text),
            AdminPrompt::RenameWorkspace(id) => AdminWrite::RenameWorkspace { id, name: text },
            AdminPrompt::AddMember(workspace_id) => AdminWrite::AddMember {
                workspace_id,
                person: text,
            },
            AdminPrompt::GrantSuperAdmin => AdminWrite::SetSuperAdmin {
                person: text,
                granted: true,
            },
        })
    }

    /// Esc in the screen's one-line input: back to the screen, nothing sent.
    pub(in crate::tui) fn cancel_admin_prompt(&mut self) {
        if let Some(screen) = self.input.admin.as_mut() {
            screen.prompt = None;
        }
        self.input.clear_buffer();
        self.input.mode = InputMode::Admin;
    }

    /// `y` or `n` at the screen's confirmation.
    pub(in crate::tui) fn handle_admin_confirm(&mut self, yes: bool) -> Vec<Command> {
        let confirm = self
            .input
            .admin
            .as_mut()
            .and_then(|screen| screen.confirm.take());
        self.input.mode = InputMode::Admin;
        self.status.clear();
        let (Some(confirm), true) = (confirm, yes) else {
            return vec![];
        };
        Self::admin_write(match confirm {
            AdminConfirm::DeleteWorkspace { id, .. } => AdminWrite::DeleteWorkspace(id),
            AdminConfirm::RemoveMember { membership_id, .. } => {
                AdminWrite::RemoveMember(membership_id)
            }
            AdminConfirm::RevokeSuperAdmin { identity } => AdminWrite::SetSuperAdmin {
                person: identity,
                granted: false,
            },
            AdminConfirm::DeleteUnseen { kind, id } => AdminWrite::DeleteUnseen { kind, id },
        })
    }
}
