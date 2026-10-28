//! `MemoryReducerCaller` behaves like the real reducers, for the domains this
//! build covers.
//!
//! Spec: `docs/specs/spacetime-memory-store.allium`'s `ReducerConformance`
//! contract and `StandInFidelity` surface (`ConformanceIsCiGated`).
//!
//! Mirrors the shape `tests/spacetime_module.rs` and
//! `src/spacetime/tests/module_schema.rs` already use to diff the real module
//! against SQLite: stand up a throwaway SpacetimeDB instance, publish the real
//! module into it, then drive the SAME sequence of [`ReducerCaller`] calls —
//! literally the same trait, the same method, the same argument values —
//! against [`SdkReducerCaller`] (the real store) and against
//! [`MemoryReducerCaller`], and assert the two land on the same `SharedRows`
//! state. Sending one `bindings::Task`/`bindings::Epic` value to both callers
//! is what rules out the row itself drifting between the two arms; only the
//! two callers' OWN behaviour is under test.
//!
//! **What this does and does not prove.** `MemoryReducerCaller` calls the
//! module's own pure helpers directly (`derive_epic_status`,
//! `stamps_completion`, `apply_task_patch`, `apply_epic_patch`,
//! `apply_learning_patch`, `validate_task_ownership`, `validate_learning_scope`,
//! `validate_status`, `subscription_id`, `claimable_by`), and runs the module's
//! own feed-ingest body (`ingest_into`, generic over `IngestStore`) against its
//! rows — for those, "the same function" makes agreement structural rather
//! than something a test needs to establish. What
//! is NOT guaranteed by construction is everything this file's own
//! reimplementation invents by hand: id generation, row storage, delete
//! cascades, claim/release exclusivity (which side refuses, not its exact
//! refusal text — see `TaskShape`/`EpicShape` below for what row state IS
//! compared field-for-field), and upsert-by-key semantics for repo config and
//! subscriptions. This suite's scenario is chosen to exercise exactly that
//! surface.
//!
//! **Skipped when `spacetime` is not on `PATH`, except under CI**, where a
//! missing CLI fails the test instead. CI's Test and Coverage jobs install and
//! pin it — see `tests/spacetime_module.rs`'s own header for the full picture.
//!
//! **Every `ReducerDomain` now covered**: tasks_and_epics/repo_config/
//! subscriptions (task #4975), usage/agent_state (task #5004), learnings
//! (task #5003), settings (task #5002), and work packages (task #45061).
//! Extend the
//! scenario in `memory_caller_conformance/scenario.rs`, rather than starting a
//! new one, for any future
//! `ReducerDomain` — `spacetime-memory-store.allium`'s `ConformanceIsCiGated`
//! guarantee.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use common::spacetime_instance::{
    column, describe, module_path, spacetime_available_or_skip, Instance,
};
use dispatch_tui::clock::{Clock, SystemClock};
use dispatch_tui::models::{
    EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId, RetrievalSource,
    SubStatus, TaskId, TaskStatus,
};
use dispatch_tui::spacetime::bindings;
use dispatch_tui::sync::{
    MemoryReducerCaller, ReducerCaller, ReducerOutcome, SdkReducerCaller, SettledIdentity,
    SharedRows, SpacetimeSdkConnector, StoreConnector, SubscriptionRequest,
};

/// A wire-format timestamp (`encode::stamp`'s spelling) as an instant.
fn ts(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.3f")
        .unwrap()
        .and_utc()
}

// ---------------------------------------------------------------------------
// Row fixtures — one `bindings::Task`/`bindings::Epic`/`bindings::TaskPatch`
// value per step, sent to BOTH callers, so the row itself cannot drift
// between the two arms. Mirrors the module's own private `blank_task`/
// `blank_epic` defaults (spacetime/module/src/lib.rs).
// ---------------------------------------------------------------------------

fn blank_epic() -> bindings::Epic {
    bindings::Epic {
        id: 0,
        title: "conformance epic".into(),
        description: String::new(),
        status: "backlog".into(),
        plan_path: String::new(),
        sort_order: None,
        created_at: "2026-01-01 00:00:00.000".into(),
        updated_at: "2026-01-01 00:00:00.000".into(),
        auto_dispatch: false,
        parent_epic_id: 0,
        feed_command: String::new(),
        feed_interval_secs: 0,
        group_by_repo: false,
        feed_role: "none".into(),
        origin: "manual".into(),
        feed_append_only: false,
        completed_at: String::new(),
        created_by: String::new(),
        workspace_id: 0,
        visibility: "public".into(),
    }
}

