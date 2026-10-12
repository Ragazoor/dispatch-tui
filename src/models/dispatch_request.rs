//! A dispatch request row (store-split M4).
//!
//! Spec: `docs/specs/store-split.allium`'s `DispatchRequest`. One request to
//! dispatch one task, from the moment a client asks until its host reports or
//! the module's backstop fails it, kept afterwards as history. The module owns
//! the row; the board reads it through its subscription
//! (`sync.allium: SubscribeToThisHostsDispatchRequests`), which delivers only
//! the rows naming this machine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DispatchMode, TaskId, TmuxWindow};
use crate::define_id_newtype;
use crate::define_str_enum;

define_id_newtype!(DispatchRequestId, dispatch_request_id_tests);

/// Where a request is in its life. `Requested` and `Provisioning` are open;
/// `Dispatched` and `Failed` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchRequestState {
    Requested,
    Provisioning,
    Dispatched,
    Failed,
}

define_str_enum!(DispatchRequestState, "dispatch request state" {
    Requested => "requested",
    Provisioning => "provisioning",
    Dispatched => "dispatched",
    Failed => "failed",
});

impl DispatchRequestState {
    /// `DispatchRequest.is_open`.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Requested | Self::Provisioning)
    }
}

/// Which launch path wrote the request (D9). `User` covers the TUI keys,
/// retry-fresh and research; `Mcp` is `dispatch_task`; `EpicChain` is
/// `epics.allium: AutoDispatchNextSubtask`; `Quick` is quick dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchOrigin {
    User,
    Mcp,
    EpicChain,
    Quick,
}

define_str_enum!(DispatchOrigin, "dispatch origin" {
    User => "user",
    Mcp => "mcp",
    EpicChain => "epic_chain",
    Quick => "quick",
});

/// One `dispatch_requests` row as the board reads it. An absent optional
/// column (`""` in the module) is `None` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRequest {
    pub id: DispatchRequestId,
    pub task_id: TaskId,
    /// The machine that provisions it, which is always the one that asked.
    pub host: String,
    /// The identity whose call wrote the request, stamped by the module
    /// (`store-split.allium`, D11). A host's board serves only rows its own
    /// owner requested.
    pub requested_by: String,
    pub mode: DispatchMode,
    pub origin: DispatchOrigin,
    pub state: DispatchRequestState,
    pub requested_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub worktree: Option<String>,
    pub tmux_window: Option<TmuxWindow>,
    /// Whether provisioning reused a worktree that already existed, as the
    /// host reported it with the dispatched report; `false` until then.
    /// `dispatch_task`'s success text names it.
    pub reused_worktree: bool,
    /// `requested_at + config.dispatch_request_timeout`, as the module stored
    /// it. The host enforces THIS value (`dispatch.allium:
    /// ProvisioningStopsAtTheDeadline`), never a copy of the config.
    pub deadline: DateTime<Utc>,
}

impl DispatchRequest {
    /// `DispatchRequest.is_open`: requested or provisioning.
    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }
}
