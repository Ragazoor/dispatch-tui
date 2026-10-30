//! The person's own display name (`docs/specs/workspaces.allium:
//! SetOwnNameFromBoard`).

use crate::tui::commands::{PeopleCommand, WorkPackageCommand};

use super::super::types::*;
use super::super::App;

impl App {
    /// `u` in the board's normal mode: open the one-line name input, empty.
    pub(in crate::tui) fn handle_start_set_display_name(&mut self) -> Vec<Command> {
        self.input.clear_buffer();
        self.input.mode = InputMode::InputDisplayName;
        self.status.clear();
        vec![]
    }

    /// Enter in the name input. A blank entry sends nothing.
    pub(in crate::tui) fn handle_submit_display_name(&mut self, name: String) -> Vec<Command> {
        self.input.clear_buffer();
        self.input.mode = InputMode::Normal;
        self.status.clear();
        let name = name.trim().to_string();
        if name.is_empty() {
            return vec![];
        }
        vec![Command::People(PeopleCommand::SetDisplayName(name))]
    }
}

impl App {
    /// The person's workspaces, come back from `PeopleCommand::LoadWorkspaces`.
    /// A person in none cannot create a root epic or a package, so a form of
    /// that kind still open is closed with the reason.
    pub(in crate::tui) fn handle_workspaces_loaded(
        &mut self,
        workspaces: Vec<crate::models::WorkspaceRef>,
    ) -> Vec<Command> {
        let none = workspaces.is_empty();
        self.input.workspaces = workspaces;
        if !none {
            return vec![];
        }
        let root_epic_form = matches!(
            self.input.mode,
            InputMode::InputEpicTitle | InputMode::InputEpicDescription
        ) && self
            .input
            .epic_draft
            .as_ref()
            .is_some_and(|d| d.parent_epic_id.is_none());
        if root_epic_form {
            self.input.mode = InputMode::Normal;
            self.input.epic_draft = None;
            self.input.clear_buffer();
        } else if self.input.mode == InputMode::InputPlacement
            && self
                .input
                .placement
                .as_ref()
                .is_some_and(|p| p.names_workspace)
        {
            let placement = self.input.placement.take();
            if let Some(PlacementFor::Package { item, .. }) = placement.map(|p| p.target) {
                self.input
                    .reopen_link_picker(item, self.board.work_packages.len());
            } else {
                self.input.mode = InputMode::Normal;
                self.input.epic_draft = None;
            }
        } else if let InputMode::InputWorkPackageTitle(item) = self.input.mode {
            self.input
                .reopen_link_picker(item, self.board.work_packages.len());
        } else {
            return vec![];
        }
        self.handle_error(crate::models::NO_WORKSPACE.to_string());
        vec![]
    }

    /// Open the placement step for what the form just collected.
    pub(in crate::tui) fn begin_placement(&mut self, target: PlacementFor) -> Vec<Command> {
        let names_workspace = match target {
            PlacementFor::Epic => self
                .input
                .epic_draft
                .as_ref()
                .is_none_or(|d| d.parent_epic_id.is_none()),
            PlacementFor::Package { .. } => true,
        };
        self.input.placement = Some(Placement {
            target,
            visibility: crate::models::Visibility::Public,
            workspace: 0,
            names_workspace,
        });
        self.input.clear_buffer();
        self.input.mode = InputMode::InputPlacement;
        self.status.clear();
        vec![]
    }

    pub(in crate::tui) fn handle_placement_toggle_visibility(&mut self) -> Vec<Command> {
        if let Some(p) = self.input.placement.as_mut() {
            p.visibility = p.visibility.toggled();
        }
        vec![]
    }

    /// `j`/`k`: move between the workspaces, stopping at either end. Does
    /// nothing when the form names no workspace or the person has one.
    pub(in crate::tui) fn handle_placement_move_workspace(&mut self, delta: isize) -> Vec<Command> {
        let last = self.input.workspaces.len().saturating_sub(1);
        if let Some(p) = self.input.placement.as_mut().filter(|p| p.names_workspace) {
            p.workspace = p.workspace.saturating_add_signed(delta).min(last);
        }
        vec![]
    }

    /// Enter at the step: create what the form collected.
    pub(in crate::tui) fn handle_submit_placement(&mut self) -> Vec<Command> {
        let Some(placement) = self.input.placement.take() else {
            return vec![];
        };
        let workspace_id = if placement.names_workspace {
            self.input
                .workspaces
                .get(placement.workspace)
                .map_or(0, |w| w.id)
        } else {
            0
        };
        match placement.target {
            PlacementFor::Epic => {
                let mut draft = self.input.epic_draft.take().unwrap_or_default();
                draft.workspace_id = workspace_id;
                draft.visibility = placement.visibility;
                self.input.mode = InputMode::Normal;
                self.status.clear();
                vec![Command::Epic(crate::tui::commands::EpicCommand::Insert(
                    draft,
                ))]
            }
            PlacementFor::Package { title, item } => {
                self.input
                    .reopen_link_picker(item, self.board.work_packages.len());
                vec![Command::WorkPackage(WorkPackageCommand::Create {
                    title,
                    item,
                    workspace_id,
                    visibility: placement.visibility,
                })]
            }
        }
    }
}
