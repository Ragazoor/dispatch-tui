//! Work-package side-effect commands (`docs/specs/work-packages.allium`).

use crate::models::{WorkPackageId, WorkPackageItem};

/// Writes the board asks the runtime to make through the work-package
/// service. Each one is followed by a reload of the board's packages.
///
/// Wrapped by [`crate::tui::types::Command::WorkPackage`].
#[derive(Debug, Clone)]
pub enum WorkPackageCommand {
    Link {
        id: WorkPackageId,
        item: WorkPackageItem,
    },
    Unlink {
        id: WorkPackageId,
        item: WorkPackageItem,
    },
    /// Create a package and link `item` to it.
    Create {
        title: String,
        item: WorkPackageItem,
        /// `0` when the person has one workspace and the form did not ask.
        workspace_id: i64,
        visibility: crate::models::Visibility,
    },
    Rename {
        id: WorkPackageId,
        title: String,
    },
    Delete(WorkPackageId),
    /// `V` on a package card (`SetWorkPackageVisibility`).
    SetVisibility {
        id: WorkPackageId,
        visibility: crate::models::Visibility,
    },
    /// Persist a package's manual card order (`work-packages.allium:
    /// ReorderPackage`).
    SetSortOrder {
        id: WorkPackageId,
        sort_order: i64,
    },
}
