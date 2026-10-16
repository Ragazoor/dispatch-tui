use super::*;

use crate::models::{
    test_tmux_window, DispatchMode, EpicId, SubStatus, TaskId, TaskStatus, TaskTag,
    ACTIVE_THRESHOLD, DEFAULT_QUICK_TASK_TITLE,
};

use crate::tui::commands::{PrCommand, SystemCommand, TaskCommand};
use crate::tui::messages::{InputMessage, PrMessage, SystemMessage, TaskMessage};

use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Auto-dispatch failure — SurfaceAutoDispatchFailure / AutoDispatchFailureIndicator
// in docs/specs/epics.allium
// ---------------------------------------------------------------------------

/// A dispatch request row for task `task` on this (unnamed) host.
fn request_row(
    id: i64,
    task: i64,
    state: crate::models::DispatchRequestState,
) -> crate::models::DispatchRequest {
    use crate::models::DispatchRequestState as S;
    let requested_at = chrono::Utc::now();
    let finished = matches!(state, S::Dispatched | S::Failed);
    crate::models::DispatchRequest {
        id: crate::models::DispatchRequestId(id),
        task_id: TaskId(task),
        host: "this-machine".to_string(),
        requested_by: "owner".to_string(),
        reused_worktree: false,
        mode: DispatchMode::Dispatch,
        origin: crate::models::DispatchOrigin::User,
        state,
        requested_at,
        claimed_at: (state != S::Requested).then_some(requested_at),
        finished_at: finished.then_some(requested_at),
        reason: (state == S::Failed).then(|| "git fetch origin main failed".to_string()),
        worktree: None,
        tmux_window: None,
        deadline: requested_at + chrono::Duration::seconds(60),
    }
}

fn requests_refreshed(rows: Vec<crate::models::DispatchRequest>) -> Message {
    Message::Task(TaskMessage::DispatchRequestsRefreshed(rows))
}

mod agent_health;
mod auto_dispatch_failure;
mod dispatch_lifecycle;
mod in_flight;
mod pr_polling;
mod quick_dispatch;
mod space_action;
mod trust;
