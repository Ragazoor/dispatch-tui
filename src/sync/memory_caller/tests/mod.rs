//! Tests for the in-process reducer twin, one file per domain.

use super::*;
use crate::models::EpicId;
use crate::service::FixedClock;
use chrono::{TimeZone, Utc};

/// A valid `created_at`/`updated_at` for fixtures — `required_timestamp`
/// (`src/sync/decode.rs`) refuses an empty one, unlike the module's own
/// `blank_task`/`blank_epic`, whose rows are never read back through
/// `SharedRows`.
const TEST_STAMP: &str = "2026-01-01 00:00:00.000";

/// A wire-format timestamp (`2026-01-01 00:00:00.000`) as an instant.
fn at(stamp: &str) -> DateTime<Utc> {
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.3f")
        .unwrap()
        .and_utc()
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    ))
}

fn caller() -> (MemoryReducerCaller, Arc<SharedRows>) {
    let rows = Arc::new(SharedRows::new());
    (MemoryReducerCaller::new(rows.clone(), clock()), rows)
}

/// `subagent_start` with `TEST_STAMP` as `started_at` — collapses the
/// repeated four-argument call every subagent-lifecycle test below makes.
async fn start_subagent(
    caller: &MemoryReducerCaller,
    task_id: TaskId,
    agent_id: &str,
    session_id: &str,
) -> i64 {
    caller
        .subagent_start(task_id, agent_id.into(), session_id.into(), at(TEST_STAMP))
        .await
        .unwrap()
}

/// The module's blank task, stamped and owned by the test user.
fn blank_task() -> bindings::Task {
    bindings::Task {
        created_at: TEST_STAMP.into(),
        updated_at: TEST_STAMP.into(),
        owner: "tester".into(),
        created_by: "tester".into(),
        ..module::blank_task().into()
    }
}

/// The module's blank epic, stamped and created by the test user.
fn blank_epic() -> bindings::Epic {
    bindings::Epic {
        created_at: TEST_STAMP.into(),
        updated_at: TEST_STAMP.into(),
        created_by: "tester".into(),
        ..module::blank_epic().into()
    }
}

fn blank_task_patch() -> bindings::TaskPatch {
    bindings::TaskPatch {
        title: None,
        description: None,
        repo_path: None,
        status: None,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: None,
        tag: None,
        sort_order: None,
        base_branch: None,
        external_id: None,
        labels: None,
        last_pre_tool_use_at: None,
        last_notification_at: None,
        wrap_up_mode: None,
        url: None,
        url_type: None,
        pr_learnings_gate_shown_at: None,
        auto_run_plan: None,
        live_subagents: None,
        stop_pending: None,
        stop_pending_at: None,
        last_peer_message_sent_at: None,
        last_peer_message_received_at: None,
        phoenix: None,
        host: None,
        owner: None,
        completed_at: None,
    }
}

fn blank_epic_patch() -> bindings::EpicPatch {
    bindings::EpicPatch {
        title: None,
        description: None,
        status: None,
        plan_path: None,
        sort_order: None,
        auto_dispatch: None,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        group_by_repo: None,
        feed_role: None,
        origin: None,
        feed_append_only: None,
        completed_at: None,
    }
}

mod agent_state;
mod config;
mod epics;
mod feed;
mod learnings;
mod tasks;
