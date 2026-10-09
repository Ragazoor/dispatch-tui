//! Work-package message handlers (`docs/specs/work-packages.allium`): the
//! board's package data, the package view, the link picker, and rename/delete
//! from a package card.

use crate::models::{WorkPackage, WorkPackageId, WorkPackageItem, WorkPackageLink};
use crate::tui::commands::WorkPackageCommand;

use super::super::types::*;
use super::super::{truncate_title, App, TITLE_DISPLAY_LENGTH};

impl App {
    pub(in crate::tui) fn handle_refresh_work_packages(
        &mut self,
        packages: Vec<WorkPackage>,
        links: Vec<WorkPackageLink>,
    ) -> Vec<Command> {
        self.board.work_packages = packages;
        self.board.work_package_links = links;
        // An open picker's cursor stays on a row that exists.
        let last = self.link_picker_len() - 1;
        if let InputMode::LinkWorkPackage { cursor, .. } = &mut self.input.mode {
            *cursor = (*cursor).min(last);
        }
        // A package deleted elsewhere leaves nothing to show: fall back out of
        // its view (and any epic view entered from it).
        if let Some(open) = self.open_package_id() {
            if self.board.find_work_package(open).is_none() {
                self.leave_package_views();
            }
        }
        self.sync_board_selection();
        self.dirty = true;
        vec![]
    }

    /// The package whose view is showing or sits under the current epic view.
    fn open_package_id(&self) -> Option<WorkPackageId> {
        let mut current = &self.board.view_mode;
        loop {
            match current {
                ViewMode::Package { package_id, .. } => return Some(*package_id),
                ViewMode::Epic { parent, .. } => current = parent,
                ViewMode::TaskDetail { previous, .. } => current = previous,
                ViewMode::Board(_) => return None,
            }
        }
    }

    /// Pop every epic and package view, back to the main board.
    fn leave_package_views(&mut self) {
        if matches!(self.board.view_mode, ViewMode::TaskDetail { .. }) {
            if let ViewMode::TaskDetail { previous, .. } = std::mem::take(&mut self.board.view_mode)
            {
                self.board.view_mode = *previous;
            }
        }
        self.handle_exit_all_epics();
    }

    /// `work-packages.allium: EnterPackage`.
    pub(in crate::tui) fn handle_enter_work_package(&mut self, id: WorkPackageId) -> Vec<Command> {
        let parent = Box::new(self.board.view_mode.clone());
        self.board.view_mode = ViewMode::Package {
            package_id: id,
            selection: BoardSelection::new_for_epic(),
            parent,
        };
        self.invalidate_layout_cache();
        self.dirty = true;
        vec![]
    }

    /// `w` on a task or epic card (`WorkPackageLinkPicker`).
    pub(in crate::tui) fn handle_open_link_picker(
        &mut self,
        item: WorkPackageItem,
    ) -> Vec<Command> {
        self.input.mode = InputMode::LinkWorkPackage { item, cursor: 0 };
        self.status.clear();
        vec![]
    }

    /// The picker's row count: every package, then the "new package" row.
    fn link_picker_len(&self) -> usize {
        self.board.work_packages.len() + 1
    }

    pub(in crate::tui) fn handle_link_picker_move(&mut self, delta: isize) -> Vec<Command> {
        let last = self.link_picker_len() - 1;
        if let InputMode::LinkWorkPackage { cursor, .. } = &mut self.input.mode {
            *cursor = cursor.saturating_add_signed(delta).min(last);
        }
        vec![]
    }

