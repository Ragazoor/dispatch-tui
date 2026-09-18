//! Epic side-effect commands.

use crate::models::{EpicId, TaskStatus};

use super::super::types::EpicDraft;

/// Side-effect commands for the epic domain.
///
/// Wrapped by [`crate::tui::types::Command::Epic`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum EpicCommand {
    Insert(EpicDraft),
    Delete(EpicId),
    Persist {
        id: EpicId,
        status: Option<TaskStatus>,
        sort_order: Option<i64>,
        /// The Done column's ordering key. Written instead of `sort_order` by
        /// a manual reorder of this epic's card in Done — see
        /// `App::handle_reorder_item`.
        completed_at: Option<chrono::DateTime<chrono::Utc>>,
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
    Reparent {
        id: EpicId,
        new_parent: Option<EpicId>,
    },
}
