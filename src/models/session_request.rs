//! A session request row (store-split M5).
//!
//! Spec: `docs/specs/store-split.allium`'s `SessionRequest`. One wrap_up or
//! exit_session or close request for one task, from the moment the module accepts it
//! until it is done or failed, kept afterwards as history. The module owns the
//! row; the board reads it through its subscription
//! (`sync.allium: SubscribeToThisHostsSessionRequests`), which delivers only the
//! rows naming this machine. A done wrap_up row's id IS the exit token (W3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{TaskId, TmuxWindow, WrapUpMode};
use crate::define_id_newtype;
use crate::define_str_enum;

define_id_newtype!(SessionRequestId, session_request_id_tests);

/// What a session request asks for: a wrap-up, the close that follows it, or
/// (M7) the teardown of `update_tasks`' done-path close, which has no wrap-up
/// behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRequestKind {
    WrapUp,
    ExitSession,
    Close,
}

define_str_enum!(SessionRequestKind, "session request kind" {
    WrapUp => "wrap_up",
    ExitSession => "exit_session",
    Close => "close",
});

impl SessionRequestKind {
    /// Whether the row is a window teardown: an `exit_session` or a `close`
    /// (`pr-workflow.allium: ExitSessionTeardown`, which a close row is held
    /// to as well).
    pub fn is_teardown(self) -> bool {
        matches!(self, Self::ExitSession | Self::Close)
    }
}

/// Where a request is in its life. `Requested` and `Running` are open; `Done`
/// and `Failed` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRequestState {
    Requested,
    Running,
    Done,
    Failed,
}

define_str_enum!(SessionRequestState, "session request state" {
    Requested => "requested",
    Running => "running",
    Done => "done",
    Failed => "failed",
});

impl SessionRequestState {
    /// `SessionRequest.is_open`.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Requested | Self::Running)
    }
}

/// One `session_requests` row as the board reads it. An absent optional column
/// (`""` or `0` in the module) is `None` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    pub id: SessionRequestId,
    pub task_id: TaskId,
    pub kind: SessionRequestKind,
    /// On a wrap_up row the action asked for, on an exit_session row the one
    /// its token was issued for.
    pub action: WrapUpMode,
    /// The machine holding the task's worktree and window, whose board does
    /// the host work.
    pub host: String,
    /// The identity whose call wrote the request, stamped by the module
    /// (`store-split.allium`, D11). A host's board serves only rows its own
    /// owner requested.
    pub requested_by: String,
    pub state: SessionRequestState,
    pub requested_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    /// On an exit_session row with action pr: the PR url the close recorded.
    pub pr_url: Option<String>,
    /// On an exit_session row: the wrap_up row whose id was the token, which
    /// this row consumed.
    pub wrap_up_request: Option<SessionRequestId>,
    /// On an exit_session or close row: the window the close cleared from the task,
    /// which the host tears down.
    pub tmux_window: Option<TmuxWindow>,
    /// On an exit_session row: the subtask the epic chain requested in the
    /// same transaction, if any, which `exit_session`'s response names.
    pub chained_task: Option<TaskId>,
    /// `requested_at` plus the timeout of the row's kind, as the module stored
    /// it. The host enforces THIS value (`pr-workflow.allium: WrapUpRebase`,
    /// `ExitSessionTeardown`), never a copy of the config. `None` on a row
    /// with no host work, which is never open and has no timer.
    pub deadline: Option<DateTime<Utc>>,
}

impl SessionRequest {
    /// `SessionRequest.is_open`: requested or running.
    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }

    /// `SessionRequest.has_host_work`: whether the row needs its host to do
    /// anything. A wrap_up with action done or pr is written already done and
    /// never does.
    pub fn has_host_work(&self) -> bool {
        self.kind.is_teardown() || self.action == WrapUpMode::Rebase
    }
}
