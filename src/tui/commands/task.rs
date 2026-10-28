//! Task-domain side-effect commands.

use crate::models::{
    DispatchMode, DrainMode, EpicId, SubStatus, Task, TaskId, TaskStatus, TaskUrl, TmuxWindow,
};

use super::super::types::TaskDraft;

/// The 7 fields a generic board-mutation write actually persists —
/// `id`, `status`, `sub_status`, `worktree`, `tmux_window`, `url`,
/// `sort_order` — carried instead of a whole [`Task`] so [`TaskCommand::Persist`]
/// doesn't force a ~470-byte deep clone (title, description, repo_path,
/// base_branch, labels, ...) at every call site for fields it never reads.
///
/// Field names and types mirror `Task`'s corresponding fields. A `None`
/// `worktree`, `host` or `tmux_window` means "leave the store's value alone",
/// not "clear it": the board's copy can be stale, and sending its absence as a
/// clear would wipe a pointer the board never saw (tasks.allium:
/// ABoardWriteLeavesThePointersAlone). A caller that means to clear says so
/// with [`releasing_worktree`](Self::releasing_worktree) or
/// [`releasing_tmux_window`](Self::releasing_tmux_window).
#[derive(Debug, Clone)]
pub struct PersistFields {
    pub id: TaskId,
    pub status: TaskStatus,
    pub sub_status: SubStatus,
    pub worktree: Option<String>,
    pub tmux_window: Option<TmuxWindow>,
    /// Paired with `worktree` per core/Task's `HostTracksWorktree` invariant
    /// (docs/specs/core.allium) — every caller that changes `worktree` on the
    /// board's copy changes `host` in the same breath, so persisting them
    /// together is the only shape that keeps the pair coherent.
    pub host: Option<String>,
    pub url: Option<TaskUrl>,
    pub sort_order: Option<i64>,
    /// The Done column's ordering key. Carried alongside `sort_order` because
    /// a manual reorder in Done writes this field instead — see
    /// `App::handle_reorder_item`.
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Clear `worktree` and `host` (and the window with them) in the store.
    /// Only RetryFresh sets it.
    pub release_worktree: bool,
    /// Clear `tmux_window` in the store: a detach, a window that went away,
    /// or a crash.
    pub release_tmux_window: bool,
}

impl PersistFields {
    pub fn from_task(task: &Task) -> Self {
        Self {
            id: task.id,
            status: task.status,
            sub_status: task.sub_status,
            worktree: task.worktree.clone(),
            tmux_window: task.tmux_window.clone(),
            host: task.host.clone(),
            url: task.url.clone(),
            sort_order: task.sort_order,
            completed_at: task.completed_at,
            release_worktree: false,
            release_tmux_window: false,
        }
    }

    /// RetryFresh's release: the store's worktree, host and window are cleared.
    pub fn releasing_worktree(mut self) -> Self {
        self.release_worktree = true;
        self.release_tmux_window = true;
        self
    }

    /// The caller took the window off the board's copy on purpose.
    pub fn releasing_tmux_window(mut self) -> Self {
        self.release_tmux_window = true;
        self
    }
}

/// What a **successful** worktree removal earns the requesting operation.
///
/// The operation that asked for the teardown cannot apply this itself: the
/// removal shells out to git in the background, so the write that acts on its
/// outcome has to happen on the teardown's completion path. See
/// [`TaskCommand::Cleanup`] and `WorktreeReleaseIsGated` in
/// docs/specs/tasks.allium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupFollowUp {
    /// Clear the task's `worktree`/`tmux_window` columns (retry-fresh).
    ClearPointer,
    /// Delete the task row. A failed removal therefore leaves the row in
    /// place, still Done and still pointing at the directory on disk, so
    /// deleting again retries the removal.
    DeleteRow,
    /// Nothing to apply: the row is being removed by the operation that asked
    /// for the teardown, so there is no column left to clear and no pointer that
    /// could be retained. Used by the epic delete, which drops every subtask row
    /// in one operation — the documented exemption from the gate. The failure is
    /// still reported and logged.
    Nothing,
}

/// What a delete must still hold of the store's TRUE rows before any
/// client-side side effect (teardown) runs. Carried by the
/// teardown commands a delete emits so the runtime can refuse, touching
/// nothing, when a stale board view let a not-done delete through. See the
/// pre-check paragraph of `DeleteTask` in docs/specs/tasks.allium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteGuard {
    /// `DeleteTask`: this one task must be done.
    Task(TaskId),
    /// `DeleteEpic`: every task in this epic's subtree must be done.
    Epic(EpicId),
    /// `BatchDelete`: every task and epic subtree in the batch, all or nothing.
    Batch {
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    },
}

