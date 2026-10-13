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
//! **Skipped when `spacetime` is not on `PATH`, except under CI**, where a
//! missing CLI fails the test instead. CI's Test and Coverage jobs install and
//! pin it — see `tests/spacetime_module.rs`'s own header for the full picture.
//!
//! **Every `ReducerDomain` now covered**: tasks_and_epics/repo_config/
//! subscriptions (task #4975), usage/agent_state (task #5004), learnings
//! (task #5003), settings (task #5002), and work packages (task #45061).
//! Extend this
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
        wait_for!(rows_real.subscribed_epics(subscriber) == vec![EpicId(epic_id_real)]);
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
                .usage_summary(&dispatch_tui::store::UsageQuery::default())
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
                .usage_summary(&dispatch_tui::store::UsageQuery::default())
                .iter()
                .map(|s| s.count)
                .sum::<i64>(),
            rows_mem
                .usage_summary(&dispatch_tui::store::UsageQuery::default())
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
        let run = || bindings::TaskPatch {
            status: Some("running".into()),
            sub_status: Some("active".into()),
            ..blank_task_patch()
        };
        real.patch_task(task_id_real, run()).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Running));
        mem.patch_task(task_id_mem, run()).await.unwrap();
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

        // -- work packages (`work-packages.allium`) -----------------------------
        // `updated_at` is the store's own clock on a patch, so it is left out
        // of the comparison, on `SameEndState`'s reasoning.
        let package_shapes = |rows: &SharedRows| -> Vec<(i64, String, String, Option<String>)> {
            rows.work_packages()
                .into_iter()
                .map(|p| (p.id.0, p.title, p.description, p.created_by))
                .collect()
        };
        let package_row = || bindings::WorkPackage {
            id: 0,
            title: "  Window 1 ".into(),
            description: "the plan".into(),
            created_by: accepted.identity.clone(),
            created_at: "2026-01-01 00:00:00.000".into(),
            updated_at: "2026-01-01 00:00:00.000".into(),
        };
        let package_real = real.create_work_package(package_row()).await.unwrap();
        wait_for!(!rows_real.work_packages().is_empty());
        let package_mem = mem.create_work_package(package_row()).await.unwrap();
        assert_eq!(package_real, package_mem, "generated work package id");
        assert_eq!(package_shapes(&rows_real), package_shapes(&rows_mem));

        let blank = bindings::WorkPackage {
            title: "  ".into(),
            ..package_row()
        };
        assert!(real.create_work_package(blank.clone()).await.is_err());
        assert!(mem.create_work_package(blank).await.is_err());

        let task_item = dispatch_tui::models::WorkPackageItem::Task(task_id_real);
        let epic_item = dispatch_tui::models::WorkPackageItem::Epic(EpicId(epic_id_real));
        for item in [task_item, epic_item, task_item] {
            let r = real
                .link_work_package_item(package_real, item)
                .await
                .unwrap();
            let m = mem.link_work_package_item(package_mem, item).await.unwrap();
            assert_eq!(r.won(), m.won(), "link {item:?}");
        }
        wait_for!(rows_real.work_package_links().len() == 2);
        assert_eq!(
            rows_real.work_package_links(),
            rows_mem.work_package_links(),
            "links after linking"
        );

        let unknown = dispatch_tui::models::WorkPackageItem::Task(TaskId(987_654));
        let r = real
            .link_work_package_item(package_real, unknown)
            .await
            .unwrap();
        let m = mem
            .link_work_package_item(package_mem, unknown)
            .await
            .unwrap();
        assert!(!r.won(), "a link to an unknown task is refused");
        assert_eq!(r.won(), m.won(), "link to an unknown task");

        for _ in 0..2 {
            let r = real
                .unlink_work_package_item(package_real, epic_item)
                .await
                .unwrap();
            let m = mem
                .unlink_work_package_item(package_mem, epic_item)
                .await
                .unwrap();
            assert_eq!(r.won(), m.won(), "unlink");
        }
        wait_for!(rows_real.work_package_links().len() == 1);
        assert_eq!(
            rows_real.work_package_links(),
            rows_mem.work_package_links()
        );

        let rename = || bindings::WorkPackagePatch {
            title: Some(" Window 2".into()),
            description: None,
        };
        real.patch_work_package(package_real, rename())
            .await
            .unwrap();
        wait_for!(rows_real
            .work_packages()
            .first()
            .is_some_and(|p| p.title == "Window 2"));
        mem.patch_work_package(package_mem, rename()).await.unwrap();
        assert_eq!(package_shapes(&rows_real), package_shapes(&rows_mem));

        let missing = dispatch_tui::models::WorkPackageId(987_654);
        let r = real.patch_work_package(missing, rename()).await.unwrap();
        let m = mem.patch_work_package(missing, rename()).await.unwrap();
        assert!(!r.won(), "patching a missing package is refused");
        assert_eq!(r.won(), m.won(), "patch of a missing package");

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

        // `work-packages.allium: DeletingAnItemRemovesItsLinks` — the task was
        // the package's last link.
        wait_for!(rows_real.work_package_links().is_empty());
        assert!(rows_mem.work_package_links().is_empty());
        real.delete_work_package(package_real).await.unwrap();
        wait_for!(rows_real.work_packages().is_empty());
        mem.delete_work_package(package_mem).await.unwrap();
        assert!(rows_mem.work_packages().is_empty());

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

