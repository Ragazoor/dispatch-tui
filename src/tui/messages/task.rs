//! Task lifecycle, dispatch, retry, selection, finish, detach messages.

use crate::models::{DispatchMode, EpicId, Task, TaskId};

use super::super::types::{MoveDirection, TaskEdit};

/// Messages targeting the task domain.
///
/// Wrapped by [`crate::tui::types::Message::Task`] for dispatch.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum TaskMessage {
    Move {
        id: TaskId,
        direction: MoveDirection,
    },
    /// +1 = down, -1 = up
    ReorderItem(isize),
    Dispatch(TaskId, DispatchMode),
    Dispatched {
        id: TaskId,
        worktree: String,
        tmux_window: String,
        switch_focus: bool,
    },
    Created {
        task: Task,
    },
    Delete(TaskId),
    OpenDetail(TaskId),
    CloseDetail,
    ToggleFlattened,
    WindowGone(TaskId),
    Refresh(Vec<Task>),
    /// Splice a single fresh task into `app.board.tasks`.
    Updated(Task),
    Resume(TaskId),
    Resumed {
        id: TaskId,
        tmux_window: String,
    },
    DispatchFailed(TaskId),
    MarkDispatching(TaskId),
    Edited(TaskEdit),
    QuickDispatch {
        repo_path: String,
        epic_id: Option<EpicId>,
    },
    AgentCrashed(TaskId),
    KillAndRetry(TaskId),
    RetryResume(TaskId),
    RetryFresh(TaskId),
    Archive(TaskId),
    ToggleSelect(TaskId),
    BatchMove {
        ids: Vec<TaskId>,
        direction: MoveDirection,
    },
    BatchArchive(Vec<TaskId>),
    FinishComplete(TaskId),
    FinishFailed {
        id: TaskId,
        error: String,
        is_conflict: bool,
    },
    DetachTmux(TaskId),
    BatchDetachTmux(Vec<TaskId>),
}