/// Side-effect commands for the task domain.
///
/// Wrapped by [`crate::tui::types::Command::Task`] for runtime dispatch.
///
/// A variant that genuinely consumes a whole `Task` (`TrustAndDispatch`)
/// boxes it — see the `assert_no_entity_inline` tests in
/// `crate::tui::types` for why nothing on this bus stores an entity inline.
/// `Persist` and `Resume` carry only the handful of fields their runtime
/// handler reads, so they need no box.
#[derive(Debug, Clone)]
pub enum TaskCommand {
    Persist(PersistFields),
    Insert {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    Delete(TaskId),
    /// `tasks.allium: BatchDelete`'s atomic call — every task and epic in the
    /// selection, deleted together in ONE reducer/DB call rather than
    /// `Delete`/`EpicCommand::Delete` issued once per item. The board has
    /// already applied every board-mutation and best-effort teardown for
    /// these ids by the time this command is built — see
    /// `crate::tui::update::selection::App::handle_batch_delete` — so this is
    /// purely the row-delete-and-validate call.
    BatchDelete {
        task_ids: Vec<TaskId>,
        epic_ids: Vec<EpicId>,
    },
    /// Request a dispatch of `task`. `mode` `None` lets the module derive it
    /// (`store-split.allium` D3) — retry-fresh's case; the dispatch and
    /// research keys name theirs.
    DispatchAgent {
        id: TaskId,
        mode: Option<DispatchMode>,
    },
    TrustAndDispatch {
        task: Box<Task>,
        mode: DispatchMode,
    },
    /// Check whether `repo_path` is trusted by Claude Code (reads
    /// `~/.claude.json` via `spawn_blocking`) and emit the message that routes
    /// to a dispatch or a trust-confirmation prompt based on the result.
    /// Keeps the file I/O off the render thread — see docs/conventions.md
    /// "No `std::fs` inside async handlers".
    CheckTrustAndDispatch {
        id: TaskId,
        repo_path: String,
        mode: DispatchMode,
    },
    /// Tear a task's live resources down (`TaskTeardown` in
    /// docs/specs/tasks.allium): kill the tmux window, remove the git worktree,
    /// best-effort delete the branch.
    ///
    /// Both resources are optional and independent
    /// (`TeardownIsOwedWheneverThereIsSomethingToRelease`); only a task owning
    /// neither is never queued at all.
    ///
    /// `follow_up` is what a **successful** removal earns, and it is only ever
    /// applied on the removal's own completion path — never beside it. That is
    /// the gate: a failed removal leaves the task pointing at the directory that
    /// is still on disk, instead of forgetting it and stranding an orphan
    /// (`WorktreeReleaseIsGated`). The gate keys on the worktree: with `worktree:
    /// None` there is nothing to release, so the follow-up applies regardless of
    /// what the window kill reported.
    Cleanup {
        id: TaskId,
        repo_path: String,
        worktree: Option<String>,
        tmux_window: Option<TmuxWindow>,
        follow_up: CleanupFollowUp,
        /// Set by the delete paths only: re-read against the store before the
        /// teardown starts; a failed check refuses and tears nothing down.
        guard: Option<DeleteGuard>,
    },
    /// Clear a task's `worktree` and `tmux_window` columns. Emitted by
    /// [`crate::tui::messages::TaskMessage::CleanupSucceeded`], and the only
    /// write that forgets a worktree path on the retry-fresh path.
    ClearWorktreePointer(TaskId),
    CheckWindow {
        id: TaskId,
        window: TmuxWindow,
    },
    /// Check all task windows in a single tmux list-windows call. Reduces N
    /// process forks per tick to 1.
    BatchCheckWindows {
        windows: Vec<(TaskId, TmuxWindow)>,
    },
    /// Request a `schedule`-origin dispatch for each due task whose repo
    /// exists on this machine (`schedules.allium: FireDueTask`), given as
    /// `(id, repo_path)`. The repo check is the runtime's: it touches the disk.
    FireScheduled {
        due: Vec<(TaskId, String)>,
    },
    /// Remove the build output of each Done task's worktree
    /// (`tasks.allium: ClearBuildOutputOfDoneTask`). Best effort, off the
    /// board's thread; nothing comes back.
    ClearBuildOutput {
        worktrees: Vec<(TaskId, String)>,
    },
    Resume {
        id: TaskId,
        worktree: Option<String>,
        /// First input of the resumed session (`dispatch.allium: ResumeTask`).
        message: Option<String>,
    },
    JumpToTmux {
        window: TmuxWindow,
    },
    QuickDispatch {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    /// Confirmed-trust follow-up to `QuickDispatch`: grant trust to the
    /// draft's repo, then proceed exactly as `QuickDispatch` would have.
    /// Mirrors `TrustAndDispatch`.
    TrustAndQuickDispatch {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    KillTmuxWindow {
        window: TmuxWindow,
    },
    PatchSubStatus {
        id: TaskId,
        sub_status: SubStatus,
    },
    /// Move a task to a different epic, or detach it (`new_epic = None`).
    MoveToEpic {
        id: TaskId,
        new_epic: Option<EpicId>,
    },
    /// Seed `last_pre_tool_use_at` on a Backlog→Running transition.
    ///
    /// Kept separate from [`Self::Persist`] so a generic in-memory persist
    /// (sort_order swaps, tick reclassification, etc.) cannot clobber a
    /// freshly hook-written timestamp with a stale in-memory value.
    SeedActivity {
        id: TaskId,
        at: chrono::DateTime<chrono::Utc>,
    },
    /// Update `sub_status` for multiple tasks in a single DB transaction.
    /// Emitted by the tick instead of N individual `Persist` commands so all
    /// reclassifications in one tick round-trip are batched together.
    BatchPatchSubStatus {
        updates: Vec<(TaskId, SubStatus)>,
    },
    RefreshFromDb,
    /// Drop every `task_subagents` entry for a task. See [`DrainMode`] for what
    /// the two modes mean and which callers pick which.
    ClearSubagents {
        id: TaskId,
        mode: DrainMode,
    },
}