/// Store-split M2 (task #53255): the data rules that moved into the reducers
/// are enforced identically by `MemoryReducerCaller`.
///
/// The twin calls the module's own pure rule functions, so the verdicts agree
/// by construction; what this checks is the part written by hand on each side —
/// routing a task into its repo sub-epic, rerouting on a repo change, cleaning
/// up the emptied sub-epic, and `regroup_epic`/`flatten_epic` — and that a
/// refusal leaves the same state behind.
#[test]
fn memory_caller_enforces_the_same_data_rules_as_the_real_reducers() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-rules");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

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
        // Epics the scenario creates are seen through `own_creations`
        // (`created_by = identity`), so the twin writes as the same identity.
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone());
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        let identity = accepted.identity.clone();

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
        let same_epic = |id: i64| {
            let shape = |rows: &SharedRows| {
                rows.epic(EpicId(id)).map(|e| EpicShape {
                    created_at: chrono::DateTime::UNIX_EPOCH,
                    ..epic_shape(&e)
                })
            };
            assert_eq!(shape(&rows_real), shape(&rows_mem), "epic {id}");
        };
        let same_task = |id: i64| {
            assert_eq!(
                rows_real.task(TaskId(id)).map(|t| task_shape(&t)),
                rows_mem.task(TaskId(id)).map(|t| task_shape(&t)),
                "task {id}"
            );
        };
        // Both refused, with the same reason, and nothing changed.
        let refused = |real: ReducerOutcome, mem: ReducerOutcome| -> String {
            match (real, mem) {
                (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                    assert!(
                        r.contains(&m) || m.contains(&r),
                        "real {r:?} vs memory {m:?}"
                    );
                    m
                }
                other => panic!("both sides must refuse, got {other:?}"),
            }
        };

        // -- a grouped root, and a task created in it ---------------------------
        let grouped = bindings::Epic {
            group_by_repo: true,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root = real.create_epic(grouped.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root)).is_some());
        assert_eq!(mem.create_epic(grouped).await.unwrap().0, root);

        let in_root = |repo: &str| bindings::Task {
            repo_path: repo.into(),
            created_by: identity.clone(),
            ..blank_task_in_epic(root)
        };
        let task = real.create_task(in_root("/x/alpha")).await.unwrap();
        wait_for!(rows_real.task(task).is_some());
        assert_eq!(mem.create_task(in_root("/x/alpha")).await.unwrap(), task);
        same_task(task.0);
        let sub = rows_real.task(task).unwrap().epic_id.unwrap().0;
        assert_ne!(sub, root, "the task was routed into a repo sub-epic");
        wait_for!(rows_real.epic(EpicId(sub)).is_some());
        same_epic(sub);
        same_epic(root);

        // -- sub-status legality -------------------------------------------------
        let illegal = bindings::TaskPatch {
            sub_status: Some("active".into()),
            ..blank_task_patch()
        };
        let why = refused(
            real.patch_task(task, illegal.clone()).await.unwrap(),
            mem.patch_task(task, illegal).await.unwrap(),
        );
        assert!(why.contains("sub_status 'active' is not valid for status 'backlog'"));
        same_task(task.0);

        // -- reparent checks -----------------------------------------------------
        let into_own_sub_epic = bindings::EpicPatch {
            parent_epic_id: Some(sub),
            ..blank_epic_patch()
        };
        let why = refused(
            real.patch_epic(EpicId(root), into_own_sub_epic.clone())
                .await
                .unwrap(),
            mem.patch_epic(EpicId(root), into_own_sub_epic)
                .await
                .unwrap(),
        );
        assert!(why.contains("cycle"), "{why}");
        let detach = bindings::EpicPatch {
            parent_epic_id: Some(0),
            ..blank_epic_patch()
        };
        let why = refused(
            real.patch_epic(EpicId(sub), detach.clone()).await.unwrap(),
            mem.patch_epic(EpicId(sub), detach).await.unwrap(),
        );
        assert!(why.contains("Cannot reparent an auto-created repo-group sub-epic"));
        same_epic(root);
        same_epic(sub);

        // -- a repo change re-routes the task and removes the emptied sub-epic ---
        let to_beta = bindings::TaskPatch {
            repo_path: Some("/x/beta".into()),
            ..blank_task_patch()
        };
        real.patch_task(task, to_beta.clone()).await.unwrap();
        wait_for!(rows_real.epic(EpicId(sub)).is_none());
        mem.patch_task(task, to_beta).await.unwrap();
        assert!(rows_mem.epic(EpicId(sub)).is_none());
        same_task(task.0);
        let beta = rows_real.task(task).unwrap().epic_id.unwrap().0;
        wait_for!(rows_real.epic(EpicId(beta)).is_some());
        same_epic(beta);
        same_epic(root);

        // -- flatten, then regroup -----------------------------------------------
        let ungroup = bindings::EpicPatch {
            group_by_repo: Some(false),
            ..blank_epic_patch()
        };
        real.patch_epic(EpicId(root), ungroup.clone())
            .await
            .unwrap();
        mem.patch_epic(EpicId(root), ungroup).await.unwrap();
        real.flatten_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real.epic(EpicId(beta)).is_none());
        mem.flatten_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        assert!(rows_mem.epic(EpicId(beta)).is_none());
        same_task(task.0);
        same_epic(root);

        let regroup = bindings::EpicPatch {
            group_by_repo: Some(true),
            ..blank_epic_patch()
        };
        real.patch_epic(EpicId(root), regroup.clone())
            .await
            .unwrap();
        mem.patch_epic(EpicId(root), regroup).await.unwrap();
        real.regroup_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.epic_id != Some(EpicId(root))));
        mem.regroup_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        same_task(task.0);
        let regrouped = rows_real.task(task).unwrap().epic_id.unwrap().0;
        wait_for!(rows_real.epic(EpicId(regrouped)).is_some());
        same_epic(regrouped);
        same_epic(root);
    });
}