fn blank_task_in_epic(epic_id: i64) -> bindings::Task {
    bindings::Task {
        id: 0,
        title: "conformance task".into(),
        description: String::new(),
        repo_path: "/repo".into(),
        status: "backlog".into(),
        worktree: String::new(),
        tmux_window: String::new(),
        plan_path: String::new(),
        epic_id,
        sub_status: "none".into(),
        tag: String::new(),
        sort_order: None,
        created_at: "2026-01-01 00:00:00.000".into(),
        updated_at: "2026-01-01 00:00:00.000".into(),
        base_branch: "main".into(),
        external_id: String::new(),
        labels: "[]".into(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        wrap_up_mode: String::new(),
        url: String::new(),
        url_type: String::new(),
        pr_learnings_gate_shown_at: String::new(),
        auto_run_plan: false,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        live_shells: 0,
        oldest_live_shell_started_at: String::new(),
        last_peer_message_sent_at: String::new(),
        last_peer_message_received_at: String::new(),
        phoenix: false,
        host: String::new(),
        owner: String::new(),
        completed_at: String::new(),
        created_by: String::new(),
        alias: String::new(),
        schedule: String::new(),
        next_due: String::new(),
        schedule_error: String::new(),
        model: String::new(),
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
        schedule: None,
        model: None,
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

/// Mirrors the module's own private `blank_learning`, for test fixtures.
/// `created_at`/`updated_at` are set to a fixed, already-stale stamp rather
/// than left blank: `create_learning` never stamps either field itself (it
/// inserts the row as given), so this is the one row shape in this scenario
/// whose `created_at`/`updated_at` are NOT clock-taken and so can be compared
/// bit-for-bit like any other field — and a stale value lets
/// `archive_stale_learnings` below be exercised without waiting on wall-clock
/// time to pass.
fn blank_learning() -> bindings::Learning {
    bindings::Learning {
        id: 0,
        kind: "pitfall".into(),
        summary: "conformance learning".into(),
        detail: None,
        scope: "user".into(),
        scope_ref: None,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2020-01-01 00:00:00.000".into(),
        updated_at: "2020-01-01 00:00:00.000".into(),
        embedding: None,
        workspace_id: 0,
    }
}

/// A projection of `crate::models::Learning`. `updated_at` is dropped
/// outright — every write but `create_learning` stamps it from each side's
/// own clock — and `last_upvoted_at` is reduced to presence, on the same
/// reasoning `TaskShape` drops/reduces its own clock-taken fields.
#[derive(Debug, PartialEq)]
struct LearningShape {
    kind: dispatch_tui::models::LearningKind,
    summary: String,
    detail: Option<String>,
    scope: dispatch_tui::models::LearningScope,
    scope_ref: Option<String>,
    tags: Vec<String>,
    status: dispatch_tui::models::LearningStatus,
    source_task_id: Option<i64>,
    upvote_count: i64,
    has_last_upvoted_at: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn learning_shape(l: &dispatch_tui::models::Learning) -> LearningShape {
    LearningShape {
        kind: l.kind,
        summary: l.summary.clone(),
        detail: l.detail.clone(),
        scope: l.scope,
        scope_ref: l.scope_ref.clone(),
        tags: l.tags.clone(),
        status: l.status,
        source_task_id: l.source_task_id.map(|t| t.0),
        upvote_count: l.upvote_count,
        has_last_upvoted_at: l.last_upvoted_at.is_some(),
        created_at: l.created_at,
    }
}

/// A projection of `crate::models::Task`. `updated_at` is dropped outright —
/// it is stamped by every write, from each side's own clock, so it would
/// never agree bit-for-bit — and `completed_at`/`last_pre_tool_use_at` are
/// each reduced to presence for the same reason, on the transitions this
/// scenario actually stamps them (`request_dispatch`'s claim for the latter).
/// Every other field of `crate::models::Task` is compared exactly, per
/// `ReducerConformance.SameEndState`'s "same resulting row state": including
/// the ones nothing in this scenario writes on either side (`tmux_window`,
/// `url`, `wrap_up_mode`, `last_notification_at`,
/// `last_peer_message_sent_at`/`last_peer_message_received_at`) costs
/// nothing — both sides hold whatever `blank_task_in_epic` gave them — and
/// would still catch either side spuriously stamping one.
///
/// **What this cannot compare through `TaskShape` itself, because
/// `crate::models::Task` does not carry it at all**: `owner`, `created_by`,
/// `pr_learnings_gate_shown_at`, `live_shells`, `oldest_live_shell_started_at`
/// and `stop_pending_at`. These are bindings/DB-only columns the board's read
/// model never surfaces — both `SdkReducerCaller` and `MemoryReducerCaller`
/// push through the same decoded `crate::models::Task`, via `SharedRows`, so a
/// divergence confined to one of these columns is invisible to a `TaskShape`
/// comparison. This matters concretely for `owner`: `set_task_epic`
/// (exercised below) mutates it natively on both sides, exactly the kind of
/// hand-reimplemented logic `ReducerConformance` exists to catch — see
/// `compare_owner` below, which reads it directly (SQL on the real side,
/// `MemoryReducerCaller::task_owner` on the fake side) rather than through
/// `SharedRows`, specifically because this suite would otherwise miss a bug
/// limited to that one field. The other five columns remain uncompared: none
/// of them is written by a covered reducer yet.
#[derive(Debug, PartialEq)]
struct TaskShape {
    title: String,
    description: String,
    repo_path: String,
    status: TaskStatus,
    sub_status: dispatch_tui::models::SubStatus,
    epic_id: Option<i64>,
    host: Option<String>,
    worktree: Option<String>,
    tmux_window: Option<dispatch_tui::models::TmuxWindow>,
    plan_path: Option<String>,
    url: Option<dispatch_tui::models::TaskUrl>,
    tag: Option<dispatch_tui::models::TaskTag>,
    sort_order: Option<i64>,
    base_branch: String,
    external_id: Option<String>,
    labels: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    last_notification_at: Option<chrono::DateTime<chrono::Utc>>,
    last_peer_message_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    last_peer_message_received_at: Option<chrono::DateTime<chrono::Utc>>,
    wrap_up_mode: Option<dispatch_tui::models::WrapUpMode>,
    auto_run_plan: bool,
    phoenix: bool,
    live_subagents: i64,
    stop_pending: bool,
    has_completed_at: bool,
    has_last_pre_tool_use_at: bool,
    schedule: Option<String>,
    next_due: Option<chrono::DateTime<chrono::Utc>>,
    schedule_error: Option<String>,
}

fn task_shape(t: &dispatch_tui::models::Task) -> TaskShape {
    TaskShape {
        title: t.title.clone(),
        description: t.description.clone(),
        repo_path: t.repo_path.clone(),
        status: t.status,
        sub_status: t.sub_status,
        epic_id: t.epic_id.map(|e| e.0),
        host: t.host.clone(),
        worktree: t.worktree.clone(),
        tmux_window: t.tmux_window.clone(),
        plan_path: t.plan_path.clone(),
        url: t.url.clone(),
        tag: t.tag,
        sort_order: t.sort_order,
        base_branch: t.base_branch.clone(),
        external_id: t.external_id.clone(),
        labels: t.labels.clone(),
        created_at: t.created_at,
        last_notification_at: t.last_notification_at,
        last_peer_message_sent_at: t.last_peer_message_sent_at,
        last_peer_message_received_at: t.last_peer_message_received_at,
        wrap_up_mode: t.wrap_up_mode,
        auto_run_plan: t.auto_run_plan,
        phoenix: t.phoenix,
        live_subagents: t.live_subagents,
        stop_pending: t.stop_pending,
        has_completed_at: t.completed_at.is_some(),
        has_last_pre_tool_use_at: t.last_pre_tool_use_at.is_some(),
        schedule: t.schedule.clone(),
        // Compared exactly: the scenarios use crons whose next match cannot
        // fall between the two sides' clocks (schedules.rs).
        next_due: t.next_due,
        schedule_error: t.schedule_error.clone(),
    }
}

/// The `Epic` twin of `TaskShape`: drops `updated_at` (clock-stamped by every
/// write) and reduces `completed_at` to presence, on the same reasoning.
/// Every other field of `crate::models::Epic` is compared exactly. `Epic`'s
/// own bindings-only column, `created_by`, is not part of
/// `crate::models::Epic` either — see `TaskShape`'s doc comment for what that
/// means for what this suite can and cannot catch.
#[derive(Debug, PartialEq)]
struct EpicShape {
    title: String,
    description: String,
    status: TaskStatus,
    plan_path: Option<String>,
    sort_order: Option<i64>,
    parent_epic_id: Option<i64>,
    auto_dispatch: bool,
    feed_command: Option<String>,
    feed_interval_secs: Option<i64>,
    group_by_repo: bool,
    feed_append_only: bool,
    feed_role: dispatch_tui::models::FeedRole,
    origin: dispatch_tui::models::EpicOrigin,
    created_at: chrono::DateTime<chrono::Utc>,
    has_completed_at: bool,
}

fn epic_shape(e: &dispatch_tui::models::Epic) -> EpicShape {
    EpicShape {
        title: e.title.clone(),
        description: e.description.clone(),
        status: e.status,
        plan_path: e.plan_path.clone(),
        sort_order: e.sort_order,
        parent_epic_id: e.parent_epic_id.map(|p| p.0),
        auto_dispatch: e.auto_dispatch,
        feed_command: e.feed_command.clone(),
        feed_interval_secs: e.feed_interval_secs,
        group_by_repo: e.group_by_repo,
        feed_append_only: e.feed_append_only,
        feed_role: e.feed_role,
        origin: e.origin,
        created_at: e.created_at,
        has_completed_at: e.completed_at.is_some(),
    }
}

/// What a session request row looks like once both sides wrote it: everything
/// but the ids and the module-clock timestamps, which differ by construction.
/// The references to other rows (the token, the chained subtask) are compared
/// as presence, because the two sides number their rows independently.
#[derive(Debug, PartialEq)]
struct SessionShape {
    task_id: TaskId,
    kind: dispatch_tui::models::SessionRequestKind,
    action: dispatch_tui::models::WrapUpMode,
    host: String,
    requested_by: String,
    state: dispatch_tui::models::SessionRequestState,
    reason: Option<String>,
    pr_url: Option<String>,
    tmux_window: Option<String>,
    has_wrap_up_request: bool,
    has_chained_task: bool,
    has_claimed_at: bool,
    has_finished_at: bool,
    deadline_after_requested: Option<chrono::Duration>,
}

fn session_shape(r: &dispatch_tui::models::SessionRequest) -> SessionShape {
    SessionShape {
        task_id: r.task_id,
        kind: r.kind,
        action: r.action,
        host: r.host.clone(),
        requested_by: r.requested_by.clone(),
        state: r.state,
        reason: r.reason.clone(),
        pr_url: r.pr_url.clone(),
        tmux_window: r.tmux_window.as_ref().map(|w| w.as_str().to_string()),
        has_wrap_up_request: r.wrap_up_request.is_some(),
        has_chained_task: r.chained_task.is_some(),
        has_claimed_at: r.claimed_at.is_some(),
        has_finished_at: r.finished_at.is_some(),
        deadline_after_requested: r.deadline.map(|d| d - r.requested_at),
    }
}

#[path = "memory_caller_conformance/agents.rs"]
mod agents;
#[path = "memory_caller_conformance/close_and_managed_epics.rs"]
mod close_and_managed_epics;
#[path = "memory_caller_conformance/data_rules.rs"]
mod data_rules;
#[path = "memory_caller_conformance/dispatch_requests.rs"]
mod dispatch_requests;
#[path = "memory_caller_conformance/feeds.rs"]
mod feeds;
#[path = "memory_caller_conformance/pr_state.rs"]
mod pr_state;
#[path = "memory_caller_conformance/scenario.rs"]
mod scenario;
#[path = "memory_caller_conformance/schedules.rs"]
mod schedules;
#[path = "memory_caller_conformance/session_requests.rs"]
mod session_requests;