    /// Space / Enter in the picker: toggle the row's link at once and keep the
    /// picker open, or open the new-package title on the last row.
    pub(in crate::tui) fn handle_link_picker_activate(&mut self) -> Vec<Command> {
        let InputMode::LinkWorkPackage { item, cursor } = self.input.mode else {
            return vec![];
        };
        let Some(package) = self.board.work_packages.get(cursor) else {
            self.input.mode = InputMode::InputWorkPackageTitle(item);
            self.input.clear_buffer();
            return vec![];
        };
        let id = package.id;
        let link = WorkPackageLink {
            work_package_id: id,
            item,
        };
        // Applied to the board at once so the check mark flips before the
        // write's refresh lands.
        let command = if let Some(at) = self
            .board
            .work_package_links
            .iter()
            .position(|l| *l == link)
        {
            self.board.work_package_links.remove(at);
            WorkPackageCommand::Unlink { id, item }
        } else {
            self.board.work_package_links.push(link);
            WorkPackageCommand::Link { id, item }
        };
        // Unlinking inside the package's own view can take the card under
        // the cursor away.
        self.sync_board_selection();
        self.dirty = true;
        vec![Command::WorkPackage(command)]
    }

    pub(in crate::tui) fn handle_link_picker_close(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.status.clear();
        vec![]
    }

    /// `work-packages.allium: CreatePackageFromPicker`. Either way the picker
    /// comes back, its cursor on the row the new package will occupy (the
    /// packages are listed oldest first, so a new one lands last).
    pub(in crate::tui) fn handle_submit_new_package_title(
        &mut self,
        title: String,
    ) -> Vec<Command> {
        let InputMode::InputWorkPackageTitle(item) = self.input.mode else {
            return vec![];
        };
        self.input.clear_buffer();
        let cursor = self.board.work_packages.len();
        self.input.mode = InputMode::LinkWorkPackage { item, cursor };
        let title = title.trim().to_string();
        if title.is_empty() {
            return vec![];
        }
        vec![Command::WorkPackage(WorkPackageCommand::Create {
            title,
            item,
        })]
    }

    /// Esc in the new-package title: back to the picker, nothing created.
    pub(in crate::tui) fn cancel_new_package_title(
        &mut self,
        item: WorkPackageItem,
    ) -> Vec<Command> {
        self.input.clear_buffer();
        self.input.mode = InputMode::LinkWorkPackage {
            item,
            cursor: self.board.work_packages.len(),
        };
        vec![]
    }

    /// `e` on a package card (`RenamePackageFromBoard`).
    pub(in crate::tui) fn handle_start_rename_work_package(
        &mut self,
        id: WorkPackageId,
    ) -> Vec<Command> {
        let Some(package) = self.board.find_work_package(id) else {
            return vec![];
        };
        let title = package.title.clone();
        self.input.set_buffer(title);
        self.input.mode = InputMode::RenameWorkPackage(id);
        self.status.clear();
        vec![]
    }

    pub(in crate::tui) fn handle_submit_rename_work_package(
        &mut self,
        title: String,
    ) -> Vec<Command> {
        let InputMode::RenameWorkPackage(id) = self.input.mode else {
            return vec![];
        };
        self.input.clear_buffer();
        self.input.mode = InputMode::Normal;
        self.status.clear();
        let title = title.trim().to_string();
        if title.is_empty() {
            return vec![];
        }
        vec![Command::WorkPackage(WorkPackageCommand::Rename {
            id,
            title,
        })]
    }

    /// `x` on a package card (`DeletePackageFromBoard`).
    pub(in crate::tui) fn handle_confirm_delete_work_package(
        &mut self,
        id: WorkPackageId,
    ) -> Vec<Command> {
        let Some(package) = self.board.find_work_package(id) else {
            return vec![];
        };
        let title = truncate_title(&package.title, TITLE_DISPLAY_LENGTH);
        self.input.mode = InputMode::ConfirmDeleteWorkPackage(id);
        self.status.set(format!(
            "Delete work package {title}? Its tasks and epics are kept [y/n]"
        ));
        vec![]
    }

    /// y / anything else on the delete prompt.
    pub(in crate::tui) fn handle_key_confirm_delete_work_package(
        &mut self,
        label: &str,
        yes: bool,
        id: WorkPackageId,
    ) -> Vec<Command> {
        self.confirm_dialog(label, yes, "confirm_delete_work_package", |_| {
            vec![Command::WorkPackage(WorkPackageCommand::Delete(id))]
        })
    }
}
