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
//! `subscription_id`, `claimable_by`) — for those, "the same function" makes
//! agreement structural rather than something a test needs to establish. What
//! is NOT guaranteed by construction is everything this file's own
//! reimplementation invents by hand: id generation, row storage, delete
//! cascades, claim/release exclusivity (which side refuses, not its exact
//! refusal text — see `TaskShape`/`EpicShape` below for what row state IS
//! compared field-for-field), and upsert-by-key semantics for repo config and
//! subscriptions. This suite's scenario is chosen to exercise exactly that
//! surface.
//!
//! **Skipped when `spacetime` is not on `PATH`.** CI's Test job installs and
//! pins it, hard-failing the job rather than letting the install silently
//! fail — see `tests/spacetime_module.rs`'s own header for the full picture,
//! including why the Coverage job still takes this skip.
//!
//! **Every `ReducerDomain` now covered**: tasks_and_epics/repo_config/
//! subscriptions (task #4975), usage/agent_state (task #5004), learnings
//! (task #5003), and settings (task #5002, which lands last). Extend this
//! same file's scenario, rather than starting a new one, for any future
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
use dispatch_tui::models::{
    EpicId, LearningId, LearningVerdict, NotificationWrite, PollScopeId, RetrievalSource,
    SubStatus, TaskId, TaskStatus,
};
use dispatch_tui::service::{Clock, SystemClock};
use dispatch_tui::spacetime::bindings;
use dispatch_tui::sync::{
    MemoryReducerCaller, ReducerCaller, SdkReducerCaller, SettledIdentity, SharedRows,
    SpacetimeSdkConnector, StoreConnector, SubscriptionRequest,
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
/// scenario actually stamps them (`claim_backlog_task` for the latter).
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

/// The orchestration-fidelity scenario: create two epics and a task inside
/// the first, claim and release the task (including the worktree-attached
/// refusal), patch the epic, move the task to the second epic and back
/// (`set_task_epic`, exercising the epic-chain recalculation on both the
/// source and destination), round-trip repo config, round-trip a
/// subscription, then delete the task and both epics — asserting after every
/// step that the real store and `MemoryReducerCaller` agree.
#[test]
fn memory_caller_matches_the_real_reducers() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-conformance");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let mem = MemoryReducerCaller::new(rows_mem.clone(), clock);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let connector = Arc::new(SpacetimeSdkConnector::new(
            instance.database(),
            rows_real.clone(),
        ));
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        // `owner_board` must be non-empty hex (it becomes a SQL literal
        // matched against `owner`/`created_by`); this scenario never relies on
        // those two subscriptions, so any valid-looking hex string does.
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![1, 2],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));

        let mut woken = rows_real.changed();
        macro_rules! wait_for {
            ($cond:expr) => {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while !$cond {
                        woken.changed().await.expect("subscription must deliver");
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("real store never reached the expected state"))
            };
        }

        let compare_task = |id: i64| {
            assert_eq!(
                rows_real.task(TaskId(id)).map(|t| task_shape(&t)),
                rows_mem.task(TaskId(id)).map(|t| task_shape(&t)),
                "task {id}"
            );
        };
        let compare_epic = |id: i64| {
            assert_eq!(
                rows_real.epic(EpicId(id)).map(|e| epic_shape(&e)),
                rows_mem.epic(EpicId(id)).map(|e| epic_shape(&e)),
                "epic {id}"
            );
        };
        // For an epic whose `created_at` the STORE stamps
        // (`create_repo_group_sub_epic`/`create_managed_role_epic`, unlike
        // `create_epic`'s caller-supplied one) rather than the caller: drops
        // `created_at` from the comparison, on the same reasoning
        // `ReducerConformance.SameEndState` excludes it generally — each
        // side's own clock, not something to compare bit-for-bit.
        let compare_epic_ignoring_created_at = |id: i64| {
            assert_eq!(
                rows_real.epic(EpicId(id)).map(|e| EpicShape {
                    created_at: chrono::DateTime::UNIX_EPOCH,
                    ..epic_shape(&e)
                }),
                rows_mem.epic(EpicId(id)).map(|e| EpicShape {
                    created_at: chrono::DateTime::UNIX_EPOCH,
                    ..epic_shape(&e)
                }),
                "epic {id} (ignoring created_at)"
            );
        };
        // `owner` is a bindings/DB-only column `crate::models::Task` does not
        // carry (see `TaskShape`'s doc comment), so this reads it directly:
        // over SQL on the real side, off `MemoryReducerCaller`'s own table on
        // the fake side. The one field `set_task_epic` mutates natively that
        // `compare_task` cannot see.
        let compare_owner = |id: i64| {
            assert_eq!(
                column(
                    &instance,
                    &format!("SELECT owner FROM tasks WHERE id = {id}")
                ),
                mem.task_owner(id).unwrap_or_default(),
                "task {id} owner"
            );
        };
        let compare_learning = |id: LearningId| {
            assert_eq!(
                rows_real.learning(id).as_ref().map(learning_shape),
                rows_mem.learning(id).as_ref().map(learning_shape),
                "learning {id:?}"
            );
        };

        // -- create_epic (two: the task's home, and a destination to move it to) --
        let epic_id_real = real.create_epic(blank_epic()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(epic_id_real)).is_some());
        let epic_id_mem = mem.create_epic(blank_epic()).await.unwrap().0;
        assert_eq!(epic_id_real, epic_id_mem, "generated epic id");
        compare_epic(epic_id_real);

        let epic2_id_real = real.create_epic(blank_epic()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(epic2_id_real)).is_some());
        let epic2_id_mem = mem.create_epic(blank_epic()).await.unwrap().0;
        assert_eq!(epic2_id_real, epic2_id_mem, "generated second epic id");
        compare_epic(epic2_id_real);

        // -- create_task (in the epic, backlog) ----------------------------------
        let task_id_real = real
            .create_task(blank_task_in_epic(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.task(task_id_real).is_some());
        let task_id_mem = mem
            .create_task(blank_task_in_epic(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(task_id_real, task_id_mem, "generated task id");
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);

        // -- claim_backlog_task ---------------------------------------------------
        let real_claim = real
            .claim_backlog_task(task_id_real, "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Running));
        let mem_claim = mem
            .claim_backlog_task(task_id_mem, "host-a".into())
            .await
            .unwrap();
        assert_eq!(real_claim.won(), mem_claim.won(), "claim outcome");
        compare_task(task_id_real.0);

        // -- release_backlog_claim refuses while a worktree is attached ------------
        let attach_worktree = || bindings::TaskPatch {
            worktree: Some("/tmp/conformance".into()),
            ..blank_task_patch()
        };
        real.patch_task(task_id_real, attach_worktree())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.worktree.as_deref() == Some("/tmp/conformance")));
        mem.patch_task(task_id_mem, attach_worktree())
            .await
            .unwrap();
        compare_task(task_id_real.0);

        let real_release = real.release_backlog_claim(task_id_real).await.unwrap();
        let mem_release = mem.release_backlog_claim(task_id_mem).await.unwrap();
        assert!(
            !real_release.won(),
            "real must refuse with a worktree attached"
        );
        assert!(
            !mem_release.won(),
            "mem must refuse with a worktree attached"
        );
        compare_task(task_id_real.0);

        // Clear the worktree so release can actually apply, on both sides.
        let clear_worktree = || bindings::TaskPatch {
            worktree: Some(String::new()),
            ..blank_task_patch()
        };
        real.patch_task(task_id_real, clear_worktree())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.worktree.is_none()));
        mem.patch_task(task_id_mem, clear_worktree()).await.unwrap();
        compare_task(task_id_real.0);

        let real_release = real.release_backlog_claim(task_id_real).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        let mem_release = mem.release_backlog_claim(task_id_mem).await.unwrap();
        assert!(
            real_release.won() && mem_release.won(),
            "release must apply now"
        );
        compare_task(task_id_real.0);

        // -- patch_epic -------------------------------------------------------------
        real.patch_epic(
            EpicId(epic_id_real),
            bindings::EpicPatch {
                title: Some("renamed conformance epic".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .epic(EpicId(epic_id_real))
            .is_some_and(|e| e.title == "renamed conformance epic"));
        mem.patch_epic(
            EpicId(epic_id_mem),
            bindings::EpicPatch {
                title: Some("renamed conformance epic".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
        compare_epic(epic_id_real);

        // -- set_task_epic: move the (backlog) task to the second epic --------------
        real.set_task_epic(task_id_real, Some(EpicId(epic2_id_real)), String::new())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.epic_id == Some(EpicId(epic2_id_real))));
        mem.set_task_epic(task_id_mem, Some(EpicId(epic2_id_mem)), String::new())
            .await
            .unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);
        compare_epic(epic2_id_real);
        compare_owner(task_id_real.0);

        // -- recalculate_epic_status is idempotent when nothing changed -------------
        real.recalculate_epic_status(EpicId(epic2_id_real))
            .await
            .unwrap();
        mem.recalculate_epic_status(EpicId(epic2_id_mem))
            .await
            .unwrap();
        compare_epic(epic2_id_real);

        // Move it back, so the delete section below only has one epic's worth
        // of tasks to clean up.
        real.set_task_epic(task_id_real, Some(EpicId(epic_id_real)), String::new())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.epic_id == Some(EpicId(epic_id_real))));
        mem.set_task_epic(task_id_mem, Some(EpicId(epic_id_mem)), String::new())
            .await
            .unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);
        compare_epic(epic2_id_real);
        compare_owner(task_id_real.0);

        // -- repo configuration ---------------------------------------------------
        real.save_repo_path("/repo".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        wait_for!(!rows_real.repo_paths().is_empty());
        mem.save_repo_path("/repo".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        assert_eq!(rows_real.repo_paths(), rows_mem.repo_paths(), "repo_paths");

        real.set_verify_command("/repo".into(), "cargo test".into())
            .await
            .unwrap();
        wait_for!(rows_real.verify_command("/repo").as_deref() == Some("cargo test"));
        mem.set_verify_command("/repo".into(), "cargo test".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.verify_command("/repo"),
            rows_mem.verify_command("/repo"),
            "verify_command"
        );

        real.record_base_branch("/repo".into(), "main".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        wait_for!(!rows_real.base_branches().is_empty());
        mem.record_base_branch("/repo".into(), "main".into(), ts("2026-01-01 00:00:00.000"))
            .await
            .unwrap();
        assert_eq!(
            rows_real.base_branches(),
            rows_mem.base_branches(),
            "base_branches"
        );

        real.delete_repo_path("/repo".into()).await.unwrap();
        wait_for!(rows_real.repo_paths().is_empty());
        mem.delete_repo_path("/repo".into()).await.unwrap();
        assert!(rows_mem.repo_paths().is_empty());

        // -- subscriptions ----------------------------------------------------------
        // The connection's OWN identity, not an arbitrary hex string: this
        // connection's subscription asks for `subscriptions WHERE subscriber =
        // '{accepted.identity}'` (`subscription_queries`), so a row filed
        // under any other subscriber would never arrive here to compare.
        let subscriber = accepted.identity.as_str();
        real.subscribe_to_epic(subscriber.into(), EpicId(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.subscribed_epics(subscriber) == vec![epic_id_real]);
        mem.subscribe_to_epic(subscriber.into(), EpicId(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(
            rows_real.subscribed_epics(subscriber),
            rows_mem.subscribed_epics(subscriber),
            "subscribed_epics after subscribe"
        );

        real.unsubscribe_from_epic(subscriber.into(), EpicId(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.subscribed_epics(subscriber).is_empty());
        mem.unsubscribe_from_epic(subscriber.into(), EpicId(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(
            rows_real.subscribed_epics(subscriber),
            rows_mem.subscribed_epics(subscriber),
            "subscribed_epics after unsubscribe"
        );

        // -- settings ---------------------------------------------------------------
        // Scoped by HOST (`docs/specs/settings.allium`), not by owner — "host-a"
        // is the same host this scenario's subscription already asks
        // `settings WHERE host = 'host-a'` for (`subscription_queries`), so a
        // save lands in `rows_real` without a second subscribe.
        real.save_setting("host-a".into(), "theme".into(), "dark".into())
            .await
            .unwrap();
        wait_for!(rows_real.setting("theme").as_deref() == Some("dark"));
        mem.save_setting("host-a".into(), "theme".into(), "dark".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.setting("theme"),
            rows_mem.setting("theme"),
            "setting after save"
        );

        // A second save of the same (host, key) upserts rather than adding a
        // row (`SaveSetting`'s `@guidance`).
        real.save_setting("host-a".into(), "theme".into(), "light".into())
            .await
            .unwrap();
        wait_for!(rows_real.setting("theme").as_deref() == Some("light"));
        mem.save_setting("host-a".into(), "theme".into(), "light".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.setting("theme"),
            rows_mem.setting("theme"),
            "setting after re-save"
        );

        // An empty host is refused on both sides, and the value already saved
        // above is left untouched.
        let real_empty_host = real
            .save_setting(String::new(), "theme".into(), "purple".into())
            .await
            .unwrap();
        let mem_empty_host = mem
            .save_setting(String::new(), "theme".into(), "purple".into())
            .await
            .unwrap();
        assert!(!real_empty_host.won(), "real must refuse an empty host");
        assert!(!mem_empty_host.won(), "mem must refuse an empty host");
        assert_eq!(
            rows_real.setting("theme"),
            rows_mem.setting("theme"),
            "setting after refused save"
        );

        // Clearing a key that was never set is a no-op, not a refusal
        // (`ClearSetting`'s `@guidance`).
        let real_noop_clear = real
            .clear_setting("host-a".into(), "never-set".into())
            .await
            .unwrap();
        let mem_noop_clear = mem
            .clear_setting("host-a".into(), "never-set".into())
            .await
            .unwrap();
        assert!(real_noop_clear.won(), "clearing an absent key is a no-op");
        assert!(mem_noop_clear.won(), "clearing an absent key is a no-op");

        real.clear_setting("host-a".into(), "theme".into())
            .await
            .unwrap();
        wait_for!(rows_real.setting("theme").is_none());
        mem.clear_setting("host-a".into(), "theme".into())
            .await
            .unwrap();
        assert!(rows_mem.setting("theme").is_none());

        // Reused below and by the final delete section: task #4971's
        // `delete_task`/`batch_delete` `requires: task.status = done` guard
        // means every task this scenario deletes must be marked done first.
        let mark_done = || bindings::TaskPatch {
            status: Some("done".into()),
            ..blank_task_patch()
        };

        // -- usage events (usage) ---------------------------------------------------
        let cap = 3i64;
        let usage_event = |actor: &str| bindings::UsageEvent {
            id: 0,
            recorded_at: "2026-01-01 00:00:00.000".into(),
            category: "tool".into(),
            action: "used".into(),
            detail: None,
            actor: actor.into(),
        };
        for _ in 0..5 {
            real.record_usage_event(usage_event("conformance"), cap)
                .await
                .unwrap();
        }
        wait_for!(
            rows_real
                .usage_summary(&dispatch_tui::db::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>()
                == cap
        );
        for _ in 0..5 {
            mem.record_usage_event(usage_event("conformance"), cap)
                .await
                .unwrap();
        }
        assert_eq!(
            rows_real
                .usage_summary(&dispatch_tui::db::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>(),
            rows_mem
                .usage_summary(&dispatch_tui::db::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>(),
            "usage_events after prune"
        );
        let refused_real = real
            .record_usage_event(usage_event("conformance"), 0)
            .await
            .unwrap();
        let refused_mem = mem
            .record_usage_event(usage_event("conformance"), 0)
            .await
            .unwrap();
        assert_eq!(refused_real.won(), refused_mem.won(), "non-positive cap");

        // -- learnings, retrievals and verdicts (learnings, task #5003) -------------
        let helped_real = real.create_learning(blank_learning()).await.unwrap();
        wait_for!(rows_real.learning(helped_real).is_some());
        let helped_mem = mem.create_learning(blank_learning()).await.unwrap();
        assert_eq!(helped_real, helped_mem, "generated learning id");
        compare_learning(helped_real);

        let wrong_real = real.create_learning(blank_learning()).await.unwrap();
        wait_for!(rows_real.learning(wrong_real).is_some());
        let wrong_mem = mem.create_learning(blank_learning()).await.unwrap();
        assert_eq!(wrong_real, wrong_mem, "generated second learning id");

        // `ApprovedLearningsHaveScopeRef` (`docs/specs/learnings.allium`),
        // enforced server-side by both `create_learning`s.
        let bad_scope_real = real
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: None,
                ..blank_learning()
            })
            .await;
        let bad_scope_mem = mem
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: None,
                ..blank_learning()
            })
            .await;
        assert_eq!(
            bad_scope_real.is_err(),
            bad_scope_mem.is_err(),
            "scope-ref validation"
        );

        let revise = || bindings::LearningPatch {
            status: None,
            summary: Some("revised".into()),
            embedding: None,
        };
        real.patch_learning(helped_real, revise()).await.unwrap();
        wait_for!(rows_real
            .learning(helped_real)
            .is_some_and(|l| l.summary == "revised"));
        mem.patch_learning(helped_mem, revise()).await.unwrap();
        compare_learning(helped_real);

        real.record_learning_retrieval(task_id_real, helped_real, RetrievalSource::QueryLearnings)
            .await
            .unwrap();
        wait_for!(!rows_real.retrievals_for_task(task_id_real).is_empty());
        mem.record_learning_retrieval(task_id_mem, helped_mem, RetrievalSource::QueryLearnings)
            .await
            .unwrap();
        assert_eq!(
            rows_real.retrievals_for_task(task_id_real).len(),
            rows_mem.retrievals_for_task(task_id_mem).len(),
            "retrievals after record_learning_retrieval"
        );

        let verdicts = |helped_id: LearningId, wrong_id: LearningId| {
            vec![
                (helped_id, LearningVerdict::Helped),
                (wrong_id, LearningVerdict::Wrong),
            ]
        };
        real.apply_learning_verdicts(verdicts(helped_real, wrong_real))
            .await
            .unwrap();
        wait_for!(rows_real
            .learning(helped_real)
            .is_some_and(|l| l.upvote_count == 1));
        mem.apply_learning_verdicts(verdicts(helped_mem, wrong_mem))
            .await
            .unwrap();
        compare_learning(helped_real);
        compare_learning(wrong_real);

        // (An unknown verdict string is unrepresentable through the typed
        // `LearningVerdict`; the module's own tests cover that refusal.)

        // -- rescope_epic_learnings ---------------------------------------------------
        let epic_scoped_real = real
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: Some(epic_id_real.to_string()),
                ..blank_learning()
            })
            .await
            .unwrap();
        wait_for!(rows_real.learning(epic_scoped_real).is_some());
        let epic_scoped_mem = mem
            .create_learning(bindings::Learning {
                scope: "epic".into(),
                scope_ref: Some(epic_id_mem.to_string()),
                ..blank_learning()
            })
            .await
            .unwrap();
        assert_eq!(
            epic_scoped_real, epic_scoped_mem,
            "generated epic-scoped learning id"
        );
        real.rescope_epic_learnings(EpicId(epic_id_real), EpicId(epic2_id_real))
            .await
            .unwrap();
        wait_for!(rows_real
            .learning(epic_scoped_real)
            .is_some_and(|l| l.scope_ref == Some(epic2_id_real.to_string())));
        mem.rescope_epic_learnings(EpicId(epic_id_mem), EpicId(epic2_id_mem))
            .await
            .unwrap();
        compare_learning(epic_scoped_real);

        // -- archive_stale_learnings --------------------------------------------------
        // `blank_learning`'s fixed, already-stale `created_at`/`updated_at`
        // makes this deterministic rather than waiting on wall-clock time.
        let stale_real = real.create_learning(blank_learning()).await.unwrap();
        wait_for!(rows_real.learning(stale_real).is_some());
        let stale_mem = mem.create_learning(blank_learning()).await.unwrap();
        assert_eq!(stale_real, stale_mem, "generated stale learning id");
        real.archive_stale_learnings(ts("2025-01-01 00:00:00.000"))
            .await
            .unwrap();
        wait_for!(rows_real
            .learning(stale_real)
            .is_some_and(|l| l.status == dispatch_tui::models::LearningStatus::Archived));
        mem.archive_stale_learnings(ts("2025-01-01 00:00:00.000"))
            .await
            .unwrap();
        compare_learning(stale_real);

        // -- delete_learning + its retrieval cascade ---------------------------------
        real.delete_learning(wrong_real).await.unwrap();
        wait_for!(rows_real.learning(wrong_real).is_none());
        mem.delete_learning(wrong_mem).await.unwrap();
        assert!(rows_mem.learning(wrong_mem).is_none());

        // A missing id is REFUSED, unlike most reducers' silent no-op.
        let missing_real = real.delete_learning(wrong_real).await.unwrap();
        let missing_mem = mem.delete_learning(wrong_mem).await.unwrap();
        assert_eq!(missing_real.won(), missing_mem.won(), "double delete");

        // -- agent session state: subagents + hooks (agent_state) -------------------
        real.claim_backlog_task(task_id_real, "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Running));
        mem.claim_backlog_task(task_id_mem, "host-a".into())
            .await
            .unwrap();
        compare_task(task_id_real.0);

        let real_live = real
            .subagent_start(
                task_id_real,
                "agent-a".into(),
                "session-1".into(),
                ts("2026-01-01 00:00:00.000"),
            )
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == real_live));
        let mem_live = mem
            .subagent_start(
                task_id_mem,
                "agent-a".into(),
                "session-1".into(),
                ts("2026-01-01 00:00:00.000"),
            )
            .await
            .unwrap();
        assert_eq!(real_live, mem_live, "subagent_start live count");
        compare_task(task_id_real.0);

        real.record_pre_tool_use(
            task_id_real,
            SubStatus::Active,
            ts("2026-01-01 00:00:01.000"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.last_pre_tool_use_at.is_some()));
        mem.record_pre_tool_use(
            task_id_mem,
            SubStatus::Active,
            ts("2026-01-01 00:00:01.000"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        real.record_notification(
            task_id_real,
            NotificationWrite::Raise,
            ts("2026-01-01 00:00:02.000"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::NeedsInput));
        mem.record_notification(
            task_id_mem,
            NotificationWrite::Raise,
            ts("2026-01-01 00:00:02.000"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        // A Stop while a subagent is live defers rather than flips.
        let real_deferred = real
            .try_record_stop(task_id_real, ts("2026-01-01 00:00:03.000"))
            .await
            .unwrap();
        wait_for!(rows_real.task(task_id_real).is_some_and(|t| t.stop_pending));
        let mem_deferred = mem
            .try_record_stop(task_id_mem, ts("2026-01-01 00:00:03.000"))
            .await
            .unwrap();
        assert_eq!(
            real_deferred, mem_deferred,
            "try_record_stop deferred outcome"
        );
        compare_task(task_id_real.0);

        real.record_user_prompt_submit(
            task_id_real,
            ts("2026-01-01 00:00:04.000"),
            ts("2026-01-01 00:00:04.500"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| !t.stop_pending));
        mem.record_user_prompt_submit(
            task_id_mem,
            ts("2026-01-01 00:00:04.000"),
            ts("2026-01-01 00:00:04.500"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        let real_stop = real
            .subagent_stop(task_id_real, "agent-a".into(), "session-1".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == 0));
        let mem_stop = mem
            .subagent_stop(task_id_mem, "agent-a".into(), "session-1".into())
            .await
            .unwrap();
        assert_eq!(real_stop.live, mem_stop.live, "subagent_stop live count");
        assert_eq!(
            real_stop.is_review, mem_stop.is_review,
            "subagent_stop is_review"
        );
        compare_task(task_id_real.0);

        // subagent_clear / subagent_clear_and_void_pending_stop, over a fresh
        // subagent so there is something live to drain.
        real.subagent_start(
            task_id_real,
            "agent-b".into(),
            "session-2".into(),
            ts("2026-01-01 00:00:05.000"),
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == 1));
        mem.subagent_start(
            task_id_mem,
            "agent-b".into(),
            "session-2".into(),
            ts("2026-01-01 00:00:05.000"),
        )
        .await
        .unwrap();
        compare_task(task_id_real.0);

        let real_cleared = real.subagent_clear(task_id_real).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.live_subagents == 0));
        let mem_cleared = mem.subagent_clear(task_id_mem).await.unwrap();
        assert_eq!(
            real_cleared.live, mem_cleared.live,
            "subagent_clear live count"
        );
        compare_task(task_id_real.0);

        real.subagent_clear_and_void_pending_stop(task_id_real)
            .await
            .unwrap();
        mem.subagent_clear_and_void_pending_stop(task_id_mem)
            .await
            .unwrap();
        compare_task(task_id_real.0);

        // mark_pr_learnings_gate_shown: no board-visible field (bindings-only,
        // per `TaskShape`'s doc comment), so only the outcome parity matters —
        // the write itself is already committed by the time `.await` returns.
        real.mark_pr_learnings_gate_shown(task_id_real, ts("2026-01-01 00:00:06.000"))
            .await
            .unwrap();
        let real_gate_repeat = real
            .mark_pr_learnings_gate_shown(task_id_real, ts("2026-01-01 00:00:07.000"))
            .await
            .unwrap();
        mem.mark_pr_learnings_gate_shown(task_id_mem, ts("2026-01-01 00:00:06.000"))
            .await
            .unwrap();
        let mem_gate_repeat = mem
            .mark_pr_learnings_gate_shown(task_id_mem, ts("2026-01-01 00:00:07.000"))
            .await
            .unwrap();
        assert_eq!(
            real_gate_repeat.won(),
            mem_gate_repeat.won(),
            "repeat PR-learnings-gate outcome"
        );

        // batch_patch_sub_status: no recalculation, ignores a missing id.
        let updates = |task_id: TaskId| {
            vec![
                (task_id, SubStatus::Stale),
                (TaskId(999_999), SubStatus::Active),
            ]
        };
        real.batch_patch_sub_status(updates(task_id_real))
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::Stale));
        mem.batch_patch_sub_status(updates(task_id_mem))
            .await
            .unwrap();
        compare_task(task_id_real.0);

        // -- task watchers (agent_state) ----------------------------------------
        // `title` must differ from `task_id_real`'s: `create_task`'s id
        // read-back matches on `(title, repo_path, owner, epic_id, created_at,
        // created_by)` (`matches_create`), and this task shares `epic_id_real`
        // with `task_id_real`'s otherwise-identical `blank_task_in_epic`
        // defaults.
        let watcher_target_real = real
            .create_task(bindings::Task {
                title: "watcher target".into(),
                ..blank_task_in_epic(epic_id_real)
            })
            .await
            .unwrap();
        wait_for!(rows_real.task(watcher_target_real).is_some());
        let watcher_target_mem = mem
            .create_task(bindings::Task {
                title: "watcher target".into(),
                ..blank_task_in_epic(epic_id_mem)
            })
            .await
            .unwrap();
        assert_eq!(watcher_target_real, watcher_target_mem, "watcher target id");

        real.create_task_watcher(task_id_real, watcher_target_real)
            .await
            .unwrap();
        wait_for!(rows_real.watchers_of(watcher_target_real) == vec![task_id_real]);
        mem.create_task_watcher(task_id_mem, watcher_target_mem)
            .await
            .unwrap();
        assert_eq!(
            rows_real.watchers_of(watcher_target_real),
            rows_mem.watchers_of(watcher_target_mem),
            "watchers_of after create"
        );

        real.delete_task_watcher(task_id_real, watcher_target_real)
            .await
            .unwrap();
        wait_for!(rows_real.watchers_of(watcher_target_real).is_empty());
        mem.delete_task_watcher(task_id_mem, watcher_target_mem)
            .await
            .unwrap();
        assert!(rows_mem.watchers_of(watcher_target_mem).is_empty());

        real.create_task_watcher(task_id_real, watcher_target_real)
            .await
            .unwrap();
        wait_for!(!rows_real.watchers_of(watcher_target_real).is_empty());
        mem.create_task_watcher(task_id_mem, watcher_target_mem)
            .await
            .unwrap();
        real.delete_watches_of_target(watcher_target_real)
            .await
            .unwrap();
        wait_for!(rows_real.watchers_of(watcher_target_real).is_empty());
        mem.delete_watches_of_target(watcher_target_mem)
            .await
            .unwrap();
        assert!(rows_mem.watchers_of(watcher_target_mem).is_empty());

        real.create_task_watcher(task_id_real, watcher_target_real)
            .await
            .unwrap();
        wait_for!(!rows_real.watchers_of(watcher_target_real).is_empty());
        mem.create_task_watcher(task_id_mem, watcher_target_mem)
            .await
            .unwrap();
        real.delete_watches_by_watcher(task_id_real).await.unwrap();
        wait_for!(rows_real.watchers_of(watcher_target_real).is_empty());
        mem.delete_watches_by_watcher(task_id_mem).await.unwrap();
        assert!(rows_mem.watchers_of(watcher_target_mem).is_empty());

        // -- poll ownership (agent_state) ----------------------------------------
        real.claim_poll_owner(PollScopeId::Task(task_id_real), "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .poll_owner(PollScopeId::Task(task_id_real))
            .is_some());
        mem.claim_poll_owner(PollScopeId::Task(task_id_mem), "host-a".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.poll_owner(PollScopeId::Task(task_id_real)),
            rows_mem.poll_owner(PollScopeId::Task(task_id_mem)),
            "poll_owner after claim"
        );

        // A second claim leaves the existing owner alone.
        real.claim_poll_owner(PollScopeId::Task(task_id_real), "host-b".into())
            .await
            .unwrap();
        mem.claim_poll_owner(PollScopeId::Task(task_id_mem), "host-b".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.poll_owner(PollScopeId::Task(task_id_real)),
            rows_mem.poll_owner(PollScopeId::Task(task_id_mem)),
            "poll_owner unchanged after a second claim"
        );

        real.override_poll_owner(PollScopeId::Task(task_id_real), "host-b".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .poll_owner(PollScopeId::Task(task_id_real))
            .is_some_and(|p| p.host == "host-b"));
        mem.override_poll_owner(PollScopeId::Task(task_id_mem), "host-b".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.poll_owner(PollScopeId::Task(task_id_real)),
            rows_mem.poll_owner(PollScopeId::Task(task_id_mem)),
            "poll_owner after override"
        );

        // -- host registry (agent_state) -----------------------------------------
        // No board-visible reader on either side (see `SharedRows`'s own
        // comment on `hosts`), so only outcome parity is comparable here.
        let real_host = real
            .register_host("conformance-host".into(), "label".into(), String::new())
            .await
            .unwrap();
        let mem_host = mem
            .register_host("conformance-host".into(), "label".into(), String::new())
            .await
            .unwrap();
        assert_eq!(real_host.won(), mem_host.won(), "register_host");
        let real_host_empty = real
            .register_host(String::new(), "label".into(), String::new())
            .await
            .unwrap();
        let mem_host_empty = mem
            .register_host(String::new(), "label".into(), String::new())
            .await
            .unwrap();
        assert_eq!(
            real_host_empty.won(),
            mem_host_empty.won(),
            "register_host with an empty id"
        );

        // -- feed ingestion + repo-group/managed-role epics (agent_state) --------
        // `title` (not just `feed_command`) must differ from every other epic
        // this scenario creates: `create_epic`'s id read-back matches on
        // `(title, parent_epic_id, created_at, created_by)`
        // (`matches_created_epic`, src/sync/sdk_connector.rs) — NOT on
        // `feed_command` — so an otherwise-`blank_epic()` fixture would
        // collide with `epic_id_real`/`epic2_id_real` and read back the
        // wrong id.
        //
        // `created_by` must be this connection's OWN identity, not the
        // literal `"tester"` `blank_epic()` defaults to: unlike
        // `epic_id_real`/`epic2_id_real` (visible only because the test
        // pre-declares them as followed epics, `SubscriptionRequest::new`'s
        // `vec![1, 2]` above), a brand-new top-level epic has no such
        // pre-declared coverage — `own_creations`
        // (`SELECT * FROM epics WHERE created_by = '{owner}'`) is what makes
        // it visible for the id read-back at all.
        // A child of `epic_id_real`, not top-level: `delete_stale_subtree_feed_tasks`
        // below walks `epic_id_real`'s DIRECT CHILD epics, and nesting it here
        // also means its own child tasks fall under the followed-epic subtree
        // subscription, not just `own_creations`.
        let feed_epic = |parent_epic_id: i64| bindings::Epic {
            title: "feed epic".into(),
            feed_command: "some-command".into(),
            parent_epic_id,
            created_by: accepted.identity.clone(),
            ..blank_epic()
        };
        let feed_epic_id_real = real.create_epic(feed_epic(epic_id_real)).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(feed_epic_id_real)).is_some());
        let feed_epic_id_mem = mem.create_epic(feed_epic(epic_id_mem)).await.unwrap().0;
        assert_eq!(feed_epic_id_real, feed_epic_id_mem, "feed epic id");

        let feed_item = |external_id: &str| bindings::FeedTaskUpsertItem {
            external_id: external_id.into(),
            title: "feed task".into(),
            description: String::new(),
            repo_path: "/repo".into(),
            status: "backlog".into(),
            sub_status: "none".into(),
            base_branch: "main".into(),
            tag: String::new(),
            labels: "[]".into(),
            sort_order: None,
            url: String::new(),
            url_type: String::new(),
            wrap_up_mode: String::new(),
        };
        // `created_by` is this connection's own identity here too: a feed
        // task's home epic (`feed_epic_id_real`) is not a followed epic, so
        // only `own_creations` (`SELECT * FROM tasks WHERE created_by =
        // '{owner}'`) makes the inserted rows arrive on this subscription at
        // all.
        real.upsert_feed_tasks(
            EpicId(feed_epic_id_real),
            vec![feed_item("feed-a"), feed_item("feed-b")],
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len() == 2);
        mem.upsert_feed_tasks(
            EpicId(feed_epic_id_mem),
            vec![feed_item("feed-a"), feed_item("feed-b")],
            "conformance".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len(),
            rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem)).len(),
            "feed task count after first upsert"
        );

        // Additive: an absent item is left alone.
        real.upsert_feed_tasks_additive(
            EpicId(feed_epic_id_real),
            vec![feed_item("feed-a")],
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        mem.upsert_feed_tasks_additive(
            EpicId(feed_epic_id_mem),
            vec![feed_item("feed-a")],
            "conformance".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len(),
            2,
            "additive upsert must not remove feed-b"
        );

        // Non-additive: absent items are removed.
        real.upsert_feed_tasks(
            EpicId(feed_epic_id_real),
            vec![feed_item("feed-a")],
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len() == 1);
        mem.upsert_feed_tasks(
            EpicId(feed_epic_id_mem),
            vec![feed_item("feed-a")],
            "conformance".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows_real.tasks_for_epic(EpicId(feed_epic_id_real)).len(),
            rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem)).len(),
            "feed task count after stale-delete"
        );

        real.delete_stale_subtree_feed_tasks(EpicId(epic_id_real), vec![])
            .await
            .unwrap();
        wait_for!(rows_real
            .tasks_for_epic(EpicId(feed_epic_id_real))
            .is_empty());
        mem.delete_stale_subtree_feed_tasks(EpicId(epic_id_mem), vec![])
            .await
            .unwrap();
        assert!(rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem)).is_empty());

        // `created_by` is this connection's own identity, same reasoning as
        // `feed_epic`'s above: these are new top-level-from-the-client's-view
        // epics (a child of the followed `epic_id_real`, but its read-back
        // runs inside the SAME transaction callback the create fired, before
        // any subtree-widening re-subscribe could land it any other way), so
        // `own_creations` is what makes them visible for the id read-back.
        let real_repo_group = real
            .create_repo_group_sub_epic(
                EpicId(epic_id_real),
                "repo-group".into(),
                accepted.identity.clone(),
            )
            .await
            .unwrap();
        wait_for!(rows_real.epic(real_repo_group).is_some());
        let mem_repo_group = mem
            .create_repo_group_sub_epic(
                EpicId(epic_id_mem),
                "repo-group".into(),
                accepted.identity.clone(),
            )
            .await
            .unwrap();
        assert_eq!(real_repo_group, mem_repo_group, "repo-group sub-epic id");
        compare_epic_ignoring_created_at(real_repo_group.0);
        // Find-or-create: a repeat resolves to the same id.
        let real_repo_group_again = real
            .create_repo_group_sub_epic(
                EpicId(epic_id_real),
                "repo-group".into(),
                accepted.identity.clone(),
            )
            .await
            .unwrap();
        let mem_repo_group_again = mem
            .create_repo_group_sub_epic(
                EpicId(epic_id_mem),
                "repo-group".into(),
                accepted.identity.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            real_repo_group_again, real_repo_group,
            "real repo-group is found, not recreated"
        );
        assert_eq!(
            mem_repo_group_again, mem_repo_group,
            "mem repo-group is found, not recreated"
        );

        let real_managed = real
            .create_managed_role_epic(
                "reviewer".into(),
                Some(EpicId(epic_id_real)),
                "review".into(),
                "cmd".into(),
                60,
                accepted.identity.clone(),
            )
            .await
            .unwrap();
        wait_for!(rows_real.epic(real_managed).is_some());
        let mem_managed = mem
            .create_managed_role_epic(
                "reviewer".into(),
                Some(EpicId(epic_id_mem)),
                "review".into(),
                "cmd".into(),
                60,
                "conformance".into(),
            )
            .await
            .unwrap();
        assert_eq!(real_managed, mem_managed, "managed-role epic id");
        compare_epic_ignoring_created_at(real_managed.0);

        // -- drop_closed_retired_feed_items (agent_state) ------------------------
        // Retirement only fires from the guarded `delete_task`, never from a
        // feed's own stale-reconciliation delete — see the module's own
        // `delete_stale_feed_tasks_in_epic` vs. `delete_task_side_effects`.
        real.upsert_feed_tasks(
            EpicId(feed_epic_id_real),
            vec![feed_item("retire-me")],
            accepted.identity.clone(),
        )
        .await
        .unwrap();
        wait_for!(!rows_real
            .tasks_for_epic(EpicId(feed_epic_id_real))
            .is_empty());
        mem.upsert_feed_tasks(
            EpicId(feed_epic_id_mem),
            vec![feed_item("retire-me")],
            "conformance".into(),
        )
        .await
        .unwrap();
        let retire_task_real = rows_real.tasks_for_epic(EpicId(feed_epic_id_real))[0].id;
        let retire_task_mem = rows_mem.tasks_for_epic(EpicId(feed_epic_id_mem))[0].id;
        real.patch_task(retire_task_real, mark_done())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(retire_task_real)
            .is_some_and(|t| t.status == TaskStatus::Done));
        mem.patch_task(retire_task_mem, mark_done()).await.unwrap();
        real.delete_task(retire_task_real).await.unwrap();
        wait_for!(rows_real.task(retire_task_real).is_none());
        mem.delete_task(retire_task_mem).await.unwrap();
        assert_eq!(
            rows_real.retired_without_task(EpicId(feed_epic_id_real), &["retire-me".to_string()]),
            rows_mem.retired_without_task(EpicId(feed_epic_id_mem), &["retire-me".to_string()]),
            "retired_without_task before drop"
        );

        real.drop_closed_retired_feed_items(EpicId(feed_epic_id_real), vec![])
            .await
            .unwrap();
        wait_for!(rows_real
            .retired_without_task(EpicId(feed_epic_id_real), &["retire-me".to_string()])
            .is_empty());
        mem.drop_closed_retired_feed_items(EpicId(feed_epic_id_mem), vec![])
            .await
            .unwrap();
        assert!(rows_mem
            .retired_without_task(EpicId(feed_epic_id_mem), &["retire-me".to_string()])
            .is_empty());

        // -- respawn_phoenix_successor (agent_state) -----------------------------
        // Each of these three tasks needs its own distinct `title`: both
        // `create_task` and `respawn_phoenix_successor`'s id read-back match
        // on `(title, repo_path, owner, epic_id, created_at, created_by)`
        // (`matches_create`), and `blank_task_in_epic(epic_id_real)`'s
        // defaults would otherwise collide with `task_id_real` and each
        // other, all three sharing `epic_id_real`.
        let real_predecessor = real
            .create_task(bindings::Task {
                title: "phoenix predecessor".into(),
                status: "done".into(),
                phoenix: true,
                ..blank_task_in_epic(epic_id_real)
            })
            .await
            .unwrap();
        wait_for!(rows_real.task(real_predecessor).is_some());
        let mem_predecessor = mem
            .create_task(bindings::Task {
                title: "phoenix predecessor".into(),
                status: "done".into(),
                phoenix: true,
                ..blank_task_in_epic(epic_id_mem)
            })
            .await
            .unwrap();
        assert_eq!(real_predecessor, mem_predecessor, "phoenix predecessor id");

        let phoenix_successor = |epic_id: i64| bindings::Task {
            title: "phoenix successor".into(),
            ..blank_task_in_epic(epic_id)
        };
        let real_successor = real
            .respawn_phoenix_successor(real_predecessor, phoenix_successor(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.task(real_successor).is_some());
        let mem_successor = mem
            .respawn_phoenix_successor(mem_predecessor, phoenix_successor(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(real_successor, mem_successor, "phoenix successor id");
        wait_for!(rows_real.task(real_predecessor).is_some_and(|t| !t.phoenix));
        compare_task(real_predecessor.0);
        compare_task(real_successor.0);

        // Clean up the extra fixtures this section created, so the delete
        // section below only has the original task/epics' worth to close out.
        for (real_id, mem_id) in [
            (real_predecessor, mem_predecessor),
            (real_successor, mem_successor),
            (watcher_target_real, watcher_target_mem),
        ] {
            real.patch_task(real_id, mark_done()).await.unwrap();
            wait_for!(rows_real
                .task(real_id)
                .is_some_and(|t| t.status == TaskStatus::Done));
            mem.patch_task(mem_id, mark_done()).await.unwrap();
            real.delete_task(real_id).await.unwrap();
            wait_for!(rows_real.task(real_id).is_none());
            mem.delete_task(mem_id).await.unwrap();
        }
        real.delete_epic(real_repo_group).await.unwrap();
        wait_for!(rows_real.epic(real_repo_group).is_none());
        mem.delete_epic(mem_repo_group).await.unwrap();
        real.delete_epic(real_managed).await.unwrap();
        wait_for!(rows_real.epic(real_managed).is_none());
        mem.delete_epic(mem_managed).await.unwrap();
        real.delete_epic(EpicId(feed_epic_id_real)).await.unwrap();
        wait_for!(rows_real.epic(EpicId(feed_epic_id_real)).is_none());
        mem.delete_epic(EpicId(feed_epic_id_mem)).await.unwrap();

        // -- delete_task's learnings cascade (learnings, task #5003) ----------------
        // `delete_task_side_effects`'s learnings half: a sourced learning is
        // detached (`SET NULL`) rather than deleted, and every retrieval
        // recorded against the doomed task is dropped.
        let sourced_real = real
            .create_learning(bindings::Learning {
                source_task_id: Some(task_id_real.0),
                ..blank_learning()
            })
            .await
            .unwrap();
        wait_for!(rows_real.learning(sourced_real).is_some());
        let sourced_mem = mem
            .create_learning(bindings::Learning {
                source_task_id: Some(task_id_mem.0),
                ..blank_learning()
            })
            .await
            .unwrap();
        assert_eq!(sourced_real, sourced_mem, "generated sourced learning id");
        real.record_learning_retrieval(task_id_real, sourced_real, RetrievalSource::QueryLearnings)
            .await
            .unwrap();
        wait_for!(!rows_real.retrievals_for_task(task_id_real).is_empty());
        mem.record_learning_retrieval(task_id_mem, sourced_mem, RetrievalSource::QueryLearnings)
            .await
            .unwrap();

        // -- delete_task, then delete_epic ---------------------------------------
        // task #4971 added `delete_task`'s `requires: task.status = done` guard
        // (spacetime/module/src/lib.rs::delete_task) — this scenario predates
        // that guard, so the task must be marked done first or the real side
        // refuses the delete while the (guard-unaware) mem side does not.
        real.patch_task(task_id_real, mark_done()).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Done));
        mem.patch_task(task_id_mem, mark_done()).await.unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);

        real.delete_task(task_id_real).await.unwrap();
        wait_for!(rows_real.task(task_id_real).is_none());
        mem.delete_task(task_id_mem).await.unwrap();
        assert!(rows_mem.task(task_id_mem).is_none());
        compare_epic(epic_id_real);

        wait_for!(rows_real
            .learning(sourced_real)
            .is_some_and(|l| l.source_task_id.is_none()));
        compare_learning(sourced_real);
        assert!(rows_real.retrievals_for_task(task_id_real).is_empty());
        assert!(rows_mem.retrievals_for_task(task_id_mem).is_empty());

        real.delete_epic(EpicId(epic_id_real)).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic_id_real)).is_none());
        mem.delete_epic(EpicId(epic_id_mem)).await.unwrap();
        assert!(rows_mem.epic(EpicId(epic_id_mem)).is_none());

        real.delete_epic(EpicId(epic2_id_real)).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic2_id_real)).is_none());
        mem.delete_epic(EpicId(epic2_id_mem)).await.unwrap();
        assert!(rows_mem.epic(EpicId(epic2_id_mem)).is_none());
    });
}