/// One feed item as the board posts it (`encode::feed_ingest_item`'s shape),
/// sent to BOTH callers.
fn ingest_item(external_id: &str, url: &str, signals: &[&str]) -> bindings::FeedIngestItem {
    bindings::FeedIngestItem {
        item: bindings::FeedTaskUpsertItem {
            external_id: external_id.into(),
            title: format!("title {external_id}"),
            description: String::new(),
            repo_path: String::new(),
            status: "backlog".into(),
            sub_status: "none".into(),
            base_branch: "main".into(),
            tag: "pr-review".into(),
            labels: "[]".into(),
            sort_order: None,
            url: url.into(),
            url_type: if url.is_empty() { "" } else { "pr" }.into(),
            wrap_up_mode: String::new(),
        },
        signals: signals.iter().map(|s| s.to_string()).collect(),
    }
}

/// Where every feed task sits: `(external_id, parent title, own title)`,
/// sorted, read from the board's rows. Ids are left out on purpose: the two
/// sides must agree about the layout, not about which integer each got.
fn feed_layout(rows: &SharedRows, root: EpicId) -> Vec<(String, String, String)> {
    let epics = rows.epics();
    let title_of = |id: EpicId| {
        epics
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.title.clone())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for epic in &epics {
        for task in rows.tasks_for_epic(epic.id) {
            if let Some(ext) = task.external_id {
                let parent = epic.parent_epic_id.map(title_of).unwrap_or_default();
                out.push((ext, parent, epic.title.clone()));
            }
        }
    }
    let _ = root;
    out.sort();
    out
}

/// `feeds.allium: IngestFeedEmission`. The in-memory caller ingests an
/// emission the way the real module does: roles, repo groups, moves, the
/// stale delete, additive mode and the refusals.
#[test]
fn memory_caller_ingests_a_feed_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-ingest");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

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
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone());
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        let identity = accepted.identity.clone();

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

        let parent = bindings::Epic {
            title: "Reviews".into(),
            feed_role: "reviews-parent".into(),
            feed_command: "gh-reviews".into(),
            feed_interval_secs: 60,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root = real.create_epic(parent.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root)).is_some());
        assert_eq!(mem.create_epic(parent).await.unwrap().0, root);

        let pr = |n: u32| format!("https://github.com/org/repo-{n}/pull/{n}");
        let ingest = |items: Vec<bindings::FeedIngestItem>, additive: bool| {
            let real = &real;
            let mem = &mem;
            let identity = identity.clone();
            async move {
                let r = real
                    .ingest_feed(EpicId(root), items.clone(), additive, identity.clone())
                    .await
                    .unwrap();
                let m = mem
                    .ingest_feed(EpicId(root), items, additive, identity)
                    .await
                    .unwrap();
                (r, m)
            }
        };
        let same_layout = |expected: usize| {
            let want = feed_layout(&rows_mem, EpicId(root));
            assert_eq!(want.len(), expected, "{want:?}");
            want
        };

        // -- cycle 1: routing, role sub-epics, an excluded PR ---------------------
        let (r, m) = ingest(
            vec![
                ingest_item("pr-1", &pr(1), &["direct-request"]),
                ingest_item("pr-2", &pr(2), &["team-request"]),
                ingest_item("pr-3", &pr(3), &["author-bot"]),
                ingest_item("pr-4", &pr(4), &["author-me"]),
            ],
            false,
        )
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(3);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        assert_eq!(
            want,
            vec![
                ("pr-1".into(), "Reviews".into(), "My Reviews".into()),
                ("pr-2".into(), "Reviews".into(), "Team Reviews".into()),
                ("pr-3".into(), "Reviews".into(), "Bots".into()),
            ]
        );

        // -- cycle 2: pr-2 is reviewed (a move), pr-3 is gone (a delete) ----------
        let (r, m) = ingest(
            vec![
                ingest_item("pr-1", &pr(1), &["direct-request"]),
                ingest_item("pr-2", &pr(2), &["team-request", "reviewed"]),
            ],
            false,
        )
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(2);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);

        // -- cycle 3: additive removes nothing ------------------------------------
        let (r, m) = ingest(vec![ingest_item("pr-5", &pr(5), &["team-request"])], true).await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(3);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);

        // -- refusals --------------------------------------------------------------
        let (r, m) = ingest(vec![], false).await;
        r.applied().unwrap();
        m.applied().unwrap();
        let role = rows_real
            .epics()
            .into_iter()
            .find(|e| e.feed_role == dispatch_tui::models::FeedRole::Bots)
            .expect("the Bots sub-epic exists")
            .id;
        let on_role = real
            .ingest_feed(role, vec![], false, identity.clone())
            .await
            .unwrap();
        let on_role_mem = mem
            .ingest_feed(role, vec![], false, identity.clone())
            .await
            .unwrap();
        match (on_role, on_role_mem) {
            (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                assert!(
                    r.contains(&m) || m.contains(&r),
                    "real {r:?} vs memory {m:?}"
                );
                assert!(m.contains("reconciled only via its reviews parent"), "{m}");
            }
            other => panic!("both sides must refuse a role sub-epic, got {other:?}"),
        }
        let missing = real
            .ingest_feed(EpicId(9999), vec![], false, identity.clone())
            .await
            .unwrap();
        let missing_mem = mem
            .ingest_feed(EpicId(9999), vec![], false, identity.clone())
            .await
            .unwrap();
        assert!(matches!(missing, ReducerOutcome::Refused(_)), "{missing:?}");
        assert!(
            matches!(missing_mem, ReducerOutcome::Refused(_)),
            "{missing_mem:?}"
        );
    });
}

