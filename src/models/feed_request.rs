//! A feed request row.
//!
//! Spec: `docs/specs/store-split.allium`'s `FeedRequest`. Work one host asks
//! another to do for a feed epic: a `refresh` asks the epic's poll owner to run
//! one feed cycle, a `teardown` asks the host that dispatched a removed feed
//! task to release its worktree and window. The module owns the row; the board
//! reads it through its subscription
//! (`sync.allium: SubscribeToThisHostsFeedRequests`), which delivers only the
//! rows naming this machine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EpicId, TaskId, TmuxWindow};
use crate::define_id_newtype;
use crate::define_str_enum;

define_id_newtype!(FeedRequestId, feed_request_id_tests);

/// What a feed request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedRequestKind {
    Refresh,
    Teardown,
}

define_str_enum!(FeedRequestKind, "feed request kind" {
    Refresh => "refresh",
    Teardown => "teardown",
});

/// Where a request is in its life. `Requested` and `Running` are open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedRequestState {
    Requested,
    Running,
    Done,
    Failed,
}

define_str_enum!(FeedRequestState, "feed request state" {
    Requested => "requested",
    Running => "running",
    Done => "done",
    Failed => "failed",
});

impl FeedRequestState {
    /// `FeedRequest.is_open`.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Requested | Self::Running)
    }
}

/// One `feed_requests` row as the board reads it. An absent column (`""` or `0`
/// in the module) is `None` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedRequest {
    pub id: FeedRequestId,
    pub kind: FeedRequestKind,
    /// The machine that serves the row.
    pub host: String,
    /// On a refresh row: the feed epic to run.
    pub epic_id: Option<EpicId>,
    /// On a teardown row: the removed task, for logs.
    pub task_id: Option<TaskId>,
    /// Teardown snapshots of the task row before its delete.
    pub repo_path: String,
    pub worktree: Option<String>,
    pub tmux_window: Option<TmuxWindow>,
    /// The identity whose call wrote the request, stamped by the module. A
    /// host's board serves only rows its own owner requested.
    pub requested_by: String,
    pub state: FeedRequestState,
    pub requested_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
}
