//! Task-domain side-effect commands.

use crate::models::{DispatchMode, EpicId, SubStatus, Task, TaskId};

use super::super::types::TaskDraft;

/// Side-effect commands for the task domain.
///
/// Wrapped by [`crate::tui::types::Command::Task`] for runtime dispatch.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum TaskCommand {
    Persist(Task),
    Insert {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    Delete(TaskId),
    DispatchAgent {
        task: Task,
        mode: DispatchMode,
    },
    Cleanup {
        id: TaskId,
        repo_path: String,
        worktree: String,
        tmux_window: Option<String>,
    },
    Finish {
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        worktree: String,
        tmux_window: Option<String>,
    },
    CaptureTmux {
        id: TaskId,
        window: String,
    },
    Resume {
        task: Task,
    },
    JumpToTmux {
        window: String,
    },
    QuickDispatch {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    KillTmuxWindow {
        window: String,
    },
    PatchSubStatus {
        id: TaskId,
        sub_status: SubStatus,
    },
    RefreshFromDb,
}
