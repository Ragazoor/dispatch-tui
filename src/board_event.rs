//! [`BoardEvent`] — the change notifications the board runtime consumes.
//!
//! A leaf module so every emitter depends on it downward: the MCP server
//! (`src/mcp/`) and the feed runner (`src/feed/`) both send these, and the
//! runtime (`src/runtime/`) drains them. See "MCP Notification Flow" in
//! `docs/mcp.md`.

use crate::models::{EpicId, TaskId};

/// Events sent from the MCP server and the feed runner to the TUI runtime.
#[derive(Debug)]
pub enum BoardEvent {
    /// Catch-all "I don't know what changed" — full reload of tasks, epics, and usage.
    /// Prefer the targeted variants below when the changed entity is known.
    Refresh,
    /// A single task changed — reload just that row.
    TaskChanged(TaskId),
    /// A single epic changed — reload just that row (and the epic's task list,
    /// since feed-sync changes appear here as a batch update for the epic).
    EpicChanged(EpicId),
    /// A `wrap_up(rebase)` succeeded, so the repository's local base branch was
    /// just fast-forwarded and its drift changed (docs/specs/repo-sync.allium:
    /// rule RefreshRepoSyncStateAfterRebase). Carries the repository taken from
    /// the rebased branch's task, which in practice always names one; the
    /// consumer still treats an empty path as "no repository" and measures
    /// nothing, so a future emitter that cannot resolve one has a safe encoding.
    BranchRebased { repo_path: String },
    /// An agent was launched off-board — by the `dispatch_task` tool or by epic
    /// auto-dispatch chaining — so the repository's worktree provisioning just
    /// fetched `origin/<base>` and its drift measurement is out of date
    /// (docs/specs/repo-sync.allium: rule RefreshRepoSyncStateAfterDispatch).
    /// That rule's obligation is per-event, not per-surface: the board emits its
    /// own refresh command directly, and these two paths owe the same refresh.
    ///
    /// Carries the repository rather than the task, for the same reason
    /// [`BoardEvent::BranchRebased`] does: the emitter already holds the task that
    /// names it, and no other key identifies a repository unambiguously. No mode
    /// travels with it because neither emitter can produce the one mode the rule
    /// excludes — `resume` provisions nothing and has no MCP entry point.
    AgentLaunched { repo_path: String },
    /// An epic's auto-dispatch chain claimed a subtask and then failed to
    /// provision it, so the subtask was released back to backlog and the epic
    /// stopped progressing (docs/specs/epics.allium: rule
    /// `SurfaceAutoDispatchFailure`).
    ///
    /// Carries the subtask, its epic and the reason, because all three are
    /// needed to say anything useful: the board marks the card, names the task
    /// in a status message, and reports why. Only the two failure arms that
    /// already hold a claimed subtask emit it — an unresolvable epic or an
    /// errored claim fails before one is selected and stays log-only.
    AutoDispatchFailed {
        task_id: TaskId,
        epic_id: EpicId,
        reason: String,
    },
}
