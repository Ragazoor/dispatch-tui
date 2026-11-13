//! Epic side-effect commands.

use crate::models::{EpicId, TaskStatus};

use super::super::types::EpicDraft;

/// Side-effect commands for the epic domain.
///
/// Wrapped by [`crate::tui::types::Command::Epic`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum EpicCommand {
    Insert(EpicDraft),
    /// `epics.allium: DeleteEpic`. The runtime pre-checks the epic's subtree
    /// ONCE, then starts `teardowns` (one repo at a time,
    /// `BatchTeardownsAreSerialPerRepo`), then makes the guarded delete. A
    /// refusal at the pre-check starts no teardown. The teardowns carry no
    /// guard of their own.
    Delete {
        id: EpicId,
        teardowns: Vec<super::TeardownTarget>,
    },
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
    /// `feeds.allium: OverrideFeedOwner`, fired after
    /// `ConfirmOverrideFeedOwner` accepts.
    OverrideFeedOwner(EpicId),
}
