//! Epic CRUD, lifecycle, batch ops, and creation-flow messages.

use crate::models::{Epic, EpicId};

use super::super::types::MoveDirection;

/// Messages targeting the epic domain.
///
/// Wrapped by [`crate::tui::types::Message::Epic`] for dispatch.
#[derive(Debug, Clone)]
pub enum EpicMessage {
    Dispatch(EpicId),
    Enter(EpicId),
    Exit,
    Refresh(Vec<Epic>),
    /// Splice a single fresh epic into `app.board.epics`.
    Updated(Epic),
    Created(Epic),
    Edit(EpicId),
    Edited(Epic),
    Delete(EpicId),
    ToggleAutoDispatch(EpicId),
    ConfirmDelete,
    MoveStatus(EpicId, MoveDirection),
    Archive(EpicId),
    ConfirmArchive,
    StartNew,
    SubmitTitle(String),
    SubmitDescription(String),
    SubmitRepoPath(String),
    ToggleSelect(EpicId),
    BatchArchive(Vec<EpicId>),
}