/// What a dispatch request row looks like once both sides wrote it: everything
/// but the ids and the module-clock timestamps, which differ by construction.
#[derive(Debug, PartialEq)]
struct RequestShape {
    task_id: TaskId,
    host: String,
    requested_by: String,
    mode: dispatch_tui::models::DispatchMode,
    origin: dispatch_tui::models::DispatchOrigin,
    state: dispatch_tui::models::DispatchRequestState,
    reason: Option<String>,
    worktree: Option<String>,
    has_claimed_at: bool,
    has_finished_at: bool,
    deadline_after_requested: chrono::Duration,
}

fn request_shape(r: &dispatch_tui::models::DispatchRequest) -> RequestShape {
    RequestShape {
        task_id: r.task_id,
        host: r.host.clone(),
        requested_by: r.requested_by.clone(),
        mode: r.mode,
        origin: r.origin,
        state: r.state,
        reason: r.reason.clone(),
        worktree: r.worktree.clone(),
        has_claimed_at: r.claimed_at.is_some(),
        has_finished_at: r.finished_at.is_some(),
        deadline_after_requested: r.deadline - r.requested_at,
    }
}

/// store-split M4 (`store-split.allium`: `RequestDispatch` and the rules after
/// it). The in-memory caller serves dispatch requests the way the real module
/// does: the claim-and-row transaction, the derived mode, the host claim, both
/// reports, the conditional release, the restart rule, the task-delete rule,
/// and the refusals — on the same calls, with the same outcome on both sides.
///
/// The module's scheduled backstop (`expire_dispatch_request`) is not driven
/// here: it fires 70s after the request and no client may call it. The double's
/// stand-in is pinned by `src/sync/memory_caller/tests/dispatch_requests.rs`.
#[test]
fn memory_caller_serves_dispatch_requests_like_the_real_module() {
    use dispatch_tui::models::DispatchRequestId;

    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-dispatch-requests");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

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
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone());
        // "host-a" is this board's host: the subscription delivers its
        // dispatch requests (sync.allium: SubscribeToThisHostsDispatchRequests).
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        let identity = accepted.identity.clone();

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
        let same_task = |id: TaskId| {
            assert_eq!(
                rows_real.task(id).map(|t| task_shape(&t)),
                rows_mem.task(id).map(|t| task_shape(&t)),
                "task {id:?}"
            );
        };
        let same_request = |real_id: DispatchRequestId, mem_id: DispatchRequestId| {
            assert_eq!(
                rows_real
                    .dispatch_request(real_id)
                    .map(|r| request_shape(&r)),
                rows_mem.dispatch_request(mem_id).map(|r| request_shape(&r)),
                "request real {real_id:?} / memory {mem_id:?}"
            );
        };
        let applied_id = |outcome: ReducerOutcome| match outcome {
            ReducerOutcome::Applied(ids) if ids.len() == 1 => DispatchRequestId(ids[0]),
            other => panic!("expected one read-back request id, got {other:?}"),
        };
        let both_refuse = |r: ReducerOutcome, m: ReducerOutcome| match (r, m) {
            (ReducerOutcome::Refused(_), ReducerOutcome::Refused(_)) => {}
            other => panic!("both sides must refuse, got {other:?}"),
        };

        let mine = |tag: &str| bindings::Task {
            created_by: identity.clone(),
            owner: identity.clone(),
            tag: tag.into(),
            ..blank_task_in_epic(0)
        };
        let create = |row: bindings::Task| {
            let real = &real;
            let mem = &mem;
            async move {
                let r = real.create_task(row.clone()).await.unwrap();
                let m = mem.create_task(row).await.unwrap();
                assert_eq!(r, m, "both sides hand out the same task id");
                r
            }
        };

        // -- D11: both sides refuse a host the caller does not own ----------------
        // host-a is this connection's own; their-host never got a row, because
        // a NEW host may not be registered ownerless, so nobody may act for it.
        // (Naming another identity as owner is itself refused:
        // `core.allium: LocalHostOwnerIsWrittenOnce`, checked below.)
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        both_refuse(
            real.register_host("their-host".into(), String::new(), String::new())
                .await
                .unwrap(),
            mem.register_host("their-host".into(), String::new(), String::new())
                .await
                .unwrap(),
        );
        // LocalHostOwnerIsWrittenOnce: neither side lets a caller name another
        // identity as owner, nor disown its own host.
        both_refuse(
            real.register_host("gift".into(), String::new(), "beef".into())
                .await
                .unwrap(),
            mem.register_host("gift".into(), String::new(), "beef".into())
                .await
                .unwrap(),
        );
        both_refuse(
            real.register_host("host-a".into(), String::new(), String::new())
                .await
                .unwrap(),
            mem.register_host("host-a".into(), String::new(), String::new())
                .await
                .unwrap(),
        );
        let theirs = create(mine("")).await;
        wait_for!(rows_real.task(theirs).is_some());
        both_refuse(
            real.request_dispatch(theirs, "their-host".into(), "user".into(), String::new())
                .await
                .unwrap(),
            mem.request_dispatch(theirs, "their-host".into(), "user".into(), String::new())
                .await
                .unwrap(),
        );

        // -- RequestDispatch: claim + row, derived mode --------------------------
        let research = create(mine("research")).await;
        wait_for!(rows_real.task(research).is_some());
        let r_req = applied_id(
            real.request_dispatch(research, "host-a".into(), "mcp".into(), String::new())
                .await
                .unwrap(),
        );
        let m_req = applied_id(
            mem.request_dispatch(research, "host-a".into(), "mcp".into(), String::new())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.dispatch_request(r_req).is_some());
        wait_for!(rows_real
            .task(research)
            .is_some_and(|t| t.status == TaskStatus::Running));
        same_request(r_req, m_req);
        same_task(research);
        assert_eq!(
            rows_mem.dispatch_request(m_req).unwrap().mode,
            dispatch_tui::models::DispatchMode::Research
        );

        // A second request while one is open: refused on both sides.
        both_refuse(
            real.request_dispatch(research, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
            mem.request_dispatch(research, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
        );
        // An unknown origin: refused on both sides.
        let spare = create(mine("")).await;
        wait_for!(rows_real.task(spare).is_some());
        both_refuse(
            real.request_dispatch(spare, "host-a".into(), "cron".into(), String::new())
                .await
                .unwrap(),
            mem.request_dispatch(spare, "host-a".into(), "cron".into(), String::new())
                .await
                .unwrap(),
        );

        // -- HostClaimsDispatch --------------------------------------------------
        both_refuse(
            real.claim_dispatch_request(r_req, "host-b".into())
                .await
                .unwrap(),
            mem.claim_dispatch_request(m_req, "host-b".into())
                .await
                .unwrap(),
        );
        assert!(real
            .claim_dispatch_request(r_req, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .claim_dispatch_request(m_req, "host-a".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .dispatch_request(r_req)
            .is_some_and(|r| r.claimed_at.is_some()));
        same_request(r_req, m_req);
        both_refuse(
            real.claim_dispatch_request(r_req, "host-a".into())
                .await
                .unwrap(),
            mem.claim_dispatch_request(m_req, "host-a".into())
                .await
                .unwrap(),
        );

        // -- HostReportsDispatched -----------------------------------------------
        let report = |id| {
            (
                id,
                "host-a".to_string(),
                "/repo/.worktrees/x".to_string(),
                "task-x".to_string(),
            )
        };
        let (id, who, wt, win) = report(r_req);
        assert!(real
            .report_dispatch_provisioned(id, who, wt, win, false)
            .await
            .unwrap()
            .won());
        let (id, who, wt, win) = report(m_req);
        assert!(mem
            .report_dispatch_provisioned(id, who, wt, win, false)
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .task(research)
            .is_some_and(|t| t.worktree.is_some()));
        same_request(r_req, m_req);
        same_task(research);

        // -- HostReportsDispatchFailed + ReleaseClaimOfFailedRequest ---------------
        let failing = create(mine("")).await;
        wait_for!(rows_real.task(failing).is_some());
        let r_fail = applied_id(
            real.request_dispatch(failing, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
        );
        let m_fail = applied_id(
            mem.request_dispatch(failing, "host-a".into(), "user".into(), "dispatch".into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.dispatch_request(r_fail).is_some());
        real.claim_dispatch_request(r_fail, "host-a".into())
            .await
            .unwrap();
        mem.claim_dispatch_request(m_fail, "host-a".into())
            .await
            .unwrap();
        assert!(real
            .report_dispatch_failed(r_fail, "host-a".into(), "worktree add failed".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_dispatch_failed(m_fail, "host-a".into(), "worktree add failed".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .task(failing)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        same_request(r_fail, m_fail);
        same_task(failing);

        // -- HostRestartFailsInterrupted ------------------------------------------
        let interrupted = create(mine("")).await;
        let waiting = create(mine("")).await;
        wait_for!(rows_real.task(waiting).is_some());
        let mut pairs = Vec::new();
        for task in [interrupted, waiting] {
            let r = applied_id(
                real.request_dispatch(task, "host-a".into(), "user".into(), "dispatch".into())
                    .await
                    .unwrap(),
            );
            let m = applied_id(
                mem.request_dispatch(task, "host-a".into(), "user".into(), "dispatch".into())
                    .await
                    .unwrap(),
            );
            wait_for!(rows_real.dispatch_request(r).is_some());
            pairs.push((r, m));
        }
        real.claim_dispatch_request(pairs[0].0, "host-a".into())
            .await
            .unwrap();
        mem.claim_dispatch_request(pairs[0].1, "host-a".into())
            .await
            .unwrap();
        real.fail_interrupted_dispatch_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.fail_interrupted_dispatch_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(interrupted)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        for (r, m) in &pairs {
            same_request(*r, *m);
        }
        same_task(interrupted);
        same_task(waiting);

        // -- DeletingATaskFailsItsOpenRequest -------------------------------------
        // Only a done task may be deleted (`tasks.allium: DeleteTask`): moved
        // to done by hand while its request is still open.
        let done = || bindings::TaskPatch {
            status: Some("done".into()),
            sub_status: Some("none".into()),
            ..blank_task_patch()
        };
        real.patch_task(waiting, done())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.patch_task(waiting, done())
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(waiting)
            .is_some_and(|t| t.status == TaskStatus::Done));
        real.delete_task(waiting).await.unwrap().applied().unwrap();
        mem.delete_task(waiting).await.unwrap().applied().unwrap();
        wait_for!(rows_real
            .dispatch_request(pairs[1].0)
            .is_some_and(|r| r.reason.is_some()));
        same_request(pairs[1].0, pairs[1].1);
        assert_eq!(
            rows_mem
                .dispatch_request(pairs[1].1)
                .unwrap()
                .reason
                .as_deref(),
            Some("task deleted")
        );
    });
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

/// store-split M5 (`store-split.allium`: `RequestWrapUp` and the rules after
/// it). The in-memory caller serves wrap-up and exit-session requests the way
/// the real module does: the request and its refusals, the claim that clears a
/// Conflict, both reports with the module setting the conflict, the token
/// check, the close (terminal write, epic rollup) and the epic chain in one
/// transaction, the restart rule and the task-delete rule — on the same
/// calls, with the same outcome on both sides.
///
/// The module's scheduled backstop (`expire_session_request`) is not driven
/// here, as for M4's: no client may call it. The double's stand-in is pinned
/// by `src/sync/memory_caller/tests/session_requests.rs`.
#[test]
fn memory_caller_serves_session_requests_like_the_real_module() {
    use dispatch_tui::models::SessionRequestId;

    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-session-requests");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

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
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone());
        // sync.allium: SubscribeToThisHostsSessionRequests — and the dispatch
        // requests the chain writes, which the same host asks for.
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        let identity = accepted.identity.clone();

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
        let same_task = |id: TaskId| {
            assert_eq!(
                rows_real.task(id).map(|t| task_shape(&t)),
                rows_mem.task(id).map(|t| task_shape(&t)),
                "task {id:?}"
            );
        };
        let same_session = |real_id: SessionRequestId, mem_id: SessionRequestId| {
            assert_eq!(
                rows_real
                    .session_request(real_id)
                    .map(|r| session_shape(&r)),
                rows_mem.session_request(mem_id).map(|r| session_shape(&r)),
                "session request real {real_id:?} / memory {mem_id:?}"
            );
        };
        let applied_id = |outcome: ReducerOutcome| match outcome {
            ReducerOutcome::Applied(ids) if ids.len() == 1 => SessionRequestId(ids[0]),
            other => panic!("expected one read-back request id, got {other:?}"),
        };
        let both_refuse = |r: ReducerOutcome, m: ReducerOutcome| match (r, m) {
            (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                assert_eq!(r, m, "both sides must refuse in the same words");
            }
            other => panic!("both sides must refuse, got {other:?}"),
        };

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let create = |row: bindings::Task| {
            let real = &real;
            let mem = &mem;
            async move {
                let r = real.create_task(row.clone()).await.unwrap();
                let m = mem.create_task(row).await.unwrap();
                assert_eq!(r, m, "both sides hand out the same task id");
                r
            }
        };
        let mine = |epic_id: i64, tag: &str| bindings::Task {
            created_by: identity.clone(),
            // A task in an epic carries no owner (`OwnerTracksUserBoardTask`).
            owner: if epic_id == 0 {
                identity.clone()
            } else {
                String::new()
            },
            tag: tag.into(),
            ..blank_task_in_epic(epic_id)
        };
        // A task running on host-a: the state every wrap-up starts from. Patched
        // in directly, not dispatched: a dispatch finishing in the same
        // millisecond as the wrap-up would void the token (W3) on one side and
        // not the other.
        let dispatched = |task: TaskId| {
            let real = &real;
            let mem = &mem;
            async move {
                for caller in [real as &dyn ReducerCaller, mem as &dyn ReducerCaller] {
                    caller
                        .patch_task(
                            task,
                            bindings::TaskPatch {
                                status: Some("running".into()),
                                sub_status: Some("active".into()),
                                worktree: Some(format!("/repo/.worktrees/{}-t", task.0)),
                                tmux_window: Some(format!("task-{}", task.0)),
                                host: Some("host-a".into()),
                                ..blank_task_patch()
                            },
                        )
                        .await
                        .unwrap()
                        .applied()
                        .unwrap();
                }
            }
        };

        // -- RequestWrapUp: refusals first --------------------------------------
        let backlog = create(mine(0, "")).await;
        wait_for!(rows_real.task(backlog).is_some());
        both_refuse(
            real.request_wrap_up(backlog, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
            mem.request_wrap_up(backlog, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let reviewer = create(mine(0, "pr-review")).await;
        wait_for!(rows_real.task(reviewer).is_some());
        dispatched(reviewer).await;
        wait_for!(rows_real
            .task(reviewer)
            .is_some_and(|t| t.worktree.is_some()));
        both_refuse(
            real.request_wrap_up(reviewer, "host-a".into(), "done".into())
                .await
                .unwrap(),
            mem.request_wrap_up(reviewer, "host-a".into(), "done".into())
                .await
                .unwrap(),
        );
        both_refuse(
            real.request_wrap_up(reviewer, "host-a".into(), "squash".into())
                .await
                .unwrap(),
            mem.request_wrap_up(reviewer, "host-a".into(), "squash".into())
                .await
                .unwrap(),
        );

        // -- A rebase: request, claim, conflict ----------------------------------
        let epic = {
            let row = bindings::Epic {
                created_by: identity.clone(),
                auto_dispatch: true,
                ..blank_epic()
            };
            let r = real.create_epic(row.clone()).await.unwrap();
            let m = mem.create_epic(row).await.unwrap();
            assert_eq!(r, m);
            r
        };
        let task = create(mine(epic.0, "")).await;
        let next = create(mine(epic.0, "")).await;
        wait_for!(rows_real.task(next).is_some());
        dispatched(task).await;
        wait_for!(rows_real.task(task).is_some_and(|t| t.worktree.is_some()));
        let r_wrap = applied_id(
            real.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_wrap = applied_id(
            mem.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.session_request(r_wrap).is_some());
        same_session(r_wrap, m_wrap);
        same_task(task);
        both_refuse(
            real.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
            mem.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        // A claim from the wrong host is refused, the right one wins once.
        assert!(!real
            .claim_session_request(r_wrap, "host-b".into())
            .await
            .unwrap()
            .won());
        assert!(real
            .claim_session_request(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .claim_session_request(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .session_request(r_wrap)
            .is_some_and(|r| r.claimed_at.is_some()));
        same_session(r_wrap, m_wrap);
        assert!(!real
            .claim_session_request(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(!mem
            .claim_session_request(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        // A conflict report: the MODULE sets the sub_status.
        assert!(real
            .report_session_request_failed(
                r_wrap,
                "host-a".into(),
                "Rebase conflict on b in a.rs".into(),
                true
            )
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_failed(
                m_wrap,
                "host-a".into(),
                "Rebase conflict on b in a.rs".into(),
                true
            )
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::Conflict));
        same_session(r_wrap, m_wrap);
        same_task(task);

        // -- A second rebase succeeds: the claim cleared the Conflict ---------------
        let r_wrap = applied_id(
            real.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_wrap = applied_id(
            mem.request_wrap_up(task, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        real.claim_session_request(r_wrap, "host-a".into())
            .await
            .unwrap();
        mem.claim_session_request(m_wrap, "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status != dispatch_tui::models::SubStatus::Conflict));
        same_task(task);
        assert!(real
            .report_session_request_done(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_done(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        // A repeated done report is accepted on both sides.
        assert!(real
            .report_session_request_done(r_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_done(m_wrap, "host-a".into())
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .session_request(r_wrap)
            .is_some_and(|r| r.finished_at.is_some()));
        same_session(r_wrap, m_wrap);

        // -- RequestExitSession: refusals leave the token valid -----------------------
        both_refuse(
            real.request_exit_session(
                task,
                SessionRequestId(r_wrap.0 + 50),
                "rebase".into(),
                String::new(),
            )
            .await
            .unwrap(),
            mem.request_exit_session(
                task,
                SessionRequestId(m_wrap.0 + 50),
                "rebase".into(),
                String::new(),
            )
            .await
            .unwrap(),
        );
        both_refuse(
            real.request_exit_session(task, r_wrap, "pr".into(), String::new())
                .await
                .unwrap(),
            mem.request_exit_session(task, m_wrap, "pr".into(), String::new())
                .await
                .unwrap(),
        );
        same_task(task);

        // -- The close, the token and the chain in one transaction ---------------------
        let r_exit = applied_id(
            real.request_exit_session(task, r_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
        );
        let m_exit = applied_id(
            mem.request_exit_session(task, m_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
        );
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.status == TaskStatus::Done));
        wait_for!(rows_real.session_request(r_exit).is_some());
        wait_for!(rows_real
            .task(next)
            .is_some_and(|t| t.status == TaskStatus::Running));
        same_session(r_exit, m_exit);
        same_task(task);
        same_task(next);
        assert!(rows_real.session_request(r_exit).unwrap().chained_task == Some(next));
        assert!(rows_mem.session_request(m_exit).unwrap().chained_task == Some(next));
        assert_eq!(
            rows_real.epic(epic).map(|e| epic_shape(&e)),
            rows_mem.epic(epic).map(|e| epic_shape(&e)),
        );
        // The token is spent.
        both_refuse(
            real.request_exit_session(task, r_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
            mem.request_exit_session(task, m_wrap, "rebase".into(), String::new())
                .await
                .unwrap(),
        );

        // -- The teardown reports, and a failed one does not revert the close ---------------
        assert!(real
            .claim_session_request(r_exit, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(mem
            .claim_session_request(m_exit, "host-a".into())
            .await
            .unwrap()
            .won());
        assert!(real
            .report_session_request_failed(
                r_exit,
                "host-a".into(),
                "tmux kill-window failed".into(),
                false
            )
            .await
            .unwrap()
            .won());
        assert!(mem
            .report_session_request_failed(
                m_exit,
                "host-a".into(),
                "tmux kill-window failed".into(),
                false
            )
            .await
            .unwrap()
            .won());
        wait_for!(rows_real
            .session_request(r_exit)
            .is_some_and(|r| r.reason.is_some()));
        same_session(r_exit, m_exit);
        same_task(task);

        // -- A pr close ----------------------------------------------------------------------
        let pr_task = create(mine(0, "")).await;
        wait_for!(rows_real.task(pr_task).is_some());
        dispatched(pr_task).await;
        let r_pr = applied_id(
            real.request_wrap_up(pr_task, "host-a".into(), "pr".into())
                .await
                .unwrap(),
        );
        let m_pr = applied_id(
            mem.request_wrap_up(pr_task, "host-a".into(), "pr".into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real.session_request(r_pr).is_some());
        same_session(r_pr, m_pr);
        let url = "https://github.com/o/r/pull/9";
        both_refuse(
            real.request_exit_session(pr_task, r_pr, "pr".into(), String::new())
                .await
                .unwrap(),
            mem.request_exit_session(pr_task, m_pr, "pr".into(), String::new())
                .await
                .unwrap(),
        );
        let r_close = applied_id(
            real.request_exit_session(pr_task, r_pr, "pr".into(), url.into())
                .await
                .unwrap(),
        );
        let m_close = applied_id(
            mem.request_exit_session(pr_task, m_pr, "pr".into(), url.into())
                .await
                .unwrap(),
        );
        wait_for!(rows_real
            .task(pr_task)
            .is_some_and(|t| t.status == TaskStatus::Review));
        wait_for!(rows_real.session_request(r_close).is_some());
        same_session(r_close, m_close);
        same_task(pr_task);

        // -- HostRestartFailsInterruptedSessionRequests --------------------------------------
        let running = create(mine(0, "")).await;
        wait_for!(rows_real.task(running).is_some());
        dispatched(running).await;
        let r_run = applied_id(
            real.request_wrap_up(running, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_run = applied_id(
            mem.request_wrap_up(running, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        real.claim_session_request(r_run, "host-a".into())
            .await
            .unwrap();
        mem.claim_session_request(m_run, "host-a".into())
            .await
            .unwrap();
        real.fail_interrupted_session_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.fail_interrupted_session_requests("host-a".into())
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .session_request(r_run)
            .is_some_and(|r| r.reason.is_some()));
        same_session(r_run, m_run);
        assert_eq!(
            rows_mem.session_request(m_run).unwrap().reason.as_deref(),
            Some("host restarted")
        );

        // -- DeletingATaskFailsItsOpenSessionRequest -----------------------------------------
        let doomed = create(mine(0, "")).await;
        wait_for!(rows_real.task(doomed).is_some());
        dispatched(doomed).await;
        let r_doom = applied_id(
            real.request_wrap_up(doomed, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        let m_doom = applied_id(
            mem.request_wrap_up(doomed, "host-a".into(), "rebase".into())
                .await
                .unwrap(),
        );
        // A task is deleted only from Done (`tasks.allium: DeleteTask`).
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .patch_task(
                    doomed,
                    bindings::TaskPatch {
                        status: Some("done".into()),
                        sub_status: Some("none".into()),
                        ..blank_task_patch()
                    },
                )
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        real.delete_task(doomed).await.unwrap().applied().unwrap();
        mem.delete_task(doomed).await.unwrap().applied().unwrap();
        wait_for!(rows_real
            .session_request(r_doom)
            .is_some_and(|r| r.reason.is_some()));
        same_session(r_doom, m_doom);
        assert_eq!(
            rows_mem.session_request(m_doom).unwrap().reason.as_deref(),
            Some("task deleted")
        );
    });
}

/// store-split M6 (`store-split.allium`: `ReportPrStatus`,
/// `ReportPrUnreadable`). The in-memory caller derives the task's PR state from
/// the host's raw `gh` strings the way the real module does: merged, closed,
/// open with a review decision, a conflict left alone, an unknown string
/// refused in the same words and the unreachable mark — on the same calls,
/// with the same outcome and the same task on both sides.
#[test]
fn memory_caller_derives_pr_state_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-pr-state");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

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
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone());
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        let identity = accepted.identity.clone();

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
        let same_task = |id: TaskId| {
            assert_eq!(
                rows_real.task(id).map(|t| task_shape(&t)),
                rows_mem.task(id).map(|t| task_shape(&t)),
                "task {id:?}"
            );
        };

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let create = |sub_status: &str| {
            let row = bindings::Task {
                created_by: identity.clone(),
                owner: identity.clone(),
                status: "review".into(),
                sub_status: sub_status.into(),
                url: "https://github.com/acme/repo/pull/7".into(),
                url_type: "pr".into(),
                tmux_window: "task-1".into(),
                ..blank_task_in_epic(0)
            };
            let real = &real;
            let mem = &mem;
            async move {
                let r = real.create_task(row.clone()).await.unwrap();
                let m = mem.create_task(row).await.unwrap();
                assert_eq!(r, m, "both sides hand out the same task id");
                r
            }
        };
        // Both sides, one report, one outcome.
        let report = |task: TaskId, state: &'static str, decision: &'static str| {
            let real = &real;
            let mem = &mem;
            async move {
                let r = real
                    .report_pr_status(task, "host-a".into(), state.into(), decision.into())
                    .await
                    .unwrap();
                let m = mem
                    .report_pr_status(task, "host-a".into(), state.into(), decision.into())
                    .await
                    .unwrap();
                match (r, m) {
                    (ReducerOutcome::Applied(_), ReducerOutcome::Applied(_)) => {}
                    (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                        assert_eq!(r, m, "both sides must refuse in the same words")
                    }
                    other => panic!("the two sides disagree: {other:?}"),
                }
            }
        };

        let task = create("awaiting_review").await;
        wait_for!(rows_real.task(task).is_some());
        same_task(task);

        report(task, "OPEN", "APPROVED").await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::Approved));
        same_task(task);

        // An unknown string is refused in the same words and writes nothing.
        report(task, "DRAFT", "").await;
        report(task, "OPEN", "MAYBE").await;
        same_task(task);

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            assert!(caller
                .report_pr_unreadable(task, "host-a".into(), "no access".into())
                .await
                .unwrap()
                .won());
        }
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::PrUnreachable));
        same_task(task);

        report(task, "CLOSED", "").await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.sub_status == dispatch_tui::models::SubStatus::PrClosed));
        same_task(task);

        report(task, "merged", "").await;
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.status == dispatch_tui::models::TaskStatus::Done));
        same_task(task);

        // A conflict survives an open report; a closed one overrides it.
        let conflicted = create("conflict").await;
        wait_for!(rows_real.task(conflicted).is_some());
        report(conflicted, "OPEN", "APPROVED").await;
        same_task(conflicted);
        assert_eq!(
            rows_real.task(conflicted).unwrap().sub_status,
            dispatch_tui::models::SubStatus::Conflict
        );
    });
}
