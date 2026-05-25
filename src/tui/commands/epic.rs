//! Epic side-effect commands.

use crate::models::{EpicId, TaskStatus};

use super::super::types::EpicDraft;

/// Side-effect commands for the epic domain.
///
/// Wrapped by [`crate::tui::types::Command::Epic`] for runtime dispatch.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum EpicCommand {
    Insert(EpicDraft),
    Delete(EpicId),
    Persist {
        id: EpicId,
        status: Option<TaskStatus>,
        sort_order: Option<i64>,
    },
    ToggleAutoDispatch {
        id: EpicId,
        auto_dispatch: bool,
    },
    ToggleGroupByRepo {
        id: EpicId,
        group_by_repo: bool,
    },
    RefreshFromDb,
}
