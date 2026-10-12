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
}
