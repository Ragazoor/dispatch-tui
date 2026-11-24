//! Epic side-effect commands.

use crate::models::{EpicId, TaskStatus};

use super::super::types::EpicDraft;

/// Side-effect commands for the epic domain.
///
/// Wrapped by [`crate::tui::types::Command::Epic`] for runtime dispatch.
#[derive(Debug, Clone)]
pub enum EpicCommand {
    Insert(EpicDraft),
    /// `epics.allium: DeleteEpic`. The runtime makes the delete first (its one
    /// check of the subtree's true rows), then starts `teardowns` (one repo at
    /// a time, `BatchTeardownsAreSerialPerRepo`). A refused delete starts no
    /// teardown. The teardowns carry no guard of their own.
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
    /// The ready key: the operator's mark on a backlog task, set or cleared.
    SetReady {
        task: crate::models::TaskId,
        ready: bool,
    },
    /// "Start an orchestrator for this epic?" answered y.
    SwitchOrchestratorOn(EpicId),
    /// The orchestrator key on an orchestrator with no session: a fresh
    /// restart, then the session and its window now.
    RestartOrchestrator {
        epic: EpicId,
        id: i64,
    },
    /// "Resolve item #N?" answered y.
    ResolveWaitingItem(i64),
}
