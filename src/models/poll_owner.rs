//! [`PollScopeId`] — which kind of thing a `core/PollOwner` claim names
//! (`core.allium: PollOwner`), and its id.
//!
//! A sum type rather than a bare `i64` plus a separate scope string: the two
//! scopes (`Task`, `Epic`) use different id newtypes throughout the rest of
//! this codebase, and collapsing "claim a task's poll ownership" and "claim
//! an epic's" into one operation must not give up that type safety to do it —
//! a caller passing a `TaskId`'s raw value where an `EpicId` was meant would
//! otherwise compile cleanly and silently claim the wrong scope.

use super::{EpicId, TaskId};

/// The scope a `core/PollOwner` row names: a host-less review task's PR
/// polling (`pr-workflow.allium: PollPrStatus`), or a feed epic's polling
/// (`feeds.allium: FeedTick`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollScopeId {
    Task(TaskId),
    Epic(EpicId),
}

impl PollScopeId {
    /// The wire-level `(scope, scope_id)` pair the SpacetimeDB module keys
    /// `poll_owners` rows by. `scope` matches the module's own
    /// `POLL_SCOPE_TASK`/`POLL_SCOPE_EPIC` constants.
    pub fn wire(self) -> (&'static str, i64) {
        match self {
            PollScopeId::Task(id) => ("task", id.0),
            PollScopeId::Epic(id) => ("epic", id.0),
        }
    }
}
