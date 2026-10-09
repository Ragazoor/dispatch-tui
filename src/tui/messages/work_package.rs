//! Work-package messages (`docs/specs/work-packages.allium`).

use crate::models::{WorkPackage, WorkPackageId, WorkPackageItem, WorkPackageLink};
use crate::tui::types::Command;
use crate::tui::App;

/// Messages for the work-package domain: the board's package data, the package
/// view, the link picker, and rename/delete from a package card.
///
/// Wrapped by [`crate::tui::types::Message::WorkPackage`].
#[derive(Debug, Clone)]
pub enum WorkPackageMessage {
    /// Replace the board's packages and links with a fresh read.
    Refresh {
        packages: Vec<WorkPackage>,
        links: Vec<WorkPackageLink>,
    },
    /// Open the package view (`EnterPackage`).
    Enter(WorkPackageId),
    /// Open the link picker for one task or epic (`WorkPackageLinkPicker`).
    OpenLinkPicker(WorkPackageItem),
    /// Move the picker cursor by `delta` rows, clamped to the list.
    PickerMove(isize),
    /// Space / Enter in the picker: toggle the row's link, or start the
    /// new-package title on the last row.
    PickerActivate,
    /// Esc / q in the picker.
    PickerClose,
    /// Enter in the new-package title input (`CreatePackageFromPicker`).
    SubmitNewTitle(String),
    /// `e` on a package card (`RenamePackageFromBoard`).
    StartRename(WorkPackageId),
    /// Enter in the rename input.
    SubmitRename(String),
    /// `x` on a package card (`DeletePackageFromBoard`).
    ConfirmDelete(WorkPackageId),
}

impl WorkPackageMessage {
    /// Route this message to its handler on [`App`]. See [`super::SplitMessage::route`].
    pub(in crate::tui) fn route(self, app: &mut App) -> Vec<Command> {
        match self {
            WorkPackageMessage::Refresh { packages, links } => {
                app.handle_refresh_work_packages(packages, links)
            }
            WorkPackageMessage::Enter(id) => app.handle_enter_work_package(id),
            WorkPackageMessage::OpenLinkPicker(item) => app.handle_open_link_picker(item),
            WorkPackageMessage::PickerMove(delta) => app.handle_link_picker_move(delta),
            WorkPackageMessage::PickerActivate => app.handle_link_picker_activate(),
            WorkPackageMessage::PickerClose => app.handle_link_picker_close(),
            WorkPackageMessage::SubmitNewTitle(title) => app.handle_submit_new_package_title(title),
            WorkPackageMessage::StartRename(id) => app.handle_start_rename_work_package(id),
            WorkPackageMessage::SubmitRename(title) => app.handle_submit_rename_work_package(title),
            WorkPackageMessage::ConfirmDelete(id) => app.handle_confirm_delete_work_package(id),
        }
    }
}
